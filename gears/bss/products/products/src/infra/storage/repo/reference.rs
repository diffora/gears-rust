//! The reference-signal store — the producer registry, the watermark head
//! and its member set (`design/07`, P-D-71, P-D-87).
//!
//! Split out of the foundation repository move-only; every item re-exports
//! through `super` (`crate::infra::storage::repo`) unchanged.
use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use crate::domain::states::ProducerState;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{reference_member, reference_producer, reference_watermark};

use super::driver_failure;

/// One registered producer, as the predicate and the snapshot read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceProducerRecord {
    /// The producer's own name.
    pub producer: String,
    /// The registry's state, typed at the storage boundary.
    pub state: ProducerState,
}

/// Every producer of one tenant, name order — the predicate quantifies over
/// the `registered` ones and the capture store snapshots them.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn reference_producers(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<ReferenceProducerRecord>, RepoError> {
    let rows = reference_producer::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(reference_producer::Column::TenantId.eq(tenant_id)))
        .order_by(reference_producer::Column::Producer, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the producer set of {tenant_id}"), e))?;
    rows.into_iter()
        .map(|row| {
            let state = ProducerState::parse(&row.state).ok_or_else(|| {
                RepoError::CorruptRow(format!(
                    "producer {:?} of {tenant_id} carries state {:?} outside the roster",
                    row.producer, row.state
                ))
            })?;
            Ok(ReferenceProducerRecord {
                producer: row.producer,
                state,
            })
        })
        .collect()
}

/// Register a producer, or report that the name is already held. A
/// **re-registration** of a retired producer flips its state back and, by
/// P-D-87 arm 2, finds no watermark to inherit — the retirement cleared it,
/// so onboarding can only tighten.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn register_reference_producer(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    producer: &str,
    ceremony_ref: Option<Uuid>,
    registered_at: DateTime<Utc>,
) -> Result<(), RepoError> {
    let existing = reference_producer::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(reference_producer::Column::TenantId.eq(tenant_id))
                .add(reference_producer::Column::Producer.eq(producer)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read producer {producer}"), e))?;

    if existing.is_some() {
        reference_producer::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(
                reference_producer::Column::State,
                Expr::value(ProducerState::Registered.as_str().to_owned()),
            )
            .col_expr(
                reference_producer::Column::RegisteredAt,
                Expr::value(registered_at),
            )
            .filter(
                Condition::all()
                    .add(reference_producer::Column::TenantId.eq(tenant_id))
                    .add(reference_producer::Column::Producer.eq(producer)),
            )
            .exec(runner)
            .await
            .map_err(|e| driver_failure(format!("re-register producer {producer}"), e))?;
        return Ok(());
    }

    let model = reference_producer::ActiveModel {
        tenant_id: Set(tenant_id),
        producer: Set(producer.to_owned()),
        state: Set(ProducerState::Registered.as_str().to_owned()),
        registered_at: Set(registered_at),
        ceremony_ref: Set(ceremony_ref),
        declaration_payload: Set(None),
    };
    reference_producer::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("producer scope of {tenant_id}"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("register producer {producer}"), e))?;
    Ok(())
}

/// Retire a producer and **clear its watermark and member rows in the same
/// transaction** (**P-D-87** arm 2): surviving rows would let
/// retire-then-re-register inside the freshness window read fresh against a
/// stale set and free every SKU that has since gained a reference.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn retire_reference_producer(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    producer: &str,
) -> Result<(), RepoError> {
    reference_producer::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            reference_producer::Column::State,
            Expr::value(ProducerState::Retired.as_str().to_owned()),
        )
        .filter(
            Condition::all()
                .add(reference_producer::Column::TenantId.eq(tenant_id))
                .add(reference_producer::Column::Producer.eq(producer)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("retire producer {producer}"), e))?;
    clear_reference_watermark(runner, scope, tenant_id, producer).await
}

