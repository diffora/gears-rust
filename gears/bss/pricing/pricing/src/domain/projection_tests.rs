//! What the payload promises, asserted against literals rather than against
//! the struct.
//!
//! The wire keys are the whole contract here: a delta is written once, frozen,
//! and read by a consumer that has only the JSON. So a Rust field rename that
//! silently renamed a persisted key would be invisible to every test that
//! reached the payload through the field it renamed — which is the reason
//! `CatalogEvent::as_str` is pinned against literals one module over, and the
//! reason every key below is spelled out.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use chrono::{TimeZone, Utc};
use serde_json::json;
use std::collections::BTreeMap;

use super::{
    CROSS_BOUNDARY_CHANGE_POLICY, PROJECTED_ROW_STATES, PROJECTED_WINDOW_STATES, PlanSubjectDelta,
    RowTaxProjection,
};
use crate::domain::concurrency::RowVersion;
use crate::domain::evaluation_policy::EVALUATION_POLICY_GENERATION;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::plan_shape::{
    BillingCycle, CustomIntervalUnit, DescriptorSet, Frequency, PhaseKind, PlanPhase,
};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{ModelKind, PriceRow, TierBand};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use crate::domain::window::{KeyWindows, WindowInterval, WindowState};

fn plan_id() -> PlanId {
    PlanId::new(uuid::Uuid::from_u128(0x9_1a4))
}

fn terminal_phase() -> PhaseId {
    PhaseId::new(uuid::Uuid::from_u128(0xfa_5e))
}

/// A shape-only delta: a plan revision with children and **no price rows**.
fn shape_only() -> PlanSubjectDelta {
    PlanSubjectDelta {
        plan_id: plan_id(),
        revision: 3,
        lifecycle_state: LifecycleState::Published,
        sku_id: Some(uuid::Uuid::from_u128(0x5_c1)),
        plan_tier: Some("gold".to_owned()),
        plan_tier_override: false,
        billing_cycle: Some(BillingCycle::Recurring),
        frequency: Some(Frequency::Monthly),
        available_from: Some(Utc.with_ymd_and_hms(2026, 8, 1, 0, 0, 0).unwrap()),
        available_to: None,
        purchase_min_qty: None,
        purchase_max_qty: None,
        invoice_grouping_key: Some("bundle-a".to_owned()),
        phases: vec![PlanPhase {
            phase_id: terminal_phase(),
            kind: PhaseKind::Evergreen,
            ordinal: 0,
            converts_to_phase_id: None,
            phase_duration_days: None,
            display_trial_days: None,
        }],
        addon_rules: Vec::new(),
        descriptor_set: Some(DescriptorSet {
            invoice_line_template: Some("{plan}".to_owned()),
            gl_code: Some("4000".to_owned()),
            itemization_rule: Some("per_charge".to_owned()),
            additional: std::collections::BTreeMap::new(),
        }),
        prices: Vec::new(),
        tax_projection: BTreeMap::new(),
        windows: Vec::new(),
    }
}

fn graduated_row() -> PriceRecord {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("api_calls".to_owned());
    row.bands = vec![
        TierBand::closed(0, 100, MinorAmount::new(0).expect("a non-negative amount")),
        TierBand::open(100, MinorAmount::new(5).expect("a non-negative amount")),
    ];
    PriceRecord {
        price_id: uuid::Uuid::from_u128(0xb_0001),
        scope_key: ScopeKey::new(
            plan_id(),
            CurrencyCode::new("EUR").expect("three letters"),
            Region::new("eu").expect("a non-blank region"),
            terminal_phase(),
            PriceEligibility::AllSubscriptions,
            ChargeKind::Usage,
            Cohort::None,
        )
        .expect("the class pairs with cohort none"),
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: None,
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Published,
        created_by: uuid::Uuid::from_u128(0xac_10),
        created_at_utc: Utc.with_ymd_and_hms(2026, 8, 1, 10, 0, 0).unwrap(),
        row_version: RowVersion::new(0),
    }
}

