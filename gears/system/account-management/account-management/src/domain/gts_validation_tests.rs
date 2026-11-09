//! Tenant-name bounds, caller propagation and registry-failure behavior.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::missing_panics_doc,
    reason = "test helpers"
)]

use crate::domain::error::DomainError;
use crate::domain::types_registry_reader::TypesRegistryReader;
use crate::infra::types_registry::test_helpers::{CallerSpy, Tampered, assert_only_caller, caller};
use serde_json::{Value, json};
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_gts::gts_uri;
use types_registry_sdk::testing_platform::{Call, Fault, MockTypesRegistry};

fn schema_doc(type_id: &str, body: &Value) -> Value {
    let mut doc = json!({
        "$id": gts_uri!(type_id),
        "$schema": "http://json-schema.org/draft-07/schema#",
    });
    doc.as_object_mut()
        .expect("object")
        .extend(body.as_object().expect("object").clone());
    doc
}

fn registry_with_tenant_schema(max_chars: usize) -> MockTypesRegistry {
    let registry = MockTypesRegistry::new();
    registry.seed(
        super::TENANT_TYPE_ID,
        schema_doc(
            super::TENANT_TYPE_ID,
            &json!({
                "type": "object",
                "additionalProperties": true,
                "required": ["id", "name"],
                "properties": {
                    "id": { "type": "string", "format": "uuid" },
                    "name": { "type": "string", "minLength": 1, "maxLength": max_chars },
                    "parent_id": { "type": ["string", "null"], "format": "uuid" },
                },
            }),
        ),
    );
    registry
}

fn unavailable_registry() -> MockTypesRegistry {
    let registry = MockTypesRegistry::new();
    registry.inject(
        Fault::on(Call::BatchGet).fail(
            CanonicalError::service_unavailable()
                .with_detail("registry transport down (test stub)")
                .create(),
        ),
    );
    registry
}

#[tokio::test]
async fn tenant_name_schema_not_registered_short_circuits_to_ok() {
    let registry = MockTypesRegistry::new();
    super::validate_tenant_name_via_gts("acme", TypesRegistryReader::caller(&registry, &caller(1)))
        .await
        .expect("schema-not-found short-circuits to Ok");
}

#[tokio::test]
async fn tenant_name_valid_passes_registered_schema() {
    let registry = registry_with_tenant_schema(255);
    super::validate_tenant_name_via_gts("acme", TypesRegistryReader::caller(&registry, &caller(1)))
        .await
        .expect("valid name passes the registered schema");
}

