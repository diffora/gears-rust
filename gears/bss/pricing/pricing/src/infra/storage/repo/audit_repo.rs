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
//! **The contention refusal is distinguishable as of D-159.** A unique violation
//! on this insert is a contention signal, not a corrupt row and not a caller
//! mistake, and it now arrives as [`RepoError::ConcurrentMutation`] naming the
//! chain — `CONCURRENT_MUTATION`, **409**, on the wire. An earlier version of this
//! paragraph recorded the absence and its reason correctly: the design set named
//! no code, and this crate does not mint codes it has not been given. §3.3 names
//! one now, so the loser of a same-segment race is told to retry rather than told
//! the store failed. What is recognised is the driver's **own**
//! unique-violation class (`crate::infra::storage::contention_or_db`), so the
//! predicate is one sentence over both backends.
//!
//! # What is owed here, and of what kind
//!
//! **Three of the four items below were paid on 2026-08-04** by
//! `tests/postgres_audit_chain.rs`, and the paragraph that opened this section —
//! "this crate has **no** testcontainers suite" — stopped being true when that
//! suite landed. `sqlite::memory:` still serializes writers by construction, so
//! what follows remains an accurate account of what the *mirror* can and cannot
//! say; it is no longer an account of what is unproven.
//!
//! The `SQLite` suite proves that the chain *links*,
//! that `seq` starts at 0 per segment, that segments of one tenant are
//! independent in content, that the append-only trigger rejects UPDATE and
//! DELETE, and that a record whose content moved no longer reproduces its
//! digest. Beyond that:
//!
//! 1. ~~**Unprovable here.**~~ **PAID 2026-08-04.** Two concurrent mutations of
//!    **different** `chain_id`s of one tenant do not contend — the entire
//!    benefit D-135 bought, and an `05-governance.md` §9 integration acceptance
//!    criterion. `postgres_audit_chain.rs::two_aggregates_of_one_tenant_do_not_contend_on_a_chain_head`
//!    parks one transaction open on chain A and requires the other to complete
//!    on chain B while it is parked. Collapsing the key to `(tenant_id, seq)` —
//!    the pre-segmentation shape — makes it time out, so it is a fact about
//!    segmentation rather than a harness too gentle to provoke anything.
//! 2. ~~**Undemonstrated, not unproven.**~~ **PAID 2026-08-04.** Two mutations of
//!    the **same** aggregate serialize rather than fork, demonstrated
//!    concurrently and deterministically: a third connection polls
//!    `pg_locks WHERE NOT granted` until the loser is provably in a lock wait —
//!    which is what proves its head read already happened — and only then is the
//!    winner released.
//! 3. ~~**Implemented, and half of it unproven here.**~~ **PAID 2026-08-04.** The
//!    loser of a same-segment race surfaces as a retriable contention (D-159):
//!    `RepoError::ConcurrentMutation` → `DomainError::ConcurrentMutation` → 409
//!    `CONCURRENT_MUTATION`, the whole ladder asserted in one test. The
//!    whole-transaction rollback is proved by a **witness** rather than by
//!    inference — the loser writes to a second, uncontended segment before it
//!    collides, and that segment holds zero rows afterwards.
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
use crate::domain::scope_key::PlanId;
use crate::infra::storage::entity::audit_log;
use crate::infra::storage::{RepoError, contention_or_db};

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
    /// The correlation id of the causing request (D-178).
    ///
    /// Not an `Option`: the column is nullable so a verifier can re-walk a row
    /// written before D-178, and **this writer cannot produce one**. See
    /// [`AuditStamp`](crate::domain::audit::AuditStamp)'s own field for the
    /// argument.
    pub correlation_id: Uuid,
}

/// The segment every audited mutation **of one plan** extends.
///
/// D-135 keys the chain on the audited subject's *aggregate*, and a price row's
/// aggregate is the plan it prices — `pricing_price` is attached to `plan_id`.
/// So a plan's authoring history, its price rows' edits and its publish commits
/// are one segment, which is what makes "who changed this plan" a walk rather
/// than a join, and what keeps concurrent mutations of *different* plans off one
/// chain head.
///
/// Written down here rather than at the six call sites: a second derivation is a
/// second answer to which segment a record belongs in, and a record on the wrong
/// segment verifies perfectly while telling an auditor nothing.
#[must_use]
pub const fn plan_chain(plan_id: PlanId) -> Uuid {
    plan_id.get()
}

