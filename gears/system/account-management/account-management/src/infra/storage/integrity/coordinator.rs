//! Hierarchy-integrity coordinator — owns the AM-local lease and
//! drives one `check + maybe-repair` cycle per loop tick. This is
//! the production implementation of
//! [`crate::domain::integrity_check::IntegrityChecker::run_tick`].
//!
//! Both phases run under a single [`crate::infra::lease::LeaseGuard`]
//! so a peer cannot slip between them and commit a repair plan
//! derived from a different snapshot.
//!
//! The CHECK phase runs read-only under REPEATABLE READ with no
//! fence — these reads are advisory and a peer steal mid-check
//! only causes us to repair against a stale snapshot, which is
//! exactly the case the REPAIR-phase fence catches at commit. The
//! REPAIR phase runs under `SERIALIZABLE` retry with the fence
//! SELECT appended; a stolen lease aborts the whole repair tx and
//! surfaces as [`IntegrityCheckOutcome::AbortedLeaseLost`] for the
//! loop driver to translate into metrics.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use toolkit_security::AccessScope;

use tracing::warn;

use crate::domain::error::DomainError;
use crate::domain::integrity_check::{IntegrityCheckOutcome, IntegrityChecker};
use crate::domain::metrics::{
    AM_HIERARCHY_INTEGRITY_REPAIRED, AM_HIERARCHY_INTEGRITY_VIOLATIONS, emit_gauge_value,
};
use crate::domain::tenant::integrity::{
    IntegrityCategory, IntegrityReport, RepairReport, Violation,
};
use crate::infra::canonical_mapping::classify_db_err_to_domain;
use crate::infra::lease::{AckError, CoordError, LeaseManager};
use crate::infra::storage::repo_impl::TenantRepoImpl;
use crate::infra::storage::repo_impl::helpers::TxError;
use crate::infra::storage::repo_impl::integrity::{apply_repair_in_tx, run_check_under_lease};

/// The lease key used by every reconciler coordinator instance.
///
/// `TEXT` PK on `am_leases` (see `m0007`) leaves room for a second
/// coordination domain, but today this is the only key in use.
pub const HIERARCHY_INTEGRITY_KEY: &str = "hierarchy_integrity";

/// Hierarchy-integrity coordinator.
///
/// One instance is constructed at gear bootstrap (see
/// [`crate::gear::AccountManagementGear::serve`]) and shared
/// across reconciler ticks. Cheap to clone (the only owned field
/// is an `Arc`).
#[derive(Clone)]
pub struct IntegrityCoordinator {
    repo: Arc<TenantRepoImpl>,
    ttl: Duration,
    renew_period: Duration,
}

impl IntegrityCoordinator {
    #[must_use]
    pub fn new(repo: Arc<TenantRepoImpl>, ttl: Duration, renew_period: Duration) -> Self {
        Self {
            repo,
            ttl,
            renew_period,
        }
    }

    /// Borrow the underlying repo. Exposed for unit tests that
    /// want to seed `am_leases` rows directly to simulate peer
    /// contention.
    #[must_use]
    pub fn repo(&self) -> &Arc<TenantRepoImpl> {
        &self.repo
    }

