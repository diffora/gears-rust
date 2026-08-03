//! The audit record's canonical, byte-reproducible encoding and its hash link.
//!
//! `pricing_audit_log` is evidence, not a log: D-14 makes tamper-evidence an
//! **in-database hash chain** written inside the mutation's own ACID
//! transaction, so there are no lost records on a crash and an unavailable
//! audit store cannot exist separately from an unavailable database. What makes
//! that chain evidence is that the bytes it hashes are reproducible — a
//! verification job walks a segment years later and recomputes every link, so
//! any encoding this module can produce two ways is a break the job will report
//! that nobody caused.
//!
//! Pure domain: no storage, no `AccessScope`, no I/O. The writer is
//! [`crate::infra::storage::repo::audit_repo`].
//!
//! ## Why the chain is segmented, and what segmentation costs (D-135)
//!
//! A chain is a **strict sequence**: writing row *N* needs row *N-1*'s hash. One
//! chain per tenant therefore serialized **every** audited mutation of that
//! tenant behind a single head, *inside* the mutation transaction — all
//! authoring serialized by construction, against a `>= 50 rows/s` repricing SLO
//! whose per-row cost model never listed the audit write at all. Segmented per
//! `(tenant_id, chain_id)`, where `chain_id` is the audited subject's aggregate
//! (plan, overlay, payer, policy, bulk operation), concurrent mutations of
//! different aggregates proceed independently, while a bulk run's rows — one
//! plan, one `chain_id` — extend sequentially inside that plan's own
//! transaction anyway.
//!
//! What segmentation **costs** is cross-segment evidence: removing a whole
//! segment leaves no gap in any surviving chain. That is restored by a periodic
//! per-tenant **roll-up** row chaining the segment heads — deleting a row breaks
//! its segment, deleting a segment breaks the roll-up. **The roll-up is not
//! written here and is not written anywhere in this gear yet.** It is periodic
//! rather than on the mutation path, and this module deliberately declares no
//! roll-up encoding, so a reader does not go looking for a writer that is
//! absent by design. `chk_pricing_audit_log_rollup` already makes a mutation row
//! carrying segment heads impossible, so nothing written through this encoding
//! can be mistaken for one.
//!
//! ## The genesis seed is bound to the segment, not to the tenant
//!
//! [`genesis_prev_hash`] takes **both** `tenant_id` and `chain_id`. A genesis
//! bound to the tenant alone would give every one of that tenant's segments the
//! same first link, and a whole segment could then be lifted onto another
//! aggregate — same tenant, same seeds, every link still verifying — with
//! nothing in the chain able to tell.
//!
//! ## Two vocabularies, one of them borrowed and one of them ours
//!
//! The migration types `action` and `subject_kind` as free `text` with no
//! `CHECK`, and **no document in the design set declares either vocabulary**.
//! S5 §6 declares a `subject_kind` enumeration for `pricing_approval`
//! (`plan_revision | price_unit | window | overlay | membership | bundle |
//! retirement | policy | historical_import | bulk_batch`) and nothing declares
//! one for `pricing_audit_log`. [`AuditSubjectKind::PlanRevision`] therefore
//! **borrows S5 §6's spelling** for the one value this group writes, so the two
//! stores do not end up naming one thing two ways; the borrowing is recorded
//! here because a token taken from a neighbouring table is one a later document
//! is free to contradict.
//!
//! [`AuditAction::Publish`] is this module's own naming decision, in the
//! `snake_case` shape every other persisted token in this crate uses. It is
//! deliberately **not** `PlanPublished`: that is a frozen *event* name owned by
//! [`CatalogEvent`](crate::domain::events::CatalogEvent), and an audit action
//! spelled the same would be a second home for one string.
//!
//! Only what the crate **writes** is declared. A variant with no writer would
//! read as coverage to everyone who greps for it — D-158's own constraint, and
//! the reason `retire`, `approve`, `reject`, `deny`, `backdate_import` and
//! `policy_update` are absent below though S5 §6 declares all six.
//!
//! ## Three of the five actions are this module's spelling and are **owed to the
//! document**. Reported.
//!
//! S5 §6 declares the `action` vocabulary and opens it with "exactly the records
//! this design set already requires somebody to write": `publish`, `abandon`,
//! `retire`, `approve`, `reject`, `deny`, `backdate_import`, `policy_update`.
//! Two sentences of the same section require more than that list enumerates —
//! *"every mutation leaves an immutable trail an auditor can rely on for 7+
//! years"* and, normatively, *"**Every mutation MUST record**
//! actor/timestamp/before-after/approval trail"* — and draft authoring is a
//! mutation. So the obligation covers the six mutating authoring routes and the
//! enumeration covers one of them (`abandon`, by D-145).
//!
//! [`AuditAction::Create`], [`AuditAction::Update`] and [`AuditAction::Delete`]
//! are therefore **this module's spelling of a record the set requires and does
//! not name**, in the `snake_case` shape D-158 fixes. D-158's own instruction is
//! that "a slice that adds an audited record adds its token **here**", meaning
//! the document — so the spelling is owed back to S5 §6 and is **reported rather
//! than treated as declared**. It is not the same act as minting a wire code: an
//! `action` is a stored discriminator on an additive vocabulary whose only reader
//! (S12 `inst-he-read`'s Auditor-only sibling) does not exist yet, and the store
//! is greenfield, so a document choosing a different verb costs a rename.
//!
//! The alternative was to leave five of six mutating routes writing nothing,
//! which is the state the register classes *wrong now* over the state where the
//! trail is visibly absent: `pricing_audit_log` is the store D-12 confines to the
//! Auditor, and it answered "who changed this plan" with nothing,
//! indistinguishably from "nobody did".
//!
//! ## There is no audit **read** surface, and that is Slice 12's
//!
//! `inst-au-read` puts the Auditor-only trail behind `audit × read`, paginated
//! per D-125. Nothing in this crate reads `pricing_audit_log` outside the tests
//! that verify the chain. So these records are written and, today, readable only
//! by an operator with database access — which is worth knowing before treating
//! the trail as an operator-facing feature.
//!
//! ## What is deliberately absent
//!
//! - **The roll-up encoding and its writer** — see above.
//! - **The verification job** (`pricing_audit_chain_verified`). It walks
//!   segments and the roll-up alike and is off the mutation path by definition;
//!   this module gives it the primitives it will recompute with and nothing
//!   more.
//! - **A `denied attempt` action.** `inst-tp-selfaudit` and S5's
//!   denied-mutation records are refusals written against subjects this group
//!   cannot reach — there is no approval record and no approval `chain_id` — so
//!   the action is not declared. A token declared here with no writer is
//!   indistinguishable from a record nobody writes.