#[test]
fn the_plan_level_wire_keys_are_what_a_consumer_reads() {
    let value = shape_only().to_value();

    assert_eq!(value.get("planId"), Some(&json!(plan_id().get())));
    assert_eq!(value.get("revision"), Some(&json!(3)));
    assert_eq!(value.get("lifecycleState"), Some(&json!("published")));
    assert_eq!(
        value.get("skuId"),
        Some(&json!(uuid::Uuid::from_u128(0x5_c1)))
    );
    assert_eq!(value.get("planTier"), Some(&json!("gold")));
    assert_eq!(value.get("planTierOverride"), Some(&json!(false)));
    assert_eq!(value.get("billingCycle"), Some(&json!("recurring")));
    assert_eq!(
        value.get("frequency"),
        Some(&json!({ "token": "monthly" })),
        "a fixed frequency carries its token and no interval"
    );
    assert_eq!(value.get("availableTo"), Some(&json!(null)));
    assert_eq!(value.get("invoiceGroupingKey"), Some(&json!("bundle-a")));
    assert_eq!(
        value.get("phases"),
        Some(&json!([{
            "phaseId": terminal_phase().get(),
            "kind": "evergreen",
            "ordinal": 0,
            "convertsToPhaseId": null,
            "phaseDurationDays": null,
            "displayTrialDays": null,
        }]))
    );
    assert_eq!(
        value.get("descriptorSet"),
        Some(&json!({
            "invoiceLineTemplate": "{plan}",
            "glCode": "4000",
            "itemizationRule": "per_charge",
            "additional": {},
        }))
    );
}

#[test]
fn a_custom_frequency_carries_its_interval_and_not_only_its_token() {
    // The pairing `chk_pricing_plan_custom_interval_pairing` exists to keep: a
    // frozen `custom_every_n` with no `n` is a billing period nobody can
    // compute, and the token alone cannot reconstruct one.
    let delta = PlanSubjectDelta {
        frequency: Some(Frequency::CustomEveryN {
            n: 45,
            unit: CustomIntervalUnit::Days,
        }),
        ..shape_only()
    };

    assert_eq!(
        delta.to_value().get("frequency"),
        Some(&json!({ "token": "custom_every_n", "n": 45, "unit": "days" }))
    );
}

#[test]
fn the_lifecycle_field_renders_retired_as_well_as_published() {
    // The whole of D-128 in one assertion. The revision's state is a projected
    // plan-subject field because it is what sellability predicate (4) reads at
    // the pin, and a retired plan can never publish again — so if `retired` did
    // not reach the payload, nothing would ever correct the delta and the read
    // model would advertise a retired plan as sellable permanently.
    for state in [LifecycleState::Published, LifecycleState::Retired] {
        let delta = PlanSubjectDelta {
            lifecycle_state: state,
            ..shape_only()
        };
        assert_eq!(
            delta.to_value().get("lifecycleState"),
            Some(&json!(state.as_str())),
            "the projected lifecycle state must reach the payload"
        );
    }
}

#[test]
fn the_payload_carries_the_declared_generation_and_this_file_does_not_spell_it() {
    // Read from the constant, never from a literal here: a second spelling of
    // the generation is a test that stays green through the one edit it exists
    // to catch (D-162 — the document declares it, `evaluation_policy_tests`
    // pins the constant to the document, and this pins the payload to the
    // constant).
    assert_eq!(
        shape_only().to_value().get("evaluationPolicyVersion"),
        Some(&json!(EVALUATION_POLICY_GENERATION))
    );
}

#[test]
fn every_plan_subject_carries_the_cross_boundary_marker_and_no_warning_text() {
    // D-169 clause (1). The marker is a launch-constant, tenant-wide value on
    // every resolved plan subject; the text that used to sit beside it is not a
    // catalog field at all - its normative home is PRD AC #66, on the surface
    // that renders the warning and takes the operator's confirmation.
    //
    // Read from the constant rather than from a literal, for
    // `evaluationPolicyVersion`'s reason one test up: a second spelling of the
    // value is a test that stays green through the one edit it exists to catch.
    let with_row = PlanSubjectDelta {
        prices: vec![graduated_row()],
        ..shape_only()
    };
    for delta in [shape_only(), with_row] {
        let value = delta.to_value();
        assert_eq!(
            value.get("crossBoundaryChangePolicy"),
            Some(&json!(CROSS_BOUNDARY_CHANGE_POLICY)),
            "{value}"
        );
        // Under any spelling: the field left the contract, so a payload
        // carrying it would be publishing a sentence nobody authored into an
        // INSERT-only store on the seven-year horizon.
        let rendered = value.to_string();
        assert!(
            !rendered.to_ascii_lowercase().contains("warningtext"),
            "no delta may carry a warning text: {rendered}"
        );
    }
}

