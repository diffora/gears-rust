//! Plan & Price Modeling authorization: PEP resource-type descriptors, action
//! names, the shared [`access_scope`] gate every ctx-bearing service path calls
//! before touching the repository, and the authz-label stub type-schemas
//! registered at gear init so RBAC role-definitions can target these labels.
//!
//! The catalog is normative in `design/05-governance.md`
//! (`cpt-cf-bss-pricing-algo-authz-catalog`); this module is its executable
//! form. Nine object-named labels, all **OUTSIDE** `gts.cf.resources.*` —
//! pricing data is commercially sensitive, so the built-in Reader / Contributor
//! / Owner roles do NOT auto-cover it and access requires explicit catalog
//! roles. Each action sits on its real object (a noun), never an authz tier.
//!
//! Two separations in the label set are load-bearing rather than tidy:
//! - `approval_policy` is its own resource, not a `config` action — segregation
//!   of duties, so a config admin cannot weaken the approval thresholds it
//!   operates under (the sibling ledger's `dual_control_policy` precedent).
//! - `customer_group` is its own resource, not part of `plan` — per-payer
//!   membership is payer-level commercial data, more sensitive than plan
//!   authoring; and `audit` is its own resource so a forensic role carries no
//!   read of live pricing and no write authority anywhere.
//!
//! The PEP advertises NO tenant-subtree capability (`PolicyEnforcer::new`), so
//! the PDP pre-expands the caller's subtree to a flat `AccessScope::In([...])`
//! that SecureORM binds to the `tenant_id` column.

use authz_resolver_sdk::PolicyEnforcer;
use authz_resolver_sdk::pep::{AccessRequest, ResourceType};
use toolkit_security::{AccessScope, SecurityContext, pep_properties};
use uuid::Uuid;

/// Authz `resource_type` label strings (the PDP-visible glob targets).
///
/// Plain `&'static str` consts so the [`resource_types`] descriptors used at
/// enforcement time and the GTS permission catalog (`crate::gts::permissions`)
/// share one source of truth; a drift test enforces that they agree.
pub mod labels {
    use toolkit_gts::gts_id;

    /// Plans, price rows and the primitives attached to a row — the authoring
    /// data plane (`write`, `publish`, `retire`, `migrate`, `read`, `preview`).
    /// Also the label the published read model and the pin frontier are read
    /// under by consumer service identities.
    pub const PLAN: &str = gts_id!("cf.bss.pricing.plan.v1~");
    /// Bundle composition and rev-share (`write`, `read`). Bundle *publish* is
    /// deliberately `plan × publish` (D-11): the composition is authored under
    /// `bundle × write` and pinned by the approval content hash at publish.
    pub const BUNDLE: &str = gts_id!("cf.bss.pricing.bundle.v1~");
    /// `PriceOverlays` of every scope (`write`, `read`). Overlay mutations are
    /// always material (D-50), so the approving role needs this `read` — the
    /// reviewability invariant (D-61).
    pub const PRICE_OVERLAY: &str = gts_id!("cf.bss.pricing.price_overlay.v1~");
    /// Customer-group taxonomy and per-payer membership (`write`, `read`). Its
    /// OWN resource: payer-level commercial data is more sensitive than plan
    /// authoring, so it is never covered by a plan grant.
    pub const CUSTOMER_GROUP: &str = gts_id!("cf.bss.pricing.customer_group.v1~");
    /// Approval decisions (`approve`, `read`). `preparer != approver` is
    /// enforced server-side even where a custom role grants both this and
    /// `plan × publish`.
    pub const APPROVAL: &str = gts_id!("cf.bss.pricing.approval.v1~");
    /// The tenant approval-threshold policy (`write`, `read`) — a SEPARATE
    /// resource from `config` by segregation of duties, and its own mutation is
    /// itself two-person (D-10).
    pub const APPROVAL_POLICY: &str = gts_id!("cf.bss.pricing.approval_policy.v1~");
    /// The tenant config plane (`write`, `read`): tax-display policy and the
    /// region / brand / partner / orgTier taxonomies (D-120). The
    /// customer-group taxonomy is deliberately NOT here.
    pub const CONFIG: &str = gts_id!("cf.bss.pricing.config.v1~");
    /// Governed backdated reference import (`write`, `read`) — its OWN resource
    /// so the restricted backdate grant is targetable without any other
    /// authority, and so the second person can read the row set they approve
    /// (D-61).
    pub const HISTORICAL_IMPORT: &str = gts_id!("cf.bss.pricing.historical_import.v1~");
    /// Audit trail read and export (`read`, `export`) — its OWN resource so an
    /// auditor role carries no read of live pricing and no write authority.
    ///
    /// **It covers the price history too**, and this paragraph said the opposite
    /// until the export arrived: "Finance's chronological price history is the
    /// separate `plan × read` surface (D-12)" was D-12's original reading and
    /// stopped being true of `GET /bss-pricing/v1/history` when that route was
    /// recatalogued on 2026-08-14 — it *is* the catalog audit trail, so filing it
    /// under catalog read handed "who changed what, when" to every holder of
    /// `plan × read`. `POST /bss-pricing/v1/history/export` follows it under
    /// `export` (`inst-he-export`), which is what that action was declared for:
    /// bulk extraction of a seven-year actor trail, grantable separately from
    /// reading it. The design set's own endpoint tables still carry the withdrawn
    /// reading (S12 §5, S5 §5) and are owed the correction.
    pub const AUDIT: &str = gts_id!("cf.bss.pricing.audit.v1~");

