//! OpenTelemetry-backed implementation of
//! [`UsageCollectorMetrics`](crate::domain::ports::metrics::UsageCollectorMetrics).
//!
//! Instruments are declared on a scoped `Meter` obtained from `ToolKit`'s
//! **global** `SdkMeterProvider` (`opentelemetry::global::meter_with_scope`)
//! at gear bootstrap. The gear never constructs an exporter and exposes no
//! `/metrics` scrape endpoint — telemetry is OTLP-pushed by `ToolKit` per
//! `cpt-cf-usage-collector-principle-otlp-push-emission`. The platform
//! `http.server.*` instruments are NOT redeclared here per
//! `cpt-cf-usage-collector-principle-gateway-http-server-instrument-reuse`.
//!
//! Names are the **full literal** Prometheus names from DESIGN §3.11.5
//! (`_total` on counters, `_seconds` on duration histograms) with **no**
//! `.with_unit(...)` hint, so the rendered series name is identical whether
//! the downstream collector runs with `add_metric_suffixes` on or off.

use std::sync::Arc;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

use crate::domain::ports::metrics::{
    AuthzDecision, DeactivationErrorCategory, IngestRequestErrorCategory, IngestRequestOutcome,
    PdpFailureCause, PdpOp, PluginErrorCategory, PluginOp, QueryErrorCategory, QueryKind,
    RecordErrorCategory, RecordKind, RecordOutcome, RequestOutcome, UsageCollectorMetrics,
    UsageTypeErrorCategory, UsageTypeOp, key,
};

/// Bucket boundaries (seconds) for `uc_pdp_duration_seconds` — brackets the
/// PDP share of the 200 ms ingestion p95 budget (DESIGN §3.11.5).
const PDP_DURATION_BUCKETS_SECONDS: [f64; 9] =
    [0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5];

/// Bucket boundaries (seconds) for `uc_plugin_call_duration_seconds` —
/// separates plugin-owned time from gear overhead (DESIGN §3.11.5).
const PLUGIN_CALL_DURATION_BUCKETS_SECONDS: [f64; 10] =
    [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.0, 5.0];

/// Buckets (seconds) for `uc_ingestion_duration_seconds` and
/// `uc_deactivation_duration_seconds` — bracket the 200 ms ingestion p95
/// budget; deactivation mirrors the ingestion write path (DESIGN §3.11.5).
const INGESTION_DURATION_BUCKETS_SECONDS: [f64; 9] =
    [0.01, 0.025, 0.05, 0.1, 0.15, 0.2, 0.3, 0.5, 1.0];

/// Buckets (seconds) for `uc_query_duration_seconds` — bracket the 500 ms
/// aggregated-query p95 budget (DESIGN §3.11.5).
const QUERY_DURATION_BUCKETS_SECONDS: [f64; 8] = [0.05, 0.1, 0.25, 0.5, 0.75, 1.0, 2.0, 5.0];

/// Buckets for `uc_ingestion_batch_size` (records per request) — upper bucket
/// equals the wire batch cap (DESIGN §3.11.5).
const INGESTION_BATCH_SIZE_BUCKETS: [f64; 7] = [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0];

/// Buckets (bytes) for `uc_record_metadata_bytes` — upper bucket equals the
/// 8 KiB metadata cap (DESIGN §3.11.5).
const RECORD_METADATA_BYTES_BUCKETS: [f64; 6] = [256.0, 512.0, 1024.0, 2048.0, 4096.0, 8192.0];

/// Buckets for `uc_query_result_rows` (rows per response) — DESIGN §3.11.5.
const QUERY_RESULT_ROWS_BUCKETS: [f64; 8] =
    [1.0, 10.0, 50.0, 100.0, 500.0, 1000.0, 10000.0, 100_000.0];

/// The full OpenTelemetry instrument set: the foundation-owned plugin-host +
/// PDP-helper instruments (Phase 1) plus the per-component gateway
/// instruments for ingestion, query, deactivation, and usage-type (Phase 2).
pub struct UcMetricsMeter {
    // ── Plugin-host (owned by foundation §2.1) ──
    plugin_ready: Gauge<i64>,
    plugin_accept_errors: Counter<u64>,
    plugin_call_duration_seconds: Histogram<f64>,

    // ── PDP helper (owned by foundation §2.1) ──
    pdp_ready: Gauge<i64>,
    pdp_failures: Counter<u64>,
    pdp_duration_seconds: Histogram<f64>,
    authz_decisions: Counter<u64>,

    // ── Ingestion gateway (§2.3 usage-emission) ──
    ingestion_requests: Counter<u64>,
    ingestion_records: Counter<u64>,
    ingestion_duration_seconds: Histogram<f64>,
    ingestion_batch_size: Histogram<f64>,
    record_metadata_bytes: Histogram<f64>,

