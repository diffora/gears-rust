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
//! [`TAXONOMY_TREE_AGGREGATE`] and [`metadata_aggregate`] are those two keys.
//! Neither is spent yet: `infra::events` declares no payload type for any of
//! the eight, so this module states the keys and the patch that adds the
//! types is handed to the surface's owner.
//!
//! **The two events that have no aggregate are not given one here.** §7 row
//! 15 asks which aggregate orders `CategoryDisplayUpdated` and
//! `AttributeDefinitionUpdated`, and says why it is not a free choice:
//! *"display writes do not take the taxonomy writer lock, so the tree key
//! would claim a serialization the door does not provide"*. Putting them on
//! [`TAXONOMY_TREE_AGGREGATE`] would be that claim, made from this module.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-taxonomy-writer-lock:p1

use chrono::{DateTime, Utc};
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::taxonomy::{ancestors_of, cycle_verdict};
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

/// Re-parent one category under the writer lock, refusing a cycle.
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
pub async fn reparent_under_lock(
    db: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
    new_parent: Option<Uuid>,
    now: DateTime<Utc>,
) -> Result<Result<repo::CategoryWrite, DomainError>, RepoError> {
    let _guard = take_writer_lock(db, tenant_id).await?;

    let conn = db
        .conn()
        .map_err(|e| RepoError::Db(format!("taxonomy connection: {e}")))?;

    // Read the tree under the lock, and judge the chain it gives.
    if let Some(parent) = new_parent {
        let edges = repo::category_parents(&conn, scope, tenant_id).await?;
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

    repo::reparent_category(&conn, scope, tenant_id, category_id, new_parent, now).await
}

/// Rename one category under the writer lock.
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
pub async fn rename_under_lock(
    db: &DBProvider<DbError>,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
    name: &str,
    now: DateTime<Utc>,
) -> Result<Result<repo::CategoryWrite, DomainError>, RepoError> {
    let _guard = take_writer_lock(db, tenant_id).await?;
    let conn = db
        .conn()
        .map_err(|e| RepoError::Db(format!("taxonomy connection: {e}")))?;
    let normalized = crate::domain::name::normalize(name);
    repo::rename_category(&conn, scope, tenant_id, category_id, name, &normalized, now).await
}

#[cfg(test)]
#[path = "taxonomy_tests.rs"]
mod taxonomy_tests;
