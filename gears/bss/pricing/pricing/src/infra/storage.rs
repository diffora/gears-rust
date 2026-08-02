//! Persistence layer for the catalog Foundation: the migration chain, the
//! `SeaORM` entities over its tables, and the repositories that read them.
//!
//! Everything here is infrastructure by construction (DE0301): the domain layer
//! never learns that a `Plan` is a row. Where a repository hands back a value
//! the rest of the system reasons about, it maps at this boundary — see
//! [`repo::PinFrontierRepo`], which reads a `pricing_pin_frontier` row and
//! returns the SDK's `PinFrontier`.
//!
//! Every physical table carries the gear-name prefix `pricing_`
//! (`design/01-foundation.md` §3.7) and lives in schema `bss` on Postgres.
//! `SQLite` is the fast non-production test backend: it mirrors the shape
//! (`uuid` -> `text`, `timestamptz` -> `text`, `jsonb` -> `text`) and, unlike
//! the sibling ledger's substrate, it also mirrors the **append-only guards** —
//! `RAISE(ABORT, ...)` triggers stand in for PL/pgSQL `RAISE EXCEPTION`, so the
//! column whitelists are exercised by the fast test suite rather than only by
//! the Docker-gated Postgres one.

pub mod entity;
pub mod migrations;
pub mod repo;

use crate::domain::error::DomainError;