/// The segment every audited act on the tenant's **approval-threshold policy**
/// extends.
///
/// A constant, and the constant is the whole design. D-135 keys a chain on the
/// audited subject's *aggregate*; S5 §6's aggregate list names `policy` as one in
/// its own right, and a tenant has exactly one threshold policy, so the aggregate
/// id would be a value with a single inhabitant. The chain key is
/// `(tenant_id, chain_id)` — the tenant axis is already there — which makes a
/// per-tenant singleton segment exactly "one constant `chain_id`".
///
/// **It cannot collide with a [`plan_chain`].** That one is `plan_id.get()`, and
/// every plan id this gear mints is a `UUIDv7` — `api::rest::plans`' create handler
/// is the only minter, at `PlanId::new(Uuid::now_v7())`. This constant is
/// `00000000-0000-8000-8000-70726963696e`, version nibble `8`, which no v7 value
/// holds, so the two chain spaces are disjoint by construction rather than by
/// improbability — and a collision would be the one defect that is invisible: two
/// aggregates interleaved on one hash chain verify perfectly.
#[must_use]
pub const fn policy_chain() -> Uuid {
    Uuid::from_u128(0x0000_0000_0000_8000_8000_7072_6963_696e_u128)
}

/// The segment every audited mutation **of one price overlay** extends.
///
/// D-135 keys a chain on the audited subject's aggregate, and S5 §6's aggregate list
/// — plan, **overlay**, payer, policy, bulk operation — makes an overlay one in its
/// own right. It has to be: an overlay is not a plan and has no plan, so putting its
/// mutations on [`plan_chain`] would interleave two aggregates on one segment and
/// make "who changed this plan" answer about an object the plan does not contain.
///
/// # It is not the overlay id, and the four bits are the whole reason
///
/// [`plan_chain`] is `plan_id.get()` verbatim, and a plan id and an overlay id are
/// **both** `Uuid::now_v7()` from one generator into two different tables — so an
/// overlay chain spelled as the raw overlay id would be disjoint from every plan
/// chain only by improbability. Neither table constrains the other, and
/// [`policy_chain`] already refuses that standard in as many words: *"disjoint by
/// construction rather than by improbability — a collision would be the one defect
/// that is invisible: two aggregates interleaved on one hash chain verify
/// perfectly."*
///
/// So this rewrites the UUID **version nibble** to `8`, which is
/// [`policy_chain`]'s own device, and it buys three properties at once:
///
/// * **disjoint from every plan chain**, because a v7 plan id's nibble is `7`;
/// * **disjoint from [`policy_chain`]**, whose value is a zero timestamp no `now_v7`
///   can mint;
/// * **injective over overlays**, because every input carries nibble `7` and only
///   that nibble moves — two overlays differing anywhere differ here too.
///
/// `audit_repo_tests::overlay_chains_are_disjoint_from_plan_chains_by_construction`
/// asserts all three rather than describing them.
#[must_use]
pub const fn overlay_chain(price_overlay_id: Uuid) -> Uuid {
    // The version nibble is bits 79..76 of the 128-bit value — the 13th hex digit,
    // `xxxxxxxx-xxxx-Vxxx-…`. Cleared and re-set rather than OR-ed onto whatever was
    // there, so the result does not depend on the input's version being 7.
    const VERSION_MASK: u128 = 0xF << 76;
    Uuid::from_u128((price_overlay_id.as_u128() & !VERSION_MASK) | (0x8 << 76))
}

/// The segment every audited mutation **of one bulk operation** extends.
///
/// D-135 keys a chain on the audited subject's aggregate, and S5 §6's aggregate list
/// — plan, overlay, payer, policy, **bulk operation** — makes a run one in its own
/// right. It has to be: a bulk operation is not a plan and has no single plan (D-134's
/// apply is per-plan precisely because one run's row set spans many), so putting its
/// mutations on [`plan_chain`] would interleave one operational batch across every
/// plan it happened to touch and make no plan's history and no run's history a single
/// segment.
///
/// # A third nibble, not the second
///
/// [`overlay_chain`] already claims version nibble `8`, and reusing it here would be
/// exactly the improbability [`policy_chain`]'s doc names as the failure mode a raw
/// share would have — two independently remapped `now_v7()` spaces sharing one nibble
/// are disjoint only if their remaining 124 bits never coincide, and this gear mints
/// both an overlay id and an operation id from the same generator into two different
/// tables, which is the exact setup [`overlay_chain`]'s own remap exists to defeat for
/// [`plan_chain`]. So this rewrites to nibble `9` instead: disjoint from every plan
/// chain (nibble `7`), disjoint from every [`overlay_chain`] (nibble `8`, real
/// timestamps), and disjoint from [`policy_chain`] (also nibble `8`, but the one fixed
/// value no `now_v7()` mints).
///
/// `audit_repo_tests::bulk_operation_chains_are_disjoint_from_plan_and_overlay_chains_by_construction`
/// asserts this rather than describing it.
#[must_use]
pub const fn bulk_operation_chain(operation_id: Uuid) -> Uuid {
    const VERSION_MASK: u128 = 0xF << 76;
    Uuid::from_u128((operation_id.as_u128() & !VERSION_MASK) | (0x9 << 76))
}

