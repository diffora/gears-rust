//! SKU, category and approval-unit authorization catalog and shared PEP gate.
use authz_resolver_sdk::PolicyEnforcer;
use authz_resolver_sdk::pep::{AccessRequest, ResourceType};
use toolkit_security::{AccessScope, SecurityContext, pep_properties};
use uuid::Uuid;
/// Concrete PDP-visible resource labels.
pub mod labels {
    use toolkit_gts::gts_id;
    pub const SKU: &str = gts_id!("cf.bss.products.sku.v1~");
    pub const CATEGORY: &str = gts_id!("cf.bss.products.category.v1~");
    pub const APPROVAL_UNIT: &str = gts_id!("cf.bss.products.approval_unit.v1~");
    pub const ALL: &[&str] = &[SKU, CATEGORY, APPROVAL_UNIT];
}
/// Independent grants; reference is reserved for the gear-to-gear registry protocol.
pub mod actions {
    pub const READ: &str = "read";
    pub const AUTHOR: &str = "author";
    pub const SUBMIT: &str = "submit";
    pub const APPROVE: &str = "approve";
    pub const SETTINGS: &str = "settings";
    pub const REFERENCE: &str = "reference";
    pub const ALL: &[&str] = &[READ, AUTHOR, SUBMIT, APPROVE, SETTINGS, REFERENCE];
}
/// Every resource is scoped by its owning tenant and optional row identity.
pub const SUPPORTED_PROPERTIES: &[&str] =
    &[pep_properties::OWNER_TENANT_ID, pep_properties::RESOURCE_ID];
/// Resource descriptors bind PDP constraints to the supported properties.
pub mod resource_types {
    use super::{ResourceType, SUPPORTED_PROPERTIES, labels};
    pub const SKU: ResourceType = ResourceType::from_static(labels::SKU, SUPPORTED_PROPERTIES);
    pub const CATEGORY: ResourceType =
        ResourceType::from_static(labels::CATEGORY, SUPPORTED_PROPERTIES);
    pub const APPROVAL_UNIT: ResourceType =
        ResourceType::from_static(labels::APPROVAL_UNIT, SUPPORTED_PROPERTIES);
}
/// Error from the registry's PEP gate.
///
/// Deliberately **not** folded into [`crate::domain::error::DomainError`]:
/// neither of that enum's authorization-adjacent variants is the right home.
/// `ScopeNotContained` names a business rule over restriction containment
/// (P-D-39, a child scope proven against its parent's), and `ApprovalRequired`
/// names governance's approval-record presence (P-D-23) — both are domain
/// judgements a door reaches *after* it is authorized. A PDP deny or an
/// unreachable PDP happens *before* the domain is consulted at all, so it
/// answers with its own two-way split (403 vs 503), the same way the ledger
/// gear's `AuthzError` does, rather than borrowing a `DomainError` code that
/// would misdescribe why the door refused.
#[derive(Debug, thiserror::Error)]
pub enum AuthzError {
    /// The PDP explicitly denied access, or returned constraints this PEP
    /// could not compile (`authz_resolver_sdk::EnforcerError::Denied` and
    /// `CompileFailed` both land here — an uncompilable *allow* is refused
    /// exactly like an explicit deny, never treated as an unconstrained one).
    #[error("permission denied: {0}")]
    Denied(String),
    /// The PDP evaluation call itself failed — the resolver is unreachable or
    /// erroring, not exercising a business judgement
    /// (`authz_resolver_sdk::EnforcerError::EvaluationFailed`).
    #[error("authz unavailable: {0}")]
    Unavailable(String),
}

/// Shared PEP gate: asks the PDP whether `(resource_type, action)` is
/// permitted for `ctx`, returning the caller's compiled `AccessScope`.
/// `resource_id` pins a single-row op (`None` for collections).
///
/// `owner_tenant_id` is an optional `OWNER_TENANT_ID` resource-property hint
/// describing the *resource's* owning tenant:
/// - **Reads** pass `None` — the PDP derives the scope from the subject +
///   role, never from a caller-supplied tenant; the returned scope is the SQL
///   filter.
/// - **Writes** pass `Some(target_tenant)` — the tenant the row is written
///   to. This is NOT self-validating at the PDP: a degraded flat-`In`
///   decision does not re-check `owner_tenant_id`, so this fn asserts
///   `target_tenant` is a member of the compiled scope and denies a
///   cross-tenant target.
///
/// `require_constraints` should be `true` on every authorizing door path —
/// reads (so the scope is a real SQL filter and an unconstrained *allow*
/// fail-closes instead of leaking every tenant) and writes (so the
/// target-membership assertion above has a constraint to test).
///
/// # Errors
///
/// [`AuthzError::Denied`] when the PDP denies or returns uncompilable
/// constraints; [`AuthzError::Unavailable`] when the PDP is unreachable.
pub async fn access_scope(
    enforcer: &PolicyEnforcer,
    ctx: &SecurityContext,
    resource: &ResourceType,
    action: &str,
    owner_tenant_id: Option<Uuid>,
    resource_id: Option<Uuid>,
    require_constraints: bool,
) -> Result<AccessScope, AuthzError> {
    let mut request = AccessRequest::new().require_constraints(require_constraints);
    if let Some(tenant) = owner_tenant_id {
        request = request.resource_property(pep_properties::OWNER_TENANT_ID, tenant);
    }
    if let Some(rid) = resource_id {
        request = request.resource_property(pep_properties::RESOURCE_ID, rid);
    }

    let scope = enforcer
        .access_scope_with(ctx, resource, action, resource_id, &request)
        .await
        .map_err(|e| match e {
            authz_resolver_sdk::EnforcerError::Denied { .. }
            | authz_resolver_sdk::EnforcerError::CompileFailed(_) => {
                AuthzError::Denied(e.to_string())
            }
            authz_resolver_sdk::EnforcerError::EvaluationFailed(_) => {
                AuthzError::Unavailable(e.to_string())
            }
        })?;

    // Write paths anchor to a target tenant and pass `require_constraints =
    // true`: a degraded flat-`In` PDP decision does NOT re-validate
    // `owner_tenant_id`, so assert the target is a member of the compiled
    // scope here — a target outside the caller's authorized tenants is a
    // cross-tenant write and is denied. Reads pass `owner_tenant_id = None`
    // and use the scope as the SQL filter, so this membership check is
    // write-only.
    if let Some(target) = owner_tenant_id
        && require_constraints
        && !scope.contains_uuid(pep_properties::OWNER_TENANT_ID, target)
    {
        return Err(AuthzError::Denied(format!(
            "subject not authorized to write resources owned by tenant {target}"
        )));
    }
    Ok(scope)
}

fn authz_type_schema_json(gts_id: &str, title: &str) -> serde_json::Value {
    serde_json::json!({
        "$id": format!("gts://{gts_id}"),
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": title,
        "type": "object",
    })
}

/// Stub type-schemas for every authz label ([`labels::ALL`]). The platform
/// RBAC role-definition validator resolves a rule's `target_type` through the
/// types-registry, so registering these lets a custom catalog role target
/// this gear's authz labels.
///
/// **Registered from `Gear::init`** (P-D-134, 2026-09-04): a refused
/// registration fails the boot, as in the sibling pricing gear.
#[must_use]
pub fn authz_label_type_schemas() -> Vec<serde_json::Value> {
    labels::ALL
        .iter()
        .map(|label| authz_type_schema_json(label, &format!("BSS Products authz label {label}")))
        .collect()
}

#[cfg(test)]
#[path = "authz_tests.rs"]
mod authz_tests;
