//! Compile and run the prototype assertions before the domain exists.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use bss_pricing::domain::{
    plan::{ReferenceState as PlanReferenceState, RevisionState, Treatment},
    price::{Eligibility, PriceState},
    price_book_entry::{ChargeKind, Model, OpState, ReferenceState},
};
#[test]
fn enum_check_values_match_migration_text() {
    let entry = include_str!(
        "../src/infra/storage/migrations/m20260926_000005_create_pricing_price_book_entry.rs"
    );
    let price =
        include_str!("../src/infra/storage/migrations/m20260926_000007_create_pricing_price.rs");
    let op = include_str!(
        "../src/infra/storage/migrations/m20260926_000006_create_pricing_reference_op.rs"
    );
    macro_rules! pin {
        ($ty:ty,$ddl:expr,$column:literal) => {{
            let values = <$ty>::ALL
                .iter()
                .map(|v| {
                    let s = v.as_str();
                    assert_eq!(s.parse::<$ty>().unwrap(), *v);
                    format!("'{s}'")
                })
                .collect::<Vec<_>>()
                .join(",");
            assert!(
                $ddl.contains(&format!("CHECK ({} IN ({}))", $column, values)),
                "{}: {}",
                $column,
                values
            );
            assert!("invalid".parse::<$ty>().is_err());
        }};
    }
    pin!(ChargeKind, entry, "charge_kind");
    pin!(Model, price, "model");
    pin!(Eligibility, price, "eligibility");
    pin!(PriceState, price, "state");
    pin!(ReferenceState, entry, "reference_state");
    pin!(OpState, op, "state");
}
/// Each phase 3 CHECK is spelled once per dialect: both halves carry the enum's exact vocabulary.
macro_rules! pin_both_dialects {
    ($ty:ty,$ddl:expr,$column:literal) => {{
        let values = <$ty>::ALL
            .iter()
            .map(|v| {
                let s = v.as_str();
                assert_eq!(s.parse::<$ty>().unwrap(), *v);
                format!("'{s}'")
            })
            .collect::<Vec<_>>()
            .join(",");
        let check = format!("CHECK ({} IN ({}))", $column, values);
        assert_eq!($ddl.matches(&check).count(), 2, "{check}");
        assert!("invalid".parse::<$ty>().is_err());
    }};
}
#[test]
fn plan_enum_check_values_match_migration_text() {
    let revision = include_str!(
        "../src/infra/storage/migrations/m20260926_000011_create_pricing_plan_revision.rs"
    );
    let item = include_str!(
        "../src/infra/storage/migrations/m20260926_000012_create_pricing_plan_item.rs"
    );
    pin_both_dialects!(RevisionState, revision, "state");
    pin_both_dialects!(Treatment, item, "treatment");
    pin_both_dialects!(PlanReferenceState, item, "reference_state");
}
