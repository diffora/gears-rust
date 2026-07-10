//! End-to-end tests for [`UcMetricsMeter`] against an in-memory
//! OpenTelemetry exporter.
//!
//! These assert the exact **rendered Prometheus names, labels, and bucket
//! layouts** from DESIGN §3.11.5 — the architectural contract. A mistyped
//! name (e.g. a missing `_total`) or a wrong bucket set fails here.

use opentelemetry::metrics::MeterProvider;
use opentelemetry_sdk::metrics::data::{AggregatedMetrics, MetricData};
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

use crate::domain::ports::metrics::{
    AuthzDecision, DeactivationErrorCategory, IngestRequestErrorCategory, IngestRequestOutcome,
    PdpFailureCause, PdpOp, PluginErrorCategory, PluginOp, QueryErrorCategory, QueryKind,
    RecordErrorCategory, RecordKind, RecordOutcome, RequestOutcome, UsageCollectorMetrics,
    UsageTypeErrorCategory, UsageTypeOp,
};
use crate::infra::metrics::{UcMetricsMeter, build_default_adapter};

const TEST_PREFIX: &str = "uc";

fn local_provider() -> (SdkMeterProvider, InMemoryMetricExporter) {
    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .build();
    (provider, exporter)
}

fn meter(provider: &SdkMeterProvider, prefix: &str) -> UcMetricsMeter {
    UcMetricsMeter::new(&provider.meter("usage-collector"), prefix)
}

fn counter_sum(exporter: &InMemoryMetricExporter, name: &str) -> u64 {
    let metrics = exporter.get_finished_metrics().unwrap();
    for rm in &metrics {
        for sm in rm.scope_metrics() {
            for metric in sm.metrics() {
                if metric.name() == name
                    && let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data()
                {
                    return sum
                        .data_points()
                        .map(opentelemetry_sdk::metrics::data::SumDataPoint::value)
                        .sum();
                }
            }
        }
    }
    0
}

fn counter_sum_with_label(
    exporter: &InMemoryMetricExporter,
    name: &str,
    key: &str,
    value: &str,
) -> u64 {
    let metrics = exporter.get_finished_metrics().unwrap();
    for rm in &metrics {
        for sm in rm.scope_metrics() {
            for metric in sm.metrics() {
                if metric.name() == name
                    && let AggregatedMetrics::U64(MetricData::Sum(sum)) = metric.data()
                {
                    return sum
                        .data_points()
                        .filter(|dp| {
                            dp.attributes()
                                .any(|kv| kv.key.as_str() == key && kv.value.as_str() == value)
                        })
                        .map(opentelemetry_sdk::metrics::data::SumDataPoint::value)
                        .sum();
                }
            }
        }
    }
    0
}

fn gauge_last(exporter: &InMemoryMetricExporter, name: &str) -> Option<i64> {
    let metrics = exporter.get_finished_metrics().unwrap();
    for rm in &metrics {
        for sm in rm.scope_metrics() {
            for metric in sm.metrics() {
                if metric.name() == name
                    && let AggregatedMetrics::I64(MetricData::Gauge(g)) = metric.data()
                {
                    return g
                        .data_points()
                        .next()
                        .map(opentelemetry_sdk::metrics::data::GaugeDataPoint::value);
                }
            }
        }
    }
    None
}

fn histogram_count(exporter: &InMemoryMetricExporter, name: &str) -> u64 {
    let metrics = exporter.get_finished_metrics().unwrap();
    for rm in &metrics {
        for sm in rm.scope_metrics() {
            for metric in sm.metrics() {
                if metric.name() == name
                    && let AggregatedMetrics::F64(MetricData::Histogram(h)) = metric.data()
                {
                    return h
                        .data_points()
                        .map(opentelemetry_sdk::metrics::data::HistogramDataPoint::count)
                        .sum();
                }
            }
        }
    }
    0
}