    // ── Query gateway (§2.4 usage-query) ──
    query_requests: Counter<u64>,
    query_duration_seconds: Histogram<f64>,
    query_inflight: opentelemetry::metrics::UpDownCounter<i64>,
    query_result_rows: Histogram<f64>,

    // ── Deactivation handler (§2.5 event-deactivation) ──
    deactivation_requests: Counter<u64>,
    deactivation_duration_seconds: Histogram<f64>,

    // ── UsageType catalog (§2.2 usage-type-lifecycle) ──
    usage_type_requests: Counter<u64>,
    usage_types: Gauge<i64>,
}

impl UcMetricsMeter {
    /// Build every foundation-owned instrument on `meter`. `prefix` is the
    /// substitutable leading namespace segment (`uc` by default) — the
    /// rendered names are `{prefix}_...`.
    ///
    // @cpt-dod:cpt-cf-usage-collector-dod-foundation-observability-instrument-bootstrap:p1
    // @cpt-dod:cpt-cf-usage-collector-dod-foundation-observability-plugin-host-instruments:p1
    // @cpt-dod:cpt-cf-usage-collector-dod-foundation-observability-pdp-helper-instruments:p1
    // @cpt-dod:cpt-cf-usage-collector-dod-foundation-principle-otlp-push-emission:p2
    // @cpt-dod:cpt-cf-usage-collector-dod-foundation-nfr-operational-visibility:p2
    // @cpt-dod:cpt-cf-usage-collector-dod-foundation-observability-label-cardinality:p1
    // @cpt-dod:cpt-cf-usage-collector-dod-foundation-observability-alert-integration:p2
    // @cpt-begin:cpt-cf-usage-collector-flow-foundation-plugin-host-binding:p1:inst-binding-meter-bootstrap
    #[must_use]
    pub fn new(meter: &Meter, prefix: &str) -> Self {
        Self {
            plugin_ready: meter
                .i64_gauge(format!("{prefix}_plugin_ready"))
                .with_description(
                    "1 iff the active storage-plugin binding is structurally resolved",
                )
                .build(),
            plugin_accept_errors: meter
                .u64_counter(format!("{prefix}_plugin_accept_errors_total"))
                .with_description(
                    "Plugin SPI dispatch failures by operation and error_category \
                     (unready / backend_error / timeout)",
                )
                .build(),
            plugin_call_duration_seconds: meter
                .f64_histogram(format!("{prefix}_plugin_call_duration_seconds"))
                .with_description("Plugin SPI dispatch wall-clock by operation")
                .with_boundaries(PLUGIN_CALL_DURATION_BUCKETS_SECONDS.to_vec())
                .build(),
            pdp_ready: meter
                .i64_gauge(format!("{prefix}_pdp_ready"))
                .with_description(
                    "1 while the authz-resolver client is bound in the PolicyEnforcer",
                )
                .build(),
            pdp_failures: meter
                .u64_counter(format!("{prefix}_pdp_failures_total"))
                .with_description("PDP authorization transport/evaluation failures by operation")
                .build(),
            pdp_duration_seconds: meter
                .f64_histogram(format!("{prefix}_pdp_duration_seconds"))
                .with_description("PDP access_scope_with round-trip by operation")
                .with_boundaries(PDP_DURATION_BUCKETS_SECONDS.to_vec())
                .build(),
            authz_decisions: meter
                .u64_counter(format!("{prefix}_authz_decisions_total"))
                .with_description("Completed PDP decisions by operation and decision (permit/deny)")
                .build(),

            // ── Ingestion gateway ──
            // @cpt-dod:cpt-cf-usage-collector-dod-usage-emission-nfr-operational-visibility-ingestion-instruments:p2
            ingestion_requests: meter
                .u64_counter(format!("{prefix}_ingestion_requests_total"))
                .with_description(
                    "Completed ingestion batch requests by outcome and error_category",
                )
                .build(),
            ingestion_records: meter
                .u64_counter(format!("{prefix}_ingestion_records_total"))
                .with_description(
                    "Per-record ingestion acknowledgements by outcome, record_kind, error_category",
                )
                .build(),
            ingestion_duration_seconds: meter
                .f64_histogram(format!("{prefix}_ingestion_duration_seconds"))
                .with_description("Ingestion request wall-clock")
                .with_boundaries(INGESTION_DURATION_BUCKETS_SECONDS.to_vec())
                .build(),
            ingestion_batch_size: meter
                .f64_histogram(format!("{prefix}_ingestion_batch_size"))
                .with_description("Records per received batch submission")
                .with_boundaries(INGESTION_BATCH_SIZE_BUCKETS.to_vec())
                .build(),
            record_metadata_bytes: meter
                .f64_histogram(format!("{prefix}_record_metadata_bytes"))
                .with_description("Serialized RecordMetadata size in bytes per submitted record")
                .with_boundaries(RECORD_METADATA_BYTES_BUCKETS.to_vec())
                .build(),

            // ── Query gateway ──
            // @cpt-dod:cpt-cf-usage-collector-dod-usage-query-nfr-operational-visibility:p2
            query_requests: meter
                .u64_counter(format!("{prefix}_query_requests_total"))
                .with_description("Completed query attempts by query_kind, outcome, error_category")
                .build(),
            query_duration_seconds: meter
                .f64_histogram(format!("{prefix}_query_duration_seconds"))
                .with_description("Query wall-clock by query_kind")
                .with_boundaries(QUERY_DURATION_BUCKETS_SECONDS.to_vec())
                .build(),
            query_inflight: meter
                .i64_up_down_counter(format!("{prefix}_query_inflight"))
                .with_description("Currently in-flight queries by query_kind")
                .build(),
            query_result_rows: meter
                .f64_histogram(format!("{prefix}_query_result_rows"))
                .with_description("Rows/groups returned per successful query by query_kind")
                .with_boundaries(QUERY_RESULT_ROWS_BUCKETS.to_vec())
                .build(),

            // ── Deactivation handler ──
            // @cpt-dod:cpt-cf-usage-collector-dod-event-deactivation-nfr-operational-visibility:p2
            deactivation_requests: meter
                .u64_counter(format!("{prefix}_deactivation_requests_total"))
                .with_description("Completed deactivation attempts by outcome and error_category")
                .build(),
            deactivation_duration_seconds: meter
                .f64_histogram(format!("{prefix}_deactivation_duration_seconds"))
                .with_description("Deactivation request wall-clock")
                .with_boundaries(INGESTION_DURATION_BUCKETS_SECONDS.to_vec())
                .build(),

            // ── UsageType catalog ──
            // @cpt-dod:cpt-cf-usage-collector-dod-usage-type-lifecycle-nfr-operational-visibility:p2
            usage_type_requests: meter
                .u64_counter(format!("{prefix}_usage_type_requests_total"))
                .with_description(
                    "Completed UsageType-lifecycle attempts by operation, outcome, error_category",
                )
                .build(),
            usage_types: meter
                .i64_gauge(format!("{prefix}_usage_types"))
                .with_description("Current entry count of the plugin-owned usage_type_catalog")
                .build(),
        }
    }
    // @cpt-end:cpt-cf-usage-collector-flow-foundation-plugin-host-binding:p1:inst-binding-meter-bootstrap
}

