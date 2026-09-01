//! The increment queue and the version-insert machinery — the request rows
//! the coalescer drains, the gapless counter, the snapshot reads and the
//! catalog-version writes its transaction is made of (`design/06`).
//!
//! Split out of the foundation repository move-only; every item re-exports
//! through `super` (`crate::infra::storage::repo`) unchanged.
use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{ColumnTrait, Condition, DbErr, EntityTrait, QuerySelect};
use toolkit_db::secure::{
    AccessScope, DBRunner, ScopeError, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    catalog_version, catalog_version_capture, catalog_version_counter, catalog_version_entry,
    catalog_version_request, freeze_ack, freeze_participant, product, sku,
};

use super::{TenantIdRow, driver_failure};

/// One increment request as the door writes it — `requested_at` already
/// stamped, the state the insert's own (`pending`).
#[derive(Clone, Copy, Debug)]
pub struct NewIncrementRequest<'a> {
    /// The registered requester.
    pub source: &'a str,
    /// The caller's idempotency handle.
    pub request_key: &'a str,
    /// `interactive` or `bulk`.
    pub lane: &'a str,
    /// The bulk batch key, absent on the interactive lane.
    pub operation_key: Option<&'a str>,
    /// The door's ingress stamp.
    pub requested_at: DateTime<Utc>,
}

/// One row of the increment queue, in this repository's vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IncrementRequestRecord {
    /// `pending` or `coalesced`.
    pub state: String,
    /// The satisfying version, present exactly when `coalesced`.
    pub satisfied_by_version_id: Option<i64>,
}

/// The request door's write (`inst-cv-request`): `INSERT` the request, or
/// report the row the key already holds — the queue's own UNIQUE **is** the
/// idempotency, so no `products_idempotency` claim participates.
///
/// Answers the row's current state either way: a fresh insert is `pending`,
/// a replay answers whatever the coalescer has made of it.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, or a held row that vanished
/// between the conflict and the read-back — the store contradicting itself.
pub async fn enqueue_increment_request(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    new: NewIncrementRequest<'_>,
) -> Result<IncrementRequestRecord, RepoError> {
    let NewIncrementRequest {
        source,
        request_key,
        lane,
        operation_key,
        requested_at,
    } = new;
    let model = catalog_version_request::ActiveModel {
        tenant_id: Set(tenant_id),
        source: Set(source.to_owned()),
        request_key: Set(request_key.to_owned()),
        lane: Set(lane.to_owned()),
        operation_key: Set(operation_key.map(str::to_owned)),
        requested_at: Set(requested_at),
        state: Set("pending".to_owned()),
        satisfied_by_version_id: Set(None),
    };

    let on_conflict = OnConflict::columns([
        catalog_version_request::Column::TenantId,
        catalog_version_request::Column::Source,
        catalog_version_request::Column::RequestKey,
    ])
    .do_nothing()
    .to_owned();

    match catalog_version_request::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| {
            driver_failure(
                format!("increment request {tenant_id}/{source}/{request_key} scope"),
                e,
            )
        })?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
    {
        Ok(_) => {
            return Ok(IncrementRequestRecord {
                state: "pending".to_owned(),
                satisfied_by_version_id: None,
            });
        }
        // The key is already held; the replay answers the stored row.
        Err(ScopeError::Db(DbErr::RecordNotInserted)) => {}
        Err(e) => {
            return Err(driver_failure(
                format!("increment request {tenant_id}/{source}/{request_key}"),
                e,
            ));
        }
    }

    find_increment_request(runner, scope, tenant_id, source, request_key)
        .await?
        .ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "increment request {tenant_id}/{source}/{request_key} conflicted on insert \
                 but no row remained to read"
            ))
        })
}

