//! The per-tenant taxonomy writer lock, and the two mutations that run
//! under it (`design/02-taxonomy-attributes.md` §3.4 `inst-tc-writer-lock`,
//! §3.1 `inst-tx-walk`).
//!
//! # Why the lock lives here and not in the repository
//!
//! `inst-tc-writer-lock` wants an *"advisory lock on Postgres, the write
//! transaction on `SQLite`"*, and an advisory lock is the **provider's**:
//! `DBProvider::lock` issues it on a dedicated session, while a `DBRunner`
//! is sealed and can reach no raw executor at all. So the repository's
//! category writes hold no lock and this module wraps them, the same
//! layering `infra::increment` uses for the coalescer's lease.
//!
//! **The `SQLite` arm is not the write transaction, and that is stronger
//! rather than weaker.** The instruction names the write transaction for
//! that backend; the toolkit's `LockManager` resolves `try_lock` to
//! `GuardInner::File` — a cross-process file marker — when neither the `pg`
//! nor the `mysql` feature is on (`toolkit-db::advisory_locks`, the
//! `GuardInner` enum's unconditional first arm). A one-connection `SQLite`
//! pool already gives the transaction the instruction asks for, and the
//! marker serializes across processes on top of it. So the same call is made
//! on both backends and neither arm is weaker than declared. This was read
//! off the library rather than assumed — the first version of this paragraph
//! asserted the write transaction *was* the mechanism, which is false.
//!
//! # Why single-writer is the rule and not a performance choice
//!
//! The instruction states its own reason: *"taxonomy ops are rare and
//! human-paced, and single-writer is what makes `TaxonomyWalk`'s verdict
//! trustworthy"*. A cycle verdict is a claim about a whole chain, and two
//! re-parents that each read a chain the other is about to change can both
//! answer "no cycle" and jointly close one. Nothing physical catches that: a
//! `CHECK` sees one row, and the two writes touch different rows, so no
//! index contends them either. The lock is the only thing between the two.
//!
//! # The lock key
//!
//! `(gear, key)` = `("bss-products", "taxonomy:<tenant_id>")`. Per tenant,
//! because the instruction says per tenant and because a gear-wide key would
//! serialize every tenant's operator against every other's.
//!
//! # The event aggregates this feature orders on
//!
//! `dod-taxonomy-events` puts the tree's five events on **one** aggregate per
//! tenant — *"`(tenant, category tree)` as one aggregate, matching the
//! single-writer discipline"* — and metadata on `(tenant, entity)`.
//! [`TAXONOMY_TREE_AGGREGATE`] and [`metadata_aggregate`] are those two keys,
//! and both are spent: the five acts below announce themselves on the tree
//! key **inside their own transaction**, and the doors' three other acts on
//! the entity's id (`events::enqueue_taxonomy`, since 2026-09-03).
//!
//! **The two display events order on their own entity's id** (P-D-116 row
//! 15). §7 row 15 asked which aggregate orders `CategoryDisplayUpdated` and
//! `AttributeDefinitionUpdated` and said why it was not a free choice:
//! *"display writes do not take the taxonomy writer lock, so the tree key
//! would claim a serialization the door does not provide"*. The row is what
//! serializes a display write — `products_category.mutation_seq` is the door's
//! precondition — so the entity id is the key that matches what actually
//! orders them, and it is [`metadata_aggregate`]'s rule applied twice more.
//!
//! # Write and announce in one transaction
//!
//! Each act here takes the lock, judges on a plain connection, then opens
//! **one transaction** for the write and its event (`inst-tx-event`, P-D-21:
//! the event is the success-path audit record). A refusal inside travels as
//! `Err(TaxonomyTxError::Refused)` so the transaction **rolls back** — on
//! Postgres a failed statement aborts the transaction and a later `COMMIT`
//! would fail on its own — and the door audits it afterwards, which is
//! `recognized_sets`' shape. The closure owns what it captures (`FnMut`, one
//! copy per attempt) because `transaction_with_retry`'s bound is
//! higher-ranked over the transaction's lifetime.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-taxonomy-writer-lock:p1
//! @cpt-dod:cpt-cf-bss-products-dod-taxonomy-walk:p1
//! @cpt-dod:cpt-cf-bss-products-dod-retire-delete-guard:p1
//! @cpt-dod:cpt-cf-bss-products-dod-taxonomy-events:p1

