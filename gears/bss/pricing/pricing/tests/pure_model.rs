//! Compile and run the prototype assertions before the domain exists.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use bss_pricing::domain::{
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