/// A storage-layer failure.
///
/// Deliberately small: the repositories translate a driver error into `Db` and
/// keep exactly the typed refusals the Foundation's own invariants need.
/// A variant per SQL error class would put the database's vocabulary in the
/// gear's error surface, which is the coupling this boundary exists to prevent.
///
/// The authoring refusals below are **three answers where a lesser design has
/// one**. A repository that reported every failed compare-and-swap as a single
/// "conflict" would leave the caller unable to tell "refresh and retry" from
/// "you are editing a revision that is frozen forever" from "that revision is
/// gone" — and those want three different behaviours from the operator, only
/// one of which is a retry.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RepoError {
    /// Underlying database or scope failure.
    #[error("pricing repo db error: {0}")]
    Db(String),
    /// A stored value could not be interpreted as the type its column is
    /// declared to hold — a `CHECK`-constrained token outside its enumeration,
    /// or a `catalog_version` outside the unsigned range the SDK type carries.
    /// An invariant breach, never a caller mistake.
    #[error("pricing repo: corrupt stored value: {0}")]
    CorruptRow(String),
    /// An attempt to move the pin-eligibility frontier to a version at or below
    /// where it already stands.
    ///
    /// Refused rather than swallowed: the projector advances the frontier only
    /// inside the transaction completing the frontier's **next** version in
    /// order, so an equal-or-lower target means the caller's ordering
    /// assumption has broken (a duplicate completion, or a re-drive running out
    /// of order). Treating it as a no-op would hide exactly the defect the
    /// materialized frontier exists to make impossible — a pin resolving two
    /// different contents over time (D-136).
    #[error(
        "pricing repo: pin frontier for tenant {tenant} stands at {current}, \
         refusing to advance to {requested}"
    )]
    FrontierRegression {
        /// The tenant whose frontier was targeted.
        tenant: String,
        /// Where the frontier stands now.
        current: u64,
        /// The version the caller asked to advance to.
        requested: u64,
    },
    /// The named subject does not exist **or lies outside the caller's scope**.
    ///
    /// Deliberately the same answer either way. A repository that answered
    /// "forbidden" for a row belonging to another tenant would confirm that the
    /// row exists, which is the existence leak the SQL-level scoping is there to
    /// close; the catalog is commercially sensitive, so absence is what a
    /// foreign scope sees.
    #[error("pricing repo: {subject} {id} not found")]
    NotFound {
        /// The kind of thing that was looked for (`plan revision`, `price`).
        subject: String,
        /// The reference the caller supplied.
        id: String,
    },
    /// A write whose submitted row version is not the row's current one.
    ///
    /// Carries **both** versions, because the difference is the diagnosis: a
    /// caller one version behind never refreshed, while a caller many versions
    /// behind is a bulk run colliding with interactive editing — the collision
    /// `fr-concurrent-edit` exists to make visible instead of silently letting
    /// one side win.
    #[error(
        "pricing repo: {subject} {id} stands at row version {current}, \
         refusing a write submitted against {submitted}"
    )]
    StaleRowVersion {
        /// The kind of thing that was written.
        subject: String,
        /// The reference the caller supplied.
        id: String,
        /// The row version the store holds.
        current: u64,
        /// The row version the caller submitted.
        submitted: u64,
    },
    /// A content mutation aimed at a revision that is not a `draft`.
    ///
    /// Distinct from [`RepoError::StaleRowVersion`] because it is not
    /// retryable: no refresh will make a published, superseded or retired
    /// revision editable, and a caller told merely "conflict" would loop. The
    /// remedy is a different operation entirely — open a new revision (§4.3).
    #[error("pricing repo: {subject} {id} is {state}; only draft content is mutable")]
    NotDraft {
        /// The kind of thing that was written.
        subject: String,
        /// The reference the caller supplied.
        id: String,
        /// The lifecycle state the row is actually in.
        state: String,
    },
    /// A request for a **successor** revision on a plan whose current revision
    /// can never be superseded.
    ///
    /// Deliberately not [`RepoError::NotDraft`], which it superficially
    /// resembles and which answered this case first. That variant's sentence
    /// — "only draft content is mutable" — is about *editing a revision's
    /// content*, and its whole value is the remedy it implies: stop editing
    /// this revision and open the next one. Here the caller asked for exactly
    /// that next revision, so the same sentence would name as the remedy the
    /// operation it is refusing, and an operator following it would loop. The
    /// real ground is different too: not that the current revision is frozen —
    /// every publishable predecessor is — but that it can never flip
    /// `superseded`, so the successor would be unpublishable from the moment it
    /// was opened. It carries `PLAN_RETIRED_NO_SUCCESSOR` (D-146) rather than
    /// the frozen revision's `LIFECYCLE_FORBIDDEN`: what the two refusals ask an
    /// operator to do differs, and until the codes did too, telling them apart
    /// meant reading prose.
    #[error(
        "pricing repo: plan {plan_id} stands at a {state} revision, which can never be \
         superseded; it takes no successor"
    )]
    NoSuccessorRevision {
        /// The plan a successor was asked for.
        plan_id: String,
        /// The lifecycle state its current revision is in.
        state: String,
    },
    /// A request to open a revision on a plan that already has its one open
    /// draft.
    ///
    /// A plan has at most one concurrently editable shape
    /// (`uq_pricing_plan_open_draft`). The refusal names the revision that
    /// holds the slot, so the caller can go and edit it rather than guess which
    /// of its own requests won — which is why it carries
    /// `OPEN_DRAFT_REVISION_EXISTS` (D-146) and not the lifecycle refusal it was
    /// once folded into: a uniqueness conflict on the draft slot is not a
    /// transition the state machine denies, and the operator's next action is a
    /// real one.
    #[error("pricing repo: plan {plan_id} already has an open draft at revision {revision}")]
    OpenDraftExists {
        /// The plan whose draft slot is taken.
        plan_id: String,
        /// The revision number of the draft holding it.
        revision: u64,
    },
    /// A client idempotency key was reused for a **different** request.
    ///
    /// Neither answer available is right, which is why this is its own refusal
    /// and not a conflict inviting a retry: replaying the stored response would
    /// report work the caller never asked for, and re-executing would break the
    /// at-most-once promise the key exists to make. The refusal names the key
    /// and the operation it was scoped to rather than either payload — the
    /// table stores digests on purpose, and what an operator needs is which key
    /// was reused, not what the two requests said.
    #[error(
        "pricing repo: idempotency key {client_key} on operation {operation} \
         was first used with a different request payload"
    )]
    IdempotencyPayloadMismatch {
        /// The operation the key is scoped to.
        operation: String,
        /// The client-supplied key that was reused.
        client_key: String,
    },
    /// Another current row already occupies the canonical scope key.
    ///
    /// The key's own rendering is carried verbatim: a `DUPLICATE_SCOPE_KEY`
    /// rejection that dropped an axis would report a collision between two rows
    /// that do not actually share a key.
    #[error("pricing repo: duplicate canonical scope key: {0}")]
    DuplicateScopeKey(String),
    /// A grandfathering horizon authored on a row that is not a grandfathered
    /// generation.
    ///
    /// `chk_pricing_price_grandfather_until` is a **physical** CHECK — only an
    /// `existing_grandfathered` row may carry a non-null `grandfather_until` —
    /// and `grandfather_until` is ordinary caller-supplied content on the draft
    /// plane. Without this refusal the pairing is discovered by the driver, so
    /// the caller is told the store failed and the operator reads a 500 for a
    /// request they could have fixed by clearing one field.
    ///
    /// The pairing is now normative and has a code of its own —
    /// `GRANDFATHER_UNTIL_FORBIDDEN` (D-147), the `cohort` biconditional's
    /// sibling: one axis-conditioned field, one code. Before it, this refusal
    /// was the code's own and could only borrow the generic bad-request answer,
    /// which told a consumer nothing about which field to clear.
    #[error(
        "pricing repo: only an existing_grandfathered row may carry a grandfathering \
         horizon; this key is {eligibility}"
    )]
    GrandfatherHorizonOffClass {
        /// The eligibility class the row's canonical scope key actually holds.
        eligibility: String,
    },
    /// An authored value lies outside the range the column that stores it can
    /// hold.
    ///
    /// The domain counts quantities in `u64` because a quantity only ever goes
    /// up, while the columns are `bigint` and `integer`, so the top of the
    /// domain range has no storage at all. Deliberately **not**
    /// [`RepoError::CorruptRow`]: the same mismatch read *out* of a column
    /// means the table was written around, while a value arriving *on a
    /// request* is a caller mistake and the request can be reshaped — which is
    /// the line between an internal fault and a bad one. The field is named so
    /// an author corrects one number instead of resubmitting a row and
    /// guessing.
    #[error("pricing repo: {field} {value} is outside the range its column can hold")]
    ValueOutOfRange {
        /// The authored field, located precisely enough to edit.
        field: String,
        /// The value, as authored.
        value: String,
    },
    /// An authored instant carries precision below the millisecond quantum
    /// (D-144).
    ///
    /// Checked where the value enters the store rather than left to the column,
    /// which would take it: `timestamptz` holds microseconds, so a finer instant
    /// persists silently and is then compared for equality against one produced
    /// at the quantum. The field is named because clearing or rounding one value
    /// is the whole remedy.
    #[error("pricing repo: {field} {value} is finer than the millisecond quantum")]
    TimestampPrecisionExceeded {
        /// The authored field the instant arrived on.
        field: String,
        /// The instant, as authored.
        value: String,
    },
    /// A duplicate arrived while the claim it collided with is unanswered
    /// (D-143).
    ///
    /// There is nothing to replay and nothing to re-execute, so the only other
    /// answer available is a response nobody ever produced. Retrying is the
    /// remedy and it is one a client can act on: shortly after, the key is
    /// either answered — and the retry is replayed the stored response — or it
    /// carries a different payload, and the retry is
    /// [`RepoError::IdempotencyPayloadMismatch`]. The refusal names the key and
    /// the operation for the same reason that one does.
    #[error(
        "pricing repo: idempotency key {client_key} on operation {operation} \
         is claimed and not yet answered"
    )]
    IdempotencyKeyInFlight {
        /// The operation the key is scoped to.
        operation: String,
        /// The client-supplied key that is held.
        client_key: String,
    },
}

