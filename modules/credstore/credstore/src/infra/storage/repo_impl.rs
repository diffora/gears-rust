//! `SeaORM`-backed implementation of [`SecretRepo`].

pub mod helpers;
mod reads;
mod writes;

#[cfg(test)]
mod repo_tests;

use std::sync::Arc;

use async_trait::async_trait;
use credstore_sdk::{OwnerId, SecretRef, SharingMode, TenantId};
use modkit_db::DBProvider;
use modkit_security::AccessScope;
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::ports::metrics::SecretCounts;
use crate::domain::secret::model::{SecretRow, SecretStatus};
use crate::domain::secret::repo::SecretRepo;
use crate::infra::storage::entity;

pub type CredstoreDbProvider = DBProvider<DomainError>;

/// `SeaORM` repository adapter for [`SecretRepo`].
pub struct SecretRepoImpl {
    pub(crate) db: Arc<CredstoreDbProvider>,
}

impl SecretRepoImpl {
    #[must_use]
    pub fn new(db: Arc<CredstoreDbProvider>) -> Self {
        Self { db }
    }
}

#[async_trait]
impl SecretRepo for SecretRepoImpl {
    async fn resolve_for_get(
        &self,
        req_tenant: TenantId,
        subject: OwnerId,
        key: &SecretRef,
        chain: &[Uuid],
    ) -> Result<Option<SecretRow>, DomainError> {
        reads::resolve_for_get(self, req_tenant, subject, key, chain).await
    }

    async fn insert_provisioning(
        &self,
        scope: &AccessScope,
        new: &crate::domain::secret::model::NewSecret,
    ) -> Result<(), DomainError> {
        writes::insert_provisioning(self, scope, new).await
    }

    async fn mark_active(&self, scope: &AccessScope, id: Uuid) -> Result<(), DomainError> {
        writes::mark_active(self, scope, id).await
    }

    async fn touch(
        &self,
        scope: &AccessScope,
        id: Uuid,
        sharing: SharingMode,
        expected_version: Option<i64>,
    ) -> Result<Option<SecretRow>, DomainError> {
        writes::touch(self, scope, id, sharing, expected_version).await
    }

    async fn find_own(
        &self,
        scope: &AccessScope,
        tenant: TenantId,
        subject: OwnerId,
        key: &SecretRef,
    ) -> Result<Option<SecretRow>, DomainError> {
        reads::find_own(self, scope, tenant, subject, key).await
    }

    async fn find_for_write(
        &self,
        scope: &AccessScope,
        tenant: TenantId,
        subject: OwnerId,
        key: &SecretRef,
        sharing: SharingMode,
    ) -> Result<Option<SecretRow>, DomainError> {
        reads::find_for_write(self, scope, tenant, subject, key, sharing).await
    }

    async fn delete_by_id(
        &self,
        scope: &AccessScope,
        id: Uuid,
        expected_version: Option<i64>,
    ) -> Result<(), DomainError> {
        writes::delete_by_id(self, scope, id, expected_version).await
    }

    async fn reap_provisioning(&self, older_than_secs: u64) -> Result<u64, DomainError> {
        writes::reap_provisioning(self, older_than_secs).await
    }

    async fn inventory(&self) -> Result<SecretCounts, DomainError> {
        reads::inventory(self).await
    }

    async fn scope_includes_tenant(
        &self,
        scope: &AccessScope,
        tenant: Uuid,
    ) -> Result<bool, DomainError> {
        reads::scope_includes_tenant(self, scope, tenant).await
    }
}

/// Map an entity row to the domain [`SecretRow`].
pub(crate) fn entity_to_model(m: entity::secrets::Model) -> Result<SecretRow, DomainError> {
    let sharing = sharing_from_i16(m.sharing).ok_or_else(|| DomainError::Internal {
        diagnostic: format!(
            "credstore_secrets.sharing out-of-domain value: {}",
            m.sharing
        ),
        cause: None,
    })?;
    let status = SecretStatus::from_smallint(m.status).ok_or_else(|| DomainError::Internal {
        diagnostic: format!("credstore_secrets.status out-of-domain value: {}", m.status),
        cause: None,
    })?;
    Ok(SecretRow {
        id: m.id,
        tenant_id: TenantId(m.tenant_id),
        reference: m.reference,
        sharing,
        owner_id: OwnerId(m.owner_id),
        status,
        version: m.version,
    })
}

/// Map [`SharingMode`] to its `SMALLINT` storage value.
pub(crate) fn sharing_to_i16(s: SharingMode) -> i16 {
    match s {
        SharingMode::Private => 1,
        SharingMode::Tenant => 2,
        SharingMode::Shared => 3,
    }
}

/// Map a `SMALLINT` storage value to [`SharingMode`].
pub(crate) fn sharing_from_i16(v: i16) -> Option<SharingMode> {
    match v {
        1 => Some(SharingMode::Private),
        2 => Some(SharingMode::Tenant),
        3 => Some(SharingMode::Shared),
        _ => None,
    }
}
