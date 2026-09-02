//! Product & SKU Registry authorization: PEP resource-type labels, action
//! names and the authz-label stub type-schemas that let RBAC role-definitions
//! target this gear's authz labels.
//!
//! The catalog is normative in `design/05-governance.md` §3.2 (the RBAC
//! catalog table) and `design/01-foundation.md` §2 (the door grants); this
//! module is their executable form for the Foundation's two entities. Only
//! `product` and `sku`, and only `read`/`write`/`publish`, are declared here —
//! the roster the Foundation's own doors name. The wider governance catalog
//! (`category`, `attribute_definition`, `approval`, `audit`, `breakglass` and
//! the rest) belongs to the slices that build those doors and is not declared
//! by this one.
//!
//! **`discard` is deliberately not a label action of its own.** `01
//! §2` narrates `POST /bss-products/v1/{products|skus}/{id}/discard` under
//! `… × discard`, but `05-governance.md` §3.2's own RBAC catalog rows the same
//! door under `product × write` / `sku × write`, and the document's own
//! open-items list records the contradiction as unresolved — "does the
//! discard door get its own grant, or inherit `product|sku × write`?" — with
//! the decision owned by that slice. Minting a `discard` permission here would
//! take one side of a question the design set has not settled; `write` is
//! what the normative catalog table currently grants the door, so that is what
//! this module declares.
//!
//! **Registering [`authz_label_type_schemas`] is still owed**, separately from
//! the gate this module now provides: the sibling pricing gear calls its
//! equivalent from `Gear::init` so the platform's RBAC role-definition
//! validator can resolve a rule's `target_type` against these labels. This
//! gear's `init` (`crate::gear::BssProductsGear::init`) does not call this
//! function yet; wiring it in is owed to the slice that adds the first
//! authoring door.
//!
//! [`access_scope`] is the shared PEP gate every future authoring door calls
//! before touching a repository: it wraps
//! `authz_resolver_sdk::PolicyEnforcer::access_scope_with` the way the
//! sibling ledger gear's `access_scope` does (`gears/bss/ledger/ledger/src/authz.rs`,
//! the smaller of the two donor shapes; pricing's own copy at
//! `gears/bss/pricing/pricing/src/authz.rs` confirms the shape is house
//! style, not one gear's quirk). No door calls it yet — there are no doors in
//! this slice — but the function is exercised directly by
//! `authz_tests.rs` against a fake `AuthZResolverClient`, the same technique
//! the ledger gear's own test suite uses, so the permit/deny path is proven
//! without a live resolver.

use authz_resolver_sdk::PolicyEnforcer;
use authz_resolver_sdk::pep::{AccessRequest, ResourceType};
use toolkit_security::{AccessScope, SecurityContext, pep_properties};
use uuid::Uuid;

/// Authz `resource_type` label strings (the PDP-visible glob targets).
///
/// Plain `&'static str` consts so the GTS permission catalog
/// (`crate::gts::permissions`) and, once Phase 4 wires the PEP calls, the
/// enforcement path share one source of truth.
pub mod labels {
    use toolkit_gts::gts_id;

    /// Products — the authoring data plane for the `Product` entity
    /// (`read`, `write`, `publish`).
    pub const PRODUCT: &str = gts_id!("cf.bss.products.product.v1~");
    /// SKUs — the authoring data plane for the `SKU` entity (`read`, `write`,
    /// `publish`).
    pub const SKU: &str = gts_id!("cf.bss.products.sku.v1~");
    /// The catalog version — the demand and freeze plane
    /// (`request` today; `05-governance` §3.2 declares more actions on this
    /// resource, each arriving with its own door — `dod-cv-authz`).
    pub const CATALOG_VERSION: &str = gts_id!("cf.bss.products.catalog_version.v1~");
    /// Bulk import and promotion — `execute` for the import door,
    /// `read` for the `RowLedger` reader (`05-governance` §3.2 rows).
    /// `bulk_lifecycle` is a **separate** label, deliberately: the gear's
    /// most destructive batch act must not be reachable with the import
    /// pair, and it arrives with its own door.
    pub const BULK: &str = gts_id!("cf.bss.products.bulk.v1~");
    /// The reference signal — `post`, the watermark door's own pair
    /// (`design/07` §2). A label of its own rather than an action on
    /// `sku`: the subject is a producer's complete set, not one entity.
    pub const REFERENCE_SIGNAL: &str = gts_id!("cf.bss.products.reference_signal.v1~");
    /// The reference-producer registry — `write`, the membership ops'
    /// governed pair.
    pub const REFERENCE_PRODUCER: &str = gts_id!("cf.bss.products.reference_producer.v1~");

