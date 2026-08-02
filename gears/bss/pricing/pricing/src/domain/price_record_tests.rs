//! What the record model has to keep true independently of storage.
//!
//! Deliberately one test. Every field of `PriceRecord` is exercised end to end
//! in `tests/sqlite_price_repo.rs`, where a mapping that dropped or transposed
//! a column actually fails; its lifecycle predicate belongs to
//! `LifecycleState`, which `lifecycle_tests.rs` already covers for all four
//! states. What is left is the split between the record and the content, which
//! is a decision rather than a derive.

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::{PriceContent, PriceRecord};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::CurrencyCode;
use crate::domain::price_row::{ModelKind, PriceRow};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};

#[test]
fn the_content_carries_every_editable_column_and_no_identity() {
    let key = ScopeKey::new(
        PlanId::new(Uuid::from_u128(0x9_1a4)),
        CurrencyCode::new("usd").expect("USD is three letters"),
        Region::new("EU").expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_5e)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none");

    // The exhaustive struct literal is the assertion, and it is a compile-time
    // one: when a slice widens what a draft edit may touch, this stops
    // compiling, so the widening is a decision somebody made rather than a
    // field that appeared.
    let record = PriceRecord {
        price_id: Uuid::from_u128(0xb_10),
        scope_key: key,
        row: PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat)),
        tax_inclusive: true,
        billing_timing: Some("advance".to_owned()),
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: Some(Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()),
        supersedes_price_id: Some(Uuid::from_u128(0xb_0f)),
        lifecycle_state: LifecycleState::Draft,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: Utc.with_ymd_and_hms(2026, 8, 2, 10, 0, 0).unwrap(),
        row_version: RowVersion::new(3),
    };

    // Round-tripping through the content must change nothing an author wrote:
    // a `content()` that dropped a column would silently blank it on the next
    // read-modify-write, which is the one bug this accessor exists to prevent.
    assert_eq!(
        record.content(),
        PriceContent {
            row: record.row.clone(),
            tax_inclusive: record.tax_inclusive,
            billing_timing: record.billing_timing.clone(),
            rounding_policy_ref: record.rounding_policy_ref.clone(),
            grandfather_until: record.grandfather_until,
            supersedes_price_id: record.supersedes_price_id,
        }
    );
}
