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
    #[error("products repo: corrupt stored value: {0}")]
    CorruptRow(String),

    /// The named subject does not exist **or lies outside the caller's
    /// scope**.
    ///
    /// Deliberately the same answer either way. A repository that answered
    /// "forbidden" for a row belonging to another tenant would confirm that
    /// the row exists, which is the existence leak the SQL-level scoping is
    /// there to close: the catalog is commercially sensitive, so absence is
    /// what a foreign scope sees.
    #[error("products repo: {subject} {id} not found")]
    NotFound {
        /// The kind of thing that was looked for (`product`, `sku`).
        subject: String,
        /// The reference the caller supplied.
        id: String,
    },
}
