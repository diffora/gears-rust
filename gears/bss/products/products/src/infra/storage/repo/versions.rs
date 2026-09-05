//! The catalog-version read-backs and the freeze ledger's edges — the
//! version row, its manifest rows, and the ack/release walk (`design/06`,
//! P-D-67, P-D-84).
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

use crate::domain::states::{FreezeAckState, FreezeState};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    catalog_version, catalog_version_capture, catalog_version_entry, freeze_ack,
    freeze_participant, metadata,
};

use super::{SnapshotEntityRef, driver_failure};

/// One committed version row, in this repository's vocabulary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CatalogVersionRecord {
    /// The gapless id.
    pub catalog_version_id: i64,
    /// Hex digest over the canonical manifest rendering.
    pub checksum: String,
    /// The digest rule the checksum was computed under.
    pub digest_version: i32,
    /// The commit instant.
    pub published_at: DateTime<Utc>,
    /// The derived participant cache (P-D-67).
    pub participant_set_snapshot: String,
    /// The ledger's derived cache, typed at the storage boundary.
    pub freeze_state: FreezeState,
}

/// Read one version row.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_catalog_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
) -> Result<Option<CatalogVersionRecord>, RepoError> {
    let row = catalog_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version::Column::TenantId.eq(tenant_id))
                .add(catalog_version::Column::CatalogVersionId.eq(catalog_version_id)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read catalog version {catalog_version_id}"), e))?;
    row.map(|row| {
        // The CHECK constraint admits only the roster, so a value outside
        // it is a corrupt row, never a default.
        let freeze_state = FreezeState::parse(&row.freeze_state).ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "catalog version {catalog_version_id} carries freeze_state {:?} outside the roster",
                row.freeze_state
            ))
        })?;
        Ok(CatalogVersionRecord {
            catalog_version_id: row.catalog_version_id,
            checksum: row.checksum,
            digest_version: row.digest_version,
            published_at: row.published_at,
            participant_set_snapshot: row.participant_set_snapshot,
            freeze_state,
        })
    })
    .transpose()
}

/// The stored manifest halves of one version — the resolver's re-render
/// operands (`inst-rv-bytes`: never a re-collect).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn catalog_version_manifest_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
) -> Result<(Vec<SnapshotEntityRef>, Vec<(String, String)>), RepoError> {
    let entries = catalog_version_entry::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version_entry::Column::TenantId.eq(tenant_id))
                .add(catalog_version_entry::Column::CatalogVersionId.eq(catalog_version_id)),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read entries of {catalog_version_id}"), e))?
        .into_iter()
        .map(|row| SnapshotEntityRef {
            entity_kind: row.entity_kind,
            entity_id: row.entity_id,
            published_version: row.published_version,
            // Not part of the rendering; the revalidation operand only.
            lifecycle_state: String::new(),
        })
        .collect();
    let captures = catalog_version_capture::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version_capture::Column::TenantId.eq(tenant_id))
                .add(catalog_version_capture::Column::CatalogVersionId.eq(catalog_version_id)),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read captures of {catalog_version_id}"), e))?
        .into_iter()
        .map(|row| (row.capture_kind, row.content))
        .collect();
    Ok((entries, captures))
}

/// Every ledger row of one version, participant order.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn freeze_ack_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
) -> Result<Vec<(String, FreezeAckState)>, RepoError> {
    let rows = freeze_ack::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(freeze_ack::Column::TenantId.eq(tenant_id))
                .add(freeze_ack::Column::CatalogVersionId.eq(catalog_version_id)),
        )
        .order_by(freeze_ack::Column::Participant, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the ledger of {catalog_version_id}"), e))?;
    rows.into_iter()
        .map(|row| {
            let state = FreezeAckState::parse(&row.state).ok_or_else(|| {
                RepoError::CorruptRow(format!(
                    "freeze-ack row of {} on {catalog_version_id} carries state {:?} \
                     outside the roster",
                    row.participant, row.state
                ))
            })?;
            Ok((row.participant, state))
        })
        .collect()
}