use chrono::{DateTime, Utc};
use std::fmt;
use toolkit_macros::domain_model;
use uuid::Uuid;

use aws_lc_rs::digest::{SHA256, digest as sha256};

/// Versioned domain-separation tag for this gear's audit chain.
///
/// Bumped **only** on an intentional re-freeze of the encoding, which also
/// means regenerating the byte-repro vector in `audit_tests.rs`. The tag is
/// what keeps this chain's digests disjoint from every other chain the platform
/// hashes — the sibling ledger's posting chain and its own audit chain both sit
/// in the same process space, and a shared preimage space is a shared collision
/// space.
pub const AUDIT_DOMAIN_SEP: &[u8] = b"VHP-BSS-PRICING-AUDIT-v1\x1f";

/// NULL-safe presence marker for an absent field: a bare byte.
const ABSENT: u8 = 0x00;

/// NULL-safe presence marker for a present field: `PRESENT` + u32-BE len + bytes.
const PRESENT: u8 = 0x01;

/// What was done to the audited subject.
///
/// Five variants, one per mutating path this crate has. See the module doc for
/// why three of the spellings are owed back to S5 §6, and why the tokens are not
/// the event names.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuditAction {
    /// A draft plan revision or a draft price row was authored.
    Create,
    /// A draft's content was replaced — a plan facet, or a price row's whole
    /// content.
    Update,
    /// A never-published draft price row was removed (`inst-ps-nodelete` keeps
    /// this off a published one).
    Delete,
    /// A plan's open draft revision was discarded — the flip, not a deletion
    /// (D-145). It is a distinct action from [`AuditAction::Delete`] because the
    /// row survives and its number stays consumed.
    Abandon,
    /// A publish commit moved the subject into `published`.
    Publish,
}