fn histogram_bounds(exporter: &InMemoryMetricExporter, name: &str) -> Option<Vec<f64>> {
    let metrics = exporter.get_finished_metrics().unwrap();
    for rm in &metrics {
        for sm in rm.scope_metrics() {
            for metric in sm.metrics() {
                if metric.name() == name
                    && let AggregatedMetrics::F64(MetricData::Histogram(h)) = metric.data()
                {
                    return h.data_points().next().map(|dp| dp.bounds().collect());
                }
            }
        }
    }
    None
}

// ── PDP-helper instruments ───────────────────────────────────────────

#[test]
fn pdp_decision_records_authz_decisions_and_duration() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);

    m.record_pdp_decision(PdpOp::Ingest, AuthzDecision::Permit, 0.02);
    m.record_pdp_decision(PdpOp::QueryRaw, AuthzDecision::Deny, 0.03);

    provider.force_flush().unwrap();

    assert_eq!(counter_sum(&exporter, "uc_authz_decisions_total"), 2);
    assert_eq!(
        counter_sum_with_label(&exporter, "uc_authz_decisions_total", "decision", "permit"),
        1,
    );
    assert_eq!(
        counter_sum_with_label(&exporter, "uc_authz_decisions_total", "decision", "deny"),
        1,
    );
    assert_eq!(
        counter_sum_with_label(&exporter, "uc_authz_decisions_total", "operation", "ingest"),
        1,
    );
    assert_eq!(histogram_count(&exporter, "uc_pdp_duration_seconds"), 2);
}

#[test]
fn pdp_failure_records_failures_and_duration() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);

    m.record_pdp_failure(PdpOp::Ingest, PdpFailureCause::Unreachable, 0.5);

    provider.force_flush().unwrap();

    assert_eq!(counter_sum(&exporter, "uc_pdp_failures_total"), 1);
    assert_eq!(
        counter_sum_with_label(&exporter, "uc_pdp_failures_total", "cause", "unreachable"),
        1,
    );
    // A failure completion is still a completion — duration is observed.
    assert_eq!(histogram_count(&exporter, "uc_pdp_duration_seconds"), 1);
}

#[test]
fn pdp_duration_buckets_match_design_3_11_5() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);
    m.record_pdp_decision(PdpOp::Ingest, AuthzDecision::Permit, 0.01);
    provider.force_flush().unwrap();

    assert_eq!(
        histogram_bounds(&exporter, "uc_pdp_duration_seconds"),
        Some(vec![
            0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5
        ]),
    );
}

#[test]
fn pdp_ready_gauge_reflects_binding() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);
    m.set_pdp_ready(true);
    provider.force_flush().unwrap();
    assert_eq!(gauge_last(&exporter, "uc_pdp_ready"), Some(1));
}

// ── Plugin-host instruments ──────────────────────────────────────────

#[test]
fn plugin_call_records_duration_with_buckets() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);
    m.record_plugin_call(PluginOp::CreateUsageRecord, 0.05);
    provider.force_flush().unwrap();

    assert_eq!(
        histogram_count(&exporter, "uc_plugin_call_duration_seconds"),
        1,
    );
    assert_eq!(
        histogram_bounds(&exporter, "uc_plugin_call_duration_seconds"),
        Some(vec![
            0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0
        ]),
    );
}

#[test]
fn plugin_accept_error_counter_carries_labels() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);
    m.record_plugin_accept_error(PluginOp::GetUsageType, PluginErrorCategory::Unready);
    m.record_plugin_accept_error(
        PluginOp::CreateUsageRecord,
        PluginErrorCategory::BackendError,
    );
    provider.force_flush().unwrap();

    assert_eq!(counter_sum(&exporter, "uc_plugin_accept_errors_total"), 2);
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_plugin_accept_errors_total",
            "error_category",
            "unready",
        ),
        1,
    );
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_plugin_accept_errors_total",
            "operation",
            "create_usage_record",
        ),
        1,
    );
}

