//! Compile and run the prototype assertions before the domain exists.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use bss_pricing::domain::{
    price::{ChargeKind, Model, OpState, ReferenceState},
    row::{Eligibility, RowState},
};
#[test]
fn enum_check_values_match_migration_text() {
    let price =
        include_str!("../src/infra/storage/migrations/m20260926_000005_create_pricing_price.rs");
    let row = include_str!(
        "../src/infra/storage/migrations/m20260926_000007_create_pricing_price_row.rs"
    );
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
    pin!(ChargeKind, price, "charge_kind");
    pin!(Model, row, "model");
    pin!(Eligibility, row, "eligibility");
    pin!(RowState, row, "state");
    pin!(ReferenceState, price, "reference_state");
    pin!(OpState, op, "state");
}
