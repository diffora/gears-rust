//! AM-local distributed-lease primitive.
//!
//! Three properties define the surface:
//!
//! 1. **Fence-in-tx.** [`LeaseGuard::with_ack_in_tx`] runs the caller's
//!    work and an `am_leases`-row SELECT inside a single
//!    `SERIALIZABLE` transaction (with retry). A peer steal between
//!    the work and the commit either aborts as `40001` and retries
//!    (and re-validates the lease), or is caught by the fence
//!    SELECT and surfaces as [`AckError::LeaseLost`] → rollback. The
//!    holder's writes and the lease validation cannot drift apart.
//!
//! 2. **Renewal heartbeat with explicit lease-loss signal.**
//!    [`LeaseGuard::spawn_renewal`] drives the lease's `locked_until`
//!    forward every `period`; a UPDATE returning zero rows surfaces
//!    as [`RenewalState::Lost`] on a `watch::Receiver`, so the holder
//!    can pre-empt itself instead of finishing a write under a stolen
//!    lease.
//!
//! 3. **Forensic `attempts` counter.** Each steal increments
//!    `am_leases.attempts`; `release` resets it, `release_with_retry`
//!    preserves it. Repeat crash-takeover patterns are observable.
//!
//! Layering: depends on `toolkit_db::secure` + `time` + `tokio` +
//! `tokio_util::sync::CancellationToken`. Scope deliberately
//! AM-local — the only consumer is the hierarchy-integrity
//! coordinator.

pub mod error;
pub mod guard;
pub mod manager;

pub use error::{AckError, CoordError};
pub use guard::{LeaseGuard, RenewalHandle, RenewalState};
pub use manager::LeaseManager;
