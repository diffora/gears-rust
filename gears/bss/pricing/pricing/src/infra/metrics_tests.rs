//! The `OTel` adapter, asserted against a real in-memory exporter.
//!
//! Every case here reads the **exported** metric stream rather than a spy. The
//! difference is the whole reason the harness exists: a spy proves the adapter
//! called its own recording method, and this proves an instrument *by that
//! name, carrying those attributes* reached an exporter — which is the claim a
//! dashboard and an alerting rule actually depend on. A metric whose name
//! drifted, or whose label was spelled differently, passes a spy and fails here.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use super::test_harness::MetricsHarness;
use super::*;

use crate::domain::ports::metrics::{AlarmSeverity, NoopPricingMetrics};

// ---------------------------------------------------------------------------
// The names, transcribed from §10 rather than from the identifiers.
// ---------------------------------------------------------------------------

/// A constant whose value drifted from its name would still compile and would
/// still be wrong on the wire.
#[test]
fn the_metric_names_are_spelled_as_section_10_spells_them() {
    assert_eq!(
        PRICING_PREVIEW_FAILCLOSED,
        "pricing_preview_failclosed_total"
    );
    assert_eq!(
        PRICING_CURRENCY_BINDING_BLOCKS,
        "pricing_currency_binding_blocks_total"
    );
    assert_eq!(PRICING_TAX_NOT_SELLABLE_GA, "pricing_tax_not_sellable_ga");
    assert_eq!(PRICING_ALARM, "pricing_alarm_total");
}

/// Counters end `_total`, the gauge does not, and nothing carries a unit
/// suffix it has not earned.
///
/// The collector runs `add_metric_suffixes: false`, so the exporter renders
/// these verbatim — a counter that forgot `_total` would ship a name no
/// convention-following dashboard queries, and would do so silently.
#[test]
fn counter_names_end_in_total_and_the_gauge_is_bare() {
    for counter in [
        PRICING_PREVIEW_FAILCLOSED,
        PRICING_CURRENCY_BINDING_BLOCKS,
        PRICING_ALARM,
    ] {
        assert!(
            counter.ends_with("_total"),
            "{counter} is a counter and must say so in its name"
        );
    }
    assert!(
        !PRICING_TAX_NOT_SELLABLE_GA.ends_with("_total"),
        "a gauge that rises and falls must not be named like a monotonic counter"
    );
}

// ---------------------------------------------------------------------------
// The instruments actually reach an exporter.
// ---------------------------------------------------------------------------

#[test]
fn a_preview_refusal_increments_its_reason_and_nothing_else() {
    let h = MetricsHarness::new();
    let m = h.metrics();

    m.preview_failclosed(PreviewFailClosed::MarketAbsent);
    h.force_flush();

    assert_eq!(
        h.counter_value(
            "pricing_preview_failclosed_total",
            &[("reason", "market_absent")]
        ),
        1
    );
    // The other reasons are untouched — a single bucket would have told an
    // operator the preview is refusing without telling them whose fault it is.
    assert_eq!(
        h.counter_value(
            "pricing_preview_failclosed_total",
            &[("reason", "no_published_version")]
        ),
        0
    );
    assert_eq!(
        h.counter_value(
            "pricing_preview_failclosed_total",
            &[("reason", "market_not_named")]
        ),
        0
    );
}

#[test]
fn every_preview_reason_is_a_series_of_its_own() {
    let h = MetricsHarness::new();
    let m = h.metrics();

    for reason in PreviewFailClosed::ALL {
        m.preview_failclosed(*reason);
    }
    h.force_flush();

    for reason in PreviewFailClosed::ALL {
        assert_eq!(
            h.counter_value(
                "pricing_preview_failclosed_total",
                &[("reason", reason.as_str())]
            ),
            1,
            "{} must be its own series",
            reason.as_str()
        );
    }
}