use chrono::{DateTime, Utc};
use toolkit_db::secure::{AccessScope, DBRunner, TxConfig};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::taxonomy::{
    DeleteCensus, RetireCensus, TaxonomyLimitExceeded, TaxonomyLimits, ancestors_of, cycle_verdict,
    delete_verdict, depth_of, limit_verdict, retire_verdict, subtree_height,
};
use crate::domain::validation::ValidationReport;
use crate::infra::broker::EventSink;
use crate::infra::events::{self, TaxonomyEventBody};
use crate::infra::storage::{RepoError, repo};

/// The gear namespace every lock this module takes is scoped to.
const LOCK_GEAR: &str = "bss-products";

/// The one aggregate every **tree** mutation of one tenant orders on.
///
/// A fixed sentinel rather than the node's own id, because
/// `dod-taxonomy-events` wants the whole tree ordered as one aggregate and
/// `inst-tc-writer-lock` already serializes tree mutations per tenant — so
/// one key matches the serialization the gear actually provides, and a
/// per-node key would promise an ordering across nodes that nothing
/// enforces. The tenant is not folded in here because
/// `infra::events::partition_for` already takes it as its own operand.
///
/// The value is arbitrary and fixed: what matters is that it is one value and
/// that it collides with no entity id, which a `Uuid` this shape does by
/// construction — no `products_category.category_id` is ever minted from a
/// literal.
pub const TAXONOMY_TREE_AGGREGATE: Uuid =
    Uuid::from_u128(0x027a_2000_0000_0000_0000_0000_0000_0001);

/// The aggregate a metadata event orders on: the owning entity itself.
///
/// `dod-taxonomy-events`: *"metadata events **MUST** order on `(tenant,
/// entity)`"*. A metadata write rides the entity row's `If-Match` and takes
/// no taxonomy lock, so the entity is both the serialization the door
/// provides and the key the events claim — the alignment §7 row 15 finds
/// missing for the other two.
#[must_use]
pub const fn metadata_aggregate(entity_id: Uuid) -> Uuid {
    entity_id
}

/// **Product and SKU attribute-value writes emit no event of their own**, and
/// this is the explicit declaration `dod-taxonomy-events` requires of that
/// absence -- stated as *what announces the act instead*, which is the only
/// form of it a consumer can act on.
///
/// The reason is `design/02` C2: those values are **entity content**. They
/// ride the owning entity's revision and freeze into its published versions,
/// so the act that changes them already announces itself through the two
/// payload types below, and at publish through `ProductPublished` /
/// `SkuPublished`. A second event per value write would announce one act
/// twice and give a consumer no way to tell an independent change from a
/// component of one it has already seen.
///
/// A named constant rather than a comment, so a census looking for what this
/// feature announces finds the absence too: an absence stated only in prose
/// is one the next reader re-derives. It holds the payload types rather than
/// a bare `true` because a boolean asserts nothing -- these two strings can
/// drift from `infra::events`' own constants, and
/// `the_no_event_declaration_names_the_types_that_do_announce` is what stops
/// them.
pub const ATTRIBUTE_VALUE_WRITE_ANNOUNCED_BY: [&str; 2] = ["ProductHeadSaved", "SkuHeadSaved"];

/// The per-tenant lock key.
fn lock_key(tenant_id: Uuid) -> String {
    format!("taxonomy:{tenant_id}")
}

/// Take the tenant's writer lock, **waiting** for a peer that holds it.
///
/// `DBProvider::lock` is a *try*: it answers `Lock already held` the instant
/// another guard on the process's shared session claims the key, which is a
/// refusal and not serialization. `inst-tc-writer-lock` asks for taxonomy
/// mutations to *"serialize per tenant"*, so this is `try_lock` under the
/// toolkit's own retry policy — measured by
/// `the_lock_is_what_refuses_the_second_reparent`, which reddened on `lock`
/// with the peer's own error rather than with the cycle verdict it exists to
/// assert.
///
/// Exhausting the budget is a contention failure, not a domain refusal: the
/// act was never judged, so it must not answer as though it had been.
async fn take_writer_lock(
    db: &DBProvider<DbError>,
    tenant_id: Uuid,
) -> Result<toolkit_db::DbLockGuard, RepoError> {
    let key = lock_key(tenant_id);
    match db
        .db()
        .try_lock(LOCK_GEAR, &key, toolkit_db::LockConfig::default())
        .await
    {
        Ok(Some(guard)) => Ok(guard),
        Ok(None) => Err(RepoError::Db(format!(
            "taxonomy writer lock {key} was held for the whole acquisition budget: the mutation \
             was never judged, so it is not refused"
        ))),
        Err(e) => Err(RepoError::Db(format!("taxonomy writer lock {key}: {e}"))),
    }
}