#[tokio::test]
async fn tenant_name_oversized_rejects_with_validation() {
    let registry = registry_with_tenant_schema(255);
    let oversized = "x".repeat(256);
    let err = super::validate_tenant_name_via_gts(
        &oversized,
        TypesRegistryReader::caller(&registry, &caller(1)),
    )
    .await
    .expect_err("oversize name must be rejected");
    match err {
        DomainError::Validation { detail } => {
            assert!(
                detail.contains("name"),
                "diagnostic must name the violating field; got: {detail}"
            );
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn tenant_name_empty_rejects_with_validation() {
    let registry = registry_with_tenant_schema(255);
    let err =
        super::validate_tenant_name_via_gts("", TypesRegistryReader::caller(&registry, &caller(1)))
            .await
            .expect_err("empty name must be rejected");
    assert!(matches!(err, DomainError::Validation { .. }));
}

#[tokio::test]
async fn tenant_name_registry_transport_error_surfaces_as_service_unavailable() {
    let registry = unavailable_registry();
    let err = super::validate_tenant_name_via_gts(
        "acme",
        TypesRegistryReader::caller(&registry, &caller(1)),
    )
    .await
    .expect_err("registry transport error must surface");
    assert!(
        matches!(err, DomainError::ServiceUnavailable { .. }),
        "expected ServiceUnavailable, got {err:?}"
    );
}

#[tokio::test]
async fn a_failed_schema_read_is_classified_at_the_boundary() {
    use crate::infra::types_registry::test_helpers::{at_the_boundary, outages, refusals};

    let expected = outages()
        .into_iter()
        .map(|error| (error, 503, 503))
        .chain(
            refusals()
                .into_iter()
                .map(|(error, status)| (error, status, 500)),
        )
        .chain([(
            CanonicalError::internal("registry exploded").create(),
            500,
            500,
        )]);
    for (error, for_caller, for_platform) in expected {
        let registry = MockTypesRegistry::new();
        registry.inject(Fault::on(Call::BatchGet).fail(error.clone()));
        let ctx = caller(1);

        let name = super::validate_tenant_name_via_gts(
            "acme",
            TypesRegistryReader::caller(&registry, &ctx),
        )
        .await
        .expect_err("tenant name read failed");
        let (category, status) = at_the_boundary(name);
        assert_eq!(status, Some(for_caller), "{error:?} -> {category}");
        if for_caller != 503 && for_caller != 500 {
            assert_eq!(category, error.gts_type(), "{error:?}");
        }

        let platform =
            super::validate_tenant_name_via_gts("acme", TypesRegistryReader::Platform(&registry))
                .await
                .expect_err("platform read failed");
        assert_eq!(at_the_boundary(platform).1, Some(for_platform), "{error:?}");
    }
}

#[tokio::test]
async fn a_snapshot_without_the_selected_resolved_schema_is_internal() {
    let registry = Arc::new(registry_with_tenant_schema(255));
    let tampered = Tampered::without_documents(registry);

    let err = super::validate_tenant_name_via_gts(
        "acme",
        TypesRegistryReader::caller(&tampered, &caller(1)),
    )
    .await
    .expect_err("an incomplete projection is a contract break");
    assert!(matches!(err, DomainError::Internal { .. }), "got {err:?}");
}

#[tokio::test]
async fn an_inline_bound_applies_to_the_tenant_name() {
    let registry = MockTypesRegistry::new();
    registry.seed(
        super::TENANT_TYPE_ID,
        schema_doc(
            super::TENANT_TYPE_ID,
            &json!({
                "type": "object",
                "properties": { "name": { "type": "string", "maxLength": 255 } },
                "allOf": [{ "properties": { "name": { "type": "string", "maxLength": 4 } } }],
            }),
        ),
    );

    let err = super::validate_tenant_name_via_gts(
        "acme-corp",
        TypesRegistryReader::caller(&registry, &caller(1)),
    )
    .await
    .expect_err("the inline bound applies");
    assert!(matches!(err, DomainError::Validation { .. }), "got {err:?}");
}

#[tokio::test]
async fn tenant_name_reads_for_each_request_caller() {
    let spy = CallerSpy::for_requests(Arc::new(registry_with_tenant_schema(255)));
    for n in [1, 2] {
        let ctx = caller(n);
        super::validate_tenant_name_via_gts(
            "acme",
            TypesRegistryReader::caller(spy.as_ref(), &ctx),
        )
        .await
        .expect("valid name");
        assert_only_caller(&spy.callers(), &ctx);
        spy.clear();
    }
}

#[tokio::test]
async fn a_failed_request_read_does_not_fall_back_to_the_platform_api() {
    let spy = CallerSpy::for_requests(Arc::new(unavailable_registry()));
    let ctx = caller(1);
    let err = super::validate_tenant_name_via_gts(
        "acme",
        TypesRegistryReader::caller(spy.as_ref(), &ctx),
    )
    .await
    .expect_err("registry down");
    assert!(
        matches!(err, DomainError::ServiceUnavailable { .. }),
        "got {err:?}"
    );
    assert_only_caller(&spy.callers(), &ctx);
    assert_eq!(spy.platform_reads(), 0);
}

#[tokio::test]
async fn the_platform_tenant_name_check_reads_only_the_platform_api() {
    let spy = CallerSpy::for_platform(Arc::new(registry_with_tenant_schema(4)));
    let err = super::validate_tenant_name_via_gts(
        "acme-corp",
        TypesRegistryReader::Platform(spy.as_ref()),
    )
    .await
    .expect_err("the platform read still applies the bound");
    assert!(matches!(err, DomainError::Validation { .. }), "got {err:?}");
    assert_eq!(spy.platform_reads(), 1);
    assert!(spy.callers().is_empty());
}
