use std::sync::Arc;

use async_trait::async_trait;
use credstore_sdk::{
    CredStoreClientV1, CredStoreError, GetSecretResponse, SecretRef, SecretValue, SharingMode,
};
use modkit_security::SecurityContext;

use crate::domain::error::DomainError;
use crate::domain::secret::service::Service;

impl From<DomainError> for CredStoreError {
    fn from(err: DomainError) -> Self {
        match err {
            DomainError::NotFound => CredStoreError::NotFound,
            // Both are 409-class; the SDK has no distinct optimistic-lock variant.
            DomainError::Conflict | DomainError::VersionConflict => CredStoreError::Conflict,
            DomainError::InvalidSecretRef { detail } => CredStoreError::invalid_ref(detail),
            DomainError::UnsupportedTransition { detail } => {
                CredStoreError::unsupported_transition(detail)
            }
            DomainError::AccessDenied { .. } => CredStoreError::AccessDenied,
            DomainError::ServiceUnavailable {
                detail,
                retry_after,
                ..
            } => CredStoreError::ServiceUnavailable {
                detail,
                retry_after,
            },
            DomainError::Internal { diagnostic, .. } => CredStoreError::internal(diagnostic),
            #[allow(unreachable_patterns)]
            _ => CredStoreError::internal("unmapped DomainError variant"),
        }
    }
}

/// In-process [`CredStoreClientV1`] that delegates to the domain [`Service`].
pub struct CredStoreLocalClient {
    svc: Arc<Service>,
}

impl CredStoreLocalClient {
    #[must_use]
    pub fn new(svc: Arc<Service>) -> Self {
        Self { svc }
    }
}

#[async_trait]
impl CredStoreClientV1 for CredStoreLocalClient {
    async fn get(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
    ) -> Result<Option<GetSecretResponse>, CredStoreError> {
        self.svc.get(ctx, key).await.map_err(Into::into)
    }

    async fn put(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
    ) -> Result<(), CredStoreError> {
        self.svc
            .put(ctx, key, value, sharing, false, None)
            .await
            .map_err(Into::into)
    }

    async fn create(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
    ) -> Result<(), CredStoreError> {
        // create_only = true: Conflict if a secret of this sharing class exists.
        self.svc
            .put(ctx, key, value, sharing, true, None)
            .await
            .map_err(Into::into)
    }

    async fn delete(&self, ctx: &SecurityContext, key: &SecretRef) -> Result<(), CredStoreError> {
        self.svc.delete(ctx, key, None).await.map_err(Into::into)
    }
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod client_tests;