impl AuditAction {
    /// Every action, stable order.
    ///
    /// **Every entry here has a production writer**, which is D-158's constraint
    /// and what `tests/sqlite_audit_chain.rs` asserts by walking this slice: a
    /// vocabulary entry nobody writes reads as coverage to everyone who greps
    /// for it.
    pub const ALL: &'static [Self] = &[
        Self::Create,
        Self::Update,
        Self::Delete,
        Self::Abandon,
        Self::Publish,
    ];

    /// The persisted `action` token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Create => "create",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Abandon => "abandon",
            Self::Publish => "publish",
        }
    }
}

impl fmt::Display for AuditAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What kind of thing the audited subject is.
///
/// Both variants are spelled as S5 §6 spells the same subjects for
/// `pricing_approval`, **verbatim** — unlike the three actions above, nothing
/// here is minted. See the module doc.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuditSubjectKind {
    /// One revision row of a plan — the `(plan_id, revision)` durable name.
    PlanRevision,
    /// One price row, named by its `price_id`.
    ///
    /// Its **chain** is still the plan's (D-135 keys the segment on the audited
    /// subject's *aggregate*, and a price row's aggregate is the plan it prices),
    /// which is what makes "who changed this plan" one segment to walk rather
    /// than a join.
    PriceUnit,
}

impl AuditSubjectKind {
    /// Every subject kind, stable order.
    pub const ALL: &'static [Self] = &[Self::PlanRevision, Self::PriceUnit];

    /// The persisted `subject_kind` token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlanRevision => "plan_revision",
            Self::PriceUnit => "price_unit",
        }
    }
}

/// Who caused a mutation and under which request — the audit fields a repository
/// cannot derive from the row it is writing.
///
/// Threaded through every mutating repository call rather than defaulted, and
/// there is **no unaudited form of any of them**: a second entry point that
/// skipped the record would make `pricing_audit_log` a trail that is *silently*
/// incomplete, which is strictly worse than one visibly absent because a reader
/// cannot tell "nobody changed this" from "this path does not write".
///
/// `recorded_at` is the **caller's** instant for [`AuditRecord`]'s reason: the
/// record is hashed over it, so a repository reading its own clock would make two
/// records of one transaction disagree about when the transaction happened.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditStamp {
    /// **Pseudonymous** principal id of the acting operator (`inst-au-pii`).
    pub actor_principal_id: Uuid,
    /// When the mutation was recorded, UTC — the caller's instant.
    pub recorded_at: DateTime<Utc>,
    /// The correlation id of the causing request.
    ///
    /// `None` on every authoring route today, and that is a gap rather than a
    /// choice: no correlation id reaches an Axum handler in this gear — the
    /// publish path takes one as a parameter because its caller is in-process.
    /// **Reported**, because `inst-au-complete` lists the correlation id among a
    /// record's required fields.
    pub correlation_id: Option<Uuid>,
}

/// A mutated subject's before/after state, as the audit record's `jsonb` columns
/// hold it (`inst-au-complete`: before/after version refs).
///
/// One rendering for every audited subject in the crate — a plan revision, a
/// price row, a publish commit — because a second one is a second answer to
/// "what did this row look like". `pending_ref` is `Some` only on the publish
/// commit's after-state, which is what connects that mutation to the
/// addressability it produced; an auditor reading the record otherwise has the
/// flip and no way to reach the version it landed in.
///
/// Wire keys `camelCase`, as the outbox payload's are.
#[must_use]
pub fn subject_state(
    state: crate::domain::lifecycle::LifecycleState,
    row_version: u64,
    pending_ref: Option<&str>,
) -> serde_json::Value {
    match pending_ref {
        Some(pending) => serde_json::json!({
            "lifecycleState": state.as_str(),
            "rowVersion": row_version,
            "pendingVersionRef": pending,
        }),
        None => serde_json::json!({
            "lifecycleState": state.as_str(),
            "rowVersion": row_version,
        }),
    }
}