    /// Execute one check + maybe-repair cycle under a single lease.
    ///
    /// `auto_repair` mirrors the loop driver's
    /// `cfg.repair.enabled && cfg.repair.auto_after_check` flag —
    /// when `false`, the repair phase is skipped and the outcome is
    /// always [`IntegrityCheckOutcome::CompletedClean`] (or
    /// `SkippedInProgress` if the lease was held).
    ///
    /// # Errors
    ///
    /// * Any non-contention DB error from the acquire path or the
    ///   check phase (rebucketed via the canonical
    ///   [`From<toolkit_db::DbError> for DomainError`] ladder).
    /// * Any domain error from the repair body (the work closure
    ///   is allowed to surface typed failures even when the lease
    ///   is held).
    ///
    /// `SkippedInProgress` and `AbortedLeaseLost` are normal `Ok`
    /// outcomes — they describe coordination outcomes, not errors.
    pub async fn run_tick(&self, auto_repair: bool) -> Result<IntegrityCheckOutcome, DomainError> {
        let mgr = LeaseManager::new(self.repo.provider().db());

        // 1. Acquire the lease. LeaseHeld is a normal outcome; any
        //    other CoordError is a hard failure.
        let guard = match mgr.acquire(HIERARCHY_INTEGRITY_KEY, self.ttl).await {
            Ok(g) => g,
            Err(CoordError::LeaseHeld) => return Ok(IntegrityCheckOutcome::SkippedInProgress),
            Err(other) => return Err(DomainError::from(other)),
        };

        // 2. Spawn the renewal heartbeat. Period is `ttl/3` by
        //    convention so the lease survives one missed tick.
        let renewal = guard.spawn_renewal(self.renew_period);

        // 3. CHECK phase — read-only REPEATABLE READ snapshot. No
        //    fence: a peer steal mid-check is irrelevant because
        //    the read produces an advisory report; only the repair
        //    phase needs fence-in-tx.
        let scope = AccessScope::allow_all();
        let check_started = Instant::now();
        let check_pairs = match run_check_under_lease(&self.repo, &scope).await {
            Ok(pairs) => pairs,
            Err(e) => {
                // Stop heartbeat and free the slot (release_with_retry
                // preserves attempts as a forensic signal that the
                // check failed under this holder).
                renewal.shutdown().await;
                _ = guard.release_with_retry().await;
                return Err(e);
            }
        };
        let check_duration = check_started.elapsed();
        let check_report = bucket_violations(check_pairs);

        // Publish per-category violation gauges. One sample per
        // category in fixed `IntegrityCategory::all` order, including
        // zero values so a category that was non-zero on a previous
        // tick and absent on this one still emits a fresh zero — the
        // dashboard reads `am.hierarchy_integrity_violations` as a
        // gauge, not a sparse map.
        emit_violation_gauges(&check_report);

        // 4. Decide whether to chain repair.
        if !(auto_repair && check_report.has_derivable_violations()) {
            renewal.shutdown().await;
            guard.release().await?;
            return Ok(IntegrityCheckOutcome::CompletedClean {
                check_report,
                check_duration,
            });
        }

        // 5. REPAIR phase — fenced + retried under SERIALIZABLE.
        let scope_for_closure = scope.clone();
        let repair_started = Instant::now();
        let ack: Result<RepairReport, AckError<TxError>> = guard
            .with_ack_in_tx::<_, RepairReport, TxError, _>(TxError::db_err, move |tx| {
                let scope = scope_for_closure.clone();
                Box::pin(async move { apply_repair_in_tx(tx, &scope).await })
            })
            .await;
        let repair_duration = repair_started.elapsed();

        renewal.shutdown().await;

        match ack {
            Ok(repair_report) => {
                emit_repaired_gauges(&repair_report);
                guard.release().await?;
                Ok(IntegrityCheckOutcome::CompletedRepaired {
                    check_report,
                    check_duration,
                    repair_report,
                    repair_duration,
                })
            }
            Err(AckError::LeaseLost) => {
                // DO NOT release — TTL handles it; releasing now
                // would steal the slot back from whoever has it.
                Ok(IntegrityCheckOutcome::AbortedLeaseLost {
                    check_report,
                    check_duration,
                })
            }
            Err(AckError::Work(TxError::Domain(d))) => {
                _ = guard.release_with_retry().await;
                Err(d)
            }
            Err(AckError::Work(TxError::Db(db_err))) => {
                _ = guard.release_with_retry().await;
                Err(classify_db_err_to_domain(db_err))
            }
            Err(AckError::Db(db_err)) => {
                _ = guard.release_with_retry().await;
                Err(DomainError::from(db_err))
            }
        }
    }
}

/// Publish one `AM_HIERARCHY_INTEGRITY_VIOLATIONS` gauge sample
/// per [`IntegrityCategory::all`] in fixed order. Zero-valued
/// categories are emitted explicitly so a category that disappears
/// between ticks resets to `0` on the dashboard instead of holding
/// its previous value.
// @cpt-begin:cpt-cf-account-management-dod-tenant-hierarchy-management-integrity-diagnostics:p2:inst-dod-integrity-diagnostics-coordinator
fn emit_violation_gauges(report: &IntegrityReport) {
    for (cat, viols) in &report.violations_by_category {
        let count = viols.len();
        emit_gauge_value(
            AM_HIERARCHY_INTEGRITY_VIOLATIONS,
            i64::try_from(count).unwrap_or(i64::MAX),
            &[("category", cat.as_str())],
        );
        if count > 0 {
            warn!(
                target: "am.integrity",
                category = cat.as_str(),
                count,
                "hierarchy integrity violations detected"
            );
        }
    }
}

