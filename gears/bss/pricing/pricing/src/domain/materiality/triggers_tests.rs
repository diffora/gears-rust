//! The registry's two properties — every trigger has an owner, and each of the
//! ones this crate can fire fires **on its own**.
//!
//! The control case at the bottom is what makes the rest evidence: without a world
//! in which no trigger fires, every assertion here passes against a
//! `triggered_by_content` that answered `Some(..)` unconditionally.

use chrono::{DateTime, TimeZone, Utc};
use uuid::Uuid;

use super::{Trigger, triggered, triggered_by_content, triggered_by_row};
use crate::domain::concurrency::RowVersion;
use crate::domain::lifecycle::LifecycleState;
use crate::domain::materiality::{ChangeSet, PublishedPriceBaseline};
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::{ModelKind, PriceRow};
use crate::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};

fn at(year: i32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, 1, 1, 0, 0, 0)
        .single()
        .expect("a real instant")
}

fn key(currency: &str) -> ScopeKey {
    ScopeKey::new(
        PlanId::new(Uuid::from_u128(1)),
        CurrencyCode::new(currency).expect("a three-letter code"),
        Region::new("EU").expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(2)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("the eight axes agree")
}

/// A published `flat` row on `currency` at `amount`.
fn row(currency: &str, amount: i64) -> PriceRecord {
    let mut shape = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    shape.amount_minor = Some(MinorAmount::new(amount).expect("a non-negative amount"));
    PriceRecord {
        price_id: Uuid::from_u128(0xd0_11),
        scope_key: key(currency),
        row: shape,
        tax_inclusive: false,
        billing_timing: Some("advance".to_owned()),
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
        lifecycle_state: LifecycleState::Published,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: at(2026),
        row_version: RowVersion::new(1),
    }
}

// ---------------------------------------------------------------------------
// The roster
// ---------------------------------------------------------------------------

/// Every registered trigger has an owning slice. **A trigger with no slice is a
/// trigger with no owner**, and a `match` over a closed enum is what makes that
/// unrepresentable — so what this asserts is that the path is a real one and not a
/// placeholder.
#[test]
fn every_trigger_names_its_slice() {
    for trigger in Trigger::ALL {
        let slice = trigger.owning_slice();
        // A path into the design set, not a slice number: the assertion is that
        // the owner is a document a reader can open. `contains` rather than
        // `ends_with` only because the extension check is a lint here.
        assert!(
            slice.starts_with("design/") && slice.contains(".md"),
            "{trigger:?} must name the document that owns its subject, got {slice}"
        );
    }
}

/// Two triggers sharing a token would make a diagnostic unable to say which act
/// required the reviewer.
#[test]
fn every_trigger_carries_a_distinct_token() {
    let mut tokens: Vec<&str> = Trigger::ALL.iter().map(|t| t.as_str()).collect();
    let declared = tokens.len();
    tokens.sort_unstable();
    tokens.dedup();

    assert_eq!(tokens.len(), declared, "every trigger needs its own token");
    assert_eq!(
        declared,
        Trigger::ALL.len(),
        "the roster and the token set are the same set"
    );
}

/// **The set whose subject this crate carries**, transcribed rather than counted.
///
/// The rest name a subject with no table, no entity and no surface here, and they
/// answer `false` so the registry does not read as incomplete. A variant added to the
/// `true` side without a writer would fail here — which is the "no token without a
/// writer" rule, asserted rather than described.
///
/// **`true` is about the subject and not about the act**, and one member of this list
/// is exactly that distinction: `grandfatherHorizonTightening` has the column, the
/// comparison and the record, and **no mounted surface can author the row pair it
/// compares** — the S7 route is unmounted and `insert_prepared` refuses a second draft
/// on an occupied key. The registry's module doc carries that at full strength; what
/// this list says is that the trigger has an owner here, which it does.
#[test]
fn only_the_triggers_with_a_subject_in_this_crate_answer_true() {
    let reachable: Vec<&str> = Trigger::ALL
        .iter()
        .filter(|t| t.subject_exists_in_this_crate())
        .map(|t| t.as_str())
        .collect();

    assert_eq!(
        reachable,
        [
            "grandfatherHorizonTightening",
            "thresholdPolicyDiff",
            "windowCancellation",
            "windowShortening",
            "noComputableRowDelta",
            "planShapeRevisionContent",
        ]
    );
}

// ---------------------------------------------------------------------------
// The act half
// ---------------------------------------------------------------------------

/// D-62's cancel, declared by the surface performing it.
#[test]
fn a_window_cancellation_is_a_registered_act() {
    let change = ChangeSet::of_act(Trigger::WindowCancellation, [row("USD", 1000)]);

    assert_eq!(triggered(&change), Some(Trigger::WindowCancellation));
}

/// D-62's shortening `PATCH`, which is a different trigger from the cancel because
/// an auditor reading the stored verdict years later needs to know which act it
/// was — the two have different remedies and different blast radii.
#[test]
fn an_effective_to_shortening_is_a_registered_act() {
    let change = ChangeSet::of_act(Trigger::WindowShortening, [row("USD", 1000)]);

    assert_eq!(triggered(&change), Some(Trigger::WindowShortening));
}

/// D-10: any policy diff, direction-agnostic.
#[test]
fn a_threshold_policy_diff_is_a_registered_act() {
    let change = ChangeSet::of_act(Trigger::ThresholdPolicyDiff, []);

    assert_eq!(triggered(&change), Some(Trigger::ThresholdPolicyDiff));
}

/// **The control for the act half.** A plan-revision publish, a window schedule
/// and a lengthening `PATCH` are not on the list, and their materiality is the
/// threshold policy's question. Without this, the three above pass against a
/// `triggered` that answered `Some(..)` for anything.
#[test]
fn an_ordinary_publish_is_not_a_registered_act() {
    let change = ChangeSet::of_records([row("USD", 1000)]);

    assert_eq!(triggered(&change), None);
}

// ---------------------------------------------------------------------------
// The content half
// ---------------------------------------------------------------------------

/// `inst-mat-registered`'s first clause. The horizon moves earlier on a row whose
/// baseline carries a later one, which cuts a grandfathered subscriber's remaining
/// life at an unchanged price.
#[test]
fn a_horizon_tightening_is_a_registered_change() {
    // Hand-built rows, and they have to be: a published row and a draft successor on
    // one key is a state no mounted surface can reach (`insert_prepared` refuses the
    // second row, `update_draft` refuses the published one), so this asserts the
    // comparison rather than a path. See the module doc.
    let mut published = row("USD", 1000);
    published.grandfather_until = Some(at(2030));
    let mut tightened = published.clone();
    tightened.grandfather_until = Some(at(2028));

    assert_eq!(
        triggered_by_row(&tightened, &published),
        Some(Trigger::GrandfatherHorizonTightening)
    );
}

/// A horizon put on a row that had none is a tightening of infinity: an absent
/// horizon is indefinite.
#[test]
fn a_horizon_set_where_there_was_none_is_a_tightening() {
    let published = row("USD", 1000);
    let mut bounded = published.clone();
    bounded.grandfather_until = Some(at(2028));

    assert_eq!(
        triggered_by_row(&bounded, &published),
        Some(Trigger::GrandfatherHorizonTightening)
    );
}

/// Loosening is **not** a trigger here: `GRANDFATHER_LOOSEN_FORBIDDEN` refuses it
/// outright at publish, and a rule that also called it material would be a second
/// owner of one refusal.
#[test]
fn a_loosened_horizon_is_not_a_registered_change() {
    let mut published = row("USD", 1000);
    published.grandfather_until = Some(at(2028));
    let mut loosened = published.clone();
    loosened.grandfather_until = Some(at(2030));

    assert_eq!(triggered_by_row(&loosened, &published), None);
}

/// D-115's row half: `billingTiming` is Billing's sole deferral input and carries
/// no price delta at all.
#[test]
fn a_contract_field_change_is_a_registered_change() {
    let published = row("USD", 1000);
    let mut deferred = published.clone();
    deferred.billing_timing = Some("arrears".to_owned());

    assert_eq!(
        triggered_by_row(&deferred, &published),
        Some(Trigger::NoComputableRowDelta)
    );
}

/// **The one D-115 exists for.** A revision whose price rows are exactly what is
/// published has changed only the plan's shape — a trial stretched 7 → 90 days, a
/// GL code moved — so the per-row evaluation has nothing to trip on and the change
/// would have gone out approver-free under any configured threshold.
#[test]
fn a_pure_shape_revision_is_a_registered_change() {
    let published = row("USD", 1000);

    let trigger = triggered_by_content(
        &ChangeSet::of_records([published.clone()]),
        &PublishedPriceBaseline::of_records([published]),
    );

    assert_eq!(trigger, Some(Trigger::PlanShapeRevisionContent));
}

/// A change set with a row the baseline does not carry is not a shape revision: it
/// moved a row, and `inst-mat-newrow` is what answers it.
#[test]
fn a_change_set_carrying_a_new_row_is_not_a_shape_revision() {
    let published = row("USD", 1000);

    let trigger = triggered_by_content(
        &ChangeSet::of_records([published.clone(), row("EUR", 900)]),
        &PublishedPriceBaseline::of_records([published]),
    );

    assert_eq!(trigger, None);
}

/// **An empty change set on a published plan IS a shape revision**, and this test
/// asserted the opposite on a false premise.
///
/// It read: *"a plan that has never carried a price row has no shape content to have
/// changed, and `inst-mat-first` owns that world"*. `inst-mat-first` owns the world
/// with **no baseline**; the argument this function takes is a baseline, so every
/// call of it is about a plan that has published. A plan whose first published
/// revision carried no price row — the world
/// `rest_windows::a_plans_first_window_is_authorable_through_the_routes_after_an_empty_publish`
/// executes — reached the row walk with nothing to trip on and auto-published its
/// **second** revision under a configured policy. That revision's whole content is
/// the plan's shape, which is what D-115 exists for.
#[test]
fn an_empty_change_set_on_a_published_plan_is_a_shape_revision() {
    let trigger = triggered_by_content(
        &ChangeSet::of_records([]),
        &PublishedPriceBaseline::of_records([]),
    );

    assert_eq!(trigger, Some(Trigger::PlanShapeRevisionContent));
}

/// **The control for the content half**, and the world §3's residue names: a pure
/// amount change on unchanged geometry is no trigger at all, so what decides it is
/// the threshold. Every assertion above would pass without this one against a
/// `triggered_by_content` that never answered `None`.
#[test]
fn a_pure_amount_change_on_unchanged_geometry_is_no_trigger_at_all() {
    let published = row("USD", 1000);

    let moved = row("USD", 1050);

    assert_eq!(
        triggered_by_content(
            &ChangeSet::of_records([moved.clone()]),
            &PublishedPriceBaseline::of_records([published.clone()]),
        ),
        None,
        "a row that moved is not a shape revision"
    );
    assert_eq!(
        triggered_by_row(&moved, &published),
        None,
        "and nothing about the row itself is registered"
    );
}
