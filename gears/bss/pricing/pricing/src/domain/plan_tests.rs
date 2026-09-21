//! What the revision model has to keep true independently of storage.
//!
//! Deliberately one test. `PlanRevision`'s field set is exercised end to end in
//! `tests/sqlite_plan_repo.rs`, where a mapping that dropped or transposed a
//! column actually fails; its lifecycle predicates belong to `LifecycleState`,
//! which `lifecycle_tests.rs` already covers for all four states; and its
//! equality is a `derive`. Re-asserting any of those here would test the
//! language rather than a rule.
//!
//! What is left is the patch, and the part of it a compiler can hold.

use uuid::Uuid;

use super::PlanShapePatch;
use crate::domain::instant::utc_ymd_hms;
use crate::domain::plan_shape::{CustomIntervalUnit, Frequency};

#[test]
fn the_patch_names_its_columns_and_cannot_mean_clear_any_of_them() {
    // The exhaustive struct literal is the assertion, and it is a compile-time
    // one: when a slice widens what a draft edit may touch, this stops
    // compiling, so the widening becomes a decision somebody made rather than a
    // field that appeared. Slice 2 widened it by five; Slice 6 by one, and this
    // census caught that widening exactly as designed.
    //
    // **The count is deliberately not in the name any more.** It read
    // `the_patch_names_eleven_columns_and_cannot_mean_clear_ten_of_them` over a
    // thirteen-field struct with ten assertions -- the shape `plan_rules.rs`
    // records as review T-6, where "a reviewer auditing it walked twenty entries
    // and stopped". A number in a test name is a second roster nothing keeps
    // true, and correcting it to thirteen would only reset the clock.
    let _every_column = PlanShapePatch {
        sku_id: Some(Uuid::from_u128(1)),
        plan_tier: Some("silver".to_owned()),
        plan_name: Some("Fixture Plan".to_owned()),
        frequency: Some(Frequency::CustomEveryN {
            n: 45,
            unit: CustomIntervalUnit::Days,
        }),
        purchase_min_qty: Some(2),
        purchase_max_qty: Some(10),
        descriptor_ext: Some(std::collections::BTreeMap::new()),
        available_from: Some(utc_ymd_hms(2027, 1, 1, 0, 0, 0)),
        available_to: Some(utc_ymd_hms(2028, 1, 1, 0, 0, 0)),
        entitlement_grants: Option::default(),
        change_contract: Option::default(),
    };

    // The empty patch has to be empty in **every** field, and that is what makes
    // "absent = leave alone" total: with nothing defaulting to a value, no patch
    // can mean "set this column to NULL". A `Default` that pre-filled one field
    // would turn the identity edit into a write nobody asked for, and is the one
    // way this encoding can lie.
    //
    // The field this mattered most for was `plan_tier_override` — the only
    // `Option` here over a `NOT NULL` column, where a `Default` of
    // `Some(false)` would have read as "leave alone" and silently withdrawn an
    // audited override on every unrelated edit. It went with **D-383**, and the
    // rule it illustrated did not: every remaining member is an `Option` over a
    // nullable column, so the encoding can no longer lie in that particular
    // way — which is a narrower guarantee than the one this case asserts, and
    // the reason the case still asserts every field rather than a sample.
    let empty = PlanShapePatch::default();
    assert!(empty.sku_id.is_none());
    assert!(empty.plan_tier.is_none());
    // `plan_name`, `entitlement_grants` and `change_contract` were outside the
    // block until 2026-08-20, so "empty in **every** field" was a claim about ten
    // of thirteen. `plan_name`'s own doc on `plan.rs` says `None` means "leave it
    // alone", which is precisely the reading a `Default` of `Some("")` would
    // break.
    assert!(empty.plan_name.is_none());
    assert!(empty.frequency.is_none());
    assert!(empty.purchase_min_qty.is_none());
    assert!(empty.purchase_max_qty.is_none());
    assert!(empty.descriptor_ext.is_none());
    assert!(empty.available_from.is_none());
    assert!(empty.available_to.is_none());
    assert!(empty.entitlement_grants.is_none());
    assert!(empty.change_contract.is_none());
}