    /// Every authz label, stable order. The single canonical list driving the
    /// per-label stub type-schema registration (see
    /// [`super::authz_label_type_schemas`]) that lets RBAC role-definitions
    /// target any pricing label. MUST match the permission catalog's distinct
    /// `resource_type`s (`crate::gts::permissions`); a drift test enforces it.
    pub const ALL: &[&str] = &[
        PLAN,
        BUNDLE,
        PRICE_OVERLAY,
        CUSTOMER_GROUP,
        APPROVAL,
        APPROVAL_POLICY,
        CONFIG,
        HISTORICAL_IMPORT,
        AUDIT,
    ];
}

/// PEP action names for the catalog surfaces.
pub mod actions {
    /// Author draft state: create / update / clone a plan or price row, delete a
    /// never-published draft, run a cutover or supersession, tighten
    /// `grandfatherUntil`, schedule a window. Also the bulk plane — bulk is
    /// authoring at scale and carries no new authority. Used by `plan`,
    /// `bundle`, `price_overlay`, `customer_group`, `approval_policy`,
    /// `config` and `historical_import`; the resource scopes what it authorizes.
    pub const WRITE: &str = "write";
    /// Submit a plan (or a bundle's composition) for publish. Distinct from
    /// [`WRITE`] so an author can prepare a change without being able to put it
    /// in front of customers.
    pub const PUBLISH: &str = "publish";
    /// Retire a plan. Its own action because retirement is a publish unit with
    /// no way back (D-128): a retired plan can never publish again.
    pub const RETIRE: &str = "retire";
    /// Schedule or cancel a plan migration, and the Subscriptions execution
    /// handshake. Distinct from [`RETIRE`] so moving subscribers between plans
    /// is grantable without the authority to end a plan's life.
    pub const MIGRATE: &str = "migrate";
    /// Read action — authoring reads (including drafts), the published read
    /// model, the pin frontier, coverage / sellability, price history and the
    /// `migrated-origin` snapshot. The resource scopes what the read
    /// authorizes.
    pub const READ: &str = "read";
    /// The partner-facing base-price preview grant, evaluated against the
    /// grant's explicit pricing-region set. Distinct from [`READ`] so a partner
    /// never holds an authoring read.
    pub const PREVIEW: &str = "preview";
    /// Approve or reject an approval record. Distinct from [`PUBLISH`] so no
    /// default role holds both, on top of the server-side
    /// `submitter != approver` rule.
    pub const APPROVE: &str = "approve";
    /// Export the audit trail. Distinct from [`READ`] so bulk extraction of a
    /// seven-year actor trail is grantable separately from reading it.
    pub const EXPORT: &str = "export";
}

/// Properties the PEP may compile from PDP constraints for catalog rows. Every
/// row is tenant-owned: `owner_tenant_id` is the tenant column the secure-ORM
/// filter binds to, `id` the row PK for single-row gates. NO subtree/group
/// property — the PDP pre-expands the subtree to a flat `In`.
pub const SUPPORTED_PROPERTIES: &[&str] =
    &[pep_properties::OWNER_TENANT_ID, pep_properties::RESOURCE_ID];

/// PEP resource-type descriptors (one `const` per authz label).
pub mod resource_types {
    use super::{ResourceType, SUPPORTED_PROPERTIES, labels};

