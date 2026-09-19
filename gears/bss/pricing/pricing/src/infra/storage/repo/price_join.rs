//! Join loaders for a price row's immutable line, version, and market.
//!
//! A `pricing_price` row carries money and three references, and nothing else:
//! its eight logical axes are columns of `pricing_charge_line`, its shared
//! calculation is `pricing_charge_line_version`, and its currency and region are
//! `pricing_market_price`. So every reader that used to ask the row a question
//! about its key or its structure asks its **graph** instead, and asks it through
//! here rather than joining by hand — one batched read per table, keyed by the
//! ids the rows already name.

use std::collections::HashMap;

use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt};
use uuid::Uuid;

use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{charge_line, charge_line_version, market_price, price};

/// One monetary row with the identity, structure, and market it references.
#[derive(Clone, Debug)]
pub struct PriceGraph {
    /// The monetary version: amounts, rates and market policy.
    pub price: price::Model,
    /// The stable logical line: the eight axes of the catalog key.
    pub line: charge_line::Model,
    /// The shared calculation the money is priced against.
    pub version: charge_line_version::Model,
    /// The currency/region variant this money belongs to.
    pub market: market_price::Model,
}

/// One row's graph.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure; [`RepoError::CorruptRow`]
/// when the row names a line, version or market that is not there.
pub async fn load_graph(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    row: price::Model,
) -> Result<PriceGraph, RepoError> {
    let mut graphs = load_graphs(runner, scope, tenant_id, std::slice::from_ref(&row)).await?;
    graphs.pop().ok_or_else(|| {
        RepoError::CorruptRow(format!(
            "price {} is missing its charge-line graph",
            row.price_id
        ))
    })
}

/// The graphs of a whole page of rows, in the order the rows came in.
///
/// Three reads for the page rather than three per row, which is what makes a
/// list surface affordable after the split.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure; [`RepoError::CorruptRow`]
/// when a row names a line, version or market that is not there.
pub async fn load_graphs(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    rows: &[price::Model],
) -> Result<Vec<PriceGraph>, RepoError> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let line_ids: Vec<Uuid> = rows.iter().map(|row| row.charge_line_id).collect();
    let version_ids: Vec<Uuid> = rows.iter().map(|row| row.line_version_id).collect();
    let market_ids: Vec<Uuid> = rows.iter().map(|row| row.market_price_id).collect();

    let lines = charge_line::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line::Column::TenantId.eq(tenant_id))
                .add(charge_line::Column::ChargeLineId.is_in(line_ids)),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_charge_line for price join: {e}")))?;
    let versions = charge_line_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(charge_line_version::Column::TenantId.eq(tenant_id))
                .add(charge_line_version::Column::LineVersionId.is_in(version_ids)),
        )
        .all(runner)
        .await
        .map_err(|e| {
            RepoError::Db(format!(
                "read pricing_charge_line_version for price join: {e}"
            ))
        })?;
    let markets = market_price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(market_price::Column::TenantId.eq(tenant_id))
                .add(market_price::Column::MarketPriceId.is_in(market_ids)),
        )
        .all(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read pricing_market_price for price join: {e}")))?;

    let lines: HashMap<Uuid, charge_line::Model> = lines
        .into_iter()
        .map(|row| (row.charge_line_id, row))
        .collect();
    let versions: HashMap<Uuid, charge_line_version::Model> = versions
        .into_iter()
        .map(|row| (row.line_version_id, row))
        .collect();
    let markets: HashMap<Uuid, market_price::Model> = markets
        .into_iter()
        .map(|row| (row.market_price_id, row))
        .collect();

    rows.iter()
        .map(|price| {
            let line = lines.get(&price.charge_line_id).cloned().ok_or_else(|| {
                RepoError::CorruptRow(format!(
                    "price {} names missing charge line {}",
                    price.price_id, price.charge_line_id
                ))
            })?;
            let version = versions
                .get(&price.line_version_id)
                .cloned()
                .ok_or_else(|| {
                    RepoError::CorruptRow(format!(
                        "price {} names missing line version {}",
                        price.price_id, price.line_version_id
                    ))
                })?;
            let market = markets
                .get(&price.market_price_id)
                .cloned()
                .ok_or_else(|| {
                    RepoError::CorruptRow(format!(
                        "price {} names missing market {}",
                        price.price_id, price.market_price_id
                    ))
                })?;
            Ok(PriceGraph {
                price: price.clone(),
                line,
                version,
                market,
            })
        })
        .collect()
}