#[test]
fn the_marker_is_the_value_the_design_set_names_verbatim() {
    // Asserted against the literal here and nowhere else in the crate, for
    // `CatalogEvent::as_str`'s reason: `06-consumer-contracts.md` sec 6 names
    // this string, a consumer matches on it exactly, and nothing else in the
    // crate would notice a typo.
    assert_eq!(CROSS_BOUNDARY_CHANGE_POLICY, "cancel_plus_new");
}

#[test]
fn a_revision_with_no_price_rows_still_renders_a_well_formed_payload() {
    // A shape-only revision — one that changes a descriptor or a phase duration
    // and authors no price rows — is an ordinary publish, so its delta is an
    // ordinary payload with an empty row set rather than an absent one. A
    // missing key and an empty array are not the same claim about a plan.
    let value = shape_only().to_value();

    assert!(value.is_object());
    assert_eq!(value.get("prices"), Some(&json!([])));
    assert_eq!(value.get("addonRules"), Some(&json!([])));
}

#[test]
fn a_price_row_freezes_its_key_its_shape_and_its_bands() {
    let delta = PlanSubjectDelta {
        prices: vec![graduated_row()],
        ..shape_only()
    };
    let value = delta.to_value();
    let rows = value
        .get("prices")
        .expect("prices")
        .as_array()
        .expect("array");
    assert_eq!(rows.len(), 1);
    let row = &rows[0];

    assert_eq!(
        row.get("priceId"),
        Some(&json!(uuid::Uuid::from_u128(0xb_0001)))
    );
    assert_eq!(row.get("lifecycleState"), Some(&json!("published")));
    assert_eq!(row.get("modelKind"), Some(&json!("graduated")));
    assert_eq!(row.get("chargeKind"), Some(&json!("usage")));
    assert_eq!(row.get("meter"), Some(&json!("api_calls")));
    assert_eq!(row.get("roundingPolicyRef"), Some(&json!("half_up")));
    assert_eq!(
        row.get("scopeKey"),
        Some(&json!({
            "planId": plan_id().get(),
            "currency": "EUR",
            "region": "eu",
            "priceOverlay": "base",
            "phase": terminal_phase().get(),
            "priceEligibility": "all_subscriptions",
            "chargeKind": "usage",
            "cohort": null,
            // Axes 9 and 10 (D-196). `null` on this row because the fixture's key
            // carries no line; the delta is where a consumer resolving a metered
            // plan reads which line a published usage row prices, and before
            // D-196 it could not need to — one market held one usage row.
            "meter": null,
            "dimensionKey": null,
        })),
        "all ten canonical axes, in the normative order"
    );
    assert_eq!(
        row.get("bands"),
        Some(&json!([
            { "fromQty": 0, "toQty": 100, "unitPriceMinor": 0 },
            { "fromQty": 100, "toQty": null, "unitPriceMinor": 5 },
        ])),
        "an open top is null, never a sentinel a reader could compare against"
    );
}

#[test]
fn the_projected_row_states_are_the_two_that_are_not_never_published_drafts() {
    // D-121's set, and `superseded` is in it although nothing in this gear can
    // produce one: rating rates past instants, so a changeover's predecessor
    // must survive re-projection or yesterday's `t` fails closed on a covered
    // period. Listing it now means the day D-88 or D-100 lands, only the
    // horizon is owed.
    assert_eq!(
        PROJECTED_ROW_STATES,
        &[LifecycleState::Published, LifecycleState::Superseded]
    );
    assert!(
        !PROJECTED_ROW_STATES.contains(&LifecycleState::Draft),
        "a never-published draft is exactly what D-121 excludes"
    );
}

// ---------------------------------------------------------------------------
// The window facts (D-99, D-121)
// ---------------------------------------------------------------------------

/// The eight axes and one window on the plan's recurring key.
fn recurring_key() -> ScopeKey {
    ScopeKey::new(
        plan_id(),
        CurrencyCode::new("USD").expect("iso currency"),
        Region::new("EU").expect("region"),
        terminal_phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("a valid canonical scope key")
}

fn at(day: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, day, 0, 0, 0)
        .single()
        .expect("a well-defined UTC instant")
}

