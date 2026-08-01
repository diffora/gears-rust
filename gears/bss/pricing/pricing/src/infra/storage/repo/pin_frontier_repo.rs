//! Repository for the materialized pin-eligibility frontier
//! (`pricing_pin_frontier`, D-136).
//!
//! Two operations, and the asymmetry between them is the design. [`read`] is on
//! the consumer path — it is what `GET /bss-pricing/v1/catalog-version/frontier`
//! serves and what a consumer pins for the duration of a resolution run — so it
//! is a single point lookup returning the SDK's
//! [`PinFrontier`](bss_pricing_sdk::PinFrontier), mapped at this boundary so the
//! entity never leaves infrastructure. [`advance`] is on the projector's path
//! and moves the watermark **forward only**.
//!
//! Forward-only is enforced twice, on purpose. The UPDATE carries its own
//! `catalog_version < :to` predicate, so even two projectors racing cannot walk
//! the frontier backwards — whichever loses simply updates nothing. And a
//! request to advance to an equal or lower version is reported as
//! [`RepoError::FrontierRegression`] rather than swallowed as a no-op: the
//! projector advances the frontier only inside the transaction completing the
//! frontier's **next** version in order, so such a request means that ordering
//! assumption has already broken. A receding frontier would let one pin resolve
//! two different contents over time, which is the entire reason the predicate is
//! materialized instead of recomputed; a silent no-op would leave the ordering
//! bug behind it invisible.
//!
//! [`read`]: PinFrontierRepo::read
//! [`advance`]: PinFrontierRepo::advance

use bss_pricing_sdk::{CatalogVersion, PinFrontier};
use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureInsertExt, SecureUpdateExt};
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

use crate::infra::storage::RepoError;
use crate::infra::storage::entity::pin_frontier;

/// `SeaORM`-backed pin-frontier repository.
#[derive(Clone)]
pub struct PinFrontierRepo {
    db: DBProvider<DbError>,
}

impl PinFrontierRepo {
    /// Build over one database provider.
    #[must_use]
    pub fn new(db: DBProvider<DbError>) -> Self {
        Self { db }
    }

    /// Read `tenant_id`'s current pin-eligibility frontier.
    ///
    /// `None` means the tenant has never had a version become pin-eligible —
    /// distinct from "version 0", and the distinction is load-bearing: a
    /// consumer with no frontier has nothing it may pin, and fails closed,
    /// rather than pinning an initial version whose content nothing warmed.
    /// SQL-level BOLA: a foreign tenant yields `None`.
    ///
    /// # Errors
    /// [`RepoError::Db`] on a scope or storage failure;
    /// [`RepoError::CorruptRow`] when the stored version is outside the
    /// unsigned range [`CatalogVersion`] carries (the column is
    /// `CHECK (catalog_version >= 0)`, so this is an invariant breach).
    pub async fn read(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
    ) -> Result<Option<PinFrontier>, RepoError> {
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;
        let row = pin_frontier::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(Condition::all().add(pin_frontier::Column::TenantId.eq(tenant_id)))
            .one(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("read pin frontier: {e}")))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let version = u64::try_from(row.catalog_version).map_err(|e| {
            RepoError::CorruptRow(format!(
                "pin frontier for tenant {tenant_id} holds catalog_version {}: {e}",
                row.catalog_version
            ))
        })?;
        Ok(Some(PinFrontier {
            catalog_version: CatalogVersion::new(version),
            advanced_at: row.advanced_at,
        }))
    }

    /// Advance `tenant_id`'s frontier to `to`, stamping `at`.
    ///
    /// Only ever forward. The first advance inserts the tenant's row; every
    /// later one is a conditional UPDATE guarded by `catalog_version < to`, so
    /// the watermark cannot recede even under a concurrent advance.
    ///
    /// # Errors
    /// [`RepoError::FrontierRegression`] when the frontier already stands at or
    /// beyond `to` — including the case where a concurrent advance overtook
    /// this one. [`RepoError::Db`] on a scope or storage failure, which
    /// includes losing the insert race for a tenant's first frontier row (the
    /// `tenant_id` primary key is what makes that a failure rather than a lost
    /// update). [`RepoError::CorruptRow`] when `to` exceeds the signed range
    /// the column stores.
    pub async fn advance(
        &self,
        scope: &AccessScope,
        tenant_id: Uuid,
        to: CatalogVersion,
        at: DateTime<Utc>,
    ) -> Result<(), RepoError> {
        let target = i64::try_from(to.get()).map_err(|e| {
            RepoError::CorruptRow(format!(
                "catalog version {} exceeds the storable range: {e}",
                to.get()
            ))
        })?;
        let conn = self
            .db
            .conn()
            .map_err(|e| RepoError::Db(format!("conn: {e}")))?;

        let current = pin_frontier::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(Condition::all().add(pin_frontier::Column::TenantId.eq(tenant_id)))
            .one(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("read pin frontier before advance: {e}")))?;

        let Some(row) = current else {
            let am = pin_frontier::ActiveModel {
                tenant_id: Set(tenant_id),
                catalog_version: Set(target),
                advanced_at: Set(at),
            };
            pin_frontier::Entity::insert(am.clone())
                .secure()
                .scope_with_model(scope, &am)
                .map_err(|e| RepoError::Db(format!("pricing_pin_frontier scope: {e}")))?
                .exec(&conn)
                .await
                .map_err(|e| RepoError::Db(format!("insert pricing_pin_frontier: {e}")))?;
            return Ok(());
        };

        if row.catalog_version >= target {
            return Err(regression(tenant_id, row.catalog_version, to));
        }

        // The `< target` predicate is the physical half of the forward-only
        // rule: a concurrent advance that already moved the frontier past
        // `target` leaves this UPDATE matching no row.
        let result = pin_frontier::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(pin_frontier::Column::CatalogVersion, Expr::value(target))
            .col_expr(pin_frontier::Column::AdvancedAt, Expr::value(at))
            .filter(
                Condition::all()
                    .add(pin_frontier::Column::TenantId.eq(tenant_id))
                    .add(pin_frontier::Column::CatalogVersion.lt(target)),
            )
            .exec(&conn)
            .await
            .map_err(|e| RepoError::Db(format!("advance pricing_pin_frontier: {e}")))?;

        if result.rows_affected == 0 {
            return Err(regression(tenant_id, row.catalog_version, to));
        }
        Ok(())
    }
}

/// Build the typed refusal, translating the stored `i64` back into the SDK's
/// unsigned vocabulary. A stored value outside that range would already have
/// failed [`PinFrontierRepo::read`]; here it is reported as `0` rather than
/// masking the regression behind a second error, since the caller's mistake is
/// the same either way.
fn regression(tenant_id: Uuid, current: i64, requested: CatalogVersion) -> RepoError {
    RepoError::FrontierRegression {
        tenant: tenant_id.to_string(),
        current: u64::try_from(current).unwrap_or(0),
        requested: requested.get(),
    }
}