#[test]
fn plugin_ready_gauge_reflects_structural_fact() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);
    m.set_plugin_ready(false);
    provider.force_flush().unwrap();
    assert_eq!(gauge_last(&exporter, "uc_plugin_ready"), Some(0));
}

// ── Prefix substitution + bootstrap smoke ────────────────────────────

#[test]
fn prefix_is_substituted_into_every_name() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, "acme");
    m.record_pdp_decision(PdpOp::Deactivate, AuthzDecision::Permit, 0.01);
    provider.force_flush().unwrap();

    assert_eq!(counter_sum(&exporter, "acme_authz_decisions_total"), 1);
    assert_eq!(counter_sum(&exporter, "uc_authz_decisions_total"), 0);
}

#[test]
fn build_default_adapter_binds_to_global_provider_without_panicking() {
    // Panic-guard smoke test only — NOT coverage of exported data. In the test
    // process the global provider is the NoopMeterProvider, so nothing is wired
    // to a reader and the emitted series can't be read back; this only proves
    // construction against the global provider and a record call don't panic.
    let m = build_default_adapter("uc");
    m.set_pdp_ready(true);
    m.record_plugin_call(PluginOp::ListUsageTypes, 0.001);
    // The constructor hands back a live, uniquely-owned handle ready to share.
    assert_eq!(std::sync::Arc::strong_count(&m), 1);
}

// ── Phase 2: ingestion-gateway instruments ───────────────────────────

#[test]
fn ingestion_instruments_render_names_labels_and_buckets() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);

    m.observe_ingestion_batch_size(20);
    m.observe_ingestion_duration(0.05);
    m.observe_record_metadata_bytes(1500);
    m.record_ingestion_record(
        RecordOutcome::Accepted,
        RecordKind::Compensation,
        RecordErrorCategory::None,
    );
    m.record_ingestion_record(
        RecordOutcome::Rejected,
        RecordKind::Usage,
        RecordErrorCategory::MetadataSize,
    );
    m.record_ingestion_request(
        IngestRequestOutcome::Partial,
        IngestRequestErrorCategory::None,
    );
    provider.force_flush().unwrap();

    assert_eq!(histogram_count(&exporter, "uc_ingestion_batch_size"), 1);
    assert_eq!(
        histogram_bounds(&exporter, "uc_ingestion_batch_size"),
        Some(vec![1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]),
    );
    assert_eq!(
        histogram_count(&exporter, "uc_ingestion_duration_seconds"),
        1
    );
    assert_eq!(
        histogram_bounds(&exporter, "uc_ingestion_duration_seconds"),
        Some(vec![0.01, 0.025, 0.05, 0.1, 0.15, 0.2, 0.3, 0.5, 1.0]),
    );
    assert_eq!(histogram_count(&exporter, "uc_record_metadata_bytes"), 1);
    assert_eq!(
        histogram_bounds(&exporter, "uc_record_metadata_bytes"),
        Some(vec![256.0, 512.0, 1024.0, 2048.0, 4096.0, 8192.0]),
    );
    assert_eq!(counter_sum(&exporter, "uc_ingestion_records_total"), 2);
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_ingestion_records_total",
            "record_kind",
            "compensation",
        ),
        1,
    );
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_ingestion_records_total",
            "error_category",
            "metadata_size",
        ),
        1,
    );
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_ingestion_requests_total",
            "outcome",
            "partial"
        ),
        1,
    );
}

// ── Phase 2: query-gateway instruments ───────────────────────────────

