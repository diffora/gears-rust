//! Phase 1's batch-only rules, executed.
//!
//! The rules here need no store, so these cases build no world — which is the
//! reason the batch-only half was separated in the first place. What they must
//! not do is assert a subset that survives the bug: every case names both the
//! rows it expects to fail **and** the rows it expects to pass, because a report
//! that failed everything and a report that failed the right thing are otherwise
//! the same green.

use super::{BatchReport, DUPLICATE_SCOPE_KEY, ImportRow, classify};
use crate::domain::money::{CurrencyCode, MinorAmount};
use crate::domain::price_record::{PriceContent, authored_content};
use crate::domain::price_row::{
    IncludedAllowance, ModelKind, PriceRow, RolloverPolicy, TierQualificationWindow,
};
use crate::domain::publish::rules::PRIMITIVE_RULES_UNBUILT;
use crate::domain::rules::{
    AMOUNT_PLACEMENT_INVALID, MODEL_KIND_CHARGEKIND_MISMATCH, row_local_rules,
};
use crate::domain::scope_key::{
    ChargeKind, ChargeLineScopeKey, Cohort, DimensionKey, MarketPriceScopeKey, Meter, PhaseId,
    PlanId, PriceEligibility, Region, SkuId,
};
use uuid::Uuid;

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x9_1a))
}

fn phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0xfa_5e))
}

fn key(region: &str, eligibility: PriceEligibility, charge: ChargeKind) -> MarketPriceScopeKey {
    MarketPriceScopeKey::new(
        ChargeLineScopeKey::new(
            plan(),
            phase(),
            eligibility,
            charge,
            Cohort::None,
            SkuId::new(Uuid::from_u128(5)),
        )
        .expect("the class pairs with the cohort"),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new(region).expect("a non-blank region"),
    )
}

fn base() -> MarketPriceScopeKey {
    key(
        "eu",
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
    )
}

fn content() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    PriceContent {
        row,
        tax_inclusive: false,
        tax_category_ref: None,
        billing_timing: Some("advance".to_owned()),
        proration_contract: None,
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

fn row(scope_key: MarketPriceScopeKey) -> ImportRow {
    ImportRow {
        scope_key,
        content: content(),
        if_match: None,
    }
}

/// Content legal on a **usage** key: `per_unit` on a usage row is the plain
/// untiered metered rate, unit price times metered `Q`.
///
/// The duplicate-key cases below need this rather than [`content`], and the reason
/// is D-312 rather than tidiness. [`content`] is `flat`, `flat` is in no part of the
/// usage set, and Phase 1 now judges that contradiction — so a usage fixture built
/// from [`content`] would assert the duplicate rule through a row the import refuses
/// for an unrelated reason, and the case would go green on the wrong report.
///
/// Those fixtures had carried the contradiction since they were written, which is
/// the same defect this arm exists to catch and a fair sample of how invisible it is
/// while nothing judges it.
fn usage_content() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::PerUnit));
    row.amount_minor = Some(MinorAmount::new(25).expect("a non-negative amount"));
    PriceContent { row, ..content() }
}

fn usage_row(scope_key: MarketPriceScopeKey) -> ImportRow {
    ImportRow {
        scope_key,
        content: usage_content(),
        if_match: None,
    }
}