/// D-121's projected window states: `scheduled | active | expired`.
///
/// A CANCELLED window is not projected - it is not history a consumer resolves
/// against, it is a schedule that never happened. An `expired` window IS
/// projected: rating pins current versions and rates past instants, so dropping
/// the predecessor's expired interval fails a legitimately covered arrears
/// period closed.
#[test]
fn the_projected_window_states_are_the_three_d121_names() {
    assert_eq!(
        PROJECTED_WINDOW_STATES,
        [
            WindowState::Scheduled,
            WindowState::Active,
            WindowState::Expired
        ]
    );
    assert!(
        !PROJECTED_WINDOW_STATES.contains(&WindowState::Cancelled),
        "a cancelled window is a schedule that never happened"
    );
    assert!(
        PROJECTED_WINDOW_STATES.contains(&WindowState::Expired),
        "the predecessor's expired interval is what an arrears period at a past \
         instant resolves against"
    );
    // Every state accounted for: three in, one out. A fifth state fails this
    // rather than silently joining or missing the projection.
    assert_eq!(WindowState::ALL.len(), 4);
}

/// The delta carries intervals and states and a derived coverage end - never a
/// point-in-time boolean (D-99).
///
/// The `activeNow`-shaped absence is what this test is for. Were the payload to
/// carry one, every activation and expiry would owe a re-projection of an
/// INSERT-only store whose whole contract is that a completed version never
/// changes - which is precisely why `inst-ws-publishunit` makes those two
/// transitions *not* publish units.
#[test]
fn the_payload_carries_intervals_and_a_coverage_end_and_no_active_flag() {
    let delta = PlanSubjectDelta {
        windows: vec![KeyWindows {
            scope_key: recurring_key(),
            intervals: vec![
                WindowInterval::new(at(10), Some(at(20)), WindowState::Expired),
                WindowInterval::new(at(20), Some(at(30)), WindowState::Active),
            ],
        }],
        ..shape_only()
    };

    let value = delta.to_value();
    let groups = value
        .get("windows")
        .expect("windows")
        .as_array()
        .expect("array");
    assert_eq!(groups.len(), 1, "one key, one group");
    let group = &groups[0];

    // The key the facts are filed under, rendered axis by axis exactly as a
    // price row's is - a consumer matches on axes, not on a display string.
    assert_eq!(
        group.get("scopeKey").and_then(|k| k.get("chargeKind")),
        Some(&json!("recurring"))
    );
    // The intervals, in time order, each with its state.
    assert_eq!(
        group.get("intervals"),
        Some(&json!([
            {
                "effectiveFrom": at(10),
                "effectiveTo": at(20),
                "state": "expired",
            },
            {
                "effectiveFrom": at(20),
                "effectiveTo": at(30),
                "state": "active",
            },
        ]))
    );
    // The derived end, as a discriminated object: a bare null would have to
    // stand for "covered forever" and "covered nowhere" at once, which are
    // opposite answers under the D-80 horizon predicate.
    assert_eq!(
        group.get("coverageEnd"),
        Some(&json!({ "kind": "ends", "at": at(30) }))
    );

    // And no point-in-time answer anywhere in the document, under any spelling.
    let rendered = value.to_string().to_ascii_lowercase();
    for forbidden in ["activenow", "isactive", "activeat", "sellablenow"] {
        assert!(
            !rendered.contains(forbidden),
            "a frozen delta must not answer a question about the reader's clock \
             ({forbidden}): {rendered}"
        );
    }
}

/// An open-ended key renders the open end as its own kind, and a key with no
/// coverage renders as uncovered - the two answers a nullable instant would
/// have had to share.
#[test]
fn open_ended_and_uncovered_are_two_different_payload_answers() {
    let open = PlanSubjectDelta {
        windows: vec![KeyWindows {
            scope_key: recurring_key(),
            intervals: vec![WindowInterval::new(at(10), None, WindowState::Active)],
        }],
        ..shape_only()
    };
    let uncovered = PlanSubjectDelta {
        windows: vec![KeyWindows {
            scope_key: recurring_key(),
            intervals: Vec::new(),
        }],
        ..shape_only()
    };

    let end = |delta: PlanSubjectDelta| {
        delta
            .to_value()
            .get("windows")
            .expect("windows")
            .as_array()
            .expect("array")[0]
            .get("coverageEnd")
            .cloned()
            .expect("coverageEnd")
    };

    assert_eq!(end(open), json!({ "kind": "open_ended", "at": null }));
    assert_eq!(end(uncovered), json!({ "kind": "uncovered", "at": null }));
}