/// A threshold-policy version's durable audit name — its version number.
///
/// The tenant is not repeated in it, for [`price_unit_ref`]'s reason: the record's
/// `tenant_id` column carries it and a reference that restated it would be a second
/// place the two could disagree. What the number has to do is name the row set the
/// pin was taken over, and `(tenant_id, version)` is exactly the store's key prefix.
#[must_use]
pub fn policy_version_ref(version: u64) -> String {
    version.to_string()
}

/// A plan revision's durable audit name — `<plan_id>/<revision>`.
///
/// `(plan_id, revision)` is the name D-145 keeps consumed forever precisely so a
/// record written against it never denotes two rows.
#[must_use]
pub fn plan_revision_ref(plan_id: PlanId, revision: u64) -> String {
    format!("{plan_id}/{revision}")
}

/// One overlay revision's durable audit and approval name — `<overlay_id>/<revision>`.
///
/// [`plan_revision_ref`]'s shape, and deliberately: an approval unit's `subject_ref`
/// has to name the revision and not only the overlay, or a pin taken over revision 3
/// would authorize a decision about revision 4. The overlay id leads so
/// [`approval_repo::subject_overlay`](super::approval_repo::subject_overlay) can read
/// the aggregate back out of the ref without a store read — the same property
/// `subject_plan` relies on one plane over.
#[must_use]
pub fn overlay_revision_ref(price_overlay_id: Uuid, revision: u64) -> String {
    format!("{price_overlay_id}/{revision}")
}

/// A bulk operation's durable audit and approval name — its `operation_id`.
///
/// No revision to encode, unlike [`plan_revision_ref`] and [`overlay_revision_ref`]:
/// a bulk operation is not authored in successive drafts, it is opened once and moves
/// through `pricing_bulk_operation`'s own state machine, so the id alone is already
/// the durable name — the same shape [`price_unit_ref`] and [`policy_version_ref`]
/// use for the subjects that need no second coordinate. `NewBulkOperation`'s own doc
/// makes the same point from the writer's side: *"Minted by the caller, so the audit
/// record of the same act can name it."* Spelled here rather than at each call site
/// for [`window_ref`]'s reason — one place to answer "how is a bulk operation named",
/// so the audit record and the approval record D-158 requires to agree cannot spell it
/// two ways.
#[must_use]
pub fn bulk_operation_ref(operation_id: Uuid) -> String {
    operation_id.to_string()
}

/// The segment every audited mutation **of one payer's membership history**
/// extends.
///
/// D-135 keys a chain on the audited subject's aggregate, and S5 §6's aggregate
/// list — plan, overlay, **payer**, policy, bulk operation — makes a payer one
/// in its own right. A membership row is not a plan and has no plan (D-09's
/// non-overlap is per `(tenant, payer)`, not per plan), so putting its
/// mutations on [`plan_chain`] would interleave one payer's group history
/// across however many plans that payer happens to be priced under.
///
/// # A fourth nibble
///
/// [`overlay_chain`] and [`policy_chain`] already claim version nibble `8`,
/// [`bulk_operation_chain`] claims `9`, and `payer_tenant_id` is **not** an id
/// this gear mints — [`super::group_membership_repo`]'s module doc says why:
/// AMS supplies the payer's identity, so its version nibble carries no promise
/// at all, let alone the promise that it is never `7`. So this rewrites to
/// nibble `A`, the next one unclaimed: disjoint from every plan chain (nibble
/// `7`, this gear's own `now_v7()`), disjoint from every [`overlay_chain`]
/// (nibble `8`, real timestamps), disjoint from [`policy_chain`] (also nibble
/// `8`, the one fixed value no `now_v7()` mints), and disjoint from every
/// [`bulk_operation_chain`] (nibble `9`).
///
/// `audit_repo_tests::payer_chains_are_disjoint_from_every_other_chain_by_construction`
/// asserts this rather than describing it.
#[must_use]
pub const fn payer_chain(payer_tenant_id: Uuid) -> Uuid {
    const VERSION_MASK: u128 = 0xF << 76;
    Uuid::from_u128((payer_tenant_id.as_u128() & !VERSION_MASK) | (0xA << 76))
}

