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
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: None,
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

/// Every registered trigger has an owning slice, **and the document opens**.
///
/// `owning_slice`'s own doc gives the field one purpose — *"a path rather than a
/// slice number, so a reader greps once"* — and a path that greps to nothing
/// serves it no better than a slice number would.
///
/// **This assertion used to be that the string was well formed**: `starts_with
///("design/")` and `contains(".md")`, under a doc claiming it asserted "the path is
/// a real one and not a placeholder". It asserted no such thing, and the gap was not
/// theoretical — it passed green over **two** wrong paths across **four** of the
/// eighteen triggers (`design/04-market-tax.md`, whose file is `04-currency-tax.md`,
/// and `design/09-overlays-groups.md`, whose file is `09-price-overlays.md`). A
/// shape check cannot tell a document from a plausible name for one, so this opens
/// the file instead.
///
/// The docs live one directory up from the crate, which is why the base is
/// `CARGO_MANIFEST_DIR/..` and not a relative path: a test's working directory is
/// the *workspace* root under one runner and the crate root under another, and a
/// relative path would make this assertion answer differently depending on how it
/// was invoked.
#[test]
fn every_trigger_names_a_design_document_that_opens() {
    let docs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../docs");
    for trigger in Trigger::ALL {
        let slice = trigger.owning_slice();
        assert!(
            slice.starts_with("design/") && slice.contains(".md"),
            "{trigger:?} must name the document that owns its subject, got {slice}"
        );
        let path = docs.join(slice);
        assert!(
            path.is_file(),
            "{trigger:?} names {slice}, which is not a file in the design set"
        );
    }
}

/// **The census behind the attestation.** A trigger that answers `true` on the
/// act half must be *named by a producing site outside this registry*.
///
/// `subject_exists_in_this_crate` is the one predicate in the file that nothing
/// checks: it is a hand-written `match`, and a `true` added to it compiles,
/// passes every other case here, and is read by later authors as a statement
/// that the work landed. That is not hypothetical — `bulkGroupMove` answered
/// `true` under a dated comment claiming both membership triggers were "paid
/// 2026-08-12", and **no file in `src/` constructed it**: the only `of_act` on
/// that plane always passes `ImmediateMembershipReresolution`, the move route
/// builds a single-payer set, and the sole other occurrence in the tree was a
/// test. The transcription case below could not see it — a transcription copies
/// whatever the `match` says.
///
/// # What is exempted, and why it is three names rather than a predicate
///
/// The **content half** is produced inside this module and by design has no call
/// site anywhere else: [`triggered_by_row`] mints `grandfatherHorizonTightening`
/// and `noComputableRowDelta`, and [`triggered_by_content`] mints
/// `planShapeRevisionContent`. Those two functions are the census's blind spot
/// and the three names below are transcribed from their bodies, in this file's
/// standing style — a roster that is copied reddens when the thing it copies
/// moves, which is the obligation it exists to create.
///
/// A **test file is not a producer.** `_tests.rs` is excluded for the reason the
/// finding turns on: a variant mentioned only by a case asserting about it is
/// exactly the state `bulkGroupMove` was in.
#[test]
fn every_act_half_trigger_answering_true_is_named_by_a_producing_site() {
    /// Minted inside this module by the content half, so no other file names
    /// them — transcribed from `triggered_by_row` and `triggered_by_content`.
    const MINTED_BY_THE_CONTENT_HALF: &[&str] = &[
        "grandfatherHorizonTightening",
        "noComputableRowDelta",
        "planShapeRevisionContent",
    ];

    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let sources = rust_sources(&src);
    assert!(
        sources.len() > 50,
        "the walk found {} files, which is not this crate",
        sources.len()
    );

    let mut unproduced: Vec<&str> = Vec::new();
    for trigger in Trigger::ALL {
        if !trigger.subject_exists_in_this_crate()
            || MINTED_BY_THE_CONTENT_HALF.contains(&trigger.as_str())
        {
            continue;
        }
        let needle = format!("Trigger::{trigger:?}");
        if !sources.iter().any(|body| body.contains(&needle)) {
            unproduced.push(trigger.as_str());
        }
    }

    assert!(
        unproduced.is_empty(),
        "these triggers answer `subject_exists_in_this_crate() == true` and no file \
         in this crate outside the registry names them, so the `true` attests to \
         work that has no code: {unproduced:?}"
    );
}

/// Every non-test Rust source of this crate, bodies read, with this registry and
/// its own cases removed.
///
/// The registry is excluded because it necessarily names every variant — the
/// enumeration, `ALL`, and three exhaustive `match`es — so leaving it in would
/// make the census answer `true` for everything.
fn rust_sources(dir: &std::path::Path) -> Vec<String> {
    let mut bodies = Vec::new();
    let entries = std::fs::read_dir(dir).expect("the crate's source tree is readable");
    for entry in entries {
        let path = entry.expect("a readable directory entry").path();
        if path.is_dir() {
            bodies.extend(rust_sources(&path));
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let is_rust = path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("rs"));
        if !is_rust || name.ends_with("_tests.rs") || name == "triggers.rs" {
            continue;
        }
        bodies.push(std::fs::read_to_string(&path).expect("a readable source file"));
    }
    bodies
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
///
/// **`bundleComposition` and `revenueShareChange` joined on 2026-08-06**, when
/// Slice 8 landed four tables, the composition's revision lifecycle,
/// `infra::bundle`'s two `ChangeSet::of_act` declarations and three mounted routes.
/// This list is a **transcription**, so it reddened on the flip and was updated in
/// the same edit — which is the obligation it exists to create.
///
/// **`grandfatheringCutover` joined later than its own store by three commits**: it
/// waited for `infra::cutover::cutover_in` to *declare* the act through
/// `ChangeSet::of_act`, because that is what the predicate is about. A table is not
/// a declaration.
///
/// **`priceOverlayMutation` joined on the merge of Slice 9's overlay half**, and it
/// waited for the same thing. The strand landed three tables, three entities, a
/// revision lifecycle and four mounted operations, and the trigger stayed `false`
/// through all of it because the submit route wrote its `materiality` token as a
/// **literal** — so nothing in the crate constructed the change set the act half
/// reads back. `api::rest::overlays::overlay_submit_materiality` is the declaration,
/// and this list moved with it.
///
/// **Two members of this list do not meet that bar**, and the honest place to say so
/// is here rather than in a register nobody greps: `bundleComposition` and
/// `revenueShareChange` are declared by `infra::bundle::composition_change_set` and
/// `rev_share_change_set`, which **have no caller** — `publish_bundle` evaluates no
/// verdict at all. Their subjects are unarguably here, which is what the predicate
/// literally asks; what is missing is the evaluation, and with it D-104's rule.
///
/// **`bulkGroupMove` left this list on 2026-08-14**, and it should never have
/// joined it: the flip credited `ApprovalService::submit_membership_move_on` with
/// a `ChangeSet::of_act` declaration that writer does not make, and the move route
/// builds a single-payer set and always declares
/// `immediateMembershipReresolution`. A transcription cannot see that — it copies
/// whatever the `match` says — which is why
/// `every_act_half_trigger_answering_true_is_named_by_a_producing_site` now
/// stands beside it, and why that census, not this list, is what reddened.
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
            "grandfatheringCutover",
            "immediateMembershipReresolution",
            "thresholdPolicyDiff",
            "priceOverlayMutation",
            "windowCancellation",
            "windowShortening",
            "bundleComposition",
            "revenueShareChange",
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
