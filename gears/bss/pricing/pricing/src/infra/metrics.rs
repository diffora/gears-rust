//! `OpenTelemetry` adapter implementing [`PricingMetricsPort`].
//!
//! Instruments are pulled from the process-global meter provider the host
//! installs; until an exporter is wired they are no-ops, so emitting is always
//! cheap and can never be the reason a publish fails.
//!
//! # Names are literal Prometheus names, suffixes baked in
//!
//! The platform's collector runs `add_metric_suffixes: false`, so the exporter
//! renders instrument names **verbatim**. Counters therefore end `_total` and
//! duration histograms `_seconds` in the name itself, and nothing calls
//! `.with_unit()` — a unit set there would be silently dropped rather than
//! appended. Gauges carry a unit in the name when they have one and stay bare
//! when they are a count. This is the sibling ledger's posture, and RBAC / RMS /
//! openbao's before it; the value of matching it exactly is that one scrape
//! config and one naming convention cover the estate.
//!
//! The names below are transcribed from `design/04-currency-tax.md` §10 rather
//! than derived from the method names, for the reason every code constant in
//! this crate is: a name that drifted from its declaration would still compile
//! and would still be wrong on the wire.

use std::collections::BTreeSet;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Meter};

use crate::domain::currency_binding::uncovered_required_addons;
use crate::domain::money::CurrencyCode;
use crate::domain::plan_shape::PlanShape;
use crate::domain::ports::metrics::{
    CurrencyBindingCase, PreviewFailClosed, PricingAlarm, PricingMetricsPort,
};
use crate::domain::publish::rules::PublishRuleParams;
use crate::domain::scope_key::Region;
use crate::domain::tax_display::{TAX_ENGINE_GA, is_not_sellable_ga};

/// Meter / instrumentation scope name — the gear's own name.
pub(crate) const METER_NAME: &str = "bss-pricing";

// ─── Metric names (literal Prometheus form; `add_metric_suffixes: false`) ────

/// §10: base-price previews that refused to answer, by reason.
const PRICING_PREVIEW_FAILCLOSED: &str = "pricing_preview_failclosed_total";
/// §10: publishes blocked by the currency-binding rule, by enumerated case.
const PRICING_CURRENCY_BINDING_BLOCKS: &str = "pricing_currency_binding_blocks_total";
/// §10: the live count of published tax-inclusive rows awaiting Tax Engine GA.
///
/// Bare — no unit suffix, because it is a count of markets rather than a
/// measurement of anything.
const PRICING_TAX_NOT_SELLABLE_GA: &str = "pricing_tax_not_sellable_ga";
/// The catalog-wide alarm rollup, `{alarm,severity}`.
///
/// Not declared under that name by any one slice: §7 of each slice declares
/// alarm *names*, and this is the one instrument they are all counted on. The
/// sibling ledger's `ledger_alarm_total` is the same construct, and matching it
/// means one alerting rule shape covers both gears.
const PRICING_ALARM: &str = "pricing_alarm_total";

/// The `OTel`-backed port.
///
/// Instruments are built once at construction rather than looked up per call:
/// `Meter::u64_counter` walks the provider's registry, and a publish path that
/// did that on every refusal would pay for the plane it is only reporting to.
pub struct PricingMetricsMeter {
    preview_failclosed: Counter<u64>,
    currency_binding_blocks: Counter<u64>,
    tax_not_sellable_ga: Gauge<i64>,
    alarm: Counter<u64>,
}