/// One registration row as the **retention gate** reads it: the participant,
/// its state, and whether `released_at` is stamped
/// (`dod-retention-gate`).
///
/// The stamp is carried separately from the state because the gate's two arms
/// need both and they are not derivable from one another: a door-released row
/// is `state = released` with the stamp **NULL**, while force-completion
/// stamps it in the same transaction as `not_frozen(forced)` and a recovered
/// participant's later ack leaves the stamp behind (P-D-67).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FreezeRegistration {
    /// The participant the row is for.
    pub participant: String,
    /// Its state in the ledger.
    pub state: FreezeAckState,
    /// Whether `released_at` carries a value.
    pub released_at_stamped: bool,
}

/// Every registration row of one version, with the release stamp
/// (`dod-retention-gate`'s operand).
///
/// # Errors
///
/// [`RepoError::CorruptRow`] where a stored state is outside the roster, and
/// [`RepoError`] as the read raises it.
pub async fn freeze_registrations(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
) -> Result<Vec<FreezeRegistration>, RepoError> {
    let rows = freeze_ack::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(freeze_ack::Column::TenantId.eq(tenant_id))
                .add(freeze_ack::Column::CatalogVersionId.eq(catalog_version_id)),
        )
        .all(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("read the freeze registrations of {catalog_version_id}"),
                e,
            )
        })?;
    rows.into_iter()
        .map(|row| {
            let state = FreezeAckState::parse(&row.state).ok_or_else(|| {
                RepoError::CorruptRow(format!(
                    "freeze-ack row of {} on {catalog_version_id} carries state {:?} \
                     outside the roster",
                    row.participant, row.state
                ))
            })?;
            Ok(FreezeRegistration {
                participant: row.participant,
                state,
                released_at_stamped: row.released_at.is_some(),
            })
        })
        .collect()
}

/// A ledger edge's outcome, for the ack and release doors to classify.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FreezeEdgeOutcome {
    /// The edge was taken.
    Flipped,
    /// The row already sits in the target state — the idempotent replay.
    AlreadyThere,
    /// The row exists but its state admits no such edge.
    IllegalFrom(String),
    /// No row: the participant is outside the version's snapshotted set.
    NoRow,
}

/// `pending -> acked`, stamping `acked_at` — the ack door's write, an
/// UPDATE and never an upsert (the row's existence IS the membership
/// check, P-D-67). A recovered forced participant's ack rides the same
/// edge list (`not_frozen(forced) -> acked`, P-D-60) and **clears the
/// ceremony's `forced_at` / `ceremony_ref` pair** — the shape CHECK binds
/// those two columns to the forced state, and `released_at` alone is the
/// stamp that survives recovery (write-once; the retention gate reads the
/// `(state, released_at)` pair). The ceremony stays joinable through its
/// audit row, which carries the same `ceremony_ref`.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn ack_freeze_row(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
    participant: &str,
    now: DateTime<Utc>,
) -> Result<FreezeEdgeOutcome, RepoError> {
    let result = freeze_ack::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            freeze_ack::Column::State,
            Expr::value(FreezeAckState::Acked.as_str().to_owned()),
        )
        .col_expr(freeze_ack::Column::AckedAt, Expr::value(Some(now)))
        .col_expr(
            freeze_ack::Column::ForcedAt,
            Expr::value(Option::<DateTime<Utc>>::None),
        )
        .col_expr(
            freeze_ack::Column::CeremonyRef,
            Expr::value(Option::<Uuid>::None),
        )
        .filter(
            Condition::all()
                .add(freeze_ack::Column::TenantId.eq(tenant_id))
                .add(freeze_ack::Column::CatalogVersionId.eq(catalog_version_id))
                .add(freeze_ack::Column::Participant.eq(participant))
                .add(freeze_ack::Column::State.is_in([
                    FreezeAckState::Pending.as_str(),
                    FreezeAckState::NotFrozenForced.as_str(),
                ])),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("ack {participant} on {catalog_version_id}"), e))?;
    if result.rows_affected > 0 {
        return Ok(FreezeEdgeOutcome::Flipped);
    }
    classify_missed_edge(
        runner,
        scope,
        tenant_id,
        catalog_version_id,
        participant,
        FreezeAckState::Acked,
    )
    .await
}

