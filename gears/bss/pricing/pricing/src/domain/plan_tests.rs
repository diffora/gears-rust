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

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::PlanShapePatch;

#[test]
fn the_patch_names_five_columns_and_cannot_mean_clear_any_of_them() {
    // The exhaustive struct literal is the assertion, and it is a compile-time
    // one: when a slice widens what a draft edit may touch, this stops
    // compiling, so the widening becomes a decision somebody made rather than a
    // field that appeared.
    let _five_columns = PlanShapePatch {
        sku_id: Some(Uuid::from_u128(1)),
        plan_tier: Some("silver".to_owned()),
        billing_cycle: Some("annual".to_owned()),
        available_from: Some(Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap()),
        available_to: Some(Utc.with_ymd_and_hms(2028, 1, 1, 0, 0, 0).unwrap()),
    };

    // The empty patch has to be empty in **every** field, and that is what makes
    // "absent = leave alone" total: with nothing defaulting to a value, no patch
    // can mean "set this column to NULL". A `Default` that pre-filled one field
    // would turn the identity edit into a write nobody asked for, and is the one
    // way this encoding can lie.
    let empty = PlanShapePatch::default();
    assert!(empty.sku_id.is_none());
    assert!(empty.plan_tier.is_none());
    assert!(empty.billing_cycle.is_none());
    assert!(empty.available_from.is_none());
    assert!(empty.available_to.is_none());
}