impl std::fmt::Debug for PricingMetricsMeter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Counter` and `Gauge` are opaque handles with no useful `Debug`, and
        // the struct is held inside a runtime that derives one.
        f.debug_struct("PricingMetricsMeter")
            .finish_non_exhaustive()
    }
}

impl PricingMetricsMeter {
    /// Build from the process-global meter provider.
    #[must_use]
    pub fn new() -> Self {
        Self::from_meter(&opentelemetry::global::meter(METER_NAME))
    }

    /// Build from a specific meter — the seam the in-memory harness uses.
    #[must_use]
    pub fn from_meter(meter: &Meter) -> Self {
        Self {
            preview_failclosed: meter
                .u64_counter(PRICING_PREVIEW_FAILCLOSED)
                .with_description(
                    "Base-price previews that refused to answer, by reason. The catalog \
                     performs no FX, so an unsold market is a refusal rather than a \
                     converted price.",
                )
                .build(),
            currency_binding_blocks: meter
                .u64_counter(PRICING_CURRENCY_BINDING_BLOCKS)
                .with_description(
                    "Publishes blocked by the single-currency-per-invoice binding, by \
                     enumerated case.",
                )
                .build(),
            tax_not_sellable_ga: meter
                .i64_gauge(PRICING_TAX_NOT_SELLABLE_GA)
                .with_description(
                    "Published tax-inclusive price rows flagged not-sellable until Tax \
                     Engine GA, counted per market.",
                )
                .build(),
            alarm: meter
                .u64_counter(PRICING_ALARM)
                .with_description("Catalog alarm firings, by declared alarm name and severity.")
                .build(),
        }
    }
}

impl Default for PricingMetricsMeter {
    fn default() -> Self {
        Self::new()
    }
}

impl PricingMetricsPort for PricingMetricsMeter {
    fn preview_failclosed(&self, reason: PreviewFailClosed) {
        self.preview_failclosed
            .add(1, &[KeyValue::new("reason", reason.as_str())]);
    }

    fn currency_binding_block(&self, case: CurrencyBindingCase) {
        self.currency_binding_blocks
            .add(1, &[KeyValue::new("case", case.as_str())]);
    }

    fn tax_not_sellable_ga(&self, count: i64) {
        self.tax_not_sellable_ga.record(count, &[]);
    }

    fn alarm(&self, alarm: PricingAlarm) {
        self.alarm.add(
            1,
            &[
                KeyValue::new("alarm", alarm.as_str()),
                // Taken from the alarm rather than the caller, so two firings of
                // one alarm cannot disagree about how urgent it is.
                KeyValue::new("severity", alarm.severity().as_str()),
            ],
        );
    }
}

// ---------------------------------------------------------------------------
// The publish path's two market-axis instruments.
// ---------------------------------------------------------------------------

/// Report §10's two market-axis instruments off one validated shape.
///
/// **A free function here rather than a method on `PublishService`**, for two
/// reasons. The publish engine is a file this slice is only permitted to extend
/// narrowly, so the emission there is a single call; and the derivation is then
/// testable against the in-memory exporter without staging a publishable plan,
/// which is what makes the label assertions below cheap enough to be exhaustive.
///
/// **Called from the pre-check, not the commit**, and that is the whole
/// decision. `pricing_currency_binding_blocks_total` counts the *rule* firing,
/// and a plan refused at pre-check never reaches a commit — counting there would
/// report only the blocks an operator went on to ignore. The GA gauge is likewise
/// a property of the candidate set: it answers "how many markets would this plan
/// gate", which is what an operator watching the C3 backlog asks while authoring.
///
/// Emitting cannot fail and cannot block: the port is a no-op until the host
/// wires an exporter.
pub fn report_market_metrics(
    metrics: &dyn PricingMetricsPort,
    shape: &PlanShape,
    params: &PublishRuleParams,
) {
    // One per distinct blocking case, not per offending add-on: §10 labels this
    // `case`, and a plan whose four add-ons all miss a market is one authoring
    // mistake rather than four.
    //
    // Asked of the **domain verdict, not of the report**. `bundle_rules` raises
    // the same `CURRENCY_NOT_COVERED` string for cases (ii) and (iii), so a
    // counter scanning violations by code would label a bundle's fault
    // `required_addon` — and would do so invisibly, since the refusal an operator
    // sees would still be correct.
    if !uncovered_required_addons(shape, params.addon_coverage()).is_empty() {
        metrics.currency_binding_block(CurrencyBindingCase::RequiredAddon);
    }

    // The candidate set's gated markets, deduplicated by `(currency, region)`:
    // `inst-td-gagate` makes the flag per market, and a hybrid plan carrying four
    // gated rows on one market is one gated market.
    let gated: BTreeSet<(CurrencyCode, Region)> = shape
        .rows
        .iter()
        .filter(|record| is_not_sellable_ga(record, TAX_ENGINE_GA))
        .map(|record| {
            (
                record.scope_key.currency().clone(),
                record.scope_key.region().clone(),
            )
        })
        .collect();
    metrics.tax_not_sellable_ga(i64::try_from(gated.len()).unwrap_or(i64::MAX));
}

// ---------------------------------------------------------------------------
// The in-memory harness.
// ---------------------------------------------------------------------------

/// Assert what was actually emitted, against a real SDK reader.
///
/// Behind `test-support` so `opentelemetry_sdk` never enters the release graph.
/// It reads the **exported** metric stream rather than a hand-rolled spy, which
/// is the whole point: a spy proves the adapter called its own recording method,
/// and this proves an instrument by that name, with those attributes, reached an
/// exporter — which is the claim a dashboard depends on.
#[cfg(feature = "test-support")]
// This module is only ever compiled into a test binary, and a harness that
// swallowed a flush failure would answer zero for every instrument and pass
// every assertion that expected zero — the panic is the point.
#[allow(clippy::expect_used)]
pub mod test_harness {
    use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

    use opentelemetry::metrics::MeterProvider as _;

    use super::{METER_NAME, PricingMetricsMeter};

    /// A meter provider whose exports are readable in the same process.
    pub struct MetricsHarness {
        exporter: InMemoryMetricExporter,
        provider: SdkMeterProvider,
    }

    impl MetricsHarness {
        /// A fresh provider and exporter, isolated from every other harness.
        ///
        /// **Not the process-global provider**, deliberately: these cases run in
        /// one process and a shared global would let one test's counter decide
        /// another's assertion — the same reason every repository suite here
        /// builds its own database.
        #[must_use]
        pub fn new() -> Self {
            let exporter = InMemoryMetricExporter::default();
            let provider = SdkMeterProvider::builder()
                .with_reader(PeriodicReader::builder(exporter.clone()).build())
                .build();
            Self { exporter, provider }
        }

        /// The adapter bound to this harness's meter.
        #[must_use]
        pub fn metrics(&self) -> PricingMetricsMeter {
            PricingMetricsMeter::from_meter(&self.provider.meter(METER_NAME))
        }

        /// Push the pending measurements through to the exporter.
        ///
        /// Required before any assertion: the reader is periodic, so nothing is
        /// exported until a collection happens, and a case that asserted without
        /// flushing would read zero for every instrument and pass whenever it
        /// expected zero.
        pub fn force_flush(&self) {
            self.provider
                .force_flush()
                .expect("flush the meter provider");
        }

        /// The value of one counter data point, selected by its attributes.
        ///
        /// `0` for a series that was never recorded, which is what makes a
        /// negative assertion ("this label was *not* incremented") expressible.
        #[must_use]
        pub fn counter_value(&self, name: &str, attributes: &[(&str, &str)]) -> u64 {
            self.sum_points(name, attributes)
        }

        /// The last recorded value of one gauge data point, or `0` when the
        /// series was never recorded.
        ///
        /// **Use [`Self::gauge_point`] to assert a recorded zero.** The two are
        /// indistinguishable here, and a case asserting `== 0` on a gauge whose
        /// claim is *"this was written, and what was written was zero"* passes
        /// whether or not anything was written at all. A probe that stopped the
        /// write entirely reddened nothing until that distinction existed.
        #[must_use]
        pub fn gauge_value(&self, name: &str, attributes: &[(&str, &str)]) -> i64 {
            self.gauge_point(name, attributes).unwrap_or(0)
        }

        /// The last recorded value of one gauge data point, or `None` when the
        /// series was never recorded at all.
        ///
        /// The distinction matters for a gauge in a way it does not for a
        /// counter: a gauge only written when it is non-zero **never falls**, so
        /// it holds the last gated count forever and tells an operator the
        /// backlog stands after the plan that caused it was fixed.
        #[must_use]
        pub fn gauge_point(&self, name: &str, attributes: &[(&str, &str)]) -> Option<i64> {
            for batch in self.exporter.get_finished_metrics().unwrap_or_default() {
                for scope in batch.scope_metrics() {
                    for metric in scope.metrics() {
                        if metric.name() != name {
                            continue;
                        }
                        if let AggregatedMetrics::I64(MetricData::Gauge(gauge)) = metric.data() {
                            for point in gauge.data_points() {
                                if matches(point.attributes(), attributes) {
                                    return Some(point.value());
                                }
                            }
                        }
                    }
                }
            }
            None
        }

        fn sum_points(&self, name: &str, attributes: &[(&str, &str)]) -> u64 {
            let mut total = 0;
            for batch in self.exporter.get_finished_metrics().unwrap_or_default() {
                for scope in batch.scope_metrics() {
                    for metric in scope.metrics() {
                        if metric.name() != name {
                            continue;
                        }
                        if let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data() {
                            for point in sum.data_points() {
                                if matches(point.attributes(), attributes) {
                                    total += point.value();
                                }
                            }
                        }
                    }
                }
            }
            total
        }
    }

    impl Default for MetricsHarness {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Does this data point carry every expected attribute?
    ///
    /// A **subset** match, not equality: a case asserting `{reason}` should not
    /// have to restate every other attribute the instrument happens to carry,
    /// and an instrument that gained one should not silently break every
    /// assertion about it.
    fn matches<'a>(
        mut point: impl Iterator<Item = &'a opentelemetry::KeyValue>,
        expected: &[(&str, &str)],
    ) -> bool {
        let carried: Vec<(String, String)> = point
            .by_ref()
            .map(|kv| (kv.key.to_string(), kv.value.to_string()))
            .collect();
        expected.iter().all(|(key, value)| {
            carried
                .iter()
                .any(|(k, v)| k == key && v.as_str() == *value)
        })
    }
}

#[cfg(all(test, feature = "test-support"))]
#[path = "metrics_tests.rs"]
mod metrics_tests;
