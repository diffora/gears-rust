//! Error types for the AM-local lease primitive.

use sea_orm::DbErr;

use crate::domain::error::DomainError;

/// Result of an acquire / renew / release call against a lease row.
///
/// `LeaseHeld` is the per-design "another worker already holds this
/// gate" outcome surfaced by the steal path; `LeaseLost` is the
/// "you used to hold it but a peer stole it" outcome surfaced by
/// `renew` and by `with_ack_in_tx`'s fence SELECT.
#[derive(Debug, thiserror::Error)]
pub enum CoordError {
    #[error("lease already held by another worker")]
    LeaseHeld,
    #[error("lease was lost (taken over) before this call could complete")]
    LeaseLost,
    #[error(transparent)]
    Db(#[from] toolkit_db::DbError),
}

impl CoordError {
    /// Extract the underlying `DbErr` for the contention-retry helper.
    ///
    /// Used as the `extract_db_err` accessor passed into
    /// [`toolkit_db::Db::transaction_with_retry`] inside the acquire path
    /// (and in tests). Returns `None` for non-DB variants so the retry
    /// loop short-circuits on `LeaseHeld` / `LeaseLost`.
    #[must_use]
    pub fn db_err(&self) -> Option<&DbErr> {
        match self {
            Self::Db(toolkit_db::DbError::Sea(e)) => Some(e),
            _ => None,
        }
    }
}

impl From<CoordError> for DomainError {
    /// Lift a coordinator-level lease error into the domain taxonomy.
    ///
    /// * `LeaseHeld` → [`DomainError::IntegrityCheckInProgress`] —
    ///   a contender sees this code when a peer already holds the
    ///   lease; same shape REST/SDK callers receive.
    /// * `LeaseLost` → [`DomainError::IntegrityCheckLeaseLost`] —
    ///   distinct code so dashboards can split mid-flight takeover
    ///   from "peer already running" without overloading either.
    /// * `Db(_)` → routes through the canonical
    ///   [`From<toolkit_db::DbError> for DomainError`] ladder.
    fn from(err: CoordError) -> Self {
        match err {
            CoordError::LeaseHeld => DomainError::IntegrityCheckInProgress,
            CoordError::LeaseLost => DomainError::IntegrityCheckLeaseLost,
            CoordError::Db(db) => DomainError::from(db),
        }
    }
}

/// Outcome envelope for [`crate::infra::lease::LeaseGuard::with_ack_in_tx`].
///
/// `E` is the caller-defined work-error type; the caller hands the
/// guard a `Fn(&E) -> Option<&DbErr>` extractor so the retry helper
/// can decide whether a `Work(_)` failure is retryable contention or
/// a hard failure.
///
/// `LeaseLost` is **never** retried — re-running under a stolen
/// lease cannot succeed and would commit work against the new
/// holder's slot.
#[derive(Debug, thiserror::Error)]
pub enum AckError<E> {
    #[error("lease was lost before the fenced commit could complete")]
    LeaseLost,
    #[error(transparent)]
    Work(E),
    #[error(transparent)]
    Db(#[from] toolkit_db::DbError),
}

impl<E> AckError<E> {
    /// Extract the underlying `DbErr` from any variant for the
    /// contention-retry helper. The caller-supplied
    /// `extract_work_db_err` drills into the `Work(E)` arm; this
    /// method composes that accessor with the built-in `Db(_)` arm
    /// so [`toolkit_db::Db::transaction_with_retry`] sees a single
    /// `Fn(&AckError<E>) -> Option<&DbErr>`.
    pub fn db_err<'a, X>(&'a self, extract_work_db_err: &X) -> Option<&'a DbErr>
    where
        X: Fn(&E) -> Option<&DbErr>,
    {
        match self {
            Self::Db(toolkit_db::DbError::Sea(e)) => Some(e),
            Self::Db(_) | Self::LeaseLost => None,
            Self::Work(w) => extract_work_db_err(w),
        }
    }
}
