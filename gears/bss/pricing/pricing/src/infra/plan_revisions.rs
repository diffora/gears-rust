//! The `plan_revision` approval kind (spec §6): one draft revision of a plan, published on
//! approval with its predecessor superseded and the plan's `published_rev` advanced.
use serde_json::{Value, json};

/// The approval kind of one plan revision.
pub const KIND_PLAN_REVISION: &str = crate::domain::plan::KIND_PLAN_REVISION;
/// A `plan_revision` unit references its revision.
pub const REF_TYPE: &str = "plan_revision";
/// The one item of a `plan_revision` unit is the revision.
pub const ITEM_TYPE: &str = "plan_revision";
/// The honest answer for the subscriptions a unit touches: pricing cannot count them before
/// the Subscriptions integration reports its pins.
pub const SUBSCRIPTIONS_UNAVAILABLE: &str = "unavailable until the Subscriptions integration";

/// What a revision's publication touches that pricing can measure: nothing yet, since
/// publishing a revision moves no existing pin (D-394).
#[must_use]
pub fn impact() -> Value {
    json!({ "subscriptions": SUBSCRIPTIONS_UNAVAILABLE })
}
