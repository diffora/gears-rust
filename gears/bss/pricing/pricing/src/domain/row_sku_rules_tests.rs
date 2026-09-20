//! Tests for D-372's row-SKU rules (I3-I6).
//!
//! Every case here is read through [`price_row_rules`] rather than through a
//! rule built by hand, because the registration order *is* part of the contract:
//! `RowSkuPublished` runs first so that a row naming a SKU nobody can read is
//! reported as that, once, rather than as three consequences of it. A test that
//! instantiated one rule would pass with the four registered in any order, or in
//! none.
//!
//! The row fixtures are deliberately **publishable** under the row-local roster
//! that `price_row_rules` appends: the negative cases assert the *first* code,
//! and the two positive cases assert there is no code at all — which a row that
//! merely fails `inst-mk-explicit` would satisfy vacuously in the first form and
//! contradict in the second.

use std::sync::Arc;

use bss_fixtures::ModelKind;
use uuid::Uuid;

use super::RowSkuContext;
use crate::domain::money::{MinorAmount, RateMinor};
use crate::domain::ports::CatalogSku;
use crate::domain::price_row::{BillingGranularity, PriceRow, TierAggregationWindow, TierBand};
use crate::domain::registry_view::SkuIndex;
use crate::domain::rules::{
    FEE_ROW_SKU_METERED, METER_SKU_MISMATCH, ROW_SKU_DEPRECATED, ROW_SKU_TYPE_INVALID,
    SKU_NOT_PUBLISHED, USAGE_ROW_SKU_UNMETERED, price_row_rules,
};
use crate::domain::scope_key::{ChargeKind, SkuId};

fn sku(id: u128, unit: Option<&str>, sellable: bool) -> CatalogSku {
    CatalogSku {
        sku_id: Uuid::from_u128(id),
        sku_code: format!("SKU-{id}"),
        name: "fixture".to_owned(),
        metering_unit: unit.map(str::to_owned),
        status: "published".to_owned(),
        plan_tier: None,
        // A row SKU is a component; `ctx` overrides the two that are not.
        sku_type: "component".to_owned(),
        sellable,
        usage_type_ref: unit.map(|unit| format!("gts.cf.usage.{unit}.v1~")),
        deprecated: false,
    }
}

/// The registry read model these rules judge against, and the plan's own SKU.
fn ctx() -> RowSkuContext {
    let offer = |id: u128, unit: Option<&str>, sellable: bool| {
        let mut sku = sku(id, unit, sellable);
        sku.sku_type = "offer".to_owned();
        sku
    };
    let index = SkuIndex::from_listing(vec![
        offer(0x1, None, true),            // the plan's own SKU -- an offer by role
        sku(0x2, Some("GB-hour"), false),  // a resource
        offer(0x3, Some("GB-hour"), true), // another plan's offer -- refused as a row SKU
        sku(0x4, None, false),             // a fee SKU without a meter
    ]);
    RowSkuContext {
        plan_sku: SkuId::new(Uuid::from_u128(0x1)),
        index: Arc::new(index),
        introducing: true,
    }
}

/// A band rate in whole minor units, as `rules_tests` states them (D-311).
fn rate(minor_units: i64) -> RateMinor {
    RateMinor::from_minor_units(minor_units).expect("test rate is non-negative")
}

/// A usage row the row-local roster publishes, copied from `rules_tests`.
fn graduated_usage() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Usage, Some(ModelKind::Graduated));
    row.meter = Some("api_calls".to_owned());
    row.billing_granularity = Some(BillingGranularity::WholeUnit);
    row.tier_aggregation_window = Some(TierAggregationWindow::CalendarMonth);
    row.bands = vec![
        TierBand::closed(0, 1_000, rate(10)),
        TierBand::open(1_000, rate(6)),
    ];
    row
}

/// A recurring row the row-local roster publishes: the plan fee.
fn flat_recurring() -> PriceRow {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(2_500).expect("test amount is non-negative"));
    row
}

/// A row on `sku`, whose row-local shape is the publishable one for `kind`.
fn row(sku: u128, kind: ChargeKind, meter: Option<&str>) -> PriceRow {
    let mut row = if kind.is_usage() {
        graduated_usage()
    } else {
        flat_recurring()
    };
    row.sku_id = SkuId::new(Uuid::from_u128(sku));
    row.charge_kind = kind;
    row.meter = meter.map(str::to_owned);
    row
}

/// Every code the row reports against `ctx`, in report order.
fn codes_against(ctx: RowSkuContext, row: &PriceRow) -> Vec<String> {
    price_row_rules(ctx)
        .run(row)
        .violations
        .into_iter()
        .map(|violation| violation.code)
        .collect()
}