/// A membership row's audit name — its `membership_id`.
///
/// [`price_unit_ref`]'s shape: the payer is not repeated in it, because
/// [`payer_chain`] already carries the aggregate and a reference that restated
/// it would be a second place the two could disagree. A membership row has no
/// revision concept either (`m20260802_000067`'s module doc: "no `state`
/// column ... nothing here duplicates it"), so unlike [`window_ref`] there is
/// no second coordinate to encode.
#[must_use]
pub fn membership_ref(membership_id: Uuid) -> String {
    membership_id.to_string()
}

/// A price row's audit name — its `price_id`.
///
/// The plan is not repeated in it: [`plan_chain`] already carries the aggregate,
/// and a reference that restated it would be a second place the two could
/// disagree.
#[must_use]
pub fn price_unit_ref(price_id: Uuid) -> String {
    price_id.to_string()
}

/// A window's audit name — `<plan_id>/<window_id>`.
///
/// [`plan_revision_ref`]'s shape, and the plan **is** repeated in it. That is a
/// correction rather than a preference: this used to be the bare `window_id`, on
/// [`price_unit_ref`]'s argument that [`plan_chain`] already carries the aggregate —
/// and the argument does not survive the one thing the two stores are for. An
/// approval unit over a window mutation carries this ref and **nothing else** that
/// names its subject, so resolving the aggregate meant reading the window row back;
/// and a controlled mutation writes *nothing*, which is the whole content of D-62's
/// refusal arm. So the unit over a window that was refused before it was written
/// named a row that did not exist: `approval_repo::subject_aggregate` answered
/// `CorruptRow` — a 500 — on every read of it, and `infra::approval::re_derive`
/// reached the same read through `plan_of`.
///
/// Encoding the plan makes the aggregate a **parse** rather than a lookup, which is
/// what `plan_revision_ref` has always given the plan-revision units and the reason
/// `subject_plan` exists. It also keeps both stores on one spelling: the audit record
/// and the approval record of one act name it identically, which is the alignment
/// D-158 is about, and it is why the helper moved rather than its three call sites.
///
/// **It does not widen the plan-prefix reads**, and that is checked rather than
/// hoped: `find_pending_for_plan` and `void_pending_for_plan` both filter
/// `subject_kind = 'plan_revision'` **as well as** the `<plan_id>/` prefix, so a
/// window unit is still invisible to them. That filter used to be belt-and-braces
/// and is now load-bearing — stated in both their docs.
///
/// This is a **name**, not vocabulary: what the design set declares is the
/// `subject_kind` token (S5 §6's enumeration, which carries `window`), and how a
/// subject of that kind is spelled is the store's own business — `subject_ref` is
/// free text on both backends.
#[must_use]
pub fn window_ref(plan_id: PlanId, window_id: Uuid) -> String {
    format!("{plan_id}/{window_id}")
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
/// [`RepoError::ConcurrentMutation`] when this write lost a **same-segment
/// race**: the primary key `(tenant_id, chain_id, seq)` decides it, the loser's
/// whole transaction rolls back, and the refusal is retriable rather than a
/// fault — `CONCURRENT_MUTATION`, 409 (D-159). Corrected 2026-08-04: this
/// section said the outcome was [`RepoError::Db`] "since … no wire code exists",
/// which the module CONTRACT above and [`contention_or_db`] at the insert had
/// both already contradicted. Proved concurrently by
/// `tests/postgres_audit_chain.rs::the_loser_of_a_same_segment_race_is_a_retriable_contention`.
///
/// [`RepoError::Db`] on any other scope or storage failure.
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
        correlation_id: Some(entry.correlation_id),
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
        correlation_id: Set(Some(entry.correlation_id)),
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
        .map_err(|e| {
            // The first of D-159's three serialization points: the head
            // `(tenant_id, chain_id, seq)`. A chain is a strict sequence, so two
            // mutations of one aggregate really do contend here - which is the
            // benefit D-135's segmentation bought, and the cost it kept.
            contention_or_db(
                &e,
                &format!("audit chain {}", entry.chain_id),
                "append pricing_audit_log",
            )
        })?;
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

#[cfg(test)]
#[path = "audit_repo_tests.rs"]
mod audit_repo_tests;