impl UsageCollectorMetrics for UcMetricsMeter {
    fn set_pdp_ready(&self, ready: bool) {
        self.pdp_ready.record(i64::from(ready), &[]);
    }

    fn record_pdp_decision(&self, op: PdpOp, decision: AuthzDecision, seconds: f64) {
        self.pdp_duration_seconds
            .record(seconds, &[KeyValue::new(key::OPERATION, op.as_str())]);
        self.authz_decisions.add(
            1,
            &[
                KeyValue::new(key::OPERATION, op.as_str()),
                KeyValue::new(key::DECISION, decision.as_str()),
            ],
        );
    }

    fn record_pdp_failure(&self, op: PdpOp, cause: PdpFailureCause, seconds: f64) {
        self.pdp_duration_seconds
            .record(seconds, &[KeyValue::new(key::OPERATION, op.as_str())]);
        self.pdp_failures.add(
            1,
            &[
                KeyValue::new(key::OPERATION, op.as_str()),
                KeyValue::new(key::CAUSE, cause.as_str()),
            ],
        );
    }

    fn set_plugin_ready(&self, ready: bool) {
        self.plugin_ready.record(i64::from(ready), &[]);
    }

    fn record_plugin_call(&self, op: PluginOp, seconds: f64) {
        self.plugin_call_duration_seconds
            .record(seconds, &[KeyValue::new(key::OPERATION, op.as_str())]);
    }

    fn record_plugin_accept_error(&self, op: PluginOp, category: PluginErrorCategory) {
        self.plugin_accept_errors.add(
            1,
            &[
                KeyValue::new(key::OPERATION, op.as_str()),
                KeyValue::new(key::ERROR_CATEGORY, category.as_str()),
            ],
        );
    }

    // ── Ingestion gateway ──

    fn observe_ingestion_batch_size(&self, size: u64) {
        // f64 histogram; batch sizes (1..=100) are exactly representable.
        #[allow(clippy::cast_precision_loss)]
        self.ingestion_batch_size.record(size as f64, &[]);
    }

