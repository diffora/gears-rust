// Created: 2026-04-16 by Constructor Tech
// @cpt-dod:cpt-cf-resource-group-algo-sdk-foundation-map-domain-error:p1
// @cpt-dod:cpt-cf-resource-group-dod-testing-error-conversions:p2
//! Domain error types for the resource-group module.

use authz_resolver_sdk::pep::EnforcerError;
use resource_group_sdk::ResourceGroupError;
use thiserror::Error;

/// Domain-specific errors for the resource-group module.
#[allow(unknown_lints, de0309_must_have_domain_model)]
#[derive(Error, Debug)]
pub enum DomainError {
    #[error("Type not found: {code}")]
    TypeNotFound { code: String },

    #[error("Type already exists: {code}")]
    TypeAlreadyExists { code: String },

    #[error("Validation failed: {message}")]
    Validation { message: String },

    #[error("Allowed parents violation: {message}")]
    AllowedParentTypesViolation { message: String },

    #[error("Active references exist: {message}")]
    ConflictActiveReferences { message: String },

    #[error("Group not found: {id}")]
    GroupNotFound { id: uuid::Uuid },

    #[error("Membership not found: {key}")]
    MembershipNotFound { key: String },

    #[error("Duplicate membership: {message}")]
    DuplicateMembership { key: String, message: String },

    #[error("Invalid parent type: {message}")]
    InvalidParentType { message: String },

    #[error("Cycle detected: {message}")]
    CycleDetected { message: String },

    #[error("Limit violation: {message}")]
    LimitViolation { message: String },

    #[error("Conflict: {message}")]
    Conflict { message: String },

    /// Second tenant-type root rejected.
    ///
    /// Raised when a `create_group`/`update_group` would leave the RG forest
    /// with more than one root group whose GTS type code starts with
    /// `TENANT_RG_TYPE_PATH`. Enforces
    /// `cpt-cf-resource-group-fr-enforce-tenant-root-uniqueness`. Maps to
    /// canonical `already_exists` (HTTP 409) with `existing_root_id` as the
    /// `resource_name`.
    #[error("Tenant root already exists (id={existing_root_id}): {detail}")]
    TenantRootAlreadyExists {
        existing_root_id: uuid::Uuid,
        detail: String,
    },

    /// Cross-tenant link rejected when adding a membership.
    ///
    /// Raised by `MembershipService::add_membership` when the target group's
    /// tenant differs from the tenant of any existing membership for the same
    /// `(resource_type, resource_id)` pair. A resource must belong to groups
    /// of a single tenant.
    #[error("Tenant incompatibility: {message}")]
    TenantIncompatibility { message: String },

    #[error("Access denied: {message}")]
    AccessDenied { message: String },

    #[error("Database error: {0}")]
    Database(sea_orm::DbErr),

    #[error("Internal error")]
    InternalError,
}

impl DomainError {
    /// Returns the underlying `DbErr` if this is a database failure.
    ///
    /// Used as the extractor for
    /// [`modkit_db::Db::transaction_with_retry`], which feeds the
    /// `DbErr` into [`modkit_db::contention::is_retryable_contention`] for
    /// backend-aware retry decisions (`PostgreSQL` serialization failures /
    /// deadlocks, `MySQL`/`InnoDB` deadlocks, `SQLite` `BUSY`/`BUSY_SNAPSHOT`).
    #[must_use]
    pub fn db_err(&self) -> Option<&sea_orm::DbErr> {
        match self {
            DomainError::Database(err) => Some(err),
            _ => None,
        }
    }

    /// Returns `true` when this error wraps a `PostgreSQL` serialization
    /// failure — SQLSTATE `40001` or the canonical "could not serialize access"
    /// message — caused by concurrent writers under SERIALIZABLE isolation.
    ///
    /// Detection is text-based on the wrapped `DbErr`, so it picks up both the
    /// SQLSTATE code and the human-readable form regardless of which backend
    /// driver formatted the error.
    #[must_use]
    pub fn is_serialization_failure(&self) -> bool {
        let Some(err) = self.db_err() else {
            return false;
        };
        let s = err.to_string();
        s.contains("40001") || s.contains("could not serialize access")
    }

    pub fn type_not_found(code: impl Into<String>) -> Self {
        Self::TypeNotFound { code: code.into() }
    }

    pub fn type_already_exists(code: impl Into<String>) -> Self {
        Self::TypeAlreadyExists { code: code.into() }
    }

    pub fn validation(message: impl Into<String>) -> Self {
        Self::Validation {
            message: message.into(),
        }
    }

    pub fn allowed_parent_types_violation(message: impl Into<String>) -> Self {
        Self::AllowedParentTypesViolation {
            message: message.into(),
        }
    }

