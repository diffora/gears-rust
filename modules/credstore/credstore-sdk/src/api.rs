use async_trait::async_trait;
use modkit_security::SecurityContext;

use crate::error::CredStoreError;
use crate::models::{GetSecretResponse, SecretRef, SecretValue, SharingMode};

/// Consumer-facing API trait for credential storage operations.
#[async_trait]
pub trait CredStoreClientV1: Send + Sync {
    /// Retrieves a secret by reference, applying hierarchical resolution.
    ///
    /// Returns `Ok(Some(_))` with the value and metadata when an accessible
    /// secret is found, `Ok(None)` when none exists or is inaccessible (a
    /// single 404 surface that prevents enumeration), and
    /// `Err(AccessDenied)` only when the caller lacks read permission.
    async fn get(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
    ) -> Result<Option<GetSecretResponse>, CredStoreError>;

    /// Stores or updates a secret (upsert).
    async fn put(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
    ) -> Result<(), CredStoreError>;

    /// Creates a secret, failing with [`CredStoreError::Conflict`] if one of the
    /// same sharing class already exists (create-only — the 409 path behind the
    /// REST `POST`). Use [`Self::put`] for upsert semantics.
    async fn create(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
    ) -> Result<(), CredStoreError>;

    /// Deletes a secret.
    async fn delete(&self, ctx: &SecurityContext, key: &SecretRef) -> Result<(), CredStoreError>;
}
