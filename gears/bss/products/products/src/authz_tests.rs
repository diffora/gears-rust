//! Tests for the registry's authz descriptors, label stub schemas, and the
//! [`access_scope`] PEP gate.
//!
//! The permit/deny/unavailable paths are exercised against a fake
//! `AuthZResolverClient` rather than a live resolver — the same technique the
//! sibling ledger gear's own `authz_tests.rs` uses (see
//! `gears/bss/ledger/ledger/src/authz_tests.rs`), so `access_scope`'s own
//! logic (the `EnforcerError` → `AuthzError` split, and the write-path
//! cross-tenant membership assertion) is proven without a resolver
//! deployment.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use async_trait::async_trait;
use authz_resolver_sdk::constraints::{Constraint, InPredicate, Predicate};
use authz_resolver_sdk::models::{
    EvaluationRequest, EvaluationResponse, EvaluationResponseContext,
};
use authz_resolver_sdk::{AuthZResolverClient, AuthZResolverError, PolicyEnforcer};
use toolkit_gts::gts_id;
use toolkit_security::{SecurityContext, pep_properties};
use uuid::Uuid;

use super::{AuthzError, access_scope, actions, authz_label_type_schemas, labels, resource_types};

#[test]
fn labels_all_is_product_and_sku() {
    assert_eq!(labels::ALL, [labels::PRODUCT, labels::SKU]);
}

#[test]
fn resource_types_carry_their_labels() {
    assert_eq!(resource_types::PRODUCT.name(), labels::PRODUCT);
    assert_eq!(resource_types::SKU.name(), labels::SKU);
}

/// Stronger than a suffix match: every authz label must parse as a
/// structurally valid GTS id AND be a concrete TYPE id (type ids end `~`).
#[test]
fn labels_are_concrete_gts_types() {
    for label in labels::ALL {
        assert!(
            ::gts::GtsId::try_new(label).is_ok(),
            "label {label} is not a structurally valid GTS id"
        );
        assert!(
            label.ends_with('~'),
            "label {label} must be a concrete type id"
        );
    }
}

/// One stub schema per label, each addressed at the label's own `$id` and
/// shaped as a bare JSON-Schema object — the shape the platform RBAC
/// role-definition validator resolves a `target_type` against.
#[test]
fn authz_label_type_schemas_covers_every_label_exactly_once() {
    let schemas = authz_label_type_schemas();
    assert_eq!(schemas.len(), labels::ALL.len());

    let ids: std::collections::BTreeSet<String> = schemas
        .iter()
        .map(|schema| {
            schema["$id"]
                .as_str()
                .expect("each stub schema carries a $id")
                .to_owned()
        })
        .collect();
    let expected: std::collections::BTreeSet<String> = labels::ALL
        .iter()
        .map(|label| format!("gts://{label}"))
        .collect();
    assert_eq!(ids, expected);

    for schema in &schemas {
        assert_eq!(schema["type"], "object");
    }
}

/// The three action names are distinct — a copy-paste that left two consts
/// holding the same string would let two permissions in the catalog collide
/// on `(resource_type, action)` without either the catalog's id-distinctness
/// test or its resource-type drift test noticing, since neither reads the
/// action names against each other.
#[test]
fn action_names_are_pairwise_distinct() {
    let names = [actions::READ, actions::WRITE, actions::PUBLISH];
    let distinct: std::collections::BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(distinct.len(), names.len(), "two action consts collide");
}

/// Degraded flat-`In` PDP fake: permits and emits a single flat
/// `In([allowed])` constraint over `OWNER_TENANT_ID` — the shape the
/// production PDP returns for a PEP that advertises no tenant-subtree
/// capability (this gear, [`PolicyEnforcer::new`] with no
/// `with_capabilities`). The request is ignored: the fake models a subject
/// authorized only for the single `allowed` tenant.
struct FlatInResolver {
    allowed: Uuid,
}

#[async_trait]
impl AuthZResolverClient for FlatInResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Ok(EvaluationResponse {
            decision: true,
            context: EvaluationResponseContext {
                constraints: vec![Constraint {
                    predicates: vec![Predicate::In(InPredicate::new(
                        pep_properties::OWNER_TENANT_ID,
                        vec![self.allowed],
                    ))],
                }],
                deny_reason: None,
            },
        })
    }
}

/// A degraded-mode enforcer (no `with_capabilities`) over a subject authorized
/// for `allowed` only — mirrors the gear's production PEP wiring
/// (`crate::gear::BssProductsGear::init`).
fn flat_in_enforcer(allowed: Uuid) -> PolicyEnforcer {
    PolicyEnforcer::new(Arc::new(FlatInResolver { allowed }))
}

fn ctx_for(tenant: Uuid) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(Uuid::now_v7())
        .subject_tenant_id(tenant)
        .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
        .token_scopes(vec!["*".to_owned()])
        .build()
        .expect("authed SecurityContext must build")
}

