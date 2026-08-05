//! Repository over `pricing_approval_threshold` — the versioned, per-currency
//! approval-threshold policy (`design/05-governance.md` §6, D-10).
//!
//! Three operations and no update, which is the store's whole shape. A proposal
//! is [`open_version`]; a reader of one version's entries is [`read_version`];
//! and [`latest_version`] is what a proposer asks before minting the next number.
//! There is no `apply`, no `activate` and no delete, because a version's content
//! is what an approval's `content_hash` covers and a mutated version would leave
//! a signature over content nobody can reconstruct.
//!
//! # Runner-taking, not provider-holding, and the reason is D-10's
//!
//! Every entry point here takes a [`DBRunner`]. The proposal writes its version
//! rows **and** opens the approval unit that reviews them in one transaction —
//! a version nobody is reviewing is a proposal with no reviewer, and a unit
//! pinning content that failed to commit is a signature over nothing — and a
//! provider-holding repository could not join that transaction at all, `Db::conn()`
//! being refused inside an open one. [`read_model_repo`](super::read_model_repo)
//! set the precedent for the same reason.
//!
//! # Which version is in effect is not this module's answer
//!
//! It is `pricing_approval`'s: the greatest version whose unit an independent
//! principal approved. [`crate::infra::threshold::effective_policy`] composes the
//! two stores; nothing here reads the approval plane, so this module cannot
//! disagree with it.

use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

use crate::domain::audit::AuditStamp;
use crate::domain::materiality::ThresholdBasis;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::approval_threshold;
use crate::infra::storage::repo::check_authored_instant;

/// One currency's entry in one version, as the store holds it.
///
/// Exactly one of the two bases is `Some` — `chk_pricing_approval_threshold_basis`
/// is what makes that a fact rather than a convention, and
/// [`crate::domain::materiality::ThresholdEntry`] is the domain type that carries
/// the same guarantee in the type system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThresholdEntryRow {
    /// The ISO 4217 code, as stored.
    pub currency: String,
    /// The absolute threshold in the currency's minor units.
    pub absolute_minor: Option<i64>,
    /// The relative threshold in basis points (`10_000` = 100%).
    pub percent_bp: Option<i32>,
}

impl ThresholdEntryRow {
    /// The row's two nullable columns read as the domain's one-of-two choice.
    ///
    /// `None` when the pair is both-set or neither-set —
    /// `chk_pricing_approval_threshold_basis` makes that unreachable through this
    /// chain, so a caller that meets it has been written around and should say so
    /// rather than pick a basis. The conversion lives here, beside the columns,
    /// because the alternative is every reader deciding for itself which column
    /// wins when both are set.
    #[must_use]
    pub fn basis(&self) -> Option<ThresholdBasis> {
        match (self.absolute_minor, self.percent_bp) {
            (Some(minor), None) => Some(ThresholdBasis::Absolute { minor }),
            (None, Some(bp)) => u32::try_from(bp)
                .ok()
                .map(|bp| ThresholdBasis::Percent { bp }),
            _ => None,
        }
    }
}

/// One stored version: its authored instant and its entries, ordered by currency.
///
/// The two are returned together because a version is what an approval pin covers
/// and `effective_from` is inside that pin. [`read_version`] used to hand back the
/// entries alone, which made the pinned subject unreconstructible from the store
/// that holds it — the reader had the currencies and the thresholds and no way to
/// say when they start applying.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredVersion {
    /// When the thresholds start applying, once the version is approved.
    ///
    /// One value for the whole version: every row of a version is written by one
    /// [`open_version`] call with one instant, so a per-entry reading would be a
    /// column with N spellings of one fact.
    pub effective_from: DateTime<Utc>,
    /// The entries, ordered by currency — the order the pin is taken over.
    pub entries: Vec<ThresholdEntryRow>,
}

/// The greatest version number this tenant has ever proposed.
///
/// `None` is a tenant that has never proposed one — which is **not** version 0
/// and is the state `inst-mat-failsafe` is named for. The next proposal is
/// therefore `0` on `None` and `n + 1` otherwise, minted by the caller inside the
/// transaction that writes it so two concurrent proposals cannot mint one number
/// twice (the primary key refuses the loser either way).
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn latest_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Option<i64>, RepoError> {
    let row = approval_threshold::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(approval_threshold::Column::TenantId.eq(tenant_id)))
        .order_by(approval_threshold::Column::Version, Order::Desc)
        .limit(1)
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read latest threshold version: {e}")))?;
    Ok(row.map(|row| row.version))
}