    /// The approval record — `submit`, `read`, `decide`
    /// (`design/05-governance.md` §3.2, the rows this slice owns). Three
    /// actions on one resource rather than three resources: the subject of
    /// all three is the same record, and a tenant that may read the pending
    /// queue is not thereby able to decide on it.
    pub const APPROVAL: &str = gts_id!("cf.bss.products.approval.v1~");
    /// The materiality policy — `write`, and **its own resource on purpose**:
    /// §3.1 step 2 makes the policy object a `GovernedLiveOp` subject on this
    /// pair *"never a config-admin's general grant, so that the holder of a
    /// config grant cannot weaken the threshold that governs them"*.
    pub const MATERIALITY_POLICY: &str = gts_id!("cf.bss.products.materiality_policy.v1~");
    /// Break-glass elevation — `elevate`. A resource of its own because the
    /// principal holding it is **outside** the tenant entirely, so it can
    /// never be folded into a tenant-scoped grant.
    pub const BREAKGLASS: &str = gts_id!("cf.bss.products.breakglass.v1~");
    /// Bulk **lifecycle** — `execute`, and a resource of its own rather than an
    /// action on [`BULK`]: §3.2 gives it its own row so the gear's most
    /// destructive batch act cannot be reached with the import pair. Declared
    /// here though its door does not ship, because **P-D-69** arm 7 assigns
    /// all four of `09`'s grant instances to this catalog — the roster being
    /// one closed set under a two-way set-equality assertion, and a closed
    /// set takes one writer.
    pub const BULK_LIFECYCLE: &str = gts_id!("cf.bss.products.bulk_lifecycle.v1~");
    /// Erasure — `execute`. `10-retention-erasure`'s own grant
    /// (`dod-retention-authz`), and a resource of its own because erasure is
    /// not a write to any one entity: it updates the pseudonym map, and every
    /// record that carries only refs completes erasure by that update alone.
    pub const ERASURE: &str = gts_id!("cf.bss.products.erasure.v1~");
    /// Compliance — `export`. The identity-export door's pair: a read that
    /// resolves pseudonyms is a different act from reading an entity, and no
    /// entity grant implies it.
    pub const COMPLIANCE: &str = gts_id!("cf.bss.products.compliance.v1~");
    /// The PII allow-list — `write`. Declared with **no route**: `design/05`
    /// §3.2 records the gap and `features/governance.md` §7 row 1 holds it
    /// open across eleven grants, so this declares the grant and invents no
    /// door.
    pub const PII_ALLOWLIST: &str = gts_id!("cf.bss.products.pii_allowlist.v1~");
    /// The audit plane — `read` and `export` (M-4's fix). Split from the
    /// entity grants deliberately: an audit reader sees refusals and actors
    /// across every subject, which no entity-scoped grant implies.
    pub const AUDIT: &str = gts_id!("cf.bss.products.audit.v1~");

    /// The recognized sets — the three non-tier families behind
    /// `POST /recognized-sets/{setKind}/members` (P-D-90 arm 2: the tier set
    /// spends its own grant below).
    pub const RECOGNIZED_SET: &str = gts_id!("cf.bss.products.recognized_set.v1~");

    /// The plan-tier taxonomy — its own grant, event and refusal code by
    /// design (`03` §3.2's taxonomy and P-D-90 arm 2).
    pub const PLAN_TIER: &str = gts_id!("cf.bss.products.plan_tier.v1~");

