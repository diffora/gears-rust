//! Unit tests for [`CredStoreLocalClient`] and the `DomainError` →
//! `CredStoreError` SDK-error conversion.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use credstore_sdk::{CredStoreClientV1, CredStoreError, SecretRef, SecretValue, SharingMode};
use uuid::Uuid;

use super::{CredStoreLocalClient, DomainError, Service};
use crate::domain::ports::metrics::NoopMetrics;
use crate::domain::secret::service::ReaperSettings;
use crate::domain::secret::test_support::{
    FakeDir, FakePlugin, FakePluginSelector, FakeSecretRepo, catalog_type_resolver, make_ctx,
    mock_enforcer,
};

#[test]
fn domain_error_maps_to_sdk_error() {
    assert!(matches!(
        CredStoreError::from(DomainError::NotFound),
        CredStoreError::NotFound
    ));
    assert!(matches!(
        CredStoreError::from(DomainError::Conflict),
        CredStoreError::Conflict
    ));
    assert!(matches!(
        CredStoreError::from(DomainError::InvalidSecretRef {
            detail: "x".to_owned()
        }),
        CredStoreError::InvalidSecretRef { .. }
    ));
    assert!(matches!(
        CredStoreError::from(DomainError::UnsupportedTransition {
            detail: "x".to_owned()
        }),
        CredStoreError::UnsupportedTransition { .. }
    ));
    assert!(matches!(
        CredStoreError::from(DomainError::AccessDenied { cause: None }),
        CredStoreError::AccessDenied
    ));
    assert!(matches!(
        CredStoreError::from(DomainError::ServiceUnavailable {
            detail: "x".to_owned(),
            retry_after: Some(Duration::from_secs(1)),
            cause: None,
        }),
        CredStoreError::ServiceUnavailable { .. }
    ));
    assert!(matches!(
        CredStoreError::from(DomainError::internal("x")),
        CredStoreError::Internal(_)
    ));
}

#[tokio::test]
async fn local_client_round_trips_through_service() {
    let tenant = Uuid::new_v4();
    let ctx = make_ctx(Uuid::new_v4(), tenant);
    let k = SecretRef::new("client-key").expect("ref");

    let repo = Arc::new(FakeSecretRepo::new());
    let dir = Arc::new(FakeDir::single(tenant));
    let selector = Arc::new(FakePluginSelector::new(FakePlugin::new()));
    let svc = Arc::new(Service::new(
        repo,
        dir,
        mock_enforcer(),
        selector,
        catalog_type_resolver(),
        Arc::new(NoopMetrics),
        ReaperSettings {
            tick_secs: 60,
            provisioning_timeout_secs: 300,
            deprovisioning_timeout_secs: 300,
        },
    ));
    let client = CredStoreLocalClient::new(svc);

    client
        .put(
            &ctx,
            &k,
            SecretValue::new(b"v".to_vec()),
            SharingMode::Tenant,
        )
        .await
        .expect("put");
    assert!(client.get(&ctx, &k).await.expect("get").is_some());
    client.delete(&ctx, &k).await.expect("delete");
}

#[tokio::test]
async fn create_is_create_only_put_is_upsert() {
    let tenant = Uuid::new_v4();
    let ctx = make_ctx(Uuid::new_v4(), tenant);
    let k = SecretRef::new("create-key").expect("ref");

    let repo = Arc::new(FakeSecretRepo::new());
    let dir = Arc::new(FakeDir::single(tenant));
    let selector = Arc::new(FakePluginSelector::new(FakePlugin::new()));
    let svc = Arc::new(Service::new(
        repo,
        dir,
        mock_enforcer(),
        selector,
        catalog_type_resolver(),
        Arc::new(NoopMetrics),
        ReaperSettings {
            tick_secs: 60,
            provisioning_timeout_secs: 300,
            deprovisioning_timeout_secs: 300,
        },
    ));
    let client = CredStoreLocalClient::new(svc);

    // First create succeeds.
    client
        .create(
            &ctx,
            &k,
            SecretValue::new(b"v1".to_vec()),
            SharingMode::Tenant,
        )
        .await
        .expect("first create");
    // Second create of the same sharing class → Conflict (create-only).
    let err = client
        .create(
            &ctx,
            &k,
            SecretValue::new(b"v2".to_vec()),
            SharingMode::Tenant,
        )
        .await
        .expect_err("second create conflicts");
    assert!(matches!(err, CredStoreError::Conflict));
    // `put` still upserts the existing secret (no conflict).
    client
        .put(
            &ctx,
            &k,
            SecretValue::new(b"v3".to_vec()),
            SharingMode::Tenant,
        )
        .await
        .expect("put upserts");
}
