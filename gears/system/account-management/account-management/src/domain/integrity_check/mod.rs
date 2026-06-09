//! Periodic hierarchy-integrity check job.
//!
//! Spawned once per platform start by
//! [`crate::gear::AccountManagementGear::serve`] when
//! [`config::IntegrityCheckConfig::enabled`] is `true`. Each tick
//! drives [`crate::infra::storage::integrity::coordinator::IntegrityCoordinator::run_tick`]
//! under the AM-local lease, tolerates `SkippedInProgress` as a no-op
//! outcome, and emits per-tick `RUNS` / `DURATION` / `LAST_SUCCESS`
//! telemetry on top of the per-category
//! [`crate::domain::metrics::AM_HIERARCHY_INTEGRITY_VIOLATIONS`]
//! gauge the coordinator emits during the check phase.
//!
//! Setting `enabled = false` is a clean opt-out — the loop is not
//! spawned at all.

pub mod config;
pub mod service;

pub use config::{IntegrityCheckConfig, IntegrityRepairConfig, LEASE_TTL};
pub use service::{IntegrityCheckOutcome, IntegrityChecker, run_integrity_check_loop};
