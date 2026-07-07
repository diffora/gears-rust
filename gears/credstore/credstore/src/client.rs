use std::sync::Arc;

use async_trait::async_trait;
use credstore_sdk::{
    CredStoreClientV1, CredStoreError, GetSecretResponse, SecretRef, SecretValue, SharingMode,
    WriteOptions,
};
use toolkit_security::SecurityContext;

use crate::domain::error::DomainError;
use crate::domain::secret::model::WriteSpec;
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
            DomainError::TypeViolation { reason, detail, .. } => CredStoreError::TypeViolation {
                reason: reason.to_owned(),
                detail,
            },
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
            // If-Match preconditions exist only on the REST surface; the
            // ClientHub API has no way to send one, so this is a
            // gateway-internal invariant breach if it ever crosses here.
            DomainError::InvalidPrecondition { detail } => {
                CredStoreError::internal(format!("invalid precondition: {detail}"))
            }
            #[allow(unreachable_patterns)]
            other => CredStoreError::internal(format!("unmapped DomainError variant: {other}")),
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

    async fn put_opts(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
        opts: WriteOptions,
    ) -> Result<(), CredStoreError> {
        self.svc
            .put(ctx, key, value, WriteSpec::upsert(sharing).with_opts(opts))
            .await
            .map_err(Into::into)
    }

    async fn create_opts(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
        opts: WriteOptions,
    ) -> Result<(), CredStoreError> {
        // create-only: Conflict if a secret of this sharing class exists.
        self.svc
            .put(ctx, key, value, WriteSpec::create(sharing).with_opts(opts))
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