/// `pending|acked|not_frozen(forced) -> released` — the release door's
/// write. The door does **not** stamp `released_at`: that column is the
/// ceremony's alone (P-D-67), and the write-once trigger holds it. A forced
/// row's release clears `forced_at` / `ceremony_ref` as the recovered ack
/// does (the shape CHECK's first arm).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn release_freeze_row(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
    participant: &str,
) -> Result<FreezeEdgeOutcome, RepoError> {
    let result = freeze_ack::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            freeze_ack::Column::State,
            Expr::value(FreezeAckState::Released.as_str().to_owned()),
        )
        .col_expr(
            freeze_ack::Column::ForcedAt,
            Expr::value(Option::<DateTime<Utc>>::None),
        )
        .col_expr(
            freeze_ack::Column::CeremonyRef,
            Expr::value(Option::<Uuid>::None),
        )
        .filter(
            Condition::all()
                .add(freeze_ack::Column::TenantId.eq(tenant_id))
                .add(freeze_ack::Column::CatalogVersionId.eq(catalog_version_id))
                .add(freeze_ack::Column::Participant.eq(participant))
                .add(freeze_ack::Column::State.is_in([
                    FreezeAckState::Pending.as_str(),
                    FreezeAckState::Acked.as_str(),
                    FreezeAckState::NotFrozenForced.as_str(),
                ])),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("release {participant} on {catalog_version_id}"), e))?;
    if result.rows_affected > 0 {
        return Ok(FreezeEdgeOutcome::Flipped);
    }
    classify_missed_edge(
        runner,
        scope,
        tenant_id,
        catalog_version_id,
        participant,
        FreezeAckState::Released,
    )
    .await
}

/// Why a guarded ledger UPDATE matched nothing: the idempotent replay, an
/// inadmissible source state, or no row at all.
async fn classify_missed_edge(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
    participant: &str,
    target: FreezeAckState,
) -> Result<FreezeEdgeOutcome, RepoError> {
    let row = freeze_ack::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(freeze_ack::Column::TenantId.eq(tenant_id))
                .add(freeze_ack::Column::CatalogVersionId.eq(catalog_version_id))
                .add(freeze_ack::Column::Participant.eq(participant)),
        )
        .one(runner)
        .await
        .map_err(|e| {
            driver_failure(format!("classify {participant} on {catalog_version_id}"), e)
        })?;
    Ok(match row {
        None => FreezeEdgeOutcome::NoRow,
        Some(row) if row.state == target.as_str() => FreezeEdgeOutcome::AlreadyThere,
        Some(row) => FreezeEdgeOutcome::IllegalFrom(row.state),
    })
}

/// Recompute and store `freeze_state`'s derived cache from the ledger it
/// derives from — the P-D-49 snapshot-driven summary, written by the three
/// acts that change the ledger (P-D-73) under P-D-84's settled predicate:
/// any `not_frozen(forced)` row -> `complete(forced)`; else any `pending`
/// -> `open`; else `complete` (a release settles exactly as an ack).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
/// Force-completion's ledger write (`dod-force-completion`): every `pending`
/// registration of the version becomes `not_frozen(forced)` **and** carries
/// `released_at` in the same statement — the stamp `10`'s gate reads as the
/// `(state, released_at)` pair, meaningful only while the state holds — plus
/// `forced_at` and the ceremony reference. Returns the participants forced.
///
/// # Errors
///
/// [`RepoError`] on a driver failure.
///
/// @cpt-dod:cpt-cf-bss-products-dod-cv-audit:p1
pub async fn force_pending_freeze_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
    ceremony_ref: Uuid,
    now: DateTime<Utc>,
) -> Result<Vec<String>, RepoError> {
    let pending: Vec<String> = freeze_ack_rows(runner, scope, tenant_id, catalog_version_id)
        .await?
        .into_iter()
        .filter(|(_, state)| *state == FreezeAckState::Pending)
        .map(|(participant, _)| participant)
        .collect();
    if pending.is_empty() {
        return Ok(pending);
    }
    freeze_ack::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            freeze_ack::Column::State,
            Expr::value(FreezeAckState::NotFrozenForced.as_str().to_owned()),
        )
        .col_expr(freeze_ack::Column::ForcedAt, Expr::value(Some(now)))
        .col_expr(freeze_ack::Column::ReleasedAt, Expr::value(Some(now)))
        .col_expr(
            freeze_ack::Column::CeremonyRef,
            Expr::value(Some(ceremony_ref)),
        )
        .filter(
            Condition::all()
                .add(freeze_ack::Column::TenantId.eq(tenant_id))
                .add(freeze_ack::Column::CatalogVersionId.eq(catalog_version_id))
                .add(freeze_ack::Column::State.eq(FreezeAckState::Pending.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("force-complete {catalog_version_id}"), e))?;
    Ok(pending)
}