/// Map a storage failure into the gear's rejection vocabulary.
///
/// A frontier regression is a **precondition** failure, not an internal fault:
/// the row is intact and the store is healthy, the requested transition is
/// simply one the watermark's monotonicity forbids — the same shape as any other
/// refused lifecycle edge, so it lands on `LIFECYCLE_FORBIDDEN`. The draft-only
/// refusals join it there: a frozen revision is state forbidding an operation,
/// not a malformed request. Those two are **all** that variant keeps.
///
/// **The collapse D-146 exists to undo.** Four refusals used to land on
/// `LIFECYCLE_FORBIDDEN`, and two of them ask an operator for different things:
/// [`RepoError::NoSuccessorRevision`] is a **stop** — a retired plan can never
/// publish again, so no successor of it is publishable — while
/// [`RepoError::OpenDraftExists`] is a **different and available action**, go
/// and edit the draft you already have. A consumer that had to read the sentence
/// to tell those apart could act on neither, so each now carries its own code
/// (`PLAN_RETIRED_NO_SUCCESSOR`, `OPEN_DRAFT_REVISION_EXISTS`). The second is
/// also not a state-machine transition at all: it is a uniqueness conflict on
/// the plan's one draft slot, which is why it leaves the precondition class for
/// the conflict one and becomes retriable.
///
/// The line is held on the other side too. The read model's forward-only pin
/// frontier is advanced by the projector and is unreachable by a caller, so a
/// regression there stays where it is: a wire code for it would document an API
/// no client can provoke.
///
/// [`RepoError::ValueOutOfRange`] is the one arm left on
/// [`DomainError::InvalidRequest`], and it is the reason that arm exists rather
/// than a second flavour of [`RepoError::CorruptRow`]: an authored quantity the
/// column cannot hold arrived on a request and the caller can change it, which
/// is not true of anything on the internal arm. It mints no wire code — it
/// carries the sentence, and the Foundation's bad-request category carries the
/// classification. [`RepoError::GrandfatherHorizonOffClass`] stood beside it
/// only for want of a code and now has one (D-147), which puts it with the
/// publish refusals it is a sibling of rather than with malformed input.
///
/// [`RepoError::StaleRowVersion`] is rendered into
/// [`DomainError::StaleVersion`] ending in the **same
/// `current {c}, submitted {s}` clause** `domain::concurrency::require_match`
/// produces, so the numbers a runbook greps for read identically whether the
/// submit was refused by the transport's pre-check or by the store's
/// compare-and-swap. The store's rendering carries one thing the pre-check
/// cannot — a `{subject} {id}:` prefix naming the row — so the two are not
/// byte-identical and are not claimed to be; what they must not do is spell the
/// same collision two different ways, which is how a runbook ends up believing
/// there are two failures. `storage_tests.rs` pins the shared clause.
///
/// Deliberately a named function rather than a `From` impl, and it **logs the
/// storage failure before flattening it**. `DomainError` is a `Clone + Eq` value
/// type carried into responses and compared in tests, so it cannot hold a boxed
/// source; the chain would be lost either way, and the log is where an operator
/// gets it back. The sibling ledger converts at its call sites for the same
/// reason.
#[must_use]
pub fn repo_failure(err: &RepoError) -> DomainError {
    match err {
        RepoError::Db(_) | RepoError::CorruptRow(_) => {
            tracing::error!(error = %err, "bss-pricing: storage failure");
            DomainError::Internal(err.to_string())
        }
        RepoError::FrontierRegression { .. } | RepoError::NotDraft { .. } => {
            DomainError::LifecycleForbidden(err.to_string())
        }
        RepoError::NoSuccessorRevision { .. } => {
            DomainError::PlanRetiredNoSuccessor(err.to_string())
        }
        RepoError::OpenDraftExists { .. } => DomainError::OpenDraftRevisionExists(err.to_string()),
        RepoError::NotFound { subject, id } => DomainError::NotFound {
            subject: subject.clone(),
            id: id.clone(),
        },
        RepoError::StaleRowVersion {
            subject,
            id,
            current,
            submitted,
        } => DomainError::StaleVersion(format!(
            "{subject} {id}: current {current}, submitted {submitted}"
        )),
        RepoError::DuplicateScopeKey(key) => DomainError::DuplicateScopeKey(key.clone()),
        RepoError::IdempotencyPayloadMismatch { .. } => {
            DomainError::IdempotencyPayloadMismatch(err.to_string())
        }
        RepoError::IdempotencyKeyInFlight { .. } => {
            DomainError::IdempotencyKeyInFlight(err.to_string())
        }
        RepoError::ValueOutOfRange { .. } => DomainError::InvalidRequest(err.to_string()),
        RepoError::GrandfatherHorizonOffClass { .. } => {
            DomainError::GrandfatherUntilForbidden(err.to_string())
        }
        RepoError::TimestampPrecisionExceeded { .. } => {
            DomainError::TimestampPrecisionExceeded(err.to_string())
        }
    }
}

#[cfg(test)]
#[path = "storage_tests.rs"]
mod storage_tests;
