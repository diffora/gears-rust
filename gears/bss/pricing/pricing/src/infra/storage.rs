//! Persistence layer for the catalog Foundation: the migration chain, the
//! `SeaORM` entities over its tables, and the repositories that read them.
//!
//! Everything here is infrastructure by construction (DE0301): the domain layer
//! never learns that a `Plan` is a row. Where a repository hands back a value
//! the rest of the system reasons about, it maps at this boundary — see
//! [`repo::PinFrontierRepo`], which reads a `pricing_pin_frontier` row and
//! returns the SDK's `PinFrontier`.
//!
//! Every physical table carries the gear-name prefix `pricing_`
//! (`design/01-foundation.md` §3.7) and lives in schema `bss` on Postgres.
//! `SQLite` is the fast non-production test backend: it mirrors the shape
//! (`uuid` -> `text`, `timestamptz` -> `text`, `jsonb` -> `text`) and, unlike
//! the sibling ledger's substrate, it also mirrors the **append-only guards** —
//! `RAISE(ABORT, ...)` triggers stand in for PL/pgSQL `RAISE EXCEPTION`, so the
//! column whitelists are exercised by the fast test suite rather than only by
//! the Docker-gated Postgres one.

pub mod entity;
pub mod migrations;
pub mod repo;

use crate::domain::error::DomainError;

/// A storage-layer failure.
///
/// Deliberately small: the repositories translate a driver error into `Db` and
/// keep exactly the one typed refusal the Foundation's own invariant needs.
/// A variant per SQL error class would put the database's vocabulary in the
/// gear's error surface, which is the coupling this boundary exists to prevent.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RepoError {
    /// Underlying database or scope failure.
    #[error("pricing repo db error: {0}")]
    Db(String),
    /// A stored value could not be interpreted as the type its column is
    /// declared to hold — a `CHECK`-constrained token outside its enumeration,
    /// or a `catalog_version` outside the unsigned range the SDK type carries.
    /// An invariant breach, never a caller mistake.
    #[error("pricing repo: corrupt stored value: {0}")]
    CorruptRow(String),
    /// An attempt to move the pin-eligibility frontier to a version at or below
    /// where it already stands.
    ///
    /// Refused rather than swallowed: the projector advances the frontier only
    /// inside the transaction completing the frontier's **next** version in
    /// order, so an equal-or-lower target means the caller's ordering
    /// assumption has broken (a duplicate completion, or a re-drive running out
    /// of order). Treating it as a no-op would hide exactly the defect the
    /// materialized frontier exists to make impossible — a pin resolving two
    /// different contents over time (D-136).
    #[error(
        "pricing repo: pin frontier for tenant {tenant} stands at {current}, \
         refusing to advance to {requested}"
    )]
    FrontierRegression {
        /// The tenant whose frontier was targeted.
        tenant: String,
        /// Where the frontier stands now.
        current: u64,
        /// The version the caller asked to advance to.
        requested: u64,
    },
}

/// Map a storage failure into the gear's rejection vocabulary.
///
/// A frontier regression is a **precondition** failure, not an internal fault:
/// the row is intact and the store is healthy, the requested transition is
/// simply one the watermark's monotonicity forbids — the same shape as any other
/// refused lifecycle edge, so it lands on the same variant and therefore the
/// same canonical category.
///
/// Deliberately a named function rather than a `From` impl, and it **logs the
/// storage failure before flattening it**. `DomainError` is a `Clone + Eq` value
/// type carried into responses and compared in tests, so it cannot hold a boxed
/// source; the chain would be lost either way, and the log is where an operator
/// gets it back. The sibling ledger converts at its call sites for the same
/// reason.
#[must_use]
pub fn repo_failure(err: &RepoError) -> DomainError {
    match err {
        RepoError::Db(_) | RepoError::CorruptRow(_) => {
            tracing::error!(error = %err, "bss-pricing: storage failure");
            DomainError::Internal(err.to_string())
        }
        RepoError::FrontierRegression { .. } => DomainError::LifecycleForbidden(err.to_string()),
    }
}