/// One version's entries, ordered by currency.
///
/// The order is the store's and not the proposer's: a version is a **set** of
/// per-currency entries, so rendering it in insertion order would make one policy
/// hash two ways depending on which currency the operator typed first, and the
/// approval pin is taken over exactly this rendering.
///
/// An unknown version reads as `None` rather than an error — the caller that cares
/// (the pinned-content render) reports "no longer derivable", which is the same
/// answer a plan whose draft has published gives.
///
/// # One version, one instant — **derived rather than assumed**
///
/// `effective_from` is a column on every entry row, and this used to take it off
/// `rows.first()` on an invariant nothing asserted: *all rows of one version share one
/// instant*. Nothing in the schema says so — the column is per row and the primary key
/// is `(tenant_id, version, currency)` — so `first()` was a silent choice among rows
/// that might disagree, made in the `ORDER BY currency` order, which means the answer
/// would have depended on which currency sorted first.
///
/// It is now **derived**: the maximum over the version's rows, with a disagreement
/// reported as [`RepoError::CorruptRow`] rather than resolved. The invariant is real —
/// `open_version` writes one authored instant across the whole entry set in one
/// statement, and the append-only trigger refuses every `UPDATE` — so a disagreement
/// means the table was written around, and the pin a reviewer signed was taken over one
/// of the two values. `max` rather than `min` never matters for a well-formed version
/// and is the fail-closed direction for a malformed one: the later instant is the one
/// that has not yet taken effect.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure; [`RepoError::CorruptRow`] when one
/// version's rows carry two different `effective_from` values.
pub async fn read_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    version: i64,
) -> Result<Option<StoredVersion>, RepoError> {
    let rows = approval_threshold::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(approval_threshold::Column::TenantId.eq(tenant_id))
                .add(approval_threshold::Column::Version.eq(version)),
        )
        .order_by(approval_threshold::Column::Currency, Order::Asc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read threshold version {version}: {e}")))?;
    let Some(effective_from) = rows.iter().map(|row| row.effective_from).max() else {
        return Ok(None);
    };
    if let Some(disagreeing) = rows.iter().find(|row| row.effective_from != effective_from) {
        return Err(RepoError::CorruptRow(format!(
            "pricing_approval_threshold: version {version} carries two effective_from values - \
             {effective_from} on one entry and {} on {} - so which instant an approver signed is \
             not determined",
            disagreeing.effective_from, disagreeing.currency
        )));
    }
    Ok(Some(StoredVersion {
        effective_from,
        entries: rows
            .into_iter()
            .map(|row| ThresholdEntryRow {
                currency: row.currency,
                absolute_minor: row.absolute_minor,
                percent_bp: row.percent_bp,
            })
            .collect(),
    }))
}

/// Write one proposed version's entries, inside `runner`'s transaction.
///
/// The whole version in one call, because a version is the unit an approval pins:
/// two calls could commit half a policy, and a reviewer would then be shown a
/// proposal the proposer never made.
///
/// `effective_from` is an authored instant and is quantized to the millisecond
/// (D-144) by the same boundary check every other authored instant in this gear
/// passes — an unquantized one is refused here rather than truncated, because a
/// truncating producer and a non-truncating consumer agree until the day they do
/// not.
///
/// An empty `entries` writes nothing and is not an error at this layer: the domain
/// decides whether an empty entry set is a legal proposal, and
/// [`crate::domain::materiality::ThresholdPolicy::of_entries`] answers `None` for
/// one, so a policy of no entries is the absence `inst-mat-failsafe` is named for
/// rather than a storage fault.
///
/// # Errors
/// [`RepoError::TimestampPrecisionExceeded`] on an unquantized `effective_from`;
/// [`RepoError::Db`] on a scope failure, a duplicate `(tenant, version, currency)`,
/// or any CHECK the row violates.
pub async fn open_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    version: i64,
    effective_from: DateTime<Utc>,
    entries: &[ThresholdEntryRow],
    stamp: AuditStamp,
) -> Result<(), RepoError> {
    check_authored_instant("effectiveFrom", Some(effective_from))?;
    for entry in entries {
        let model = approval_threshold::ActiveModel {
            tenant_id: sea_orm::ActiveValue::Set(tenant_id),
            version: sea_orm::ActiveValue::Set(version),
            currency: sea_orm::ActiveValue::Set(entry.currency.clone()),
            absolute_minor: sea_orm::ActiveValue::Set(entry.absolute_minor),
            percent_bp: sea_orm::ActiveValue::Set(entry.percent_bp),
            effective_from: sea_orm::ActiveValue::Set(effective_from),
            created_by: sea_orm::ActiveValue::Set(stamp.actor_principal_id),
            created_at: sea_orm::ActiveValue::Set(stamp.recorded_at),
        };
        approval_threshold::Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .map_err(|e| RepoError::Db(format!("threshold entry scope: {e}")))?
            .exec(runner)
            .await
            .map_err(|e| {
                RepoError::Db(format!(
                    "write threshold entry {}/{version}/{}: {e}",
                    tenant_id, entry.currency
                ))
            })?;
    }
    Ok(())
}

/// Every version number this tenant has proposed, greatest first.
///
/// The effective-policy resolution needs it: "the greatest version whose unit was
/// approved" is a walk down this list asking the approval store about each, and
/// asking about a version that does not exist would report a policy the tenant
/// never proposed.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn versions_desc(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<i64>, RepoError> {
    let rows = approval_threshold::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(approval_threshold::Column::TenantId.eq(tenant_id)))
        .order_by(approval_threshold::Column::Version, Order::Desc)
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("list threshold versions: {e}")))?;
    // Deduplicated here rather than by `SELECT DISTINCT`: `SecureSelect` carries
    // no projection, and a version holds one row per configured currency, so the
    // set is a handful of rows per proposal rather than a scan of anything.
    let mut versions: Vec<i64> = Vec::new();
    for row in rows {
        if versions.last() != Some(&row.version) {
            versions.push(row.version);
        }
    }
    Ok(versions)
}
