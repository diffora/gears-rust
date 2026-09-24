//! Repositories return in phase 1c; this file keeps the error helper every repository uses.

use crate::infra::storage::RepoError;
use toolkit_db::secure::ScopeError;

/// Preserve the driver error for contention classification.
#[must_use]
pub fn driver_failure(context: String, source: ScopeError) -> RepoError {
    match source {
        ScopeError::Db(source) => RepoError::Driver { context, source },
        other => RepoError::Db(format!("{context}: {other}")),
    }
}
