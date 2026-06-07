use authz_resolver_sdk::pep::{AccessRequest, EnforcerError, PolicyEnforcer, ResourceType};
use modkit_security::{AccessScope, SecurityContext, pep_properties};

use crate::domain::error::DomainError;

/// PDP resource type for credstore secrets. The type id comes from the SDK's
/// single source of truth ([`credstore_sdk::SECRET_RESOURCE_TYPE`]).
pub const SECRET: ResourceType = ResourceType::from_static(
    credstore_sdk::SECRET_RESOURCE_TYPE,
    &[pep_properties::OWNER_TENANT_ID],
);

pub mod actions {
    pub const READ: &str = "read";
    pub const WRITE: &str = "write";
    pub const DELETE: &str = "delete";
}

/// Map a PEP enforcement failure to a domain error (fail-closed).
/// `Denied` / `CompileFailed` → `AccessDenied` (403); `EvaluationFailed` → `ServiceUnavailable` (503).
#[must_use]
pub fn map_enforcer_err(err: EnforcerError) -> DomainError {
    match err {
        EnforcerError::Denied { .. } | EnforcerError::CompileFailed(_) => {
            DomainError::AccessDenied {
                cause: Some(Box::new(err)),
            }
        }
        EnforcerError::EvaluationFailed(source) => DomainError::ServiceUnavailable {
            detail: "authorization evaluation failed".to_owned(),
            retry_after: None,
            cause: Some(Box::new(EnforcerError::EvaluationFailed(source))),
        },
    }
}

/// Returns the PDP `AccessScope` for `action` on the caller's tenant.
///
/// # Errors
///
/// Returns `DomainError::AccessDenied` if the PDP denies access or fails to compile constraints.
/// Returns `DomainError::ServiceUnavailable` if the PDP evaluation call fails.
pub async fn scope_for(
    enforcer: &PolicyEnforcer,
    ctx: &SecurityContext,
    action: &str,
) -> Result<AccessScope, DomainError> {
    let tenant = ctx.subject_tenant_id();
    let request = AccessRequest::new()
        .resource_property(pep_properties::OWNER_TENANT_ID, tenant)
        .require_constraints(true);
    enforcer
        .access_scope_with(ctx, &SECRET, action, None, &request)
        .await
        .map_err(map_enforcer_err)
}
