//! The chain-key constructors, and the one property none of them can be asserted
//! about after the fact: **two aggregates on one segment verify perfectly.**
//!
//! A collision here is the invisible defect [`super::policy_chain`]'s doc names — the
//! hash chain over an interleaved segment is internally consistent, every row's
//! `prev_hash` matches, and an auditor reading it is told a coherent story about a
//! plan that includes another aggregate's mutations. So the disjointness is asserted
//! where it is decided rather than where it would be discovered.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use uuid::Uuid;

use super::{bulk_operation_chain, overlay_chain, plan_chain, policy_chain};
use crate::domain::scope_key::PlanId;

/// The version nibble — the 13th hex digit, `xxxxxxxx-xxxx-Vxxx-…`.
const VERSION_MASK: u128 = 0xF << 76;

fn version_nibble(id: Uuid) -> u8 {
    u8::try_from((id.as_u128() >> 76) & 0xF).expect("four bits")
}

#[test]
fn overlay_chains_are_disjoint_from_plan_chains_by_construction() {
    // Not a sample of ids that happen not to collide — that would be the
    // improbability argument the module refuses. The three claims are structural and
    // each is checked as such.
    for _ in 0..64 {
        let overlay_id = Uuid::now_v7();
        let plan_id = PlanId::new(Uuid::now_v7());

        // (1) Every plan chain carries v7's nibble, every overlay chain carries 8, so
        //     no plan chain is any overlay chain — whatever the ids.
        assert_eq!(version_nibble(plan_chain(plan_id)), 7);
        assert_eq!(version_nibble(overlay_chain(overlay_id)), 8);

        // (2) …including the adversarial case the raw-id spelling would fail: the
        //     plan and the overlay carrying **the same** id. Under `plan_chain`'s
        //     spelling these two would be one segment.
        let shared = Uuid::now_v7();
        assert_ne!(overlay_chain(shared), plan_chain(PlanId::new(shared)));

        // (3) The policy segment is a constant with a zero timestamp, which `now_v7`
        //     cannot mint, so it is not any overlay's either.
        assert_ne!(overlay_chain(overlay_id), policy_chain());
    }
}

#[test]
fn distinct_overlays_get_distinct_chains() {
    // Injectivity, which the nibble rewrite could plausibly have cost: it is the
    // property that keeps two overlays' histories from being one segment, and it
    // holds because only the version nibble moves and every input carries the same
    // one.
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    assert_ne!(a, b, "two v7 ids, minted apart");
    assert_ne!(overlay_chain(a), overlay_chain(b));

    // And the rewrite is a rewrite rather than a mask: everything but the nibble
    // survives, so the chain still carries the overlay's timestamp and entropy.
    assert_eq!(
        overlay_chain(a).as_u128() & !VERSION_MASK,
        a.as_u128() & !VERSION_MASK
    );
}

#[test]
fn bulk_operation_chains_are_disjoint_from_plan_and_overlay_chains_by_construction() {
    // The same structural argument `overlay_chains_are_disjoint_from_plan_chains_by_construction`
    // makes, one nibble over: nibble `9` is disjoint from `7` (every plan chain) and
    // from `8` (every overlay chain and the policy chain) by construction, not by
    // the ids involved happening not to collide.
    for _ in 0..64 {
        let operation_id = Uuid::now_v7();
        let overlay_id = Uuid::now_v7();
        let plan_id = PlanId::new(Uuid::now_v7());

        assert_eq!(version_nibble(bulk_operation_chain(operation_id)), 9);
        assert_eq!(version_nibble(plan_chain(plan_id)), 7);
        assert_eq!(version_nibble(overlay_chain(overlay_id)), 8);

        // The adversarial case: one id minted once, read as all three kinds. Under a
        // raw-id spelling these would be one segment.
        let shared = Uuid::now_v7();
        assert_ne!(
            bulk_operation_chain(shared),
            plan_chain(PlanId::new(shared))
        );
        assert_ne!(bulk_operation_chain(shared), overlay_chain(shared));

        // The policy segment's fixed, zero-timestamp value is not any bulk
        // operation's either.
        assert_ne!(bulk_operation_chain(operation_id), policy_chain());
    }
}

#[test]
fn distinct_bulk_operations_get_distinct_chains() {
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    assert_ne!(a, b, "two v7 ids, minted apart");
    assert_ne!(bulk_operation_chain(a), bulk_operation_chain(b));

    assert_eq!(
        bulk_operation_chain(a).as_u128() & !VERSION_MASK,
        a.as_u128() & !VERSION_MASK
    );
}