#[test]
fn query_instruments_render_names_labels_and_buckets() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);

    m.query_inflight_inc(QueryKind::Aggregated);
    m.query_inflight_inc(QueryKind::Aggregated);
    m.query_inflight_dec(QueryKind::Aggregated);
    m.observe_query_result_rows(QueryKind::Raw, 42);
    m.record_query_request(
        QueryKind::Aggregated,
        RequestOutcome::Success,
        QueryErrorCategory::None,
        0.3,
    );
    m.record_query_request(
        QueryKind::Raw,
        RequestOutcome::Error,
        QueryErrorCategory::QueryBudget,
        0.01,
    );
    provider.force_flush().unwrap();

    // UpDownCounter renders as a non-monotonic sum: +1 +1 -1 = 1.
    assert_eq!(
        query_inflight_value(&exporter, "uc_query_inflight", "aggregated"),
        Some(1),
    );
    assert_eq!(histogram_count(&exporter, "uc_query_result_rows"), 1);
    assert_eq!(
        histogram_bounds(&exporter, "uc_query_result_rows"),
        Some(vec![
            1.0, 10.0, 50.0, 100.0, 500.0, 1000.0, 10000.0, 100_000.0
        ]),
    );
    assert_eq!(histogram_count(&exporter, "uc_query_duration_seconds"), 2);
    assert_eq!(
        histogram_bounds(&exporter, "uc_query_duration_seconds"),
        Some(vec![0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 2.0, 5.0]),
    );
    assert_eq!(counter_sum(&exporter, "uc_query_requests_total"), 2);
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_query_requests_total",
            "error_category",
            "query_budget"
        ),
        1,
    );
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_query_requests_total",
            "query_kind",
            "aggregated"
        ),
        1,
    );
}

// ── Phase 2: deactivation-handler instruments ────────────────────────

#[test]
fn deactivation_instruments_render_names_and_labels() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);

    m.record_deactivation_request(
        RequestOutcome::Denied,
        DeactivationErrorCategory::Authz,
        0.02,
    );
    provider.force_flush().unwrap();

    assert_eq!(
        histogram_count(&exporter, "uc_deactivation_duration_seconds"),
        1
    );
    assert_eq!(counter_sum(&exporter, "uc_deactivation_requests_total"), 1);
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_deactivation_requests_total",
            "outcome",
            "denied"
        ),
        1,
    );
}

// ── Phase 2: usage-type-catalog instruments ──────────────────────────

#[test]
fn usage_type_instruments_render_names_and_labels() {
    let (provider, exporter) = local_provider();
    let m = meter(&provider, TEST_PREFIX);

    m.record_usage_type_request(
        UsageTypeOp::Create,
        RequestOutcome::Error,
        UsageTypeErrorCategory::Conflict,
    );
    m.set_usage_types(7);
    provider.force_flush().unwrap();

    assert_eq!(counter_sum(&exporter, "uc_usage_type_requests_total"), 1);
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_usage_type_requests_total",
            "operation",
            "create"
        ),
        1,
    );
    assert_eq!(
        counter_sum_with_label(
            &exporter,
            "uc_usage_type_requests_total",
            "error_category",
            "conflict"
        ),
        1,
    );
    assert_eq!(gauge_last(&exporter, "uc_usage_types"), Some(7));
}

/// Read the summed value of an `i64` `UpDownCounter` series filtered to a
/// `query_kind` label.
fn query_inflight_value(exporter: &InMemoryMetricExporter, name: &str, kind: &str) -> Option<i64> {
    let metrics = exporter.get_finished_metrics().unwrap();
    for rm in &metrics {
        for sm in rm.scope_metrics() {
            for metric in sm.metrics() {
                if metric.name() == name
                    && let AggregatedMetrics::I64(MetricData::Sum(sum)) = metric.data()
                {
                    return Some(
                        sum.data_points()
                            .filter(|dp| {
                                dp.attributes().any(|kv| {
                                    kv.key.as_str() == "query_kind" && kv.value.as_str() == kind
                                })
                            })
                            .map(opentelemetry_sdk::metrics::data::SumDataPoint::value)
                            .sum(),
                    );
                }
            }
        }
    }
    None
}
