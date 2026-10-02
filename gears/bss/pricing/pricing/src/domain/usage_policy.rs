//! Shape and semantic identity of immutable entry policies.
use super::RuleError;
use bss_pricing_sdk::{Digest, terms::UsageRatingPolicyInput};

/// Validate shape only; authoritative meter semantics are a separate gate.
/// # Errors
/// Returns `METER_POLICY_MISMATCH` for an empty meter, version, unit or accrual version.
pub fn validate_policy_shape(policy: &UsageRatingPolicyInput) -> Result<(), RuleError> {
    let q = &policy.quantity_semantics;
    if [
        &q.meter.usage_type_id,
        &q.meter.version,
        &q.unit,
        &q.accrual_policy_version,
    ]
    .iter()
    .any(|v| v.trim().is_empty())
    {
        return Err(RuleError::new("METER_POLICY_MISMATCH"));
    }
    Ok(())
}

/// Content identity for the full entry key; absence is reserved for legacy/non-usage entries.
#[must_use]
pub fn entry_policy_key(policy: Option<&UsageRatingPolicyInput>) -> Option<Digest> {
    policy.map(bss_pricing_sdk::digest::policy_digest)
}

/// Compare the complete immutable declaration and SKU unit with an entry's policy.
/// # Errors
/// `METER_POLICY_MISMATCH` means the declaration does not certify these quantities.
pub fn validate_meter_policy(
    policy: &bss_pricing_sdk::terms::UsageRatingPolicy,
    sku_unit: &str,
    semantics: &bss_pricing_sdk::meter_semantics::MeterSemantics,
) -> Result<(), RuleError> {
    validate_policy_shape(&policy.content)?;
    let q = &policy.content.quantity_semantics;
    if q.meter != semantics.meter
        || q.unit != sku_unit
        || q.unit != semantics.canonical_unit
        || q.fold != semantics.fold
        || q.fold != bss_pricing_sdk::terms::Fold::Sum
        || q.accrual_policy_version != semantics.accrual_policy_version
        || !semantics.source_integrated
    {
        return Err(RuleError::new("METER_POLICY_MISMATCH"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "the assertion is the refusal")]
    use bss_pricing_sdk::terms::*;
    #[test]
    fn meter_version_is_required() {
        let policy = UsageRatingPolicyInput {
            rating_window: RatingWindow::CalendarHour {
                timezone: Timezone::Utc,
            },
            aggregation_scope: AggregationScope::SubscriptionLine,
            reset: Reset::RatingWindowStart,
            quantity_semantics: QuantitySemantics {
                meter: MeterRef {
                    usage_type_id: "vm-hours".into(),
                    version: String::new(),
                },
                unit: "VM\u{b7}hour".into(),
                fold: Fold::Sum,
                accrual_policy_version: "integrated-v1".into(),
            },
            partial_window: PartialWindow::ActualQuantityFullThresholds,
        };
        assert_eq!(
            super::validate_policy_shape(&policy).unwrap_err().code,
            "METER_POLICY_MISMATCH"
        );
    }
}