/// The transactions' error channel, for `transaction_with_retry`.
///
/// A refusal is an **error** here on purpose: returning `Err(Refused)` rolls
/// the transaction back — nothing was announced for an act that did not
/// happen — and [`settle`] turns it back into the `Ok(Err(refusal))` the door
/// audits. See the module doc.
enum TaxonomyTxError {
    Refused(DomainError),
    Repo(RepoError),
    Events(events::EventsError),
}

impl From<DbError> for TaxonomyTxError {
    fn from(error: DbError) -> Self {
        Self::Repo(RepoError::Db(error.to_string()))
    }
}

impl From<RepoError> for TaxonomyTxError {
    fn from(error: RepoError) -> Self {
        Self::Repo(error)
    }
}

/// The retryable-contention extractor, mirroring
/// `recognized_sets::member_contention_db_err`: only a driver error can be
/// contention. A refusal is a decided answer and an outbox failure is not a
/// row conflict.
fn taxonomy_contention_db_err(error: &TaxonomyTxError) -> Option<&sea_orm::DbErr> {
    match error {
        TaxonomyTxError::Repo(RepoError::Driver { source, .. }) => Some(source),
        TaxonomyTxError::Repo(_) | TaxonomyTxError::Refused(_) | TaxonomyTxError::Events(_) => None,
    }
}

/// Fold a transaction's outcome into the shape every door here reads:
/// storage failures outside, refusals inside.
fn settle<T>(outcome: Result<T, TaxonomyTxError>) -> Result<Result<T, DomainError>, RepoError> {
    match outcome {
        Ok(value) => Ok(Ok(value)),
        Err(TaxonomyTxError::Refused(refusal)) => Ok(Err(refusal)),
        Err(TaxonomyTxError::Repo(repo)) => Err(repo),
        // An event that could not be enqueued rolled the act back; the door
        // renders it as the storage failure it is — the split
        // `dod-create-doors` exists to prevent is an act with no announcement.
        Err(TaxonomyTxError::Events(e)) => Err(RepoError::Db(format!("taxonomy event: {e}"))),
    }
}

/// Announce one tree act on [`TAXONOMY_TREE_AGGREGATE`], inside the caller's
/// transaction.
#[allow(clippy::too_many_arguments)]
async fn announce_tree_act(
    sink: &EventSink,
    tx: &(impl DBRunner + Sync),
    tenant_id: Uuid,
    category_id: Uuid,
    payload_type: &str,
    act: &'static str,
    state: &'static str,
    operation_kind: Option<&'static str>,
    actor_ref: Uuid,
) -> Result<(), TaxonomyTxError> {
    events::enqueue_taxonomy(
        sink,
        tx,
        TAXONOMY_TREE_AGGREGATE,
        payload_type,
        &TaxonomyEventBody {
            tenant_id,
            entity_kind: "category",
            entity_id: category_id,
            act,
            state,
            mutation_seq: None,
            operation_kind,
        },
        actor_ref,
    )
    .await
    .map_err(TaxonomyTxError::Events)
}