/// Every code the row reports, in report order -- the diagnostic the `None`
/// cases print when they fail.
fn codes(row: &PriceRow) -> Vec<String> {
    codes_against(ctx(), row)
}

fn first_code(row: &PriceRow) -> Option<String> {
    codes(row).into_iter().next()
}

#[test]
fn a_usage_row_on_an_unmetered_sku_is_refused() {
    assert_eq!(
        first_code(&row(0x4, ChargeKind::Usage, None)).as_deref(),
        Some(USAGE_ROW_SKU_UNMETERED)
    );
}

#[test]
fn a_fee_row_on_a_metered_sku_is_refused() {
    assert_eq!(
        first_code(&row(0x2, ChargeKind::Recurring, None)).as_deref(),
        Some(FEE_ROW_SKU_METERED)
    );
}

#[test]
fn a_stored_meter_that_disagrees_with_the_sku_is_refused() {
    assert_eq!(
        first_code(&row(0x2, ChargeKind::Usage, Some("vCPU-hour"))).as_deref(),
        Some(METER_SKU_MISMATCH)
    );
}

#[test]
fn a_foreign_offer_is_refused_but_the_plans_own_is_admitted() {
    assert_eq!(
        first_code(&row(0x3, ChargeKind::Usage, Some("GB-hour"))).as_deref(),
        Some(ROW_SKU_TYPE_INVALID)
    );

    let own = row(0x1, ChargeKind::Recurring, None);
    assert_eq!(
        first_code(&own),
        None,
        "the plan fee on the plan's own SKU: {:?}",
        codes(&own)
    );

    let resource = row(0x2, ChargeKind::Usage, Some("GB-hour"));
    assert_eq!(
        first_code(&resource),
        None,
        "a sellable=false resource: {:?}",
        codes(&resource)
    );
}

#[test]
fn an_unknown_or_unpublished_sku_is_refused_sku_not_published() {
    assert_eq!(
        first_code(&row(0x9, ChargeKind::Usage, None)).as_deref(),
        Some(SKU_NOT_PUBLISHED)
    );
    assert_eq!(SKU_NOT_PUBLISHED, "SKU_NOT_PUBLISHED");
}

/// A SKU the registry holds but has **not** published is the same refusal, and
/// it is the half that carries the code's name.
///
/// Both spellings matter: the unknown-SKU arm above would be satisfied by a rule
/// that only checked the lookup, and this gear's registry read model carries
/// `draft` and `deprecated` rows too -- `status` is verbatim registry vocabulary
/// (`CatalogSku::status`), not an enum this gear narrows.
#[test]
fn a_deprecated_sku_is_refused_by_the_same_rule() {
    let mut deprecated = sku(0x7, Some("GB-hour"), false);
    deprecated.status = "deprecated".to_owned();
    let ctx = RowSkuContext {
        plan_sku: SkuId::new(Uuid::from_u128(0x1)),
        index: Arc::new(SkuIndex::from_listing(vec![deprecated])),
        introducing: true,
    };
    // Otherwise a well-formed row on that SKU: the only fault is the status.
    let subject = row(0x7, ChargeKind::Usage, Some("GB-hour"));

    assert_eq!(codes_against(ctx, &subject), vec![SKU_NOT_PUBLISHED]);
}

/// I4 judges a **fee** row too: an unmetered SKU derives no meter, so a stored
/// one is a mismatch.
///
/// The cell the `is_usage()` guard this rule used to open with left unjudged, and
/// it is reachable: the D-372 migration backfills `sku_id` onto fee rows and
/// clears nothing, and it drops `uq_pricing_price_meter_line_current`, the one
/// constraint that noticed a non-usage row carrying a meter.
#[test]
fn a_fee_row_carrying_a_meter_its_sku_does_not_derive_is_refused() {
    assert_eq!(
        codes(&row(0x4, ChargeKind::Recurring, Some("GB-hour"))),
        vec![METER_SKU_MISMATCH]
    );
}

/// The inconsistent pair is **one** fault, and it is `inst-pr-sku-metered`'s.
///
/// Whole-report equality rather than a first-code check, because the fault this
/// pins is a *second* code: I4 reporting a derivation against a SKU that declares
/// nothing to derive from.
#[test]
fn a_usage_row_on_an_unmetered_sku_reports_the_binding_and_not_the_meter() {
    assert_eq!(
        codes(&row(0x4, ChargeKind::Usage, Some("GB-hour"))),
        vec![USAGE_ROW_SKU_UNMETERED]
    );
}

