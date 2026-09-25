//! Exact decimal amounts and half-open tier arithmetic.
use super::{RuleError, price::Model};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tier {
    pub up_to: Option<Decimal>,
    pub rate: Decimal,
}

/// Untagged storage content: the model is a separate column.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum PriceData {
    Flat {
        amount: Decimal,
    },
    PerUnit {
        rate: Decimal,
    },
    Package {
        package_size: Decimal,
        package_price: Decimal,
    },
    Tiers {
        tiers: Vec<Tier>,
    },
}
/// Decode only the shape declared by the model.
/// # Errors
/// Malformed JSON, extra fields, or a model/shape mismatch are refused.
pub fn decode(model: Model, value: serde_json::Value) -> Result<PriceData, RuleError> {
    let data: PriceData =
        serde_json::from_value(value).map_err(|_| RuleError::new("PRICE_MISSING"))?;
    if shape_matches(model, &data) {
        Ok(data)
    } else {
        Err(RuleError::new("PRICE_MISSING"))
    }
}
fn shape_matches(model: Model, data: &PriceData) -> bool {
    matches!(
        (model, data),
        (Model::Flat, PriceData::Flat { .. })
            | (Model::PerUnit, PriceData::PerUnit { .. })
            | (Model::Package, PriceData::Package { .. })
            | (Model::Graduated | Model::Volume, PriceData::Tiers { .. })
    )
}
/// Validate rates and ascending bounds with one open top band.
#[must_use]
pub fn validate_tiers(tiers: &[Tier]) -> Vec<RuleError> {
    let mut errors = Vec::new();
    if tiers.is_empty() {
        return vec![RuleError::new("TIER_BAND_EMPTY")];
    }
    let mut previous = Decimal::ZERO;
    for (index, tier) in tiers.iter().enumerate() {
        if tier.rate < Decimal::ZERO {
            errors.push(RuleError::new("AMOUNT_INVALID"));
        }
        if index + 1 == tiers.len() {
            if tier.up_to.is_some() {
                errors.push(RuleError::new("TIER_TOP_CLOSED"));
            }
        } else if let Some(top) = tier.up_to.filter(|top| *top > previous) {
            previous = top;
        } else {
            errors.push(RuleError::new("TIER_BANDS_ORDER"));
        }
    }
    errors
}
/// Validate prices without computing a period minimum or consumer allowances.
#[must_use]
pub fn validate(model: Model, data: &PriceData) -> Vec<RuleError> {
    if !shape_matches(model, data) {
        return vec![RuleError::new("PRICE_MISSING")];
    }
    match data {
        PriceData::Flat { amount } | PriceData::PerUnit { rate: amount }
            if *amount < Decimal::ZERO =>
        {
            vec![RuleError::new("AMOUNT_INVALID")]
        }
        PriceData::Package {
            package_size,
            package_price,
        } if *package_size <= Decimal::ZERO || *package_price < Decimal::ZERO => {
            vec![RuleError::new("PACKAGE_FIELDS_INVALID")]
        }
        PriceData::Tiers { tiers } => validate_tiers(tiers),
        _ => Vec::new(),
    }
}
fn checked(value: Option<Decimal>) -> Result<Decimal, RuleError> {
    value.ok_or_else(|| RuleError::new("AMOUNT_INVALID"))
}
/// Evaluate one quantity in major currency units. No allowance, proration or floor.
/// # Errors
/// Invalid model content, negative quantities and decimal overflow are refused.
pub fn amount_for(model: Model, data: &PriceData, quantity: Decimal) -> Result<Decimal, RuleError> {
    if let Some(error) = validate(model, data).first() {
        return Err(*error);
    }
    if quantity < Decimal::ZERO {
        return Err(RuleError::new("AMOUNT_INVALID"));
    }
    match data {
        PriceData::Flat { amount } => Ok(*amount),
        PriceData::PerUnit { rate } => checked(rate.checked_mul(quantity)),
        PriceData::Package {
            package_size,
            package_price,
        } => checked(
            checked(quantity.checked_div(*package_size))?
                .ceil()
                .checked_mul(*package_price),
        ),
        PriceData::Tiers { tiers } if model == Model::Volume => {
            let band = tiers
                .iter()
                .find(|tier| tier.up_to.is_none_or(|top| quantity < top))
                .ok_or_else(|| RuleError::new("TIER_TOP_CLOSED"))?;
            checked(quantity.checked_mul(band.rate))
        }
        PriceData::Tiers { tiers } => {
            let mut total = Decimal::ZERO;
            let mut lower = Decimal::ZERO;
            for tier in tiers {
                let upper = tier.up_to.unwrap_or(quantity).min(quantity);
                let units = checked(upper.checked_sub(lower))?.max(Decimal::ZERO);
                total = checked(total.checked_add(checked(units.checked_mul(tier.rate))?))?;
                if upper == quantity {
                    break;
                }
                lower = upper;
            }
            Ok(total)
        }
    }
}
#[cfg(test)]
#[path = "money_tests.rs"]
mod tests;