/// Re-parent one category under the writer lock, refusing a cycle, and
/// announce it in the same transaction.
///
/// The order is the instruction's: take the lock, **then** read the tree,
/// **then** judge, **then** write. Reading before the lock would judge a
/// chain a peer can still change, which is the whole defect the lock exists
/// to close — and it is invisible to every single-writer test.
///
/// # Errors
///
/// [`DomainError::TaxonomyCycle`] when the new parent's chain contains the
/// node; [`DomainError::DuplicateCategoryName`] when the node's name is
/// taken in the new sibling set. [`RepoError`] on a lock, storage or scope
/// failure.
#[allow(clippy::too_many_arguments)]
pub async fn reparent_under_lock(
    db: &DBProvider<DbError>,
    sink: &EventSink,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    category_id: Uuid,
    new_parent: Option<Uuid>,
    limits: TaxonomyLimits,
    now: DateTime<Utc>,
    authorization: &crate::domain::governance::GateAuthorization,
) -> Result<Result<repo::CategoryWrite, DomainError>, RepoError> {
    let _guard = take_writer_lock(db, tenant_id).await?;
    let conn = db
        .conn()
        .map_err(|e| RepoError::Db(format!("taxonomy connection: {e}")))?;

    // Read the tree **once** under the lock; both verdicts judge that read.
    let edges = repo::category_parents(&conn, scope, tenant_id).await?;
    if let Some(parent) = new_parent {
        let parent_of = |id: Uuid| {
            edges
                .iter()
                .find(|(node, _)| *node == id)
                .and_then(|(_, p)| *p)
        };
        if let Err(refusal) = cycle_verdict(category_id, &ancestors_of(parent, &parent_of)) {
            return Ok(Err(refusal));
        }
    }

    // Judged on the same read, after the cycle rule: a chain that closes on
    // itself has no meaningful depth, so the cycle verdict must answer first.
    if let Err(refusal) = limits_verdict_for(Some(category_id), new_parent, &edges, limits) {
        return Ok(Err(refusal));
    }

    let sink = sink.clone();
    let scope_tx = scope.clone();
    let authorization_tx = authorization.clone();
    settle(
        db.db()
            .transaction_with_retry::<repo::CategoryWrite, TaxonomyTxError, _, _>(
                TxConfig::default(),
                taxonomy_contention_db_err,
                move |tx| {
                    let authorization = authorization_tx.clone();
                    let sink = sink.clone();
                    let scope = scope_tx.clone();
                    Box::pin(async move {
                        // The one-shot rides the op's own transaction
                        // (`inst-gv-one-shot`): spent where the write commits.
                        repo::settle_authorization(tx, &scope, tenant_id, &authorization, now)
                            .await
                            .map_err(|error| match error {
                                repo::SettleError::Refused(refusal) => {
                                    TaxonomyTxError::Refused(refusal)
                                }
                                repo::SettleError::Repo(error) => TaxonomyTxError::Repo(error),
                            })?;
                        let written = repo::reparent_category(
                            tx,
                            &scope,
                            tenant_id,
                            category_id,
                            new_parent,
                            now,
                        )
                        .await?
                        .map_err(TaxonomyTxError::Refused)?;
                        if matches!(written, repo::CategoryWrite::Applied) {
                            announce_tree_act(
                                &sink,
                                tx,
                                tenant_id,
                                category_id,
                                events::CATEGORY_REPARENTED_PAYLOAD_TYPE,
                                "reparented",
                                "active",
                                Some("category.reparent"),
                                actor_ref,
                            )
                            .await?;
                        }
                        Ok(written)
                    })
                },
            )
            .await,
    )
}

/// Rename one category under the writer lock, and announce it in the same
/// transaction.
///
/// The rename cannot close a cycle, so there is no walk — but it takes the
/// same lock: `inst-tc-writer-lock` serializes *"taxonomy mutations"*, and a
/// rename racing a re-parent is exactly how a chain moves under a walk that
/// already ran.
///
/// # Errors
///
/// [`DomainError::DuplicateCategoryName`] on a collision; [`RepoError`] on a
/// lock, storage or scope failure.
#[allow(clippy::too_many_arguments)]
pub async fn rename_under_lock(
    db: &DBProvider<DbError>,
    sink: &EventSink,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    category_id: Uuid,
    name: &str,
    now: DateTime<Utc>,
    authorization: &crate::domain::governance::GateAuthorization,
) -> Result<Result<repo::CategoryWrite, DomainError>, RepoError> {
    let _guard = take_writer_lock(db, tenant_id).await?;
    let normalized = crate::domain::name::normalize(name);
    let name = name.to_owned();
    let sink = sink.clone();
    let scope_tx = scope.clone();
    let authorization_tx = authorization.clone();
    settle(
        db.db()
            .transaction_with_retry::<repo::CategoryWrite, TaxonomyTxError, _, _>(
                TxConfig::default(),
                taxonomy_contention_db_err,
                move |tx| {
                    let authorization = authorization_tx.clone();
                    let sink = sink.clone();
                    let scope = scope_tx.clone();
                    let name = name.clone();
                    let normalized = normalized.clone();
                    Box::pin(async move {
                        // The one-shot rides the op's own transaction
                        // (`inst-gv-one-shot`): spent where the write commits.
                        repo::settle_authorization(tx, &scope, tenant_id, &authorization, now)
                            .await
                            .map_err(|error| match error {
                                repo::SettleError::Refused(refusal) => {
                                    TaxonomyTxError::Refused(refusal)
                                }
                                repo::SettleError::Repo(error) => TaxonomyTxError::Repo(error),
                            })?;
                        let written = repo::rename_category(
                            tx,
                            &scope,
                            tenant_id,
                            category_id,
                            &name,
                            &normalized,
                            now,
                        )
                        .await?
                        .map_err(TaxonomyTxError::Refused)?;
                        if matches!(written, repo::CategoryWrite::Applied) {
                            announce_tree_act(
                                &sink,
                                tx,
                                tenant_id,
                                category_id,
                                events::CATEGORY_RENAMED_PAYLOAD_TYPE,
                                "renamed",
                                "active",
                                Some("category.rename"),
                                actor_ref,
                            )
                            .await?;
                        }
                        Ok(written)
                    })
                },
            )
            .await,
    )
}