    fn observe_ingestion_duration(&self, seconds: f64) {
        self.ingestion_duration_seconds.record(seconds, &[]);
    }

    fn observe_record_metadata_bytes(&self, bytes: u64) {
        #[allow(clippy::cast_precision_loss)]
        self.record_metadata_bytes.record(bytes as f64, &[]);
    }

    fn record_ingestion_record(
        &self,
        outcome: RecordOutcome,
        kind: RecordKind,
        error_category: RecordErrorCategory,
    ) {
        self.ingestion_records.add(
            1,
            &[
                KeyValue::new(key::OUTCOME, outcome.as_str()),
                KeyValue::new(key::RECORD_KIND, kind.as_str()),
                KeyValue::new(key::ERROR_CATEGORY, error_category.as_str()),
            ],
        );
    }

    fn record_ingestion_request(
        &self,
        outcome: IngestRequestOutcome,
        error_category: IngestRequestErrorCategory,
    ) {
        self.ingestion_requests.add(
            1,
            &[
                KeyValue::new(key::OUTCOME, outcome.as_str()),
                KeyValue::new(key::ERROR_CATEGORY, error_category.as_str()),
            ],
        );
    }

    // ── Query gateway ──

    fn query_inflight_inc(&self, kind: QueryKind) {
        self.query_inflight
            .add(1, &[KeyValue::new(key::QUERY_KIND, kind.as_str())]);
    }

    fn query_inflight_dec(&self, kind: QueryKind) {
        self.query_inflight
            .add(-1, &[KeyValue::new(key::QUERY_KIND, kind.as_str())]);
    }

    fn observe_query_result_rows(&self, kind: QueryKind, rows: u64) {
        #[allow(clippy::cast_precision_loss)]
        self.query_result_rows.record(
            rows as f64,
            &[KeyValue::new(key::QUERY_KIND, kind.as_str())],
        );
    }

    fn record_query_request(
        &self,
        kind: QueryKind,
        outcome: RequestOutcome,
        error_category: QueryErrorCategory,
        seconds: f64,
    ) {
        self.query_duration_seconds
            .record(seconds, &[KeyValue::new(key::QUERY_KIND, kind.as_str())]);
        self.query_requests.add(
            1,
            &[
                KeyValue::new(key::QUERY_KIND, kind.as_str()),
                KeyValue::new(key::OUTCOME, outcome.as_str()),
                KeyValue::new(key::ERROR_CATEGORY, error_category.as_str()),
            ],
        );
    }

    // ── Deactivation handler ──

    fn record_deactivation_request(
        &self,
        outcome: RequestOutcome,
        error_category: DeactivationErrorCategory,
        seconds: f64,
    ) {
        self.deactivation_duration_seconds.record(seconds, &[]);
        self.deactivation_requests.add(
            1,
            &[
                KeyValue::new(key::OUTCOME, outcome.as_str()),
                KeyValue::new(key::ERROR_CATEGORY, error_category.as_str()),
            ],
        );
    }

    // ── UsageType catalog ──

    fn record_usage_type_request(
        &self,
        op: UsageTypeOp,
        outcome: RequestOutcome,
        error_category: UsageTypeErrorCategory,
    ) {
        self.usage_type_requests.add(
            1,
            &[
                KeyValue::new(key::OPERATION, op.as_str()),
                KeyValue::new(key::OUTCOME, outcome.as_str()),
                KeyValue::new(key::ERROR_CATEGORY, error_category.as_str()),
            ],
        );
    }

    fn set_usage_types(&self, count: u64) {
        #[allow(clippy::cast_possible_wrap)]
        self.usage_types.record(count as i64, &[]);
    }
}

/// Convenience constructor used at gear bootstrap: build an
/// `Arc<UcMetricsMeter>` against the process-global OpenTelemetry meter
/// provider, scoped to the `usage-collector` instrumentation library.
///
/// The instruments bind to whatever global provider exists at construction
/// time; in production `ToolKit` installs the real `SdkMeterProvider` before
/// `Gear::init` runs, so this binds to the OTLP-push pipeline.
// @cpt-begin:cpt-cf-usage-collector-flow-foundation-plugin-host-binding:p1:inst-binding-meter-bootstrap
#[must_use]
pub fn build_default_adapter(prefix: &str) -> Arc<UcMetricsMeter> {
    let scope = opentelemetry::InstrumentationScope::builder("usage-collector").build();
    let meter = opentelemetry::global::meter_with_scope(scope);
    Arc::new(UcMetricsMeter::new(&meter, prefix))
}
// @cpt-end:cpt-cf-usage-collector-flow-foundation-plugin-host-binding:p1:inst-binding-meter-bootstrap

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "metrics_tests.rs"]
mod metrics_tests;