/// A metered line on a usage key — the shape a Slice-10 primitive is authored
/// on.
///
/// The three unbuilt-primitive cases below used [`base`] + [`content`], a `flat`
/// **recurring** row, and hung an `includedAllowance` on it. That was invisible
/// while nothing judged the field and it became a fault the moment `inst-ac-gate`
/// landed: `ALLOWANCE_ON_NON_USAGE` is a write-stage refusal, so those rows would
/// have failed Phase 1 for a *second* reason and each case would have gone green
/// on the wrong report. The fixture is fixed rather than the assertion — the rows
/// were always wrong, and D-312's own arm found the same class of fault in the
/// duplicate-key fixtures one wave earlier.
fn metered_key() -> MarketPriceScopeKey {
    key("eu", PriceEligibility::AllSubscriptions, ChargeKind::Usage)
        .with_usage_line(
            Some(&Meter::new("api-calls").expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a usage key")
}

fn codes(report: &BatchReport, at: usize) -> Vec<String> {
    report
        .rows()
        .iter()
        .find(|outcome| outcome.row == at)
        .map(|outcome| {
            outcome
                .violations
                .iter()
                .map(|violation| violation.code.clone())
                .collect()
        })
        .unwrap_or_default()
}

fn failed_rows(report: &BatchReport) -> Vec<usize> {
    report.rows().iter().map(|outcome| outcome.row).collect()
}

#[test]
fn a_batch_of_distinct_keys_passes_and_does_not_block() {
    let report = classify(&[
        row(base()),
        row(key(
            "us",
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
        )),
        row(key(
            "eu",
            PriceEligibility::AllSubscriptions,
            ChargeKind::OneTime,
        )),
        row(key(
            "eu",
            PriceEligibility::NewSubscriptionsOnly,
            ChargeKind::Recurring,
        )),
    ]);
    assert_eq!(failed_rows(&report), Vec::<usize>::new());
    assert!(!report.blocks_the_batch());
}

#[test]
fn two_rows_on_one_key_both_fail_and_each_names_the_other() {
    let report = classify(&[row(base()), row(base())]);

    assert_eq!(
        failed_rows(&report),
        vec![0, 1],
        "a collision has two rows in it and neither is more at fault"
    );
    for outcome in report.rows() {
        assert_eq!(outcome.violations.len(), 1);
        assert_eq!(outcome.violations[0].code, DUPLICATE_SCOPE_KEY);
    }
    assert!(
        report.rows()[0].violations[0].detail.contains("row(s) 1"),
        "row 0 must name row 1: {}",
        report.rows()[0].violations[0].detail
    );
    assert!(
        report.rows()[1].violations[0].detail.contains("row(s) 0"),
        "row 1 must name row 0: {}",
        report.rows()[1].violations[0].detail
    );
    assert!(report.blocks_the_batch());
}

#[test]
fn three_rows_on_one_key_each_name_the_other_two() {
    let report = classify(&[row(base()), row(base()), row(base())]);
    assert_eq!(failed_rows(&report), vec![0, 1, 2]);
    assert!(
        report.rows()[1].violations[0]
            .detail
            .contains("row(s) 0, 2")
    );
}

#[test]
fn two_rows_differing_only_in_their_sku_are_two_keys_and_both_author() {
    // **D-103's confirmed worked example**, and the case a first build got
    // backwards (D-283). A PaaS plan pricing cloudlets, storage and egress is one
    // plan, and these two rows are two keys. `tests/sqlite_price_repo.rs` proves
    // the store admits them both; this proves Phase 1 does not refuse them first.
    //
    // **The discriminator is the SKU since D-372**, not the unit: two units of one
    // SKU render one key, so a case written on the unit alone would assert that
    // `classify` accepts a genuine duplicate.
    let line = |sku: u128, meter: &str| {
        MarketPriceScopeKey::new(
            ChargeLineScopeKey::new(
                plan(),
                phase(),
                PriceEligibility::AllSubscriptions,
                ChargeKind::Usage,
                Cohort::None,
                SkuId::new(Uuid::from_u128(sku)),
            )
            .expect("the class pairs with the cohort"),
            CurrencyCode::new("EUR").expect("three letters"),
            Region::new("eu").expect("a non-blank region"),
        )
        .with_usage_line(
            Some(&Meter::new(meter).expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a usage key")
    };
    let metered = line(0x11, "api-calls");
    let other_sku = line(0x12, "storage-gb");

    let report = classify(&[usage_row(metered.clone()), usage_row(other_sku)]);
    assert_eq!(
        failed_rows(&report),
        Vec::<usize>::new(),
        "two SKUs are two keys, and Phase 1 must not refuse what the store admits"
    );

    // And the same line twice IS a duplicate — otherwise this case would pass
    // for a `classify` that never reports anything at all.
    let doubled = classify(&[usage_row(metered.clone()), usage_row(metered)]);
    assert_eq!(failed_rows(&doubled), vec![0, 1]);
}

#[test]
fn two_rows_differing_only_in_their_dimension_key_are_also_two_keys() {
    // The other axis the widened index carries. Untested, it is an axis the
    // duplicate rule could stop comparing with nothing noticing.
    let usage = key("eu", PriceEligibility::AllSubscriptions, ChargeKind::Usage);
    let eu = usage
        .clone()
        .with_usage_line(
            Some(&Meter::new("api-calls").expect("a meter")),
            DimensionKey::new("region=eu"),
        )
        .expect("a usage line on a usage key");
    let us = usage
        .with_usage_line(
            Some(&Meter::new("api-calls").expect("a meter")),
            DimensionKey::new("region=us"),
        )
        .expect("a usage line on a usage key");

    let report = classify(&[usage_row(eu.clone()), usage_row(us)]);
    assert_eq!(failed_rows(&report), Vec::<usize>::new());

    // The same companion its sibling carries, and for the same reason: without
    // it this passes for a `classify` that never reports anything at all.
    let doubled = classify(&[usage_row(eu.clone()), usage_row(eu)]);
    assert_eq!(failed_rows(&doubled), vec![0, 1]);
}

#[test]
fn a_duplicate_pair_does_not_drag_the_rest_of_the_batch_down() {
    // The other half of "one invalid row blocks the batch": the *batch* is
    // blocked, but the report must still say which rows are actually wrong.
    // A report that failed all four would block the same batch and tell the
    // operator nothing.
    let report = classify(&[
        row(base()),
        row(key(
            "us",
            PriceEligibility::AllSubscriptions,
            ChargeKind::Recurring,
        )),
        row(base()),
        row(key(
            "eu",
            PriceEligibility::AllSubscriptions,
            ChargeKind::OneTime,
        )),
    ]);
    assert_eq!(failed_rows(&report), vec![0, 2]);
    assert!(report.blocks_the_batch());
}

#[test]
fn a_row_carrying_an_unbuilt_primitive_fails_and_inherits_the_publish_code() {
    // **The refusal moves earlier, it does not move** (D-177/D-179): publish
    // still refuses these fields on its own authority. What this arm changes is
    // that the operator hears it while the batch can still be fixed.
    let mut row = usage_row(metered_key());
    row.content.row.included_allowance = Some(IncludedAllowance {
        quantity: 100,
        rollover_policy: RolloverPolicy::Carry,
    });

    let report = classify(&[row]);
    assert_eq!(failed_rows(&report), vec![0]);
    assert_eq!(report.rows()[0].violations[0].code, PRIMITIVE_RULES_UNBUILT);
    assert!(
        report.rows()[0].violations[0]
            .detail
            .contains("includedAllowance"),
        "the sentence names the field the operator has to remove: {}",
        report.rows()[0].violations[0].detail
    );
    assert!(report.blocks_the_batch());
}

/// The positive control the bulk door needed (D-45).
///
/// The three cases around it hand Phase 1 a row it refuses, so all three would
/// pass identically against the pre-D-45 arm that refused `includedAllowance`
/// outright. This is the one that says a compiled allowance **imports**.
#[test]
fn a_row_carrying_a_compiled_allowance_imports() {
    let mut row = usage_row(metered_key());
    row.content.row.included_allowance = Some(IncludedAllowance {
        quantity: 100,
        rollover_policy: RolloverPolicy::None,
    });

    let report = classify(&[row]);
    assert_eq!(
        failed_rows(&report),
        Vec::<usize>::new(),
        "rolloverPolicy = none is judged by inst-ac-gate and honoured by the compile: {:?}",
        codes(&report, 0)
    );
    assert!(!report.blocks_the_batch());
}

#[test]
fn a_row_carrying_both_unbuilt_primitives_is_told_about_both() {
    // The all-or-nothing posture only pays for itself if the report is complete
    // — a row fixed one field at a time is a second batch for nothing.
    let mut row = usage_row(metered_key());
    row.content.row.included_allowance = Some(IncludedAllowance {
        quantity: 100,
        rollover_policy: RolloverPolicy::Carry,
    });
    row.content.row.tier_qualification_window = Some(TierQualificationWindow::TrailingPeriod);

    let report = classify(&[row]);
    assert_eq!(report.rows()[0].violations.len(), 2, "both, not the first");
    let details: Vec<&str> = report.rows()[0]
        .violations
        .iter()
        .map(|violation| violation.detail.as_str())
        .collect();
    assert!(details.iter().any(|d| d.contains("includedAllowance")));
    assert!(
        details
            .iter()
            .any(|d| d.contains("tierQualificationWindow"))
    );
}

#[test]
fn a_row_can_carry_two_different_faults_and_hears_about_both() {
    // The two rules are independent and both answer for every row. A report that
    // stopped at the first would send the operator round twice.
    let mut first = usage_row(metered_key());
    first.content.row.included_allowance = Some(IncludedAllowance {
        quantity: 100,
        rollover_policy: RolloverPolicy::Carry,
    });
    let report = classify(&[first, usage_row(metered_key())]);

    assert_eq!(failed_rows(&report), vec![0, 1]);
    let codes: Vec<&str> = report.rows()[0]
        .violations
        .iter()
        .map(|violation| violation.code.as_str())
        .collect();
    assert_eq!(
        codes,
        vec![DUPLICATE_SCOPE_KEY, PRIMITIVE_RULES_UNBUILT],
        "row 0 is both a duplicate and unjudged, and the order is stable"
    );
    assert_eq!(
        report.rows()[1]
            .violations
            .iter()
            .map(|violation| violation.code.as_str())
            .collect::<Vec<_>>(),
        vec![DUPLICATE_SCOPE_KEY],
        "row 1 carries no primitive and must not inherit its neighbour's fault"
    );
}

#[test]
fn an_empty_batch_blocks_nothing() {
    let report = classify(&[]);
    assert!(!report.blocks_the_batch());
    assert_eq!(failed_rows(&report), Vec::<usize>::new());
}

/// A usage key carrying a usage line, by region so two of them are two keys.
fn metered(region: &str) -> MarketPriceScopeKey {
    key(
        region,
        PriceEligibility::AllSubscriptions,
        ChargeKind::Usage,
    )
    .with_usage_line(
        Some(&Meter::new("api-calls").expect("a meter")),
        DimensionKey::new("region=eu"),
    )
    .expect("a usage line on a usage key")
}

fn one(scope_key: MarketPriceScopeKey, content: PriceContent) -> Vec<ImportRow> {
    vec![ImportRow {
        scope_key,
        content,
        if_match: None,
    }]
}

#[test]
fn a_row_contradicting_its_own_frozen_key_is_refused_before_the_batch_commits() {
    // D-312's third door. `flat` is in no part of the usage set and `chargeKind` is
    // an immutable axis of the key, so the row is unpublishable the instant it
    // arrives and no later call adds anything that legalises it. Before this arm
    // existed the batch imported clean and was refused at publish — after the rows
    // were committed, which is the one thing all-or-nothing exists to prevent.
    //
    // The well-formed sibling stands on **another line** (its own eligibility
    // class): two markets of one line carrying a usage structure and a flat one
    // are also `IMPORT_LINE_DEFINITION_CONFLICT`, which would name row 0 too and
    // make this case about two rules instead of the one it is for.
    let sibling = key(
        "eu",
        PriceEligibility::NewSubscriptionsOnly,
        ChargeKind::Usage,
    )
    .with_usage_line(
        Some(&Meter::new("api-calls").expect("a meter")),
        DimensionKey::new("region=eu"),
    )
    .expect("a usage line on a usage key");
    let report = classify(&[usage_row(sibling), row(metered("us"))]);

    assert_eq!(failed_rows(&report), vec![1]);
    assert_eq!(
        codes(&report, 1),
        vec![MODEL_KIND_CHARGEKIND_MISMATCH.to_owned()],
        "the code is inherited from the rule, not minted for the import"
    );
    assert!(report.blocks_the_batch());
    // The positive control. Row 0 is a legal usage row on the same shape of key, and
    // a report that refused the batch entire would pass the assertions above.
    assert_eq!(codes(&report, 0), Vec::<String>::new());
}

#[test]
fn an_incomplete_draft_still_imports_and_that_is_the_whole_distinction() {
    // The negative control that decides whether this arm is D-312 or its opposite.
    // §4.2 puts the rule set at publish because a row is assembled over several
    // calls, and an import lands **drafts**. A row with no kind, no amount and no
    // bands is incomplete, not contradictory: its fault is an absent operand, which
    // a later call supplies. Without this case the change reads as "Phase 1
    // validates rows", which is the posture §4.2 exists to refuse.
    let mut bare = content();
    bare.row = PriceRow::new(ChargeKind::Recurring, None);

    let report = classify(&one(base(), bare));

    assert_eq!(failed_rows(&report), Vec::<usize>::new());
    assert!(!report.blocks_the_batch());
}

#[test]
fn a_usage_row_is_judged_on_its_key_and_not_on_the_content_placeholder() {
    // `api::rest::prices::content_of` fills `charge_kind` with `Recurring` as a
    // placeholder — the wire view has no such field — and an import row carries
    // content built the same way. `graduated` is legal on a usage row and illegal on
    // a recurring one, so this row lands only if the projection through
    // `authored_content` happened. Judged un-normalized it reads as recurring and is
    // refused `MODEL_KIND_CHARGEKIND_MISMATCH`, which would make every graduated,
    // volume and package batch in the catalog unimportable.
    //
    // Skipping that projection has produced two Criticals elsewhere in this crate.
    // This is the case that notices it here.
    let mut placeholder = content();
    placeholder.row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Graduated));

    let report = classify(&one(metered("eu"), placeholder));

    assert_eq!(
        failed_rows(&report),
        Vec::<usize>::new(),
        "a graduated usage row is legal; only an un-normalized judgement refuses it"
    );
}

#[test]
fn a_content_against_content_fault_is_left_where_publish_can_still_see_it() {
    // Money on a `graduated` row belongs in its bands, and an `amount_minor` beside
    // them is `AMOUNT_PLACEMENT_INVALID` — a real blocking violation, and a
    // publish-stage one, because both operands are content and either can still
    // move. The import must not take it.
    let mut misplaced_money = content();
    let mut graduated = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Graduated));
    graduated.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    misplaced_money.row = graduated;

    // The fault is real, and stating that here is what stops this case passing for a
    // row that simply has nothing wrong with it — the failure mode this file's own
    // preamble names.
    let judged = authored_content(&metered("eu"), misplaced_money.clone()).row;
    let full = row_local_rules().run(&judged);
    assert!(
        full.violations
            .iter()
            .any(|violation| violation.code == AMOUNT_PLACEMENT_INVALID),
        "the fixture must actually violate the rule this case is about"
    );
    assert!(
        full.write_stage_only().is_none(),
        "and none of what it violates may be write-stage"
    );

    let report = classify(&one(metered("eu"), misplaced_money));

    assert_eq!(failed_rows(&report), Vec::<usize>::new());
    assert!(!report.blocks_the_batch());
}

