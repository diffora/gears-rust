//! Helpers shared across the `repo_impl` split.

use modkit_db::secure::ScopeError;

use crate::domain::error::DomainError;
use crate::infra::canonical_mapping::classify_db_err_to_domain;

/// Map a [`ScopeError`] to a [`DomainError`] outside a retry boundary.
pub(super) fn map_scope_err(err: ScopeError) -> DomainError {
    match err {
        ScopeError::Db(db) => classify_db_err_to_domain(db),
        ScopeError::Invalid(msg) => DomainError::Internal {
            diagnostic: format!("scope invalid: {msg}"),
            cause: None,
        },
        ScopeError::TenantNotInScope { .. } => DomainError::AccessDenied { cause: None },
        ScopeError::Denied(msg) => DomainError::Internal {
            diagnostic: format!("unexpected access denied in credstore repo: {msg}"),
            cause: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use modkit_db::secure::ScopeError;
    use sea_orm::DbErr;
    use uuid::Uuid;

    use super::map_scope_err;
    use crate::domain::error::DomainError;

    #[test]
    fn maps_each_scope_error_variant() {
        assert!(matches!(
            map_scope_err(ScopeError::Invalid("bad scope")),
            DomainError::Internal { .. }
        ));
        assert!(matches!(
            map_scope_err(ScopeError::TenantNotInScope {
                tenant_id: Uuid::new_v4()
            }),
            DomainError::AccessDenied { .. }
        ));
        assert!(matches!(
            map_scope_err(ScopeError::Denied("not accessible")),
            DomainError::Internal { .. }
        ));
        // Db errors delegate to the classification ladder.
        assert!(matches!(
            map_scope_err(ScopeError::Db(DbErr::Custom(
                "CHECK constraint failed".to_owned()
            ))),
            DomainError::InvalidSecretRef { .. }
        ));
    }
}