/// A plan with no windows renders an empty array rather than an absent key: a
/// missing key and an empty array are not the same claim about a plan.
#[test]
fn a_plan_with_no_windows_still_renders_the_key() {
    assert_eq!(shape_only().to_value().get("windows"), Some(&json!([])));
}

// ---------------------------------------------------------------------------
// D-154's resolved category and C3's GA gate — the derived pair (§6).
// ---------------------------------------------------------------------------

/// A delta carrying one row and the tax facts derived for it.
fn delta_with_tax(record: PriceRecord, tax: RowTaxProjection) -> PlanSubjectDelta {
    let mut delta = shape_only();
    let price_id = record.price_id;
    delta.prices = vec![record];
    delta.tax_projection = [(price_id, tax)].into_iter().collect();
    delta
}

/// The payload carries **both** derived facts beside the authored column.
///
/// The authored `taxCategoryRef` and the resolved `resolvedTaxCategory` are two
/// fields, not one: D-110 makes the row the source of truth and D-154 freezes
/// what a consumer should use, and a payload rendering only the resolved value
/// would leave an operator unable to see what they actually authored.
#[test]
fn a_projected_row_carries_the_resolved_category_and_the_ga_flag() {
    let mut record = graduated_row();
    record.tax_category_ref = None;
    record.tax_inclusive = true;
    let delta = delta_with_tax(
        record,
        RowTaxProjection {
            resolved_tax_category: Some("standard".to_owned()),
            not_sellable_ga: true,
        },
    );

    let value = delta.to_value();
    let row = &value["prices"][0];

    assert_eq!(
        row["taxCategoryRef"],
        json!(null),
        "the authored column is what the row states, and it states nothing"
    );
    assert_eq!(
        row["resolvedTaxCategory"], "standard",
        "and the resolved value is the region default D-154 froze"
    );
    assert_eq!(row["notSellableGa"], true);
}

/// A row with no derived facts renders the pair as absent rather than omitting
/// the keys.
///
/// A consumer must be able to tell *this version resolved nothing* from *this
/// field is not part of the contract*; an omitted key says the second.
#[test]
fn a_row_with_no_tax_projection_still_renders_both_keys() {
    let delta = {
        let mut d = shape_only();
        d.prices = vec![graduated_row()];
        d
    };

    let value = delta.to_value();
    let row = &value["prices"][0];

    assert_eq!(row["resolvedTaxCategory"], json!(null));
    assert_eq!(
        row["notSellableGa"], false,
        "absent is not gated: the gate is a positive fact a publish derives"
    );
}

/// The projection is **per row**, so one gated market does not gate its sibling.
///
/// `inst-td-gagate` is explicit that the flag is per `(currency, region)` market
/// and never per plan: "a plan selling tax-exclusive in US and tax-inclusive in
/// EU is gated **only** on its EU market(s)".
#[test]
fn the_ga_flag_is_carried_per_row_and_not_across_the_plan() {
    let mut gated = graduated_row();
    gated.tax_inclusive = true;
    let mut open = graduated_row();
    open.price_id = uuid::Uuid::from_u128(0xb0_02);
    open.tax_inclusive = false;

    let mut delta = shape_only();
    delta.tax_projection = [
        (
            gated.price_id,
            RowTaxProjection {
                resolved_tax_category: Some("standard".to_owned()),
                not_sellable_ga: true,
            },
        ),
        (
            open.price_id,
            RowTaxProjection {
                resolved_tax_category: Some("standard".to_owned()),
                not_sellable_ga: false,
            },
        ),
    ]
    .into_iter()
    .collect();
    delta.prices = vec![gated, open];

    let value = delta.to_value();

    assert_eq!(value["prices"][0]["notSellableGa"], true);
    assert_eq!(
        value["prices"][1]["notSellableGa"], false,
        "the sibling market stays sellable"
    );
}
