//! The two facts the outbox writer decides before it touches a database: what
//! makes a repeat of one publish the same event, and what a consumer receives.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use serde_json::json;
use uuid::Uuid;

use super::{NewOutboxEvent, PlanPublishedPayload, outbox_id, plan_published_dedup_key};
use crate::domain::events::CatalogEvent;
use crate::domain::scope_key::PlanId;

const PLAN: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00a1);
const TENANT: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00b1);
const CORRELATION: Uuid = Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00c1);

fn payload() -> PlanPublishedPayload {
    PlanPublishedPayload {
        plan_id: PlanId::new(PLAN),
        revision: 2,
        pending_version_ref: "pend-7".to_owned(),
        price_ids: vec![Uuid::from_u128(1), Uuid::from_u128(2)],
        correlation_id: CORRELATION,
    }
}

#[test]
fn one_revision_publishes_under_one_dedup_key() {
    // A repeat of the same publish is the same key, which is what makes
    // `uq_pricing_outbox_dedup_key` refuse it at the writer.
    assert_eq!(
        plan_published_dedup_key(PlanId::new(PLAN), 2),
        plan_published_dedup_key(PlanId::new(PLAN), 2)
    );
    assert_eq!(
        plan_published_dedup_key(PlanId::new(PLAN), 2),
        format!("PlanPublished/{PLAN}/2")
    );
}

#[test]
fn two_different_publishes_are_two_different_keys() {
    let plan = PlanId::new(PLAN);
    let other = PlanId::new(Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00a2));

    assert_ne!(
        plan_published_dedup_key(plan, 2),
        plan_published_dedup_key(plan, 3),
        "a second revision of one plan is a second publish"
    );
    assert_ne!(
        plan_published_dedup_key(plan, 2),
        plan_published_dedup_key(other, 2),
        "the same revision number of two plans is two publishes"
    );
}

#[test]
fn the_event_name_in_the_key_comes_from_the_frozen_set() {
    assert!(
        plan_published_dedup_key(PlanId::new(PLAN), 0)
            .starts_with(CatalogEvent::PlanPublished.as_str())
    );
}

#[test]
fn the_surrogate_key_agrees_with_the_dedup_index() {
    // Derived rather than random, so a repeated publish collides on the primary
    // key as well as on the unique index.
    let key = plan_published_dedup_key(PlanId::new(PLAN), 2);

    assert_eq!(outbox_id(TENANT, &key), outbox_id(TENANT, &key));
    assert_ne!(
        outbox_id(TENANT, &key),
        outbox_id(TENANT, &plan_published_dedup_key(PlanId::new(PLAN), 3))
    );
    assert_ne!(
        outbox_id(TENANT, &key),
        outbox_id(
            Uuid::from_u128(0x0000_0000_0000_0000_0000_0000_0000_00b2),
            &key
        ),
        "the dedup index is per tenant, and so is the surrogate"
    );
}

#[test]
fn the_payload_carries_the_pending_handle_and_the_published_rows() {
    assert_eq!(
        payload().to_value(),
        json!({
            "planId": PLAN,
            "revision": 2,
            "pendingVersionRef": "pend-7",
            "priceIds": [Uuid::from_u128(1), Uuid::from_u128(2)],
            "correlationId": CORRELATION,
        })
    );
}

#[test]
fn the_payload_stamps_no_snapshot_ref() {
    // `PricingSnapshotRef` has three parts and the commit produces two: nothing
    // in this gear produces `evaluation_policy_version` and no document says
    // what does. A placeholder here is a value a consumer would pin against.
    let rendered = payload().to_value();
    let object = rendered.as_object().expect("a payload is a JSON object");

    assert!(!object.contains_key("pricingSnapshotRef"));
    assert!(!object.contains_key("evaluationPolicyVersion"));
}

#[test]
fn the_constructor_fixes_the_name_the_aggregate_and_the_key() {
    let at = Utc.with_ymd_and_hms(2026, 8, 3, 9, 0, 0).unwrap();
    let event = NewOutboxEvent::plan_published(TENANT, &payload(), at);

    assert_eq!(event.event, CatalogEvent::PlanPublished);
    assert_eq!(event.aggregate_id, PLAN, "ordering is per plan");
    assert_eq!(
        event.dedup_key,
        plan_published_dedup_key(PlanId::new(PLAN), 2)
    );
    assert_eq!(event.correlation_id, CORRELATION);
    assert_eq!(event.enqueued_at, at);
}
