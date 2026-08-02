//! The writer of `pricing_audit_log` — one link of a `(tenant_id, chain_id)`
//! segmented hash chain, appended **inside the caller's transaction**.
//!
//! This is the table's first writer. It lands with the publish path because
//! that is the first path that has an actor, a subject and a transaction to put
//! a record inside of; a repository with no caller is dead code, and the
//! evidence a record is supposed to be depends entirely on *which* transaction
//! it commits in.
//!
//! **Stateless, and it takes a runner rather than a provider.** D-14 requires
//! the audit row to commit inside the mutation's own ACID transaction: that is
//! what makes a crash unable to lose a record, and what makes an unavailable
//! audit store unable to exist separately from an unavailable database. A
//! repository that opened a transaction of its own would break that by
//! construction — the mutation could roll back with the record already
//! committed, or the record could fail with the mutation already durable, and
//! both produce a trail that disagrees with the truth tables. The
//! [`IdempotencyGate`](super::idempotency_repo::IdempotencyGate) takes a
//! transaction handle for the same reason.
//!
//! # CONTRACT — concurrency, and what actually serializes this chain
//!
//! **A fork is unrepresentable here, at any isolation level.** The segment's
//! head is `MAX(seq)` in the chain itself — there is no separate tip row — and
//! the primary key is `(tenant_id, chain_id, seq)`. Two writers that read the
//! same head and both insert at `seq + 1` therefore cannot both succeed: one
//! takes a unique violation, **its whole mutation transaction rolls back**, and
//! a retry re-reads a head that has moved. So the linearity of the chain is a
//! property of the key rather than of an isolation level, and what is at risk
//! is **liveness, not integrity**.
//!
//! That is a stronger property than the sibling ledger's chain has, and the
//! difference is worth stating because the ledger's CONTRACT reads the other
//! way. `ledger/src/infra/posting/chain.rs` keeps its tip in a separate
//! `chain_state` row and reads it locklessly, so two concurrent seals under a
//! weak isolation level would both read the same tip and both link the same
//! `prev_hash` — a genuine fork — which is why that file requires
//! `SERIALIZABLE`. **This repository does not**, and copying that contract here
//! without its premise would be asserting a requirement that nothing in this
//! design has.
//!
//! **The contention refusal is not distinguishable, and that is a gap rather
//! than a decision.** A unique violation on this insert is a contention signal,
//! not a corrupt row and not a caller mistake — but the design set names no wire
//! code for "audit chain contention", and this crate does not mint codes it has
//! not been given (`domain/rules.rs`'s absence discipline, applied to the error
//! ladder). So the violation arrives as [`RepoError::Db`] and lands on
//! `DomainError::Internal`, which is indistinguishable from a dead connection.
//! The consequence, stated so it is not mistaken for a design: **the retry
//! decision sits entirely with the caller's transaction retry**, and a
//! distinguishable variant is owed to whichever group is given a code for it.
//!
//! # What is owed here, and of what kind
//!
//! This crate has **no testcontainers suite**, and `sqlite::memory:` serializes
//! writers by construction. The `SQLite` suite proves that the chain *links*,
//! that `seq` starts at 0 per segment, that segments of one tenant are
//! independent in content, that the append-only trigger rejects UPDATE and
//! DELETE, and that a record whose content moved no longer reproduces its
//! digest. Beyond that:
//!
//! 1. **Unprovable here.** Two concurrent mutations of **different**
//!    `chain_id`s of one tenant do not contend — the entire benefit D-135
//!    bought, and an `05-governance.md` §9 integration acceptance criterion.
//!    Fairly owed to a Postgres suite, whose template is
//!    `gears/bss/ledger/ledger/tests/postgres_chain.rs`
//!    (`concurrent_posts_form_linear_chain`).
//! 2. **Undemonstrated, not unproven.** Two mutations of the **same** aggregate
//!    serialize rather than fork. The key half is proved by
//!    `tests/sqlite_audit_chain.rs` and the rollback half is `in_transaction`'s
//!    contract; what Postgres adds is the concurrent demonstration.
//! 3. **Unimplemented, owed to a decision.** The loser of a same-segment race
//!    surfacing as a retriable contention. It cannot today, for want of a code —
//!    see the paragraph above. A suite would not change that; a code would.
//!
//! A property that genuinely cannot be checked here is recorded as such; one
//! that *can* be checked and has not been is recorded as undemonstrated, and one
//! that is simply not built is recorded as unimplemented. Filing all three as
//! "owed to Postgres" is how a checkable argument becomes a debt nobody
//! collects.
//!
//! # What is deliberately absent
//!
//! - **The roll-up writer** (`entry_kind = 'rollup'`, `segment_heads`). It is
//!   periodic and off the mutation path by definition, and
//!   `chk_pricing_audit_log_rollup` already makes a mutation row carrying heads
//!   impossible — so nothing this module writes can be mistaken for one.
//! - **The verification job** (`pricing_audit_chain_verified`). Same reason.
//! - **Every read surface.** The Auditor read API, the D-125 cursor walk and
//!   the retention sweep have no caller in this group. A read method nothing
//!   calls would be a shape fixed before its reader exists.

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use serde_json::Value as JsonValue;
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