/// Judge a node's landing place against the configured ceilings.
///
/// Both rules read the **one** edge list the lock already bought, so the
/// cycle verdict, the depth rule and the fan-out rule cannot disagree about
/// the tree they judged.
///
/// `node` is `None` for a create, where nothing is being dragged, and
/// `Some(id)` for a re-parent, where the moved subtree's own height counts:
/// a limit of eight is broken at the **leaves** of a three-level subtree
/// landing at depth six, and a rule reading the moved node alone admits it.
fn limits_verdict_for(
    node: Option<Uuid>,
    new_parent: Option<Uuid>,
    edges: &[(Uuid, Option<Uuid>)],
    limits: TaxonomyLimits,
) -> Result<(), DomainError> {
    let parent_of = |id: Uuid| {
        edges
            .iter()
            .find(|(child, _)| *child == id)
            .and_then(|(_, p)| *p)
    };
    // A root sits at depth 0, so a child of `parent` sits one below it.
    let landing = new_parent.map_or(0, |p| depth_of(p, &parent_of).saturating_add(1));
    let deepest = node.map_or(landing, |id| {
        landing.saturating_add(subtree_height(id, edges))
    });
    // The count the mutation **would make it**, which is what
    // `TaxonomyLimitExceeded::measured`'s own doc says the field holds — the
    // sibling set gains this node unless it is already in it, a re-parent to
    // the parent a node already has being a no-op that must not be refused
    // for growing a set it does not grow. Passing the *current* count instead
    // made the effective ceiling one higher than the configured one, and the
    // fan-out door case is what caught it.
    let already_there = node.is_some_and(|id| {
        edges
            .iter()
            .any(|(child, parent)| *child == id && *parent == new_parent)
    });
    let siblings = crate::domain::taxonomy::children_of(new_parent, edges)
        .saturating_add(u32::from(!already_there));
    limit_verdict(deepest, siblings, limits).map_err(|exceeded| {
        // `TAXONOMY_LIMIT` is one of the ten of this slice's sixteen codes
        // with no `DomainError` variant, so it reaches the wire the way the
        // pipeline's own codes do: as a violation carrying its code, which
        // `error_mapping`'s `Validation` arm renders as the wire `type`.
        let mut report = ValidationReport::new();
        report.violate(
            TaxonomyLimitExceeded::CODE,
            "parentId",
            format!(
                "{} is {}, and the configured ceiling is {}",
                exceeded.limit, exceeded.measured, exceeded.allowed
            ),
        );
        DomainError::Validation(report)
    })
}