    pub fn conflict_active_references(message: impl Into<String>) -> Self {
        Self::ConflictActiveReferences {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn group_not_found(id: uuid::Uuid) -> Self {
        Self::GroupNotFound { id }
    }

    pub fn membership_not_found(key: impl Into<String>) -> Self {
        Self::MembershipNotFound { key: key.into() }
    }

    pub fn duplicate_membership(key: impl Into<String>, message: impl Into<String>) -> Self {
        Self::DuplicateMembership {
            key: key.into(),
            message: message.into(),
        }
    }

    pub fn invalid_parent_type(message: impl Into<String>) -> Self {
        Self::InvalidParentType {
            message: message.into(),
        }
    }

    pub fn cycle_detected(message: impl Into<String>) -> Self {
        Self::CycleDetected {
            message: message.into(),
        }
    }

    pub fn limit_violation(message: impl Into<String>) -> Self {
        Self::LimitViolation {
            message: message.into(),
        }
    }

    pub fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict {
            message: message.into(),
        }
    }

    pub fn tenant_root_already_exists(
        existing_root_id: uuid::Uuid,
        detail: impl Into<String>,
    ) -> Self {
        Self::TenantRootAlreadyExists {
            existing_root_id,
            detail: detail.into(),
        }
    }

    pub fn tenant_incompatibility(message: impl Into<String>) -> Self {
        Self::TenantIncompatibility {
            message: message.into(),
        }
    }

    /// Wrap an arbitrary message as a `DomainError::Database`.
    ///
    /// Used by infra code that produces non-`DbErr` failures (e.g., a row that
    /// the schema guarantees exists is unexpectedly missing). The message is
    /// stored inside `DbErr::Custom`, preserving the typed-`DbErr` invariant
    /// expected by [`Self::db_err`].
    pub fn database(message: impl Into<String>) -> Self {
        Self::Database(sea_orm::DbErr::Custom(message.into()))
    }
}

/// Convert domain errors to SDK errors for public API consumption.
impl From<DomainError> for ResourceGroupError {
    fn from(e: DomainError) -> Self {
        match e {
            DomainError::TypeNotFound { code } => ResourceGroupError::not_found(code),
            DomainError::TypeAlreadyExists { code } => {
                ResourceGroupError::type_already_exists(code)
            }
            DomainError::Validation { message } => ResourceGroupError::validation(message),
            DomainError::InvalidParentType { message } => {
                ResourceGroupError::invalid_parent_type(message)
            }
            DomainError::CycleDetected { message } => ResourceGroupError::cycle_detected(message),
            DomainError::LimitViolation { message } => ResourceGroupError::limit_violation(message),
            DomainError::AllowedParentTypesViolation { message } => {
                ResourceGroupError::allowed_parent_types_violation(message)
            }
            DomainError::ConflictActiveReferences { message } => {
                ResourceGroupError::conflict_active_references(message)
            }
            DomainError::Conflict { message }
            | DomainError::DuplicateMembership { message, .. } => {
                ResourceGroupError::conflict(message)
            }
            DomainError::TenantRootAlreadyExists { detail, .. } => {
                ResourceGroupError::conflict(detail)
            }
            DomainError::GroupNotFound { id } => ResourceGroupError::not_found(id.to_string()),
            DomainError::MembershipNotFound { key } => ResourceGroupError::not_found(key),
            DomainError::TenantIncompatibility { message } => {
                ResourceGroupError::tenant_incompatibility(message)
            }
            DomainError::AccessDenied { .. } => ResourceGroupError::access_denied(),
            DomainError::Database(_) | DomainError::InternalError => ResourceGroupError::internal(),
        }
    }
}

impl From<sea_orm::DbErr> for DomainError {
    fn from(e: sea_orm::DbErr) -> Self {
        DomainError::Database(e)
    }
}

// TODO(DE1302): the non-`Sea` arm collapses `modkit_db::DbError` into a
// `Custom(String)` via `.to_string()`, dropping the source chain. Refactor
// `DomainError::Database` (or add a `Box<dyn Error + Send + Sync>` variant)
// so non-Sea variants can be wrapped without stringification, then remove
// this allow.
#[allow(unknown_lints, de1302_error_from_to_string)]
impl From<modkit_db::DbError> for DomainError {
    fn from(e: modkit_db::DbError) -> Self {
        // Preserve the typed `DbErr` when present (so retry detection via
        // `db_err()` stays accurate); otherwise fall back to a `Custom` wrap.
        match e {
            modkit_db::DbError::Sea(db_err) => DomainError::Database(db_err),
            other => DomainError::database(other.to_string()),
        }
    }
}

impl From<EnforcerError> for DomainError {
    fn from(e: EnforcerError) -> Self {
        match e {
            EnforcerError::Denied { deny_reason } => DomainError::AccessDenied {
                message: deny_reason.map_or_else(
                    || "access denied by PDP".to_owned(),
                    |reason| format!("access denied by PDP: {reason:?}"),
                ),
            },
            // PDP RPC or constraint compilation failures are infrastructure problems,
            // not authorization denials — surface as internal errors.
            EnforcerError::EvaluationFailed(_) | EnforcerError::CompileFailed(_) => {
                DomainError::InternalError
            }
        }
    }
}