#[test]
fn each_currency_binding_case_counts_separately() {
    let h = MetricsHarness::new();
    let m = h.metrics();

    m.currency_binding_block(CurrencyBindingCase::RequiredAddon);
    m.currency_binding_block(CurrencyBindingCase::RequiredAddon);
    m.currency_binding_block(CurrencyBindingCase::BundleOwnPrice);
    h.force_flush();

    assert_eq!(
        h.counter_value(
            "pricing_currency_binding_blocks_total",
            &[("case", "required_addon")]
        ),
        2
    );
    assert_eq!(
        h.counter_value(
            "pricing_currency_binding_blocks_total",
            &[("case", "bundle_own_price")]
        ),
        1
    );
    assert_eq!(
        h.counter_value(
            "pricing_currency_binding_blocks_total",
            &[("case", "bundle_sum_of_parts")]
        ),
        0
    );
}

/// The GA gauge reports the **latest** backlog, not a running total.
///
/// The question C3 poses is how many markets are gated *now*; the number falls
/// when the engine GAs and the affected plans re-publish. A counter would only
/// rise, and would answer "how many were ever gated" — which is not the backlog
/// anyone manages.
#[test]
fn the_ga_gauge_reports_the_latest_backlog_rather_than_a_running_total() {
    let h = MetricsHarness::new();
    let m = h.metrics();

    m.tax_not_sellable_ga(7);
    m.tax_not_sellable_ga(3);
    h.force_flush();

    assert_eq!(h.gauge_value("pricing_tax_not_sellable_ga", &[]), 3);
}

/// The gauge can fall to zero, which is the state an operator is waiting for.
#[test]
fn the_ga_gauge_can_fall_to_zero() {
    let h = MetricsHarness::new();
    let m = h.metrics();

    m.tax_not_sellable_ga(4);
    m.tax_not_sellable_ga(0);
    h.force_flush();

    assert_eq!(h.gauge_value("pricing_tax_not_sellable_ga", &[]), 0);
}

// ---------------------------------------------------------------------------
// The alarm rollup.
// ---------------------------------------------------------------------------

/// One counter for every alarm, discriminated by label.
#[test]
fn an_alarm_counts_on_the_rollup_under_its_declared_name() {
    let h = MetricsHarness::new();
    let m = h.metrics();

    m.alarm(PricingAlarm::TaxNotSellableGaActive);
    h.force_flush();

    assert_eq!(
        h.counter_value(
            "pricing_alarm_total",
            &[
                ("alarm", "pricing.tax.not_sellable_ga_active"),
                ("severity", "info")
            ]
        ),
        1
    );
}

/// **The severity comes from the alarm, not from the caller.**
///
/// Two firings of one alarm must not be able to disagree about how urgent it
/// is — an alerting rule keyed on `severity` would then fire for some of an
/// alarm's occurrences and not others.
#[test]
fn an_alarms_severity_is_a_property_of_the_alarm() {
    let h = MetricsHarness::new();
    let m = h.metrics();

    m.alarm(PricingAlarm::TaxReadinessDivergent);
    m.alarm(PricingAlarm::TaxReadinessDivergent);
    h.force_flush();

    assert_eq!(
        h.counter_value(
            "pricing_alarm_total",
            &[
                ("alarm", "pricing.tax.readiness_divergent"),
                ("severity", "warn")
            ]
        ),
        2,
        "both firings land on the one severity the design set declares"
    );
    assert_eq!(
        h.counter_value(
            "pricing_alarm_total",
            &[
                ("alarm", "pricing.tax.readiness_divergent"),
                ("severity", "info")
            ]
        ),
        0
    );
}

/// Every declared alarm's name is the **dotted** form the design set spells.
///
/// An operator greps the design document for `pricing.tax.readiness_divergent`
/// and has to find the series. A `snake_case` rewrite would make the document
/// and the dashboard two vocabularies.
#[test]
fn alarm_labels_are_the_dotted_names_the_design_set_declares() {
    for alarm in PricingAlarm::ALL {
        let name = alarm.as_str();
        assert!(
            name.starts_with("pricing."),
            "{name} must be the declared dotted name"
        );
        assert!(
            !name.contains("__") && name.contains('.'),
            "{name} must not be rewritten into snake_case"
        );
    }
}