    /// Plans, price rows and row-attached primitives — the authoring data plane.
    pub const PLAN: ResourceType = ResourceType::from_static(labels::PLAN, SUPPORTED_PROPERTIES);
    /// Bundle composition and rev-share.
    pub const BUNDLE: ResourceType =
        ResourceType::from_static(labels::BUNDLE, SUPPORTED_PROPERTIES);
    /// `PriceOverlays` of every scope.
    pub const PRICE_OVERLAY: ResourceType =
        ResourceType::from_static(labels::PRICE_OVERLAY, SUPPORTED_PROPERTIES);
    /// Customer-group taxonomy and per-payer membership.
    pub const CUSTOMER_GROUP: ResourceType =
        ResourceType::from_static(labels::CUSTOMER_GROUP, SUPPORTED_PROPERTIES);
    /// Approval decisions.
    pub const APPROVAL: ResourceType =
        ResourceType::from_static(labels::APPROVAL, SUPPORTED_PROPERTIES);
    /// The tenant approval-threshold policy.
    pub const APPROVAL_POLICY: ResourceType =
        ResourceType::from_static(labels::APPROVAL_POLICY, SUPPORTED_PROPERTIES);
    /// The tenant config plane.
    pub const CONFIG: ResourceType =
        ResourceType::from_static(labels::CONFIG, SUPPORTED_PROPERTIES);
    /// Governed backdated reference import.
    pub const HISTORICAL_IMPORT: ResourceType =
        ResourceType::from_static(labels::HISTORICAL_IMPORT, SUPPORTED_PROPERTIES);
    /// Audit trail read and export.
    pub const AUDIT: ResourceType = ResourceType::from_static(labels::AUDIT, SUPPORTED_PROPERTIES);
}

/// Error from the catalog authz gate.
#[derive(Debug, thiserror::Error)]
pub enum AuthzError {
    /// The PDP explicitly denied access (or returned uncompilable constraints).
    #[error("permission denied: {0}")]
    Denied(String),
    /// The PDP was unreachable or its response could not be compiled.
    #[error("authz unavailable: {0}")]
    Unavailable(String),
}

/// Minimal, deterministic type-schema body for an authz label. Key order is
/// fixed by construction, so a re-registration is byte-identical — the registry
/// accepts identical duplicates and does not validate body richness.
fn authz_type_schema_json(gts_id: &str, title: &str) -> serde_json::Value {
    serde_json::json!({
        "$id": format!("gts://{gts_id}"),
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": title,
        "type": "object",
    })
}

/// Stub type-schemas for every authz label ([`labels::ALL`]). The platform RBAC
/// role-definition validator resolves a rule's `target_type` through the
/// types-registry, so registering these at gear init lets a custom catalog role
/// target any pricing authz label.
#[must_use]
pub fn authz_label_type_schemas() -> Vec<serde_json::Value> {
    labels::ALL
        .iter()
        .map(|label| {
            authz_type_schema_json(
                label,
                &format!("BSS Plan & Price Modeling authz label {label}"),
            )
        })
        .collect()
}

/// Shared PEP gate: asks the PDP whether `(resource_type, action)` is permitted
/// for `ctx`, returning the caller's compiled [`AccessScope`]. `resource_id`
/// pins a single-row op (`None` for collections).
///
/// `owner_tenant_id` is an optional `OWNER_TENANT_ID` resource-property hint
/// describing the *resource's* owning tenant:
/// - **Reads** pass `None` — the PDP derives the scope from the subject + role,
///   never from a caller-supplied tenant; the returned scope is the SQL filter.
/// - **Writes** pass `Some(target_tenant)` — the tenant the row is written to.
///   This is NOT self-validating at the PDP: the degraded flat-`In` decision
///   does not re-check `owner_tenant_id`, so this fn asserts `target_tenant` is
///   a member of the compiled scope and denies a cross-tenant target.
///
/// `require_constraints` must be `true` on every authorizing path — reads (so
/// the scope is a real SQL filter and an unconstrained *allow* fail-closes
/// instead of exposing every tenant's price book) and writes (so the
/// target-membership assertion has a constraint to test).
///
/// # Errors
///
/// [`AuthzError::Denied`] when the PDP denies or returns uncompilable
/// constraints; [`AuthzError::Unavailable`] when the PDP is unreachable.
pub async fn access_scope(
    enforcer: &PolicyEnforcer,
    ctx: &SecurityContext,
    rt: &ResourceType,
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
        .access_scope_with(ctx, rt, action, resource_id, &request)
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
    // true`: the degraded flat-`In` PDP decision does NOT re-validate
    // `owner_tenant_id`, so assert the target is a member of the compiled scope
    // here — a target outside the caller's authorized tenants is a cross-tenant
    // write and is denied. Reads pass `owner_tenant_id = None` and use the scope
    // as the SQL filter, so this membership check is write-only.
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

#[cfg(test)]
#[path = "authz_tests.rs"]
mod authz_tests;