/// Delete one producer's watermark head and every member row under it.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn clear_reference_watermark(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    producer: &str,
) -> Result<(), RepoError> {
    reference_member::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(reference_member::Column::TenantId.eq(tenant_id))
                .add(reference_member::Column::Producer.eq(producer)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("clear members of {producer}"), e))?;
    reference_watermark::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(reference_watermark::Column::TenantId.eq(tenant_id))
                .add(reference_watermark::Column::Producer.eq(producer)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("clear the watermark of {producer}"), e))?;
    Ok(())
}

/// One producer's posted watermark, read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferenceWatermarkRecord {
    /// The instant the set is complete as of.
    pub watermark_at: DateTime<Utc>,
    /// The hex digest of the posted set (P-D-71).
    pub set_hash: String,
}

/// Read one producer's watermark head.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_reference_watermark(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    producer: &str,
) -> Result<Option<ReferenceWatermarkRecord>, RepoError> {
    let row = reference_watermark::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(reference_watermark::Column::TenantId.eq(tenant_id))
                .add(reference_watermark::Column::Producer.eq(producer)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read the watermark of {producer}"), e))?;
    Ok(row.map(|row| ReferenceWatermarkRecord {
        watermark_at: row.watermark_at,
        set_hash: row.set_hash,
    }))
}

/// One posted watermark, as the door hands it to the repository.
#[derive(Clone, Copy, Debug)]
pub struct PostedWatermark<'a> {
    /// The posting producer.
    pub producer: &'a str,
    /// The instant the set is complete as of.
    pub watermark_at: DateTime<Utc>,
    /// When the post arrived.
    pub posted_at: DateTime<Utc>,
    /// The hex digest of the set (P-D-71).
    pub set_hash: &'a str,
    /// The complete SKU set.
    pub members: &'a [Uuid],
}

/// Replace one producer's watermark and its whole member set — the post's
/// write, run inside the door's transaction so the head and the set can
/// never disagree.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn post_reference_watermark(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    post: PostedWatermark<'_>,
) -> Result<(), RepoError> {
    let PostedWatermark {
        producer,
        watermark_at,
        posted_at,
        set_hash,
        members,
    } = post;
    clear_reference_watermark(runner, scope, tenant_id, producer).await?;
    let head = reference_watermark::ActiveModel {
        tenant_id: Set(tenant_id),
        producer: Set(producer.to_owned()),
        watermark_at: Set(watermark_at),
        posted_at: Set(posted_at),
        set_hash: Set(set_hash.to_owned()),
    };
    reference_watermark::Entity::insert(head.clone())
        .secure()
        .scope_with_model(scope, &head)
        .map_err(|e| driver_failure(format!("watermark scope of {tenant_id}"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("post the watermark of {producer}"), e))?;
    // One statement per bind-budget chunk rather than one round-trip per
    // SKU: the contract mandates the producer's complete set, so a
    // realistic post carries thousands of ids, and per-row INSERTs would
    // stretch the door's transaction with N sequential round-trips.
    let rows: Vec<reference_member::ActiveModel> = members
        .iter()
        .map(|sku_id| reference_member::ActiveModel {
            tenant_id: Set(tenant_id),
            producer: Set(producer.to_owned()),
            sku_id: Set(*sku_id),
        })
        .collect();
    if !rows.is_empty() {
        toolkit_db::secure::secure_insert_many::<reference_member::Entity>(rows, scope, runner)
            .await
            .map_err(|e| driver_failure(format!("post the members of {producer}"), e))?;
    }
    Ok(())
}

/// Whether one producer's posted set contains one SKU — the predicate's
/// per-producer operand.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn reference_member_exists(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    producer: &str,
    sku_id: Uuid,
) -> Result<bool, RepoError> {
    let row = reference_member::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(reference_member::Column::TenantId.eq(tenant_id))
                .add(reference_member::Column::Producer.eq(producer))
                .add(reference_member::Column::SkuId.eq(sku_id)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read member {sku_id} of {producer}"), e))?;
    Ok(row.is_some())
}
