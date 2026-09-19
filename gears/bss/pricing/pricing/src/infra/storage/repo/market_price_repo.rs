//! Market-price identity: one currency/region variant of a charge line.
//!
//! Monetary versions of a market live on `pricing_price` and may be several
//! when their windows do not overlap.

use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

use crate::domain::money::CurrencyCode;
use crate::domain::scope_key::Region;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::market_price;

const MARKET_NS: Uuid = Uuid::from_u128(0x7c_11_e9_a0_9c_0f_4b_21_a1_e0_c4_12_f0_01_00_03);

/// Find or insert the market variant of `charge_line_id`.
pub async fn find_or_create(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    charge_line_id: Uuid,
    currency: &CurrencyCode,
    region: &Region,
) -> Result<Uuid, RepoError> {
    if let Some(existing) = find(runner, scope, tenant_id, charge_line_id, currency, region).await?
    {
        return Ok(existing.market_price_id);
    }
    let market_price_id = market_id(tenant_id, charge_line_id, currency, region);
    let row = market_price::ActiveModel {
        tenant_id: Set(tenant_id),
        market_price_id: Set(market_price_id),
        charge_line_id: Set(charge_line_id),
        currency: Set(currency.as_str().to_owned()),
        region: Set(region.as_str().to_owned()),
    };
    let insert = market_price::Entity::insert(row.clone())
        .secure()
        .scope_with_model(scope, &row)
        .map_err(|e| RepoError::Db(format!("pricing_market_price scope: {e}")))?;
    match insert.exec(runner).await {
        Ok(_) => Ok(market_price_id),
        Err(err) => {
            if let Some(existing) =
                find(runner, scope, tenant_id, charge_line_id, currency, region).await?
            {
                Ok(existing.market_price_id)
            } else {
                Err(RepoError::Db(format!("insert pricing_market_price: {err}")))
            }
        }
    }
}

/// Load one market by id.
pub async fn require(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    market_price_id: Uuid,
) -> Result<market_price::Model, RepoError> {
    market_price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(market_price::Column::TenantId.eq(tenant_id))
                .add(market_price::Column::MarketPriceId.eq(market_price_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_market_price: {e}")))?
        .ok_or_else(|| RepoError::NotFound {
            subject: "market price".to_owned(),
            id: market_price_id.to_string(),
        })
}

/// Load the market of one line and currency/region, if it exists.
pub async fn find_by_line_market(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    charge_line_id: Uuid,
    currency: &CurrencyCode,
    region: &Region,
) -> Result<Option<market_price::Model>, RepoError> {
    find(runner, scope, tenant_id, charge_line_id, currency, region).await
}

/// Every market of a tenant standing in one of `regions`.
///
/// The region axis left `pricing_price` for this table, so a caller that used
/// to filter rows by region now resolves the markets first and filters rows by
/// `market_price_id`. Bounded by the tenant's markets in those regions, never by
/// its price rows.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure.
pub async fn in_regions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    regions: &[String],
) -> Result<Vec<market_price::Model>, RepoError> {
    if regions.is_empty() {
        return Ok(Vec::new());
    }
    market_price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(market_price::Column::TenantId.eq(tenant_id))
                .add(market_price::Column::Region.is_in(regions.to_vec())),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_market_price by region: {e}")))
}

async fn find(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    charge_line_id: Uuid,
    currency: &CurrencyCode,
    region: &Region,
) -> Result<Option<market_price::Model>, RepoError> {
    market_price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(market_price::Column::TenantId.eq(tenant_id))
                .add(market_price::Column::ChargeLineId.eq(charge_line_id))
                .add(market_price::Column::Currency.eq(currency.as_str()))
                .add(market_price::Column::Region.eq(region.as_str())),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_market_price by scope: {e}")))
}

fn market_id(
    tenant_id: Uuid,
    charge_line_id: Uuid,
    currency: &CurrencyCode,
    region: &Region,
) -> Uuid {
    let mut bytes = Vec::new();
    bytes.extend(tenant_id.as_bytes());
    bytes.extend(charge_line_id.as_bytes());
    bytes.extend(currency.as_str().as_bytes());
    bytes.extend(region.as_str().as_bytes());
    Uuid::new_v5(&MARKET_NS, &bytes)
}