/// The positive control the two cases above need: the same fee row with no meter
/// is judged by nothing at all.
#[test]
fn a_fee_row_on_an_unmetered_sku_with_no_meter_is_admitted() {
    let subject = row(0x4, ChargeKind::Recurring, None);

    assert_eq!(codes(&subject), Vec::<String>::new());
}

/// Every one of these refusals reaches the **authoring write**, not only the
/// publish.
///
/// D-372's invariants are enforced at save *and* publish, and the price write door
/// (`api::rest::prices::require_no_key_contradiction`) keeps only what
/// `write_stage_only()` returns — so a publish-stage stamp here would have the
/// door read the registry, judge the row against it, and discard every verdict.
///
/// Asserted through that filter rather than by reading `Violation::stage`,
/// because the filter is what the door actually applies, and over all five codes
/// rather than one, because a family half-stamped is the state a single case
/// would pass.
#[test]
fn every_row_sku_refusal_is_judged_at_the_authoring_write() {
    let cases = [
        (row(0x9, ChargeKind::Usage, None), SKU_NOT_PUBLISHED),
        (
            row(0x3, ChargeKind::Usage, Some("GB-hour")),
            ROW_SKU_TYPE_INVALID,
        ),
        (row(0x4, ChargeKind::Usage, None), USAGE_ROW_SKU_UNMETERED),
        (row(0x2, ChargeKind::Recurring, None), FEE_ROW_SKU_METERED),
        (
            row(0x2, ChargeKind::Usage, Some("vCPU-hour")),
            METER_SKU_MISMATCH,
        ),
    ];

    for (subject, code) in cases {
        let write = price_row_rules(ctx())
            .run(&subject)
            .write_stage_only()
            .unwrap_or_else(|| panic!("{code} must reach the authoring write"));
        assert!(
            write.violations.iter().any(|v| v.code == code),
            "{code} is absent from the write-stage report: {:?}",
            write.violations
        );
    }

    let (dep_ctx, dep_row) = served_deprecated_row(true);
    let write = price_row_rules(dep_ctx)
        .run(&dep_row)
        .write_stage_only()
        .unwrap_or_else(|| panic!("{ROW_SKU_DEPRECATED} must reach the authoring write"));
    assert!(
        write
            .violations
            .iter()
            .any(|v| v.code == ROW_SKU_DEPRECATED),
        "{ROW_SKU_DEPRECATED} is absent from the write-stage report: {:?}",
        write.violations
    );
}

/// The other three rules decline to judge a row whose SKU they cannot read.
///
/// The row below carries a meter that agrees with no declaration at all, so a
/// `MeterMatchesSku` that judged an absent SKU would report `METER_SKU_MISMATCH`
/// beside the refusal -- an author told to fix a derivation against a SKU that
/// does not exist. Set equality rather than a first-code check, because that is
/// the half a first-code check cannot see.
#[test]
fn the_rules_after_the_lookup_decline_when_the_sku_is_absent() {
    let subject = row(0x9, ChargeKind::Usage, Some("vCPU-hour"));

    assert_eq!(codes(&subject), vec![SKU_NOT_PUBLISHED]);
}

/// Task 7 serves a deprecated SKU as `status: "published"` + `deprecated: true`,
/// so [`SKU_NOT_PUBLISHED`] does not fire. The whole report is this one code.
fn served_deprecated_row(introducing: bool) -> (RowSkuContext, PriceRow) {
    let mut deprecated = sku(0x7, Some("GB-hour"), false);
    deprecated.deprecated = true;
    let ctx = RowSkuContext {
        plan_sku: SkuId::new(Uuid::from_u128(0x1)),
        index: Arc::new(SkuIndex::from_listing(vec![
            sku(0x1, None, true),
            deprecated,
        ])),
        introducing,
    };
    (ctx, row(0x7, ChargeKind::Usage, Some("GB-hour")))
}

/// Create a row naming a deprecated registry SKU.
#[test]
fn creating_a_row_that_names_a_deprecated_sku_is_refused() {
    let (ctx, subject) = served_deprecated_row(true);

    assert_eq!(codes_against(ctx, &subject), vec![ROW_SKU_DEPRECATED]);
    assert_eq!(ROW_SKU_DEPRECATED, "ROW_SKU_DEPRECATED");
}

/// Patch a draft's `sku_id` onto a deprecated SKU -- an introduction of the
/// reference.
#[test]
fn patching_a_drafts_sku_id_onto_a_deprecated_sku_is_refused() {
    let (ctx, subject) = served_deprecated_row(true);

    assert_eq!(codes_against(ctx, &subject), vec![ROW_SKU_DEPRECATED]);
}