    /// Every authz label this module declares, stable order. The single
    /// canonical list driving [`super::authz_label_type_schemas`]'s stub
    /// registration. MUST match the permission catalog's distinct
    /// `resource_type`s (`crate::gts::permissions`); a drift test enforces it.
    pub const ALL: &[&str] = &[
        PRODUCT,
        SKU,
        CATALOG_VERSION,
        BULK,
        REFERENCE_SIGNAL,
        REFERENCE_PRODUCER,
        APPROVAL,
        MATERIALITY_POLICY,
        BREAKGLASS,
        AUDIT,
        BULK_LIFECYCLE,
        ERASURE,
        COMPLIANCE,
        PII_ALLOWLIST,
        RECOGNIZED_SET,
        PLAN_TIER,
    ];
}

/// PEP action names for the labels above.
pub mod actions {
    /// Read action — authoring reads of a head row and its version history
    /// (`GET /bss-products/v1/{products|skus}/{id}`,
    /// `GET /bss-products/v1/{products|skus}/{id}/versions`).
    pub const READ: &str = "read";
    /// Write action — authoring mutations: create, update, clone, discard,
    /// deprecate, un-deprecate, retire and retire/cancel
    /// (`POST /bss-products/v1/{products|skus}/{id}/undeprecate`,
    /// `POST /bss-products/v1/skus/{id}/deprecate`,
    /// `POST /bss-products/v1/{products|skus}/{id}/retire`,
    /// `POST /bss-products/v1/{products|skus}/{id}/retire/cancel`;
    /// `05-governance.md` §3.2 rows the discard door under this action; see
    /// this module's doc for why those acts are not declared separately).
    pub const WRITE: &str = "write";
    /// Publish action — turning an approved draft into a published version
    /// (`POST /bss-products/v1/{products|skus}/{id}/publish`).
    pub const PUBLISH: &str = "publish";
    /// Request action — enqueueing a `CatalogVersion` increment
    /// (`POST /bss-products/v1/catalog-version-requests` and the in-process
    /// binding alike — `design/06` §2 rule 1's one gate for both).
    pub const REQUEST: &str = "request";
    /// Ack action — a freeze participant confirming a version
    /// (`POST /bss-products/v1/catalog-versions/{id}/acks`, P-D-67).
    pub const ACK: &str = "ack";
    /// Release action — a participant ending its version liveness
    /// (`POST /bss-products/v1/catalog-versions/{id}/releases`, P-D-18/67).
    pub const RELEASE: &str = "release";
    /// Execute action — running a batch
    /// (`POST /bss-products/v1/bulk/imports`).
    pub const EXECUTE: &str = "execute";
    /// Post action — a producer posting its reference watermark
    /// (`POST /bss-products/v1/reference-watermarks`).
    pub const POST: &str = "post";

    /// Submit action — offering a change set for approval
    /// (`inst-gv-materiality`'s entry act). Distinct from `write`: the
    /// content is already authored, and what this grant admits is starting
    /// the ceremony over it.
    pub const SUBMIT: &str = "submit";
    /// Decide action — casting one principal's verdict on an approval
    /// (`inst-gv-self`, which opens *"`approval x decide` grant required"*).
    /// Never implied by `submit`: an author who may open a ceremony must not
    /// thereby be able to close it, which is the whole point of C2's
    /// self-approval refusal — built in `domain::approval::decision_admitted`.
    pub const DECIDE: &str = "decide";
    /// Elevate action — opening a break-glass session (`inst-bg-open`).
    pub const ELEVATE: &str = "elevate";
    /// Export action — taking audit content out of the gear, as opposed to
    /// reading it in place.
    pub const EXPORT: &str = "export";

    // `EXECUTE` is declared above, by `09`'s import door — `10`'s erasure
    // request spends the same action name on its own resource, which is what
    // makes the action vocabulary shared and the resource the discriminator.
}

/// Properties the PEP may compile from PDP constraints for registry rows.
/// Every `product`/`sku` row is tenant-owned: `owner_tenant_id` is the tenant
/// column the secure-ORM filter binds to, `id` the row PK (single-row gates).
/// Mirrors the ledger gear's `SUPPORTED_PROPERTIES` — no subtree/group
/// property, matching a PEP built via [`authz_resolver_sdk::PolicyEnforcer::new`]
/// with no advertised capabilities.
pub const SUPPORTED_PROPERTIES: &[&str] =
    &[pep_properties::OWNER_TENANT_ID, pep_properties::RESOURCE_ID];