// ---------------------------------------------------------------------------
// One line, several markets: the shared half is said once per row and has to
// say the same thing every time.
// ---------------------------------------------------------------------------

/// Two markets of one line that agree about the structure author, whatever
/// their money says — the world in which the refusal below is observable.
#[test]
fn two_markets_of_one_line_with_one_structure_and_different_money_pass() {
    let mut us = row(key(
        "us",
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
    ));
    us.content.row.amount_minor = Some(MinorAmount::new(12_500).expect("non-negative"));
    us.content.tax_inclusive = true;

    let report = classify(&[row(base()), us]);

    assert!(!report.blocks_the_batch(), "{report:?}");
}

/// The same two markets disagreeing about the **shared** half are refused, and
/// both sides are named: the batch does not say which definition was meant.
#[test]
fn two_markets_of_one_line_that_disagree_about_its_structure_are_both_refused() {
    let mut us = row(key(
        "us",
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
    ));
    us.content.row.gl_code_ref = Some("4999".to_owned());

    let report = classify(&[row(base()), us]);

    assert!(report.blocks_the_batch());
    assert_eq!(failed_rows(&report), vec![0, 1]);
    assert_eq!(
        codes(&report, 0),
        vec![super::IMPORT_LINE_DEFINITION_CONFLICT.to_owned()]
    );
    assert_eq!(
        codes(&report, 1),
        vec![super::IMPORT_LINE_DEFINITION_CONFLICT.to_owned()]
    );
}

