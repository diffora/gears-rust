//! Unit tests for the OpenTelemetry-backed [`CredStoreMetricsMeter`].

#[cfg(feature = "test-support")]
use super::test_harness::MetricsHarness;
#[cfg(feature = "test-support")]
use crate::domain::ports::metrics::{CredStoreMetricsPort, Dep, DepOp, Outcome, ReadOutcome};

/// Smoke test that exercises instrument construction and every recording
/// path against the global (no-op) meter — no SDK exporter required, so it
/// runs under the default feature set (unlike the `test-support` tests).
#[test]
fn global_meter_records_all_instruments() {
    use super::CredStoreMetricsMeter;
    use crate::domain::ports::metrics::{
        CredStoreMetricsPort, Dep, DepOp, Outcome, ReadOutcome, SecretCounts,
    };

    let m = CredStoreMetricsMeter::from_global();
    assert!(!format!("{m:?}").is_empty());

    m.record_inventory(SecretCounts {
        private: 1,
        tenant: 2,
        shared: 3,
        provisioning: 4,
        tenants: 2,
    });
    for outcome in [
        ReadOutcome::HitOwn,
        ReadOutcome::HitInherited,
        ReadOutcome::Miss,
    ] {
        m.read_outcome(outcome);
    }
    m.walkup_depth(2);
    m.dependency(Dep::Plugin, DepOp::PluginGet, Outcome::Success, 0.01);
    m.dependency(Dep::Pdp, DepOp::Evaluate, Outcome::Error, 0.02);
    m.provisioning_reaped(5);
    m.provisioning_rollback(Outcome::Success);
    m.provisioning_rollback(Outcome::Error);
    m.cross_tenant_denied();
}

#[test]
#[cfg(feature = "test-support")]
fn read_outcome_emits_with_label() {
    let h = MetricsHarness::new();
    let m = h.metrics();
    m.read_outcome(ReadOutcome::HitInherited);
    m.read_outcome(ReadOutcome::Miss);
    h.force_flush();
    assert_eq!(
        h.counter_value(
            "credstore_read_outcome_total",
            &[("outcome", "hit_inherited")]
        ),
        1
    );
    assert_eq!(
        h.counter_value("credstore_read_outcome_total", &[("outcome", "miss")]),
        1
    );
}

#[test]
#[cfg(feature = "test-support")]
fn dependency_emits_duration_and_health() {
    let h = MetricsHarness::new();
    let m = h.metrics();
    m.dependency(Dep::Plugin, DepOp::PluginGet, Outcome::Success, 0.005);
    h.force_flush();
    assert_eq!(
        h.histogram_count(
            "credstore_dependency_query_duration_seconds",
            &[("dependency", "plugin"), ("operation", "plugin_get")]
        ),
        1
    );
    assert_eq!(
        h.counter_value(
            "credstore_dependency_health_total",
            &[
                ("dependency", "plugin"),
                ("operation", "plugin_get"),
                ("outcome", "success")
            ]
        ),
        1
    );
}

#[test]
#[cfg(feature = "test-support")]
fn provisioning_rollback_emits_with_outcome_label() {
    let h = MetricsHarness::new();
    let m = h.metrics();
    m.provisioning_rollback(Outcome::Success);
    m.provisioning_rollback(Outcome::Error);
    m.provisioning_rollback(Outcome::Error);
    h.force_flush();
    assert_eq!(
        h.counter_value(
            "credstore_provisioning_rollback_total",
            &[("outcome", "success")]
        ),
        1
    );
    assert_eq!(
        h.counter_value(
            "credstore_provisioning_rollback_total",
            &[("outcome", "error")]
        ),
        2
    );
}