impl fmt::Display for AuditSubjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The hashing input for one audit record.
///
/// Borrows its string and JSON fields: the record is hashed on the way into the
/// store and never round-trips through this type, so copying every field to
/// hash it would be a copy taken for nothing. It carries `#[domain_model]`
/// because it is a `pub` domain type (DE0309), even though it never crosses the
/// repository boundary.
///
/// The field set is `inst-au-complete`'s: actor, timestamp, before/after refs,
/// approval trail, correlation id — plus the segment's own coordinates, which
/// is what binds a record to its position rather than merely to its content.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AuditRecord<'a> {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The audited subject's aggregate — this record's chain segment.
    pub chain_id: Uuid,
    /// Position within the segment, `0` at genesis.
    pub seq: u64,
    /// When the mutation was recorded, UTC.
    pub recorded_at: DateTime<Utc>,
    /// **Pseudonymous** principal id of the acting operator, never a display
    /// name and never an email (`inst-au-pii`).
    pub actor_principal_id: Uuid,
    /// What was done.
    pub action: AuditAction,
    /// What kind of thing it was done to.
    pub subject_kind: AuditSubjectKind,
    /// Which one — for a plan revision, the `plan_id/revision` reference.
    pub subject_ref: &'a str,
    /// The subject's before state, as the record's own `jsonb` column holds it.
    pub before_state: Option<&'a serde_json::Value>,
    /// The subject's after state.
    pub after_state: Option<&'a serde_json::Value>,
    /// The approval record the mutation ran under, when it had one.
    pub approval_ref: Option<Uuid>,
    /// The correlation id of the request that caused the mutation.
    pub correlation_id: Option<Uuid>,
}

/// The genesis `prev_hash` of a `(tenant_id, chain_id)` segment.
///
/// Bound to both, and never NULL, so a segment's first row carries a real link
/// rather than an absence a verifier has to interpret. See the module doc for
/// what a tenant-only seed would let somebody transplant.
#[must_use]
pub fn genesis_prev_hash(tenant_id: Uuid, chain_id: Uuid) -> [u8; 32] {
    let mut buf = Vec::with_capacity(96);
    buf.extend_from_slice(AUDIT_DOMAIN_SEP);
    put_uuid(&mut buf, tenant_id);
    put_uuid(&mut buf, chain_id);
    put_str(&mut buf, "GENESIS");
    digest32(&buf)
}

/// `row_hash = SHA-256(domain_sep || fields || prev_hash)`.
///
/// `prev_hash` is the predecessor row's `row_hash`, or [`genesis_prev_hash`] at
/// `seq = 0`. SHA-256 comes from the FIPS-validated `aws-lc-rs` provider the
/// platform installs; `sha2` is blocked by dylint DE0708.
///
/// Every field is length-prefixed and NULL-safe, which is not decoration: it is
/// what makes two different field **boundaries** unable to collide. Without the
/// prefixes a record whose `action` is `ab` and `subject_kind` is `c` hashes
/// identically to one whose fields are `a` and `bc`, and a chain that cannot
/// distinguish those can be forged by moving a character across a field border
/// with every link still verifying.
///
/// # Errors
///
/// The `serde_json::Error` from serializing a canonicalized `before_state` or
/// `after_state`. Unreachable for an in-memory `Value`, and propagated rather
/// than degraded to a fixed byte string all the same: a silent fallback would
/// make an un-canonicalizable record hash **identically** to one with no state
/// at all, which is a collision target rather than a fallback. A record whose
/// content cannot be canonicalized must never be sealed into the chain.
pub fn audit_row_hash(
    record: &AuditRecord<'_>,
    prev_hash: &[u8; 32],
) -> Result<[u8; 32], serde_json::Error> {
    let mut buf = Vec::with_capacity(320);
    buf.extend_from_slice(AUDIT_DOMAIN_SEP);

    put_uuid(&mut buf, record.tenant_id);
    put_uuid(&mut buf, record.chain_id);
    put_u64(&mut buf, record.seq);
    put_i64(&mut buf, record.recorded_at.timestamp_micros());
    put_uuid(&mut buf, record.actor_principal_id);
    put_str(&mut buf, record.action.as_str());
    put_str(&mut buf, record.subject_kind.as_str());
    put_str(&mut buf, record.subject_ref);
    put_opt_json(&mut buf, record.before_state)?;
    put_opt_json(&mut buf, record.after_state)?;
    put_opt_uuid(&mut buf, record.approval_ref);
    put_opt_uuid(&mut buf, record.correlation_id);

    put(&mut buf, prev_hash);
    Ok(digest32(&buf))
}