/// Register a freeze participant (`dod-participant-set`); `Ok(false)` when
/// the tenant already carries it.
///
/// # Errors
///
/// [`RepoError`] on a driver failure.
pub async fn register_freeze_participant(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    participant: &str,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let present = freeze_participant::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(freeze_participant::Column::TenantId.eq(tenant_id))
                .add(freeze_participant::Column::Participant.eq(participant)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read participant {participant}"), e))?;
    if present.is_some() {
        return Ok(false);
    }
    let model = freeze_participant::ActiveModel {
        tenant_id: Set(tenant_id),
        participant: Set(participant.to_owned()),
        registered_at: Set(now),
    };
    freeze_participant::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("participant scope of {tenant_id}"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("register participant {participant}"), e))?;
    Ok(true)
}

/// Retire a freeze participant from the **live** set (`dod-participant-set`);
/// every version's own `participant_set_snapshot` is untouched, so a
/// historical `freezeComplete` never re-resolves (AC #23). `Ok(false)` when
/// the tenant did not carry it.
///
/// # Errors
///
/// [`RepoError`] on a driver failure.
pub async fn retire_freeze_participant(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    participant: &str,
) -> Result<bool, RepoError> {
    let result = freeze_participant::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(freeze_participant::Column::TenantId.eq(tenant_id))
                .add(freeze_participant::Column::Participant.eq(participant)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("retire participant {participant}"), e))?;
    Ok(result.rows_affected == 1)
}

pub async fn refresh_freeze_state(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    catalog_version_id: i64,
) -> Result<FreezeState, RepoError> {
    let rows = freeze_ack_rows(runner, scope, tenant_id, catalog_version_id).await?;
    let state = if rows
        .iter()
        .any(|(_, s)| *s == FreezeAckState::NotFrozenForced)
    {
        FreezeState::CompleteForced
    } else if rows.iter().any(|(_, s)| *s == FreezeAckState::Pending) {
        FreezeState::Open
    } else {
        FreezeState::Complete
    };
    catalog_version::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            catalog_version::Column::FreezeState,
            Expr::value(state.as_str().to_owned()),
        )
        .filter(
            Condition::all()
                .add(catalog_version::Column::TenantId.eq(tenant_id))
                .add(catalog_version::Column::CatalogVersionId.eq(catalog_version_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("refresh freeze_state of {catalog_version_id}"), e))?;
    Ok(state)
}

/// Every `open` version older than the timeout, with its still-pending
/// participants — the `freeze_overdue` telemetry's operand
/// (`dod-freeze-timeout`; the timeout fails closed, so this read only
/// names the silence).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn overdue_open_versions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    published_before: DateTime<Utc>,
) -> Result<Vec<(Uuid, i64)>, RepoError> {
    let rows = catalog_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(catalog_version::Column::FreezeState.eq(FreezeState::Open.as_str()))
                .add(catalog_version::Column::PublishedAt.lt(published_before)),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure("scan for overdue open versions".to_owned(), e))?;
    Ok(rows
        .into_iter()
        .map(|row| (row.tenant_id, row.catalog_version_id))
        .collect())
}

/// Every metadata row of the tenant, as `(entity_kind, entity_id, key,
/// value)` sorted by that tuple — the `metadata_maps` capture's source
/// (`dod-metadata-placement`).
///
/// Sorted in SQL rather than in the caller: the rendering is checksummed, so
/// the order must be the same on both engines and on every read, and an
/// `ORDER BY` over the key columns is the only ordering both engines agree
/// on without a locale.
///
/// # Errors
///
/// [`RepoError`] as the read raises it.
pub async fn metadata_rows(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<(String, Uuid, String, String)>, RepoError> {
    let rows = metadata::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(metadata::Column::TenantId.eq(tenant_id)))
        .order_by(metadata::Column::EntityKind, sea_orm::Order::Asc)
        .order_by(metadata::Column::EntityId, sea_orm::Order::Asc)
        .order_by(metadata::Column::Key, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the metadata maps of {tenant_id}"), e))?;
    Ok(rows
        .into_iter()
        .map(|row| (row.entity_kind, row.entity_id, row.key, row.value))
        .collect())
}
