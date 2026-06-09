//! Hierarchy-integrity check subsystem (Rust-side).
//!
//! Eight pure-Rust classifiers operate on an in-memory
//! [`Snapshot`](snapshot::Snapshot) of `(tenants, tenant_closure)`.
//! The split between transient DB I/O and synchronous classification
//! is the central design invariant:
//!
//! * `integrity/snapshot.rs` — value types + index builder.
//! * `integrity/classifiers/` — eight DB-free classifier functions; the
//!   only crate-internal entry is [`classifiers::run`].
//! * `integrity/loader.rs` — SecureORM-only snapshot loader.
//!
//! Coordination is owned by [`crate::infra::lease`] (`am_leases`
//! table) plus the
//! [`crate::domain::integrity_check::coordinator::IntegrityCoordinator`]
//! that wraps it.
//!
//! The single public entry [`run_classifiers`] consumes a snapshot and
//! returns an
//! [`IntegrityReport`](crate::domain::tenant::integrity::IntegrityReport)
//! with one fixed-order entry per
//! [`IntegrityCategory::all`](crate::domain::tenant::integrity::IntegrityCategory::all).
//! An empty `Vec` for a category means "no violations".

mod classifiers;
pub mod coordinator;
pub mod loader;
pub mod repair;
pub mod snapshot;

use std::collections::HashMap;

use toolkit_db::secure::DbTx;
use toolkit_security::AccessScope;

use crate::domain::error::DomainError;
use crate::domain::tenant::integrity::{IntegrityCategory, IntegrityReport, Violation};

pub use coordinator::{HIERARCHY_INTEGRITY_KEY, IntegrityCoordinator};
pub use snapshot::{ClosureSnap, Snapshot, TenantSnap};

/// Run every classifier and aggregate the results into an
/// [`IntegrityReport`].
#[must_use]
pub fn run_classifiers(snap: &Snapshot) -> IntegrityReport {
    let raw = classifiers::run(snap);

    let mut by_cat: HashMap<IntegrityCategory, Vec<Violation>> = HashMap::new();
    for v in raw {
        by_cat.entry(v.category).or_default().push(v);
    }
    let violations_by_category = IntegrityCategory::all()
        .into_iter()
        .map(|c| (c, by_cat.remove(&c).unwrap_or_default()))
        .collect();
    debug_assert!(
        by_cat.is_empty(),
        "IntegrityCategory::all() missing classifier categories: {:?}",
        by_cat.keys().collect::<Vec<_>>()
    );
    IntegrityReport {
        violations_by_category,
    }
}

/// Snapshot-load → `run_classifiers` on a caller-provided `DbTx<'_>`.
///
/// This entry covers only the snapshot-and-classify phase. Single-flight
/// gating is the call site's responsibility: production callers wrap
/// this in a [`lock::acquire_committed`] / [`lock::release_committed`]
/// pair around a `REPEATABLE READ` transaction — keeping the gate row
/// committed for the duration of the work so concurrent contenders
/// observe the held gate and surface
/// [`DomainError::IntegrityCheckInProgress`] instead of queueing on an
/// uncommitted PK.
///
/// # Errors
///
/// Any DB error from the snapshot SELECTs, funnelled through the
/// canonical `From<DbError> for DomainError` ladder.
pub async fn run_integrity_check(
    tx: &DbTx<'_>,
    scope: &AccessScope,
) -> Result<IntegrityReport, DomainError> {
    let snapshot = loader::load_snapshot(tx, scope).await?;
    Ok(run_classifiers(&snapshot))
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
mod tests {
    use super::*;
    use crate::domain::tenant::model::TenantStatus;
    use uuid::Uuid;

    fn tenant(id: u128, parent: Option<u128>, depth: i32) -> TenantSnap {
        TenantSnap {
            id: Uuid::from_u128(id),
            parent_id: parent.map(Uuid::from_u128),
            status: TenantStatus::Active,
            depth,
            self_managed: false,
        }
    }

    fn edge(a: u128, d: u128) -> ClosureSnap {
        ClosureSnap {
            ancestor_id: Uuid::from_u128(a),
            descendant_id: Uuid::from_u128(d),
            barrier: 0,
            descendant_status: TenantStatus::Active,
        }
    }

    #[test]
    fn empty_snapshot_has_one_entry_per_category() {
        let snap = Snapshot::new(vec![], vec![]);
        let report = run_classifiers(&snap);
        let got_order: Vec<_> = report
            .violations_by_category
            .iter()
            .map(|(category, _)| *category)
            .collect();
        assert_eq!(got_order, IntegrityCategory::all(), "fixed category order");
        assert_eq!(
            report.violations_by_category.len(),
            IntegrityCategory::all().len()
        );
        assert_eq!(report.total(), 0);
    }

    #[test]
    fn clean_tree_has_zero_violations() {
        let snap = Snapshot::new(
            vec![tenant(1, None, 0), tenant(2, Some(1), 1)],
            vec![edge(1, 1), edge(2, 2), edge(1, 2)],
        );
        let report = run_classifiers(&snap);
        assert_eq!(report.total(), 0);
    }
}