/// Publish one `AM_HIERARCHY_INTEGRITY_REPAIRED` gauge sample per
/// [`IntegrityCategory::all`] in fixed order with the
/// `bucket = repaired | deferred` label. Iterates the full
/// category list (not the report's potentially sparse maps) so a
/// category that was non-zero on a previous tick and absent on this
/// one still emits a fresh zero.
fn emit_repaired_gauges(report: &RepairReport) {
    let repaired_lookup: std::collections::HashMap<IntegrityCategory, usize> =
        report.repaired_per_category.iter().copied().collect();
    let deferred_lookup: std::collections::HashMap<IntegrityCategory, usize> =
        report.deferred_per_category.iter().copied().collect();
    for cat in IntegrityCategory::all() {
        let (bucket, count) = if cat.is_derivable() {
            ("repaired", repaired_lookup.get(&cat).copied().unwrap_or(0))
        } else {
            ("deferred", deferred_lookup.get(&cat).copied().unwrap_or(0))
        };
        emit_gauge_value(
            AM_HIERARCHY_INTEGRITY_REPAIRED,
            i64::try_from(count).unwrap_or(i64::MAX),
            &[("category", cat.as_str()), ("bucket", bucket)],
        );
    }
    if report.total_deferred() > 0 {
        warn!(
            target: "am.integrity",
            deferred_total = report.total_deferred(),
            repaired_total = report.total_repaired(),
            "hierarchy integrity repair deferred non-derivable violations to operator triage"
        );
    }
}
// @cpt-end:cpt-cf-account-management-dod-tenant-hierarchy-management-integrity-diagnostics:p2:inst-dod-integrity-diagnostics-coordinator

/// Rebucket a flat `Vec<(category, violation)>` returned by
/// [`crate::infra::storage::repo_impl::integrity::run_check_under_lease`]
/// into an [`IntegrityReport`] with one entry per
/// [`IntegrityCategory::all`] in fixed order (empty `Vec` for
/// absent categories). The fixed shape keeps dashboards keyed on
/// per-category gauges stable across ticks.
fn bucket_violations(pairs: Vec<(IntegrityCategory, Violation)>) -> IntegrityReport {
    let mut bucketed: HashMap<IntegrityCategory, Vec<Violation>> = HashMap::new();
    for (cat, viol) in pairs {
        bucketed.entry(cat).or_default().push(viol);
    }
    IntegrityReport {
        violations_by_category: IntegrityCategory::all()
            .iter()
            .map(|cat| (*cat, bucketed.remove(cat).unwrap_or_default()))
            .collect(),
    }
}

#[async_trait]
impl IntegrityChecker for IntegrityCoordinator {
    /// Single-phase check accessor: drives a tick with
    /// `auto_repair = false` and returns the check report.
    /// `SkippedInProgress` maps to
    /// [`DomainError::IntegrityCheckInProgress`] so per-phase
    /// callers see the same error code as direct repo callers.
    async fn run_whole_integrity_check(&self) -> Result<IntegrityReport, DomainError> {
        match self.run_tick(false).await? {
            IntegrityCheckOutcome::CompletedClean { check_report, .. }
            | IntegrityCheckOutcome::CompletedRepaired { check_report, .. }
            | IntegrityCheckOutcome::AbortedLeaseLost { check_report, .. } => Ok(check_report),
            IntegrityCheckOutcome::SkippedInProgress => Err(DomainError::IntegrityCheckInProgress),
        }
    }

    /// Single-phase repair accessor: drives a tick with
    /// `auto_repair = true` and returns the repair report. A clean
    /// check (no derivable violations) returns an empty
    /// [`RepairReport`]; an aborted-lease-lost outcome surfaces as
    /// [`DomainError::IntegrityCheckInProgress`].
    async fn run_whole_integrity_repair(&self) -> Result<RepairReport, DomainError> {
        match self.run_tick(true).await? {
            IntegrityCheckOutcome::CompletedRepaired { repair_report, .. } => Ok(repair_report),
            IntegrityCheckOutcome::CompletedClean { .. } => {
                Ok(crate::domain::tenant::integrity::RepairReport::empty())
            }
            IntegrityCheckOutcome::SkippedInProgress
            | IntegrityCheckOutcome::AbortedLeaseLost { .. } => {
                Err(DomainError::IntegrityCheckInProgress)
            }
        }
    }

    /// The unified-lease tick — both phases share a single
    /// [`crate::infra::lease::LeaseGuard`].
    async fn run_tick(&self, auto_repair: bool) -> Result<IntegrityCheckOutcome, DomainError> {
        IntegrityCoordinator::run_tick(self, auto_repair).await
    }
}