/// Read one increment request by its full key — the poll's operand
/// (P-D-81 arm 3).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_increment_request(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    source: &str,
    request_key: &str,
) -> Result<Option<IncrementRequestRecord>, RepoError> {
    let row = catalog_version_request::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version_request::Column::TenantId.eq(tenant_id))
                .add(catalog_version_request::Column::Source.eq(source))
                .add(catalog_version_request::Column::RequestKey.eq(request_key)),
        )
        .one(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("read increment request {tenant_id}/{source}/{request_key}"),
                e,
            )
        })?;
    Ok(row.map(|row| IncrementRequestRecord {
        state: row.state,
        satisfied_by_version_id: row.satisfied_by_version_id,
    }))
}

/// One pending demand row, as the coalescer reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingIncrementRequest {
    /// The registered requester.
    pub source: String,
    /// The caller's idempotency handle.
    pub request_key: String,
    /// `interactive` or `bulk`.
    pub lane: String,
    /// The bulk batch key.
    pub operation_key: Option<String>,
    /// The door's ingress stamp — the window arithmetic's zero point.
    pub requested_at: DateTime<Utc>,
}

/// Every tenant holding at least one `pending` increment request — the
/// sweep's discovery read, under the system scope
/// (`AccessScope::allow_all`), narrowed to `for_tenant` before any
/// per-tenant work (the sibling pricing jobs' own documented pattern).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn tenants_with_pending_requests(
    runner: &impl DBRunner,
    scope: &AccessScope,
) -> Result<Vec<Uuid>, RepoError> {
    // A DISTINCT projection, not a full-queue fetch: this discovery read
    // runs once per coalescer tick, and during a bulk window one tenant can
    // hold thousands of pending rows — the sweep needs the handful of
    // distinct tenant ids, never the rows themselves.
    let rows: Vec<TenantIdRow> = catalog_version_request::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(catalog_version_request::Column::State.eq("pending")))
        .project_all(runner, |q| {
            q.select_only()
                .column(catalog_version_request::Column::TenantId)
                .distinct()
                .into_model::<TenantIdRow>()
        })
        .await
        .map_err(|e| {
            driver_failure(
                "discover tenants with pending increment demand".to_owned(),
                e,
            )
        })?;
    let mut tenants: Vec<Uuid> = rows.into_iter().map(|row| row.tenant_id).collect();
    tenants.sort();
    Ok(tenants)
}

/// One tenant's `pending` demand, oldest first — the coalescer's window
/// operand.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn pending_increment_requests(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<PendingIncrementRequest>, RepoError> {
    let rows = catalog_version_request::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version_request::Column::TenantId.eq(tenant_id))
                .add(catalog_version_request::Column::State.eq("pending")),
        )
        .order_by(
            catalog_version_request::Column::RequestedAt,
            sea_orm::Order::Asc,
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read pending demand of {tenant_id}"), e))?;
    Ok(rows
        .into_iter()
        .map(|row| PendingIncrementRequest {
            source: row.source,
            request_key: row.request_key,
            lane: row.lane,
            operation_key: row.operation_key,
            requested_at: row.requested_at,
        })
        .collect())
}

/// Allocate the next `catalog_version_id` for `tenant_id` — gapless
/// because this update and the version insert share the increment
/// transaction, and race-free because the per-tenant lease serializes the
/// only writer (`inst-cvc-serial`, P-D-53).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, or a counter that moved
/// under the lease — a second writer, which must not be absorbed silently.
pub async fn allocate_catalog_version_id(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<i64, RepoError> {
    let held = catalog_version_counter::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(catalog_version_counter::Column::TenantId.eq(tenant_id)))
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read the version counter of {tenant_id}"), e))?;

    match held {
        None => {
            let model = catalog_version_counter::ActiveModel {
                tenant_id: Set(tenant_id),
                next_id: Set(2),
            };
            catalog_version_counter::Entity::insert(model.clone())
                .secure()
                .scope_with_model(scope, &model)
                .map_err(|e| driver_failure(format!("counter scope of {tenant_id}"), e))?
                .exec(runner)
                .await
                .map_err(|e| {
                    driver_failure(format!("seed the version counter of {tenant_id}"), e)
                })?;
            Ok(1)
        }
        Some(row) => {
            let allocated = row.next_id;
            let result = catalog_version_counter::Entity::update_many()
                .secure()
                .scope_with(scope)
                .col_expr(
                    catalog_version_counter::Column::NextId,
                    Expr::value(allocated + 1),
                )
                .filter(
                    Condition::all()
                        .add(catalog_version_counter::Column::TenantId.eq(tenant_id))
                        .add(catalog_version_counter::Column::NextId.eq(allocated)),
                )
                .exec(runner)
                .await
                .map_err(|e| {
                    driver_failure(format!("advance the version counter of {tenant_id}"), e)
                })?;
            if result.rows_affected == 0 {
                return Err(RepoError::Db(format!(
                    "version counter of {tenant_id} moved under the increment lease"
                )));
            }
            Ok(allocated)
        }
    }
}