/// Create one category under the writer lock, refusing a limit breach, and
/// announce it in the same transaction.
///
/// The same order as [`reparent_under_lock`] and for the same reason: lock,
/// read, judge, write. A create cannot close a cycle — it has no descendants
/// yet — so the walk is the depth and fan-out rules only.
///
/// # Errors
///
/// [`crate::domain::taxonomy::TaxonomyLimitExceeded`] (`TAXONOMY_LIMIT`, as a
/// `Validation` violation) when the landing place breaks a configured
/// ceiling, [`DomainError::DuplicateCategoryName`] on a name already taken in
/// the sibling set. [`RepoError`] on a lock, storage or scope failure.
pub async fn create_under_lock(
    db: &DBProvider<DbError>,
    sink: &EventSink,
    scope: &AccessScope,
    new: repo::NewCategory<'_>,
    actor_ref: Uuid,
    limits: TaxonomyLimits,
    now: DateTime<Utc>,
) -> Result<Result<(), DomainError>, RepoError> {
    let tenant_id = new.tenant_id;
    let _guard = take_writer_lock(db, tenant_id).await?;
    let conn = db
        .conn()
        .map_err(|e| RepoError::Db(format!("taxonomy connection: {e}")))?;

    let edges = repo::category_parents(&conn, scope, tenant_id).await?;
    if let Err(refusal) = limits_verdict_for(None, new.parent_id, &edges, limits) {
        return Ok(Err(refusal));
    }

    // Owned copies: the closure below runs once per attempt and may not
    // borrow from this frame (see the module doc).
    let category_id = new.category_id;
    let parent_id = new.parent_id;
    let name = new.name.to_owned();
    let normalized = new.name_normalized.to_owned();
    let sink = sink.clone();
    let scope_tx = scope.clone();
    settle(
        db.db()
            .transaction_with_retry::<(), TaxonomyTxError, _, _>(
                TxConfig::default(),
                taxonomy_contention_db_err,
                move |tx| {
                    let sink = sink.clone();
                    let scope = scope_tx.clone();
                    let name = name.clone();
                    let normalized = normalized.clone();
                    Box::pin(async move {
                        repo::insert_category(
                            tx,
                            &scope,
                            repo::NewCategory {
                                tenant_id,
                                category_id,
                                parent_id,
                                name: &name,
                                name_normalized: &normalized,
                            },
                            now,
                        )
                        .await?
                        .map_err(TaxonomyTxError::Refused)?;
                        announce_tree_act(
                            &sink,
                            tx,
                            tenant_id,
                            category_id,
                            events::CATEGORY_CREATED_PAYLOAD_TYPE,
                            "created",
                            "active",
                            None,
                            actor_ref,
                        )
                        .await
                    })
                },
            )
            .await,
    )
}

/// Retire one category under the writer lock, refusing a referenced node, and
/// announce it in the same transaction.
///
/// The census and the retire share the lock so the count a refusal reports
/// and the state a success writes are the same instant's: a product assigned
/// between an unlocked census and its retire would leave a retired category
/// holding a live reference, which is exactly what the guard exists to
/// prevent.
///
/// # Errors
///
/// [`DomainError::CategoryReferenced`] (`CATEGORY_REFERENCED`, **409** —
/// `design/02` §3.3's status; it rode `Validation` as a 400 until 2026-09-03)
/// when a non-terminal product or an active child still holds the node.
/// [`RepoError`] on a lock, storage or scope failure.
#[allow(clippy::too_many_arguments)]
pub async fn retire_under_lock(
    db: &DBProvider<DbError>,
    sink: &EventSink,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    category_id: Uuid,
    sample: u64,
    now: DateTime<Utc>,
    authorization: &crate::domain::governance::GateAuthorization,
) -> Result<Result<repo::CategoryWrite, DomainError>, RepoError> {
    let _guard = take_writer_lock(db, tenant_id).await?;
    let conn = db
        .conn()
        .map_err(|e| RepoError::Db(format!("taxonomy connection: {e}")))?;

    let census: RetireCensus =
        repo::retire_census(&conn, scope, tenant_id, category_id, sample).await?;
    if let Err(referenced) = retire_verdict(&census) {
        return Ok(Err(DomainError::from(referenced)));
    }
    let sink = sink.clone();
    let scope_tx = scope.clone();
    let authorization_tx = authorization.clone();
    settle(
        db.db()
            .transaction_with_retry::<repo::CategoryWrite, TaxonomyTxError, _, _>(
                TxConfig::default(),
                taxonomy_contention_db_err,
                move |tx| {
                    let authorization = authorization_tx.clone();
                    let sink = sink.clone();
                    let scope = scope_tx.clone();
                    Box::pin(async move {
                        // The one-shot rides the op's own transaction
                        // (`inst-gv-one-shot`): spent where the write commits.
                        repo::settle_authorization(tx, &scope, tenant_id, &authorization, now)
                            .await
                            .map_err(|error| match error {
                                repo::SettleError::Refused(refusal) => {
                                    TaxonomyTxError::Refused(refusal)
                                }
                                repo::SettleError::Repo(error) => TaxonomyTxError::Repo(error),
                            })?;
                        let written =
                            repo::retire_category(tx, &scope, tenant_id, category_id, now).await?;
                        if matches!(written, repo::CategoryWrite::Applied) {
                            announce_tree_act(
                                &sink,
                                tx,
                                tenant_id,
                                category_id,
                                events::CATEGORY_RETIRED_PAYLOAD_TYPE,
                                "retired",
                                "retired",
                                Some("category.retire"),
                                actor_ref,
                            )
                            .await?;
                        }
                        Ok(written)
                    })
                },
            )
            .await,
    )
}