/// A write gate (`require_constraints = true` + a target `owner_tenant_id`)
/// must DENY when the target tenant is outside the PDP's compiled scope, and
/// ALLOW when it is inside. This pins the cross-tenant-write hole: the
/// degraded flat-`In` decision does not re-validate `owner_tenant_id` at the
/// PDP, so the gate itself must assert target membership.
#[tokio::test]
async fn write_gate_denies_target_outside_authorized_scope() {
    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();
    let enforcer = flat_in_enforcer(tenant_a); // authorized for tenant_a only
    let ctx = ctx_for(tenant_a);

    // Cross-tenant write: target B is outside the authorized In([A]) -> Denied.
    let denied = access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRODUCT,
        actions::WRITE,
        Some(tenant_b),
        None,
        true,
    )
    .await;
    assert!(
        matches!(denied, Err(AuthzError::Denied(_))),
        "writing into tenant B with scope In([A]) must be denied, got {denied:?}"
    );

    // In-scope write: target A is inside the authorized scope -> allowed, and
    // the returned scope carries the In([A]) filter for SQL-level binding.
    let allowed = access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRODUCT,
        actions::WRITE,
        Some(tenant_a),
        None,
        true,
    )
    .await
    .expect("writing into own tenant A must be allowed");
    assert!(
        allowed.contains_uuid(pep_properties::OWNER_TENANT_ID, tenant_a),
        "the granted scope must carry the tenant-A filter"
    );
}

/// `publish` gates exactly like `write` on the `sku` resource: a cross-tenant
/// target is denied and an in-scope target is allowed with the `In([A])`
/// filter. Pins that `access_scope` treats every action name uniformly and
/// that the write-membership assertion is not `product`-specific.
#[tokio::test]
async fn publish_gate_on_sku_matches_write_semantics() {
    let tenant_a = Uuid::now_v7();
    let tenant_b = Uuid::now_v7();
    let enforcer = flat_in_enforcer(tenant_a);
    let ctx = ctx_for(tenant_a);

    let denied = access_scope(
        &enforcer,
        &ctx,
        &resource_types::SKU,
        actions::PUBLISH,
        Some(tenant_b),
        None,
        true,
    )
    .await;
    assert!(
        matches!(denied, Err(AuthzError::Denied(_))),
        "publishing into tenant B with scope In([A]) must be denied, got {denied:?}"
    );

    let allowed = access_scope(
        &enforcer,
        &ctx,
        &resource_types::SKU,
        actions::PUBLISH,
        Some(tenant_a),
        None,
        true,
    )
    .await
    .expect("publishing within own tenant A must be allowed");
    assert!(
        allowed.contains_uuid(pep_properties::OWNER_TENANT_ID, tenant_a),
        "the granted scope must carry the tenant-A filter"
    );
}

/// PDP fake that always fails to evaluate (models an unreachable PDP).
struct FailingResolver;

#[async_trait]
impl AuthZResolverClient for FailingResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Err(AuthZResolverError::Internal("pdp unreachable".to_owned()))
    }
}

/// PDP fake that explicitly denies (`decision = false`).
struct DenyingResolver;

#[async_trait]
impl AuthZResolverClient for DenyingResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Ok(EvaluationResponse {
            decision: false,
            context: EvaluationResponseContext {
                constraints: vec![],
                deny_reason: None,
            },
        })
    }
}

/// An unreachable PDP must fail closed as `Unavailable` (→ 503), NOT `Denied`
/// (→ 403): the two carry different operator semantics and retry behaviour.
#[tokio::test]
async fn pdp_evaluation_failure_maps_to_unavailable() {
    let enforcer = PolicyEnforcer::new(Arc::new(FailingResolver));
    let ctx = ctx_for(Uuid::now_v7());
    let res = access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRODUCT,
        actions::READ,
        None,
        None,
        true,
    )
    .await;
    assert!(
        matches!(res, Err(AuthzError::Unavailable(_))),
        "an unreachable PDP must fail closed as Unavailable, got {res:?}"
    );
}

/// An explicit PDP deny maps to `Denied` (→ 403).
#[tokio::test]
async fn pdp_decision_false_maps_to_denied() {
    let enforcer = PolicyEnforcer::new(Arc::new(DenyingResolver));
    let ctx = ctx_for(Uuid::now_v7());
    let res = access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRODUCT,
        actions::READ,
        None,
        None,
        true,
    )
    .await;
    assert!(
        matches!(res, Err(AuthzError::Denied(_))),
        "an explicit PDP deny must map to Denied, got {res:?}"
    );
}

/// A read (`owner_tenant_id = None`) skips the write-membership assertion and
/// returns the PDP's compiled `In([tenant])` scope verbatim for SQL binding.
#[tokio::test]
async fn read_path_returns_pdp_scope_without_membership_check() {
    let tenant = Uuid::now_v7();
    let enforcer = flat_in_enforcer(tenant);
    let ctx = ctx_for(tenant);
    let scope = access_scope(
        &enforcer,
        &ctx,
        &resource_types::PRODUCT,
        actions::READ,
        None,
        None,
        true,
    )
    .await
    .expect("read must be allowed");
    assert!(
        scope.contains_uuid(pep_properties::OWNER_TENANT_ID, tenant),
        "the read scope must carry the tenant filter"
    );
}