/// One collected head reference: the snapshot's entry half and the
/// stage-vs-commit revalidation's operand (`inst-sn-revalidate`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotEntityRef {
    /// `product` or `sku`.
    pub entity_kind: String,
    /// The head's id.
    pub entity_id: Uuid,
    /// The frozen version the manifest pins.
    pub published_version: i64,
    /// The state at collect time, compared again before commit.
    pub lifecycle_state: String,
}

/// Every `published` or `deprecated` head of one tenant, as manifest
/// references — 01's only-consumer-read-surface rule makes
/// `products_entity_version` the sole content source, so the manifest
/// carries `(kind, id, published_version)` and never content.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn snapshot_entity_refs(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<SnapshotEntityRef>, RepoError> {
    let visible = ["published", "deprecated"];
    let mut refs = Vec::new();
    let products = product::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::LifecycleState.is_in(visible)),
        )
        .order_by(product::Column::ProductId, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("collect published products of {tenant_id}"), e))?;
    for row in products {
        refs.push(SnapshotEntityRef {
            entity_kind: "product".to_owned(),
            entity_id: row.product_id,
            published_version: row.published_version,
            lifecycle_state: row.lifecycle_state,
        });
    }
    let skus = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::LifecycleState.is_in(visible)),
        )
        .order_by(sku::Column::SkuId, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("collect published skus of {tenant_id}"), e))?;
    for row in skus {
        refs.push(SnapshotEntityRef {
            entity_kind: "sku".to_owned(),
            entity_id: row.sku_id,
            published_version: row.published_version,
            lifecycle_state: row.lifecycle_state,
        });
    }
    Ok(refs)
}

/// The governed live participant set of one tenant, name order.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn freeze_participants(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<String>, RepoError> {
    let rows = freeze_participant::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(freeze_participant::Column::TenantId.eq(tenant_id)))
        .order_by(freeze_participant::Column::Participant, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the participant set of {tenant_id}"), e))?;
    Ok(rows.into_iter().map(|row| row.participant).collect())
}

/// The version row's operands, rendered by the caller — this layer
/// deliberately imports no canonicalizer.
#[derive(Clone, Debug)]
pub struct NewCatalogVersion {
    /// The allocated gapless id.
    pub catalog_version_id: i64,
    /// Hex digest over the canonical manifest rendering.
    pub checksum: String,
    /// The digest rule the checksum was computed under.
    pub digest_version: i32,
    /// The commit instant.
    pub published_at: DateTime<Utc>,
    /// The participant cache column's rendering (P-D-67).
    pub participant_set_snapshot: String,
    /// `open`, or `complete` for an empty participant set.
    pub freeze_state: String,
}

/// Insert the version row.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn insert_catalog_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    new: NewCatalogVersion,
) -> Result<(), RepoError> {
    let id = new.catalog_version_id;
    let model = catalog_version::ActiveModel {
        tenant_id: Set(tenant_id),
        catalog_version_id: Set(new.catalog_version_id),
        checksum: Set(new.checksum),
        digest_version: Set(new.digest_version),
        published_at: Set(new.published_at),
        participant_set_snapshot: Set(new.participant_set_snapshot),
        freeze_state: Set(new.freeze_state),
    };
    catalog_version::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("version row scope of {tenant_id}"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("insert catalog version {id} of {tenant_id}"), e))?;
    Ok(())
}