use crate::domain::audit::{
    AuditAction, AuditRecord, AuditSubjectKind, audit_row_hash, genesis_prev_hash,
};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::audit_log;

/// The `entry_kind` of everything this module writes.
///
/// Spelled once. `chk_pricing_audit_log_entry_kind` pins the same two tokens and
/// `chk_pricing_audit_log_rollup` ties this one to a NULL `segment_heads`, so a
/// second spelling would be caught by the database — as a driver error inside a
/// publish transaction, which is not where a table's vocabulary should be
/// discovered.
const ENTRY_KIND_MUTATION: &str = "mutation";

/// One audit record, as its writer is handed it.
///
/// `seq`, `prev_hash` and `row_hash` are deliberately **not** here: they are
/// the chain's, computed from the segment this record is about to extend, and a
/// caller that could supply them could link a row wherever it liked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewAuditEntry {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The audited subject's aggregate — which segment this record extends.
    pub chain_id: Uuid,
    /// When the mutation happened, UTC. The caller's instant, not the
    /// database's: the catalog mutates state only in response to an explicit
    /// authoring call (§2.2), so the record's timestamp belongs to the request
    /// rather than to whichever node evaluated `now()`.
    pub recorded_at: DateTime<Utc>,
    /// **Pseudonymous** principal id of the acting operator (`inst-au-pii`).
    /// The column is `uuid` precisely so that constraint is physical.
    pub actor_principal_id: Uuid,
    /// What was done.
    pub action: AuditAction,
    /// What kind of thing it was done to.
    pub subject_kind: AuditSubjectKind,
    /// Which one.
    pub subject_ref: String,
    /// The subject's state before the mutation.
    pub before_state: Option<JsonValue>,
    /// The subject's state after it.
    pub after_state: Option<JsonValue>,
    /// The approval record the mutation ran under, when it had one.
    pub approval_ref: Option<Uuid>,
    /// The correlation id of the causing request.
    pub correlation_id: Option<Uuid>,
}

