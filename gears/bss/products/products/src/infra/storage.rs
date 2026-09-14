//! `SeaORM` entities, the migration chain, and the repositories that read and
//! write them.

use sea_orm::DbErr;

pub mod entity;
pub mod migrations;
pub mod repo;

/// A storage-layer failure.
///
/// Deliberately small: the repositories translate a driver error into `Db` or
/// [`RepoError::Driver`] and keep exactly the typed refusals the Foundation's
/// own invariants need. A variant per SQL error class would put the database's
/// vocabulary in the gear's error surface, which is the coupling this boundary
/// exists to prevent — `Driver` is not that: it carries `sea-orm`'s one error
/// type opaquely, and nothing in this gear reads inside it except the
/// contention classifier the doors already delegate to.
///
/// # Why one variant keeps the driver error and the other does not
///
/// `Db(String)` renders a failure the moment it is raised, which erases the
/// `sea_orm::DbErr` variant. That erasure is silent and it is load-bearing:
/// `toolkit_db::contention::is_retryable_contention` matches only
/// `DbErr::Exec` and `DbErr::Query`, so a lock-contention failure flattened
/// to a string — and re-wrapped by a door as `DbErr::Custom` — classifies as
/// *not retryable* and reaches the caller as a bare 500 instead of being
/// re-attempted by `Db::transaction_with_retry`. Every statement this gear
/// executes therefore raises [`RepoError::Driver`], which carries the
/// `DbErr` unchanged; `Db(String)` remains for the failures that never held
/// one (a scope refusal that is not a driver error, and this repository's own
/// consistency refusals).
///
/// # Why this type is no longer `PartialEq`
///
/// `sea_orm::DbErr` is `Clone` but not `PartialEq`, so carrying one costs the
/// derive. Nothing compared two `RepoError`s: the assertions in
/// `repo_tests.rs` all match a variant, and the one production reader
/// (`api::rest::repo_error_to_canonical`) renders it. Equality on an error
/// whose payload is a driver's own message would be a comparison of driver
/// text in any case, which is the coupling the type doc above refuses.
#[derive(Debug, Clone, thiserror::Error)]
pub enum RepoError {
    /// An underlying database or scope failure that carries no
    /// `sea_orm::DbErr` — or carries one no caller can act on.
    #[error("products repo db error: {0}")]
    Db(String),

    /// A statement's driver failure, with `sea-orm`'s own error preserved
    /// **unchanged** so a caller can classify it.
    ///
    /// The one caller that does is `Db::transaction_with_retry`'s contention
    /// glue in the REST doors, which hands the inner error to
    /// `toolkit_db::contention::is_retryable_contention`; see the type doc
    /// above for why the erased form cannot be classified. Read it through
    /// [`RepoError::to_db_err`] rather than matching the variant, so a door
    /// that wants "the `DbErr` for this failure, whatever kind it is" gets
    /// one answer for both variants.
    #[error("products repo db error: {context}: {source}")]
    Driver {
        /// What the gear was doing — the same phrase `Db` would have carried,
        /// minus the rendered driver text the `source` now supplies.
        context: String,
        /// `sea-orm`'s error, exactly as the driver raised it.
        #[source]
        source: DbErr,
    },

    /// A stored value could not be interpreted as the type its column is
    /// declared to hold — a `CHECK`-constrained token outside its
    /// enumeration, most commonly. An invariant breach, never a caller
    /// mistake: this is never the right refusal for a request-borne value,
    /// which is the domain layer's problem, and using it for one would be a
    /// 500 plus a false operator alarm.
    ///
    /// Unrelated to the design set's `CorruptRow`-style guard-refusal
    /// probes (`design/01-foundation.md`'s "the guard judges the data,
    /// never the door" section, `design/03-sku-classification.md`): those
    /// probes prove a DB trigger refuses a poisoned **write**. This variant
    /// reports the opposite direction — a **read** of a row the trigger
    /// never prevented from being written, most plausibly a row the
    /// database wrote around the application layer. The shared name is
    /// coincidental, not a shared mechanism.
    #[error("products repo: corrupt stored value: {0}")]
    CorruptRow(String),
}

impl RepoError {
    /// This failure as a `sea_orm::DbErr`, preserving the driver's own
    /// variant when there is one to preserve.
    ///
    /// [`RepoError::Driver`] answers its `source` cloned — the same variant
    /// the driver raised, which is the only form
    /// `toolkit_db::contention::is_retryable_contention` can classify as
    /// retryable. The two string-carrying variants answer `DbErr::Custom`,
    /// which that classifier answers `false` for, deliberately: neither
    /// describes a statement the database asked the caller to retry.
    ///
    /// The `Driver` arm drops `context` from the rendered text. That is the
    /// price of handing on an unmodified error rather than a re-wrapped one:
    /// rebuilding the message inside a fresh `DbErr::Exec` would fabricate a
    /// driver error the driver never raised, and the door logs the
    /// `RepoError` itself — context included — before it reaches here.
    #[must_use]
    pub fn to_db_err(&self) -> DbErr {
        match self {
            Self::Driver { source, .. } => source.clone(),
            Self::Db(_) | Self::CorruptRow(_) => DbErr::Custom(self.to_string()),
        }
    }
}

/// The `DbErr` inside a [`DbError`], for `Db::transaction_with_retry`'s
/// contention classifier — the one piece of glue that helper asks a caller
/// for.
///
/// Only [`DbError::Sea`] can carry one. Every other variant is a
/// configuration or connection-string fault that no retry can clear, and
/// returning `None` for those is what short-circuits the retry loop instead
/// of paying the backoff for a failure that will repeat identically.
pub(crate) fn contention_db_err(error: &toolkit_db::DbError) -> Option<&DbErr> {
    if let toolkit_db::DbError::Sea(err) = error {
        Some(err)
    } else {
        None
    }
}