/// **A ladder is not part of what a line's markets share.** Two markets of one
/// line stating different ladders — another number of bands, other break-points,
/// other rates — are two prices, not two definitions of the line. Whatever else
/// the batch says about these rows, it does not say this.
#[test]
fn two_markets_of_one_line_with_different_ladders_are_not_a_definition_conflict() {
    use crate::domain::money::RateMinor;
    use crate::domain::price_row::TierBand;
    let rate = |nano: i64| RateMinor::from_nano_minor(nano).expect("non-negative");

    let mut eu = row(base());
    eu.content.row.bands = vec![
        TierBand::closed(0, 100, rate(500)),
        TierBand::open(100, rate(400)),
    ];
    let mut us = row(key(
        "us",
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
    ));
    us.content.row.bands = vec![
        TierBand::closed(0, 50, rate(600)),
        TierBand::closed(50, 500, rate(450)),
        TierBand::open(500, rate(300)),
    ];

    let report = classify(&[eu, us]);

    for at in [0, 1] {
        assert!(
            !codes(&report, at).contains(&super::IMPORT_LINE_DEFINITION_CONFLICT.to_owned()),
            "row {at}: {report:?}"
        );
    }
}

/// Billing timing is filed on the line version too, so two markets stating two
/// timings are the same conflict — and a third row on **another** line is left
/// alone.
#[test]
fn a_timing_disagreement_is_the_same_conflict_and_does_not_taint_another_line() {
    let mut us = row(key(
        "us",
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
    ));
    us.content.billing_timing = Some("arrears".to_owned());
    let other_line = row(key(
        "eu",
        PriceEligibility::NewSubscriptionsOnly,
        ChargeKind::Recurring,
    ));

    let report = classify(&[row(base()), us, other_line]);

    assert_eq!(failed_rows(&report), vec![0, 1]);
}
