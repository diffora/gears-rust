//! `SeaORM` entities, the migration chain, and the repositories that read and
//! write them.

pub mod entity;
pub mod migrations;
pub mod repo;

/// A storage-layer failure.
///
/// Deliberately small: the repositories translate a driver error into `Db` and
/// keep exactly the typed refusals the Foundation's own invariants need. A
/// variant per SQL error class would put the database's vocabulary in the
/// gear's error surface, which is the coupling this boundary exists to
/// prevent.
#[derive(Debug, Clone, thiserror::Error, PartialEq, Eq)]
pub enum RepoError {
    /// An underlying database or scope failure.
    #[error("products repo db error: {0}")]
    Db(String),

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