/// Insert the manifest's entry rows.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn insert_catalog_version_entries(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
    entries: &[SnapshotEntityRef],
) -> Result<(), RepoError> {
    // One statement per bind-budget chunk rather than one round-trip per
    // entry: the manifest carries every published/deprecated head of the
    // tenant, and per-row INSERTs inside the lease-guarded serializable
    // increment transaction stretch it toward the lease TTL.
    let rows: Vec<catalog_version_entry::ActiveModel> = entries
        .iter()
        .map(|entry| catalog_version_entry::ActiveModel {
            tenant_id: Set(tenant_id),
            catalog_version_id: Set(catalog_version_id),
            entity_kind: Set(entry.entity_kind.clone()),
            entity_id: Set(entry.entity_id),
            published_version: Set(entry.published_version),
        })
        .collect();
    if !rows.is_empty() {
        toolkit_db::secure::secure_insert_many::<catalog_version_entry::Entity>(
            rows, scope, runner,
        )
        .await
        .map_err(|e| {
            driver_failure(
                format!("insert the manifest entries of {catalog_version_id} of {tenant_id}"),
                e,
            )
        })?;
    }
    Ok(())
}

/// Insert one manifest capture — a stored canonical copy.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn insert_catalog_version_capture(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
    capture_kind: &str,
    content: &str,
) -> Result<(), RepoError> {
    let model = catalog_version_capture::ActiveModel {
        tenant_id: Set(tenant_id),
        catalog_version_id: Set(catalog_version_id),
        capture_kind: Set(capture_kind.to_owned()),
        content: Set(content.to_owned()),
    };
    catalog_version_capture::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("capture scope of {tenant_id}"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("insert capture {capture_kind} of {tenant_id}"), e))?;
    Ok(())
}

/// Seed one `pending` ledger row per snapshotted participant (P-D-67).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn seed_freeze_acks(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
    participants: &[String],
) -> Result<(), RepoError> {
    for participant in participants {
        let model = freeze_ack::ActiveModel {
            tenant_id: Set(tenant_id),
            catalog_version_id: Set(catalog_version_id),
            participant: Set(participant.clone()),
            state: Set("pending".to_owned()),
            acked_at: Set(None),
            released_at: Set(None),
            forced_at: Set(None),
            ceremony_ref: Set(None),
        };
        freeze_ack::Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .map_err(|e| driver_failure(format!("ack seed scope of {tenant_id}"), e))?
            .exec(runner)
            .await
            .map_err(|e| {
                driver_failure(format!("seed freeze ack {participant} of {tenant_id}"), e)
            })?;
    }
    Ok(())
}

/// Flip the satisfied requests `pending -> coalesced` and stamp the
/// version, in the transaction that produced the set (P-D-60, P-D-50).
///
/// # Errors
///
/// [`RepoError`] on a storage failure, or when a named request was not
/// flipped — under the lease every named row must still be `pending`, so a
/// miss is the store contradicting the collect this same transaction made.
pub async fn mark_requests_coalesced(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    keys: &[(String, String)],
    catalog_version_id: i64,
) -> Result<(), RepoError> {
    for (source, request_key) in keys {
        let result = catalog_version_request::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(
                catalog_version_request::Column::State,
                Expr::value("coalesced".to_owned()),
            )
            .col_expr(
                catalog_version_request::Column::SatisfiedByVersionId,
                Expr::value(Some(catalog_version_id)),
            )
            .filter(
                Condition::all()
                    .add(catalog_version_request::Column::TenantId.eq(tenant_id))
                    .add(catalog_version_request::Column::Source.eq(source.clone()))
                    .add(catalog_version_request::Column::RequestKey.eq(request_key.clone()))
                    .add(catalog_version_request::Column::State.eq("pending")),
            )
            .exec(runner)
            .await
            .map_err(|e| driver_failure(format!("coalesce request {source}/{request_key}"), e))?;
        if result.rows_affected == 0 {
            return Err(RepoError::Db(format!(
                "request {source}/{request_key} of {tenant_id} was not pending at commit \
                 despite the increment lease"
            )));
        }
    }
    Ok(())
}
