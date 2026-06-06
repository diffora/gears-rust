use async_trait::async_trait;
use credstore_sdk::{OwnerId, SecretRef, SharingMode, TenantId};
use modkit_security::AccessScope;
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::ports::metrics::SecretCounts;
use crate::domain::secret::model::{NewSecret, SecretRow};

#[async_trait]
pub trait SecretRepo: Send + Sync {
    /// Resolve the winning active secret for `req_tenant` walking the ordered
    /// `chain` (req first, root last), applying two-phase priority + sharing.
    async fn resolve_for_get(
        &self,
        req_tenant: TenantId,
        subject: OwnerId,
        key: &SecretRef,
        chain: &[Uuid],
    ) -> Result<Option<SecretRow>, DomainError>;

    /// Insert a `provisioning` row (scope-checked); maps UNIQUE → Conflict.
    async fn insert_provisioning(
        &self,
        scope: &AccessScope,
        new: &NewSecret,
    ) -> Result<(), DomainError>;

    /// Flip provisioning → active (scope-checked). 0 rows → Conflict.
    async fn mark_active(&self, scope: &AccessScope, id: Uuid) -> Result<(), DomainError>;

    /// Bump the version (and set sharing) of an existing active row, keyed by
    /// id. Atomic `version = version + 1`. When `expected_version` is `Some`,
    /// the bump is gated on `version = expected` (optimistic concurrency).
    /// Returns the post-write row, or `None` if no active row matched (row gone,
    /// or — under `expected_version` — the version no longer matches).
    async fn touch(
        &self,
        scope: &AccessScope,
        id: Uuid,
        sharing: SharingMode,
        expected_version: Option<i64>,
    ) -> Result<Option<SecretRow>, DomainError>;

    /// Find the caller's own-tenant row (two-phase: private-for-subject, else tenant/shared).
    async fn find_own(
        &self,
        scope: &AccessScope,
        tenant: TenantId,
        subject: OwnerId,
        key: &SecretRef,
    ) -> Result<Option<SecretRow>, DomainError>;

    /// Find the row a write of `sharing` would target, by sharing-class identity
    /// (mirrors the partial unique indexes): `Private` → `(tenant, ref, owner)`,
    /// `Tenant`/`Shared` → `(tenant, ref)` among non-private. Unlike [`find_own`]
    /// this never crosses the private boundary, so a private write does not see a
    /// coexisting tenant/shared secret (and vice-versa) — they coexist per design.
    async fn find_for_write(
        &self,
        scope: &AccessScope,
        tenant: TenantId,
        subject: OwnerId,
        key: &SecretRef,
        sharing: SharingMode,
    ) -> Result<Option<SecretRow>, DomainError>;

    /// Delete a row by id (scope-checked). When `expected_version` is `Some`,
    /// the delete is gated on `version = expected` (optimistic concurrency).
    /// 0 rows → `NotFound`.
    async fn delete_by_id(
        &self,
        scope: &AccessScope,
        id: Uuid,
        expected_version: Option<i64>,
    ) -> Result<(), DomainError>;

    /// Delete stuck provisioning rows older than `older_than_secs`; returns rows removed.
    async fn reap_provisioning(&self, older_than_secs: u64) -> Result<u64, DomainError>;

    /// Inventory counts by sharing + provisioning + distinct tenants (for gauges).
    async fn inventory(&self) -> Result<SecretCounts, DomainError>;

    /// True iff `tenant` is within the read `scope` (closure-backed for subtree).
    async fn scope_includes_tenant(
        &self,
        scope: &AccessScope,
        tenant: Uuid,
    ) -> Result<bool, DomainError>;
}