/// PEP resource-type descriptors, one per authz label ([`labels::ALL`]).
///
/// [`ResourceType`] pairs a label with the resource properties the PEP is
/// allowed to compile from PDP constraints; every authoring door passes one
/// of these, never a bare label string, to [`access_scope`].
pub mod resource_types {
    use super::{ResourceType, SUPPORTED_PROPERTIES, labels};

    /// Products — `read`, `write`, `publish`.
    pub const PRODUCT: ResourceType =
        ResourceType::from_static(labels::PRODUCT, SUPPORTED_PROPERTIES);
    /// SKUs — `read`, `write`, `publish`.
    pub const SKU: ResourceType = ResourceType::from_static(labels::SKU, SUPPORTED_PROPERTIES);
    /// The catalog version — `request`, `ack`, `release`, `read`.
    pub const CATALOG_VERSION: ResourceType =
        ResourceType::from_static(labels::CATALOG_VERSION, SUPPORTED_PROPERTIES);
    /// Bulk — `execute`, `read`.
    pub const BULK: ResourceType = ResourceType::from_static(labels::BULK, SUPPORTED_PROPERTIES);
    /// The reference signal — `post`.
    pub const REFERENCE_SIGNAL: ResourceType =
        ResourceType::from_static(labels::REFERENCE_SIGNAL, SUPPORTED_PROPERTIES);
    /// The reference-producer registry — `write`.
    pub const REFERENCE_PRODUCER: ResourceType =
        ResourceType::from_static(labels::REFERENCE_PRODUCER, SUPPORTED_PROPERTIES);
    /// The approval record — `submit`, `read`, `decide`.
    pub const APPROVAL: ResourceType =
        ResourceType::from_static(labels::APPROVAL, SUPPORTED_PROPERTIES);
    /// The materiality policy — `write`.
    pub const MATERIALITY_POLICY: ResourceType =
        ResourceType::from_static(labels::MATERIALITY_POLICY, SUPPORTED_PROPERTIES);
    /// Break-glass elevation — `elevate`.
    pub const BREAKGLASS: ResourceType =
        ResourceType::from_static(labels::BREAKGLASS, SUPPORTED_PROPERTIES);
    /// The audit plane — `read`, `export`.
    pub const AUDIT: ResourceType = ResourceType::from_static(labels::AUDIT, SUPPORTED_PROPERTIES);
    /// The recognized sets — `write` (P-D-90 arm 2: the non-tier families).
    pub const RECOGNIZED_SET: ResourceType =
        ResourceType::from_static(labels::RECOGNIZED_SET, SUPPORTED_PROPERTIES);
    /// The plan-tier taxonomy — `write`, its own grant by design.
    pub const PLAN_TIER: ResourceType =
        ResourceType::from_static(labels::PLAN_TIER, SUPPORTED_PROPERTIES);
    /// Bulk lifecycle — `execute`.
    pub const BULK_LIFECYCLE: ResourceType =
        ResourceType::from_static(labels::BULK_LIFECYCLE, SUPPORTED_PROPERTIES);
    /// Erasure — `execute`.
    pub const ERASURE: ResourceType =
        ResourceType::from_static(labels::ERASURE, SUPPORTED_PROPERTIES);
    /// Compliance — `export`.
    pub const COMPLIANCE: ResourceType =
        ResourceType::from_static(labels::COMPLIANCE, SUPPORTED_PROPERTIES);
    /// The PII allow-list — `write`.
    pub const PII_ALLOWLIST: ResourceType =
        ResourceType::from_static(labels::PII_ALLOWLIST, SUPPORTED_PROPERTIES);
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
/// **Not yet registered.** See this module's doc: the sibling pricing gear
/// registers its equivalent from `Gear::init`; this gear's `init` does not
/// call this function yet, and that wiring is owed to a later slice.
#[must_use]
pub fn authz_label_type_schemas() -> Vec<serde_json::Value> {
    labels::ALL
        .iter()
        .map(|label| {
            authz_type_schema_json(
                label,
                &format!("BSS Product & SKU Registry authz label {label}"),
            )
        })
        .collect()
}

#[cfg(test)]
#[path = "authz_tests.rs"]
mod authz_tests;
