//! The adjustment-flow repository error mapping.

use super::map_adjustment_repo_err;
use crate::domain::error::DomainError;
use crate::domain::model::RepoError;
use bss_ledger_sdk::MoneyError;

#[test]
fn conflicts_retry_caller_mismatches_are_client_errors_the_rest_is_internal() {
    assert!(matches!(
        map_adjustment_repo_err("ctx", RepoError::Conflict("stale version".into())),
        DomainError::ConcurrentModification(_)
    ));
    assert!(matches!(
        map_adjustment_repo_err("ctx", RepoError::Money(MoneyError::CurrencyMismatch)),
        DomainError::CurrencyMismatch(_)
    ));
    assert!(matches!(
        map_adjustment_repo_err("ctx", RepoError::Money(MoneyError::ScaleMismatch)),
        DomainError::InconsistentScale(_)
    ));
    assert!(matches!(
        map_adjustment_repo_err("ctx", RepoError::InvalidStoredMoney("bad".into())),
        DomainError::Internal(_)
    ));
}