/// Append one record to its segment, inside `runner`'s transaction.
///
/// Reads the segment's greatest `seq` and its `row_hash`, links the new record
/// to it — or to [`genesis_prev_hash`] when the segment is empty — hashes the
/// record over the canonical encoding, and inserts at `seq + 1` with
/// `entry_kind = 'mutation'` and `segment_heads` NULL. Returns the `seq` the
/// row landed at.
///
/// The head is taken as `MAX(seq)` over the **whole** segment rather than over
/// its mutation rows: the head is what a verifier's re-walk arrives at, and a
/// definition that skipped a row would produce a chain whose links this writer
/// and that walker disagree about.
///
/// A free function rather than a method on a struct with no fields: there is no
/// state to hold, and a `self` that exists only to be a namespace invites
/// somebody to give it a `DBProvider` — which is exactly the thing the CONTRACT
/// above forbids.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure — which **includes losing a
/// same-segment race**, since the primary key is what decides it and no wire
/// code exists for that outcome (see the module CONTRACT).
/// [`RepoError::CorruptRow`] when the segment head's `row_hash` is not 32 bytes,
/// when its `seq` is not a position this chain can count in or has no
/// successor, or when the record's before/after state cannot be canonicalized —
/// the last is unreachable for an in-memory value and is propagated rather than
/// swallowed, because a silent fallback would make an un-canonicalizable record
/// hash identically to one with no state at all.
pub async fn append(
    runner: &impl DBRunner,
    scope: &AccessScope,
    entry: NewAuditEntry,
) -> Result<u64, RepoError> {
    let head = read_head(runner, scope, entry.tenant_id, entry.chain_id).await?;
    let (seq, prev_hash) = match head {
        None => (0_u64, genesis_prev_hash(entry.tenant_id, entry.chain_id)),
        Some(head) => (next_seq(&head)?, head_hash(&head)?),
    };

    let record = AuditRecord {
        tenant_id: entry.tenant_id,
        chain_id: entry.chain_id,
        seq,
        recorded_at: entry.recorded_at,
        actor_principal_id: entry.actor_principal_id,
        action: entry.action,
        subject_kind: entry.subject_kind,
        subject_ref: &entry.subject_ref,
        before_state: entry.before_state.as_ref(),
        after_state: entry.after_state.as_ref(),
        approval_ref: entry.approval_ref,
        correlation_id: entry.correlation_id,
    };
    let row_hash = audit_row_hash(&record, &prev_hash).map_err(|e| {
        RepoError::CorruptRow(format!(
            "audit record {} of chain {} cannot be canonicalized: {e}",
            entry.subject_ref, entry.chain_id
        ))
    })?;
    let stored_seq = i64::try_from(seq).map_err(|_| {
        RepoError::CorruptRow(format!(
            "audit chain {} reached seq {seq}, which its column cannot hold",
            entry.chain_id
        ))
    })?;

    let am = audit_log::ActiveModel {
        tenant_id: Set(entry.tenant_id),
        chain_id: Set(entry.chain_id),
        seq: Set(stored_seq),
        entry_kind: Set(ENTRY_KIND_MUTATION.to_owned()),
        recorded_at: Set(entry.recorded_at),
        actor_principal_id: Set(entry.actor_principal_id),
        action: Set(entry.action.as_str().to_owned()),
        subject_kind: Set(entry.subject_kind.as_str().to_owned()),
        subject_ref: Set(entry.subject_ref.clone()),
        before_state: Set(entry.before_state.clone()),
        after_state: Set(entry.after_state.clone()),
        approval_ref: Set(entry.approval_ref),
        correlation_id: Set(entry.correlation_id),
        // NULL, and the CHECK requires it of a mutation row: a roll-up is the
        // only row that chains segment heads, and it is not written here.
        segment_heads: Set(None),
        prev_hash: Set(Some(prev_hash.to_vec())),
        row_hash: Set(row_hash.to_vec()),
    };
    audit_log::Entity::insert(am.clone())
        .secure()
        .scope_with_model(scope, &am)
        .map_err(|e| RepoError::Db(format!("pricing_audit_log scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("append pricing_audit_log: {e}")))?;
    Ok(seq)
}

/// The segment's greatest row, scoped. `None` is an empty segment.
async fn read_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    chain_id: Uuid,
) -> Result<Option<audit_log::Model>, RepoError> {
    audit_log::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(audit_log::Column::TenantId.eq(tenant_id))
                .add(audit_log::Column::ChainId.eq(chain_id)),
        )
        .order_by(audit_log::Column::Seq, Order::Desc)
        .limit(1)
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read audit chain head: {e}")))
}

/// The position after the head.
fn next_seq(head: &audit_log::Model) -> Result<u64, RepoError> {
    let current = u64::try_from(head.seq).map_err(|e| {
        RepoError::CorruptRow(format!(
            "pricing_audit_log chain {} holds seq {}: {e}",
            head.chain_id, head.seq
        ))
    })?;
    current.checked_add(1).ok_or_else(|| {
        RepoError::CorruptRow(format!(
            "pricing_audit_log chain {} stands at seq {current}, which has no successor",
            head.chain_id
        ))
    })
}

/// The head's `row_hash`, as the link the next record hashes against.
///
/// A stored hash that is not 32 bytes is an invariant breach rather than a
/// caller mistake: the column is written only here and the table refuses every
/// UPDATE, so a short hash means something reached the table around this gear.
/// It is refused rather than padded, because a padded link is a link the
/// verification job will report as a break with no way back to what was hashed.
fn head_hash(head: &audit_log::Model) -> Result<[u8; 32], RepoError> {
    <[u8; 32]>::try_from(head.row_hash.as_slice()).map_err(|_| {
        RepoError::CorruptRow(format!(
            "pricing_audit_log chain {} seq {} holds a {}-byte row_hash",
            head.chain_id,
            head.seq,
            head.row_hash.len()
        ))
    })
}
