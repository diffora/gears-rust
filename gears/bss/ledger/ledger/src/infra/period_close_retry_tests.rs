//! The close gate's repo reads keep classified contention retryable.

use sea_orm::DbBackend;
use toolkit_db::contention::is_retryable_contention;

use super::{as_db_err, repo_to_db};
use crate::domain::model::RepoError;

#[test]
fn repo_contention_stays_retryable_and_other_failures_do_not() {
    let conflict = repo_to_db(RepoError::Conflict("database contention".into()));
    let extracted = as_db_err(&conflict).expect("a classified DbErr");
    for backend in [DbBackend::Postgres, DbBackend::Sqlite] {
        assert!(
            is_retryable_contention(backend, extracted),
            "{backend:?}: a repo Conflict retries the close transaction"
        );
    }
    for error in [
        RepoError::Db("storage failure".into()),
        RepoError::InvalidStoredMoney("corrupt segment".into()),
    ] {
        assert!(
            as_db_err(&repo_to_db(error)).is_none(),
            "a non-contention repo failure is not retried"
        );
    }
}