/// Delete one **retired** category under the writer lock, refusing a node
/// with history, and announce it in the same transaction.
///
/// **The delete has its own census, and it reads presence** (**P-D-116 row
/// 21**): any `products_product_category` row naming the node, in any Product
/// state, and any child row, in any state, refuse it — `CATEGORY_REFERENCED`,
/// naming a sample. That is the opposite operand from the retire's, on
/// purpose: a discarded draft's link row must not lock a category out of
/// `retired`, and a delete must not leave that row pointing at nothing in the
/// table the design calls the single source of truth. A category with history
/// is retired, never deleted. Until 2026-09-03 this function ran **no** census
/// and the parent foreign key met the act as a storage error — a 500 for what
/// the design files as a 409.
///
/// The store's own statement still carries the `retired` predicate, so a live
/// category answers `Unmatched` rather than being deleted; the lock is here
/// because a delete racing a re-parent would remove a node a walk had just
/// judged, and the census shares it so the count a refusal reports is the
/// same instant's as the row it protects.
///
/// # Errors
///
/// [`DomainError::CategoryReferenced`] when any link row or child row still
/// names the node. [`RepoError`] on a lock, storage or scope failure.
#[allow(clippy::too_many_arguments)]
pub async fn delete_under_lock(
    db: &DBProvider<DbError>,
    sink: &EventSink,
    scope: &AccessScope,
    tenant_id: Uuid,
    actor_ref: Uuid,
    category_id: Uuid,
    sample: u64,
    now: DateTime<Utc>,
    authorization: &crate::domain::governance::GateAuthorization,
) -> Result<Result<repo::CategoryWrite, DomainError>, RepoError> {
    let _guard = take_writer_lock(db, tenant_id).await?;
    let conn = db
        .conn()
        .map_err(|e| RepoError::Db(format!("taxonomy connection: {e}")))?;

    let census: DeleteCensus =
        repo::delete_census(&conn, scope, tenant_id, category_id, sample).await?;
    if let Err(referenced) = delete_verdict(&census) {
        return Ok(Err(DomainError::from(referenced)));
    }
    let sink = sink.clone();
    let scope_tx = scope.clone();
    let authorization_tx = authorization.clone();
    settle(
        db.db()
            .transaction_with_retry::<repo::CategoryWrite, TaxonomyTxError, _, _>(
                TxConfig::default(),
                taxonomy_contention_db_err,
                move |tx| {
                    let authorization = authorization_tx.clone();
                    let sink = sink.clone();
                    let scope = scope_tx.clone();
                    Box::pin(async move {
                        // The one-shot rides the op's own transaction
                        // (`inst-gv-one-shot`): spent where the write commits.
                        repo::settle_authorization(tx, &scope, tenant_id, &authorization, now)
                            .await
                            .map_err(|error| match error {
                                repo::SettleError::Refused(refusal) => {
                                    TaxonomyTxError::Refused(refusal)
                                }
                                repo::SettleError::Repo(error) => TaxonomyTxError::Repo(error),
                            })?;
                        let written =
                            repo::delete_retired_category(tx, &scope, tenant_id, category_id)
                                .await?;
                        if matches!(written, repo::CategoryWrite::Applied) {
                            announce_tree_act(
                                &sink,
                                tx,
                                tenant_id,
                                category_id,
                                events::CATEGORY_DELETED_PAYLOAD_TYPE,
                                "deleted",
                                "deleted",
                                Some("category.delete"),
                                actor_ref,
                            )
                            .await?;
                        }
                        Ok(written)
                    })
                },
            )
            .await,
    )
}

#[cfg(test)]
#[path = "taxonomy_tests.rs"]
mod taxonomy_tests;
