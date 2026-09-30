//! Explicit policy authoring input for usage-entry fixtures.
#![allow(clippy::expect_used, clippy::unwrap_used)]
pub fn input() -> serde_json::Value {
    serde_json::json!({
        "rating_window":{"kind":"billing_cycle"},
        "aggregation_scope":"subscription_line","reset":"rating_window_start",
        "quantity_semantics":{"meter":{"usage_type_id":"vm-hours","version":"v1"},
            "unit":"VM\u{b7}hour","fold":"SUM","accrual_policy_version":"integrated-v1"},
        "partial_window":"actual_quantity_full_thresholds"
    })
}