/// Every severity is a legal label, so a later slice adding a Critical alarm
/// does not discover the vocabulary is short.
#[test]
fn every_severity_renders_a_label() {
    let rendered: Vec<&str> = AlarmSeverity::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(rendered, ["info", "warn", "critical"]);
}

// ---------------------------------------------------------------------------
// The no-op.
// ---------------------------------------------------------------------------

/// The default port is usable behind the `Arc<dyn …>` the surfaces hold.
///
/// It is what every unit test and every construction before an exporter is wired
/// holds, so a missing exporter can never be the reason a publish fails. Covers
/// the dyn-dispatch path, which a concrete-type call would not.
#[test]
fn the_noop_port_is_usable_as_a_dyn_port_and_does_nothing() {
    let m: std::sync::Arc<dyn PricingMetricsPort> = std::sync::Arc::new(NoopPricingMetrics);
    m.preview_failclosed(PreviewFailClosed::MarketAbsent);
    m.currency_binding_block(CurrencyBindingCase::RequiredAddon);
    m.tax_not_sellable_ga(9);
    m.alarm(PricingAlarm::TaxNotSellableGaActive);
}

/// The adapter is usable behind the same `Arc<dyn …>`.
#[test]
fn the_otel_adapter_is_usable_as_a_dyn_port() {
    let h = MetricsHarness::new();
    let m: std::sync::Arc<dyn PricingMetricsPort> = std::sync::Arc::new(h.metrics());

    m.preview_failclosed(PreviewFailClosed::NoPublishedVersion);
    h.force_flush();

    assert_eq!(
        h.counter_value(
            "pricing_preview_failclosed_total",
            &[("reason", "no_published_version")]
        ),
        1,
        "dyn dispatch reaches the same instrument"
    );
}

/// Two harnesses do not share a meter provider.
///
/// These cases run in one process; a shared global would let one test's counter
/// decide another's assertion — the same reason every repository suite in this
/// crate builds its own database.
#[test]
fn two_harnesses_are_isolated_from_each_other() {
    let a = MetricsHarness::new();
    let b = MetricsHarness::new();

    a.metrics()
        .preview_failclosed(PreviewFailClosed::MarketAbsent);
    a.force_flush();
    b.force_flush();

    assert_eq!(
        a.counter_value(
            "pricing_preview_failclosed_total",
            &[("reason", "market_absent")]
        ),
        1
    );
    assert_eq!(
        b.counter_value(
            "pricing_preview_failclosed_total",
            &[("reason", "market_absent")]
        ),
        0,
        "one harness's emission must not be visible in another"
    );
}

// ---------------------------------------------------------------------------
// The publish path's derivation (`report_market_metrics`).
// ---------------------------------------------------------------------------

mod publish_path {
    use std::collections::{BTreeMap, BTreeSet};

    use chrono::{DateTime, TimeZone, Utc};
    use uuid::Uuid;

    use super::MetricsHarness;
    use crate::domain::concurrency::RowVersion;
    use crate::domain::currency_binding::{AddonCoverage, Market};
    use crate::domain::lifecycle::LifecycleState;
    use crate::domain::money::{CurrencyCode, MinorAmount};
    use crate::domain::plan_rules::{CustomIntervalBounds, DescriptorSetComplete};
    use crate::domain::plan_shape::{AddonRule, PlanShape};
    use crate::domain::price_record::PriceRecord;
    use crate::domain::price_row::PriceRow;
    use crate::domain::publish::rules::{PublishRuleParams, SoftSizeCaps};
    use crate::domain::scope_key::{
        ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
    };
    use crate::infra::metrics::report_market_metrics;