/// Publish a draft that names a deprecated SKU -- still an introduction: the
/// live reference has not existed yet.
#[test]
fn publishing_a_draft_that_names_a_deprecated_sku_is_refused() {
    let (ctx, subject) = served_deprecated_row(true);

    assert_eq!(codes_against(ctx, &subject), vec![ROW_SKU_DEPRECATED]);
}

/// Re-publish a plan whose already-published row names a since-deprecated SKU.
/// D-370: deprecation withdraws the value from new use and changes nothing
/// about what already resolves through it.
#[test]
fn republishing_an_already_published_row_that_names_a_since_deprecated_sku_is_admitted() {
    let (ctx, subject) = served_deprecated_row(false);

    assert_eq!(
        codes_against(ctx, &subject),
        Vec::<String>::new(),
        "an already-published row is not an introduction"
    );
}

/// A foreign **offer** is not a component, and closing its sales does not make
/// it one.
///
/// This is the whole of what the role split changes down here. The rule used to
/// read `sellable`, so "may this be sold" and "may this sit in someone else's
/// plan" were one flag: an operator who closed sales on an offer silently made
/// it eligible as a constituent of every other plan in the catalogue, and an
/// operator who wanted a constituent had to declare it unsellable to get one.
/// The role says it outright, and the flag goes back to meaning only what its
/// name says.
#[test]
fn a_foreign_offer_is_not_a_component_when_sales_are_closed() {
    let mut foreign = sku(0x4, None, false);
    foreign.sku_type = "offer".to_owned();
    let context = RowSkuContext {
        plan_sku: SkuId::new(Uuid::from_u128(0x1)),
        index: Arc::new(SkuIndex::from_listing(vec![foreign])),
        introducing: true,
    };
    assert_eq!(
        codes_against(context, &row(0x4, ChargeKind::Recurring, None)),
        vec!["ROW_SKU_TYPE_INVALID".to_owned()]
    );
}

/// The converse, and the case the old rule refused: a **component** open for
/// sale on its own is still a legitimate row reference.
///
/// Under `sellable` this was impossible — a `sellable = true` SKU that was not
/// the plan's own was refused outright — so a constituent that a tenant also
/// sells separately could not be priced into a plan at all. Both flags are
/// admitted now, because the flag was never the right operand for this question.
#[test]
fn a_component_is_a_legitimate_row_reference_under_either_sale_flag() {
    for sellable in [true, false] {
        let mut component = sku(0x4, None, sellable);
        component.sku_type = "component".to_owned();
        let context = RowSkuContext {
            plan_sku: SkuId::new(Uuid::from_u128(0x1)),
            index: Arc::new(SkuIndex::from_listing(vec![component])),
            introducing: true,
        };
        assert!(
            codes_against(context, &row(0x4, ChargeKind::Recurring, None)).is_empty(),
            "a component with sellable={sellable} may be priced by a row"
        );
    }
}

/// A foreign **bundle** is refused for the same reason an offer is: a plan
/// prices constituents, and a bundle is somebody else's composition.
#[test]
fn a_foreign_bundle_is_not_a_component_either() {
    let mut foreign = sku(0x4, None, false);
    foreign.sku_type = "bundle".to_owned();
    let context = RowSkuContext {
        plan_sku: SkuId::new(Uuid::from_u128(0x1)),
        index: Arc::new(SkuIndex::from_listing(vec![foreign])),
        introducing: true,
    };
    assert_eq!(
        codes_against(context, &row(0x4, ChargeKind::Recurring, None)),
        vec!["ROW_SKU_TYPE_INVALID".to_owned()]
    );
}

/// The plan's own SKU keeps its exemption, whatever role it carries and whatever
/// its flag says. Its own role is `plan_sku_rules`' question, one level up.
#[test]
fn the_plans_own_sku_is_exempt_from_the_role_rule() {
    for (role, sellable) in [("offer", true), ("offer", false), ("bundle", true)] {
        let mut own = sku(0x1, None, sellable);
        own.sku_type = role.to_owned();
        let context = RowSkuContext {
            plan_sku: SkuId::new(Uuid::from_u128(0x1)),
            index: Arc::new(SkuIndex::from_listing(vec![own])),
            introducing: true,
        };
        assert!(
            codes_against(context, &row(0x1, ChargeKind::Recurring, None)).is_empty(),
            "the plan's own {role}/{sellable} SKU is priceable by its own rows"
        );
    }
}
