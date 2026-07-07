use async_trait::async_trait;
use toolkit_security::SecurityContext;

use crate::error::CredStoreError;
use crate::models::{GetSecretResponse, SecretRef, SecretValue, SharingMode, WriteOptions};

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

    /// Stores or updates a secret (upsert) with default [`WriteOptions`]
    /// (type preserved on overwrite, `generic` on create, no expiry change
    /// semantics of [`Self::put_opts`]).
    async fn put(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
    ) -> Result<(), CredStoreError> {
        self.put_opts(ctx, key, value, sharing, WriteOptions::default())
            .await
    }

    /// Stores or updates a secret (upsert) with explicit [`WriteOptions`]
    /// (secret type, expiry). The type is immutable: an `opts.secret_type`
    /// differing from an existing secret's type is rejected.
    ///
    /// The default implementation reports the operation as unsupported so
    /// value-store test doubles that only override [`Self::put`] stay valid;
    /// real gateway clients override it.
    async fn put_opts(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
        opts: WriteOptions,
    ) -> Result<(), CredStoreError> {
        let _ = (ctx, key, value, sharing, opts);
        Err(CredStoreError::internal(
            "put_opts is not supported by this CredStoreClientV1 implementation",
        ))
    }

    /// Creates a secret, failing with [`CredStoreError::Conflict`] if one of the
    /// same sharing class already exists (create-only — the 409 path behind the
    /// REST `POST`). Use [`Self::put`] for upsert semantics.
    async fn create(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
    ) -> Result<(), CredStoreError> {
        self.create_opts(ctx, key, value, sharing, WriteOptions::default())
            .await
    }

    /// Create-only variant of [`Self::put_opts`]. See it for the default
    /// implementation contract.
    async fn create_opts(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
        opts: WriteOptions,
    ) -> Result<(), CredStoreError> {
        let _ = (ctx, key, value, sharing, opts);
        Err(CredStoreError::internal(
            "create_opts is not supported by this CredStoreClientV1 implementation",
        ))
    }

    /// Deletes a secret.
    async fn delete(&self, ctx: &SecurityContext, key: &SecretRef) -> Result<(), CredStoreError>;
}
