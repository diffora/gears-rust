#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use crate::domain::sku::SkuPatch;
use serde_json::json;

#[test]
fn nullable_patch_values_and_invalid_enums_keep_their_fields() {
    let absent: SkuPatchRequest = serde_json::from_value(json!({})).unwrap();
    assert_eq!(SkuPatch::try_from(absent).unwrap().gl_code, None);
    let clear: SkuPatchRequest =
        serde_json::from_value(json!({"gl_code":null,"billing_timing":null,"usage_type_ref":null}))
            .unwrap();
    let clear = SkuPatch::try_from(clear).unwrap();
    assert_eq!(clear.gl_code, Some(None));
    assert_eq!(clear.billing_timing, Some(None));
    assert_eq!(clear.usage_type_ref, Some(None));
    let bad: SkuPatchRequest =
        serde_json::from_value(json!({"type":"bad","lifecycle":"bad","billing_timing":"bad"}))
            .unwrap();
    let report = SkuPatch::try_from(bad).unwrap_err();
    for field in ["type", "lifecycle", "billing_timing"] {
        assert!(
            report
                .violations()
                .iter()
                .any(|v| v.subject == field && v.code == "VALIDATION")
        );
    }
}