    const ADDON: Uuid = Uuid::from_u128(0x0add_000a);
    const BLOCKS: &str = "pricing_currency_binding_blocks_total";
    const GAUGE: &str = "pricing_tax_not_sellable_ga";

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 6, 12, 0, 0)
            .single()
            .expect("the fixed instant is unambiguous")
    }

    fn market(currency: &str, region: &str) -> Market {
        (
            CurrencyCode::new(currency).expect("three letters"),
            Region::new(region).expect("non-blank"),
        )
    }

    /// One candidate row on a market, at an eligibility and a tax basis.
    fn row(
        price_id: u128,
        currency: &str,
        region: &str,
        tax_inclusive: bool,
        eligibility: PriceEligibility,
    ) -> PriceRecord {
        let cohort = if eligibility == PriceEligibility::ExistingGrandfathered {
            Cohort::Generation(now())
        } else {
            Cohort::None
        };
        let scope_key = ScopeKey::new(
            PlanId::new(Uuid::from_u128(0x91a4)),
            CurrencyCode::new(currency).expect("three letters"),
            Region::new(region).expect("non-blank"),
            PhaseId::new(Uuid::from_u128(0xf1)),
            eligibility,
            ChargeKind::Recurring,
            cohort,
        )
        .expect("the eligibility and cohort pair");

        let mut shape = PriceRow::new(ChargeKind::Recurring, None);
        shape.amount_minor = Some(MinorAmount::new(1000).expect("non-negative"));

        PriceRecord {
            price_id: Uuid::from_u128(price_id),
            scope_key,
            row: shape,
            tax_inclusive,
            tax_category_ref: None,
            billing_timing: None,
            rounding_policy_ref: None,
            grandfather_until: None,
            supersedes_price_id: None,
            lifecycle_state: LifecycleState::Draft,
            created_by: Uuid::from_u128(0xac_10),
            created_at_utc: now(),
            row_version: RowVersion::new(0),
        }
    }

    /// A plan whose rows are exactly the given `(currency, region, taxInclusive)`
    /// triples, with one required add-on when `required_addon` is set.
    fn shape_of(rows: &[(&str, &str, bool)], required_addon: bool) -> PlanShape {
        let mut shape = PlanShape::new(PlanId::new(Uuid::from_u128(0x91a4)), 1, now());
        shape.rows = rows
            .iter()
            .enumerate()
            .map(|(n, (c, r, tax))| {
                row(
                    0xb000 + n as u128,
                    c,
                    r,
                    *tax,
                    PriceEligibility::AllSubscriptions,
                )
            })
            .collect();
        if required_addon {
            shape.addon_rules = vec![AddonRule {
                addon_sku_id: ADDON,
                required: true,
                min_qty: None,
                max_qty: None,
                step_qty: None,
                price_override_ref: None,
                depends_on: Vec::new(),
                conflicts_with: Vec::new(),
            }];
        }
        shape
    }

    /// Parameters whose add-on coverage is exactly the given markets.
    fn params_covering(markets: &[(&str, &str)]) -> PublishRuleParams {
        // The ratified launch caps rather than zeros: nothing here is about the
        // interval or size rules, and a zero cap would put the shape in a state
        // they reject instead.
        PublishRuleParams::new(
            CustomIntervalBounds::new(366, 24),
            DescriptorSetComplete::default(),
            None,
            SoftSizeCaps::new(100, 500),
        )
        .with_addon_coverage(AddonCoverage::new(
            [(
                ADDON,
                markets
                    .iter()
                    .map(|(c, r)| market(c, r))
                    .collect::<BTreeSet<Market>>(),
            )]
            .into_iter()
            .collect::<BTreeMap<_, _>>(),
        ))
    }

    /// An add-on missing a market the plan sells is counted, under case (i)'s
    /// label.
    #[test]
    fn an_uncovered_required_addon_counts_one_block() {
        let h = MetricsHarness::new();

        report_market_metrics(
            &h.metrics(),
            &shape_of(&[("EUR", "EU", false), ("USD", "US", false)], true),
            &params_covering(&[("EUR", "EU")]),
        );
        h.force_flush();

        assert_eq!(h.counter_value(BLOCKS, &[("case", "required_addon")]), 1);
    }

    /// **One block per publish, not one per offending add-on or market.**
    ///
    /// §10 labels this `case`, and a plan missing three markets is one authoring
    /// mistake. A counter that grew with the gap would make the same fault look
    /// worse the wider it was.
    #[test]
    fn a_gap_of_three_markets_is_still_one_block() {
        let h = MetricsHarness::new();

        report_market_metrics(
            &h.metrics(),
            &shape_of(
                &[
                    ("EUR", "EU", false),
                    ("USD", "US", false),
                    ("GBP", "UK", false),
                    ("JPY", "JP", false),
                ],
                true,
            ),
            &params_covering(&[("EUR", "EU")]),
        );
        h.force_flush();

        assert_eq!(h.counter_value(BLOCKS, &[("case", "required_addon")]), 1);
    }

    /// A plan whose add-on covers everything it sells counts nothing.
    ///
    /// The negative control: a derivation that counted every publish would pass
    /// every assertion above and would report a healthy catalog as permanently
    /// blocked.
    #[test]
    fn a_fully_covered_plan_counts_no_block() {
        let h = MetricsHarness::new();

        report_market_metrics(
            &h.metrics(),
            &shape_of(&[("EUR", "EU", false)], true),
            &params_covering(&[("EUR", "EU"), ("USD", "US")]),
        );
        h.force_flush();

        assert_eq!(h.counter_value(BLOCKS, &[("case", "required_addon")]), 0);
    }

    /// Cases (ii) and (iii) are **not** counted under case (i)'s label.
    ///
    /// `bundle_rules` raises the *same* `CURRENCY_NOT_COVERED` string, so a
    /// derivation scanning the report by code would label a bundle's fault
    /// `required_addon`. This asks the domain verdict instead, and a plan with no
    /// add-on rule at all has no case (i) to report however the bundle plane
    /// answered.
    #[test]
    fn the_required_addon_label_is_not_worn_by_the_bundle_cases() {
        let h = MetricsHarness::new();

        report_market_metrics(
            &h.metrics(),
            &shape_of(&[("EUR", "EU", false), ("USD", "US", false)], false),
            &params_covering(&[]),
        );
        h.force_flush();

        assert_eq!(
            h.counter_value(BLOCKS, &[("case", "required_addon")]),
            0,
            "a plan composing no add-on cannot be a case (i) block"
        );
        assert_eq!(
            h.counter_value(BLOCKS, &[("case", "bundle_sum_of_parts")]),
            0
        );
        assert_eq!(h.counter_value(BLOCKS, &[("case", "bundle_own_price")]), 0);
    }

    /// The gauge counts **markets**, not rows.
    ///
    /// `inst-td-gagate` makes the flag per row and hence per `(currency,
    /// region)`; a hybrid plan carrying four gated rows on one market is one
    /// gated market, and an operator reading the C3 backlog is counting markets
    /// to unblock.
    #[test]
    fn the_gauge_counts_gated_markets_rather_than_gated_rows() {
        let h = MetricsHarness::new();

        report_market_metrics(
            &h.metrics(),
            &shape_of(
                &[
                    ("EUR", "EU", true),
                    ("EUR", "EU", true),
                    ("GBP", "UK", true),
                    ("USD", "US", false),
                ],
                false,
            ),
            &params_covering(&[]),
        );
        h.force_flush();

        assert_eq!(
            h.gauge_value(GAUGE, &[]),
            2,
            "two markets are gated; the third is tax-exclusive and the duplicate \
             row is the same market"
        );
    }

    /// A plan gating nothing reports **zero**, and reports it rather than staying
    /// silent.
    ///
    /// A gauge only written when it is non-zero never falls: it would hold the
    /// last gated count forever and tell an operator the backlog stands after the
    /// plan that made it was fixed.
    #[test]
    fn a_plan_gating_nothing_reports_zero() {
        let h = MetricsHarness::new();

        report_market_metrics(
            &h.metrics(),
            &shape_of(&[("USD", "US", false)], false),
            &params_covering(&[]),
        );
        h.force_flush();

        assert_eq!(
            h.gauge_point(GAUGE, &[]),
            Some(0),
            "the zero must be **written**: a gauge only written when non-zero \
             never falls, and would report a backlog that no longer exists"
        );
    }
}