// ---------------------------------------------------------------------------
// Length-prefixed, NULL-safe framing primitives.
//
// The ledger's `domain/canonical.rs` discipline, copied rather than imported:
// this gear does not depend on that crate, and a shared helper would put two
// gears' frozen encodings behind one edit.
// ---------------------------------------------------------------------------

/// A present field: marker, u32-BE length, bytes.
fn put(buf: &mut Vec<u8>, bytes: &[u8]) {
    buf.push(PRESENT);
    // Saturating rather than failing: a field longer than 4 GiB cannot arise
    // from any column this gear writes, and a truncated length would still be
    // hashed together with the bytes that follow it.
    let len = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
    buf.extend_from_slice(&len.to_be_bytes());
    buf.extend_from_slice(bytes);
}

/// An absent field: a bare marker, so `None` and an empty value differ.
fn put_none(buf: &mut Vec<u8>) {
    buf.push(ABSENT);
}

fn put_str(buf: &mut Vec<u8>, value: &str) {
    put(buf, value.as_bytes());
}

fn put_uuid(buf: &mut Vec<u8>, value: Uuid) {
    put(buf, value.as_bytes());
}

fn put_opt_uuid(buf: &mut Vec<u8>, value: Option<Uuid>) {
    match value {
        Some(value) => put_uuid(buf, value),
        None => put_none(buf),
    }
}

fn put_u64(buf: &mut Vec<u8>, value: u64) {
    put(buf, &value.to_be_bytes());
}

fn put_i64(buf: &mut Vec<u8>, value: i64) {
    put(buf, &value.to_be_bytes());
}

/// A `jsonb` field, hashed over its **canonical** bytes.
fn put_opt_json(
    buf: &mut Vec<u8>,
    value: Option<&serde_json::Value>,
) -> Result<(), serde_json::Error> {
    if let Some(value) = value {
        let bytes = serde_json::to_vec(&canonicalized(value))?;
        put(buf, &bytes);
    } else {
        put_none(buf);
    }
    Ok(())
}

/// Rebuild `value` with every object's keys in sorted order, recursively.
///
/// The `jsonb` columns are hashed over this rather than over whatever byte
/// order a serializer happened to emit. Two reasons, and the second is the one
/// that bites: `jsonb` does not preserve key order at all, so a value written
/// one way and read back another is the normal case; and in a monorepo build
/// another crate may enable `serde_json/preserve_order`, which turns
/// `serde_json::Map` into an insertion-ordered `IndexMap` and would otherwise
/// make this hash depend on how the caller happened to build the value. Either
/// way the verification job would report a break that never happened.
///
/// Array order is left alone — order is semantic in a JSON array.
fn canonicalized(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(&String, &serde_json::Value)> = map.iter().collect();
            entries.sort_unstable_by(|left, right| left.0.cmp(right.0));
            let mut out = serde_json::Map::with_capacity(map.len());
            for (key, nested) in entries {
                out.insert(key.clone(), canonicalized(nested));
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.iter().map(canonicalized).collect())
        }
        other => other.clone(),
    }
}

/// The 32-byte SHA-256 digest of a preimage.
fn digest32(buf: &[u8]) -> [u8; 32] {
    let digested = sha256(&SHA256, buf);
    let mut out = [0_u8; 32];
    out.copy_from_slice(digested.as_ref());
    out
}

/// Render a digest as lowercase hex — the spelling the byte-repro vector and
/// every operator-facing diagnostic use.
#[must_use]
pub fn hex32(digest: &[u8; 32]) -> String {
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(hex_nibble(*byte >> 4));
        out.push(hex_nibble(*byte & 0x0f));
    }
    out
}

/// One lowercase hex digit. Total over its only two call sites, which hand it
/// the high and low nibble of a byte and so can never exceed 15.
fn hex_nibble(nibble: u8) -> char {
    char::from(if nibble < 10 {
        b'0' + nibble
    } else {
        b'a' + nibble - 10
    })
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod audit_tests;
