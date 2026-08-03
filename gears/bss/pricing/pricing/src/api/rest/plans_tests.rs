//! What the plan views promise on the wire, and that the paths are spelled once.
//!
//! Everything that needs a database or a PDP is in `tests/rest_plans.rs`. What
//! is here is the shape of the body — in particular the one distinction a
//! collapsing serializer would silently destroy.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use uuid::Uuid;

use super::PlanView;
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::plan::PlanRevision;
use crate::domain::plan_shape::{BillingCycle, CustomIntervalUnit, DescriptorSet, Frequency};
use crate::domain::scope_key::PlanId;

fn revision(plan_id: PlanId) -> PlanRevision {
    PlanRevision {
        plan_id,
        revision: 3,
        sku_id: None,
        plan_tier: Some("gold".to_owned()),
        billing_cycle: Some(BillingCycle::Recurring),
        frequency: Some(Frequency::CustomEveryN {
            n: 45,
            unit: CustomIntervalUnit::Days,
        }),
        plan_tier_override: false,
        purchase_min_qty: None,
        purchase_max_qty: None,
        invoice_grouping_key: None,
        available_from: None,
        available_to: None,
        lifecycle_state: LifecycleState::Draft,
        created_by: Uuid::from_u128(0xac_12),
        created_at_utc: Utc.with_ymd_and_hms(2026, 8, 3, 9, 0, 0).unwrap(),
        row_version: RowVersion::new(4),
    }
}

fn body(view: &PlanView) -> serde_json::Value {
    serde_json::to_value(view).expect("the view serializes")
}

#[test]
fn an_unattached_descriptor_set_is_null_and_an_empty_one_is_an_object() {
    // The store keeps the distinction — an unattached set has no row — and
    // `DESCRIPTOR_INCOMPLETE` is asked of an ATTACHED set, so collapsing the two
    // would make "nobody attached one" and "somebody attached an empty one" the
    // same publish input.
    let plan_id = PlanId::new(Uuid::now_v7());

    let unattached = PlanView::new(revision(plan_id), Vec::new(), Vec::new(), None);
    assert!(
        body(&unattached)["descriptor_set"].is_null(),
        "{}",
        body(&unattached)
    );

    let attached = PlanView::new(
        revision(plan_id),
        Vec::new(),
        Vec::new(),
        Some(DescriptorSet::default()),
    );
    let rendered = body(&attached);
    assert!(rendered["descriptor_set"].is_object(), "{rendered}");
    assert!(
        rendered["descriptor_set"]["gl_code"].is_null(),
        "{rendered}"
    );
    assert!(
        rendered["descriptor_set"]["additional"].is_object(),
        "{rendered}"
    );
}

#[test]
fn the_view_names_which_revision_it_answered() {
    // A caller must never have to infer whether it was given the draft or the
    // current revision - the next PATCH depends on it.
    let rendered = body(&PlanView::new(
        revision(PlanId::new(Uuid::now_v7())),
        Vec::new(),
        Vec::new(),
        None,
    ));

    assert_eq!(rendered["revision"], serde_json::json!(3));
    assert_eq!(rendered["lifecycle_state"], serde_json::json!("draft"));
    assert_eq!(rendered["row_version"], serde_json::json!(4));
}

#[test]
fn a_custom_frequency_carries_its_interval_and_a_fixed_one_carries_none() {
    // Three flat members would let a caller send `monthly` with an interval —
    // the pairing the CHECK constraint and `Frequency` both exist to refuse.
    let mut fixed = revision(PlanId::new(Uuid::now_v7()));
    fixed.frequency = Some(Frequency::Monthly);
    let rendered = body(&PlanView::new(fixed, Vec::new(), Vec::new(), None));
    assert_eq!(rendered["frequency"]["kind"], serde_json::json!("monthly"));
    assert!(rendered["frequency"]["custom_interval_n"].is_null());

    let custom = body(&PlanView::new(
        revision(PlanId::new(Uuid::now_v7())),
        Vec::new(),
        Vec::new(),
        None,
    ));
    assert_eq!(
        custom["frequency"]["kind"],
        serde_json::json!("custom_every_n")
    );
    assert_eq!(
        custom["frequency"]["custom_interval_n"],
        serde_json::json!(45)
    );
    assert_eq!(
        custom["frequency"]["custom_interval_unit"],
        serde_json::json!("days")
    );
}
