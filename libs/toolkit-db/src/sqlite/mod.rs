//! SQLite-specific helpers and utilities.
//!
//! This gear contains SQLite-specific functionality including:
//! - DSN parsing and cleaning
//! - PRAGMA parameter handling with typed enums
//! - Path preparation for `SQLite` databases

pub mod dsn;
// Crate-internal path helpers. `pub(crate)` is intentional API hygiene even though the parent
// `sqlite` module is private (clippy::redundant_pub_crate would prefer `pub`).
#[allow(clippy::redundant_pub_crate)]
pub(crate) mod path;
pub mod pragmas;

pub use dsn::{extract_sqlite_pragmas, is_memory_dsn};

/// Disables connection expiry, whatever the pool configuration asks for.
///
/// An in-memory database exists only while one of its connections is open. `SQLx` retires a
/// connection past `idle_timeout` or `max_lifetime` and opens a replacement, which would
/// silently start over with an empty database.
pub fn keep_memory_connections(
    options: sqlx::sqlite::SqlitePoolOptions,
) -> sqlx::sqlite::SqlitePoolOptions {
    options.idle_timeout(None).max_lifetime(None)
}
pub use path::prepare_sqlite_path;
pub use pragmas::Pragmas;
