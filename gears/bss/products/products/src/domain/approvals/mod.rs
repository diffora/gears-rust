//! Approval kinds and immutable proposal content shared by the subjects.

use bss_products_sdk::models::{Lifecycle, SkuContent};
/// Publish a draft SKU.
pub const KIND_SKU_PUBLISH: &str = "sku_publish";
/// Change published business content and optionally lifecycle.
pub const KIND_SKU_CHANGE: &str = "sku_change";
/// Retire a fenced SKU.
pub const KIND_SKU_RETIRE: &str = "sku_retire";
/// Missing policy rows still require review (P-D-190).
pub const DEFAULT_QUORUM: u32 = 1;
/// The content and requested lifecycle that reviewers decide together.
#[toolkit_macros::domain_model]
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkuProposal {
    pub content: SkuContent,
    pub lifecycle: Option<Lifecycle>,
}

#[cfg(test)]
#[path = "approvals_tests.rs"]
mod approvals_tests;

pub(crate) mod change;
pub(crate) mod publish;
pub(crate) mod retire;

use crate::infra::storage::{RepoError, repo};
use bss_approval::ApprovalError;
use toolkit_db::{DbTx, secure::AccessScope};
use uuid::Uuid;

/// Keep contention errors typed all the way to the transaction boundary.
pub(crate) fn store_err(error: RepoError) -> ApprovalError {
    match error {
        RepoError::Driver { source, .. } => ApprovalError::Db(source),
        RepoError::Db(ref code)
            if matches!(
                code.as_str(),
                "SKU_NAME_TAKEN" | "SKU_CODE_TAKEN" | "CATEGORY_RETIRED" | "VERSION_ORDER"
            ) =>
        {
            let code = match code.as_str() {
                "SKU_NAME_TAKEN" => "SKU_NAME_TAKEN",
                "SKU_CODE_TAKEN" => "SKU_CODE_TAKEN",
                "CATEGORY_RETIRED" => "CATEGORY_RETIRED",
                _ => "VERSION_ORDER",
            };
            ApprovalError::ApplyRefused {
                code,
                detail: error.to_string(),
            }
        }
        other => ApprovalError::Store(other.to_string()),
    }
}
fn invalid(code: &'static str, field: &str, detail: impl Into<String>) -> ApprovalError {
    ApprovalError::InvalidSubmit {
        code,
        field: field.into(),
        detail: detail.into(),
    }
}
fn apply_error(e: ApprovalError) -> ApprovalError {
    match e {
        ApprovalError::InvalidSubmit { code, detail, .. } => {
            ApprovalError::ApplyRefused { code, detail }
        }
        other => other,
    }
}
fn json<T: serde::Serialize>(v: &T) -> Result<serde_json::Value, ApprovalError> {
    serde_json::to_value(v).map_err(|e| ApprovalError::Store(e.to_string()))
}
fn decode<T: serde::de::DeserializeOwned>(v: &serde_json::Value) -> Result<T, ApprovalError> {
    serde_json::from_value(v.clone()).map_err(|e| ApprovalError::Store(e.to_string()))
}
async fn sku(
    tx: &DbTx<'_>,
    scope: &AccessScope,
    tenant: Uuid,
    id: Uuid,
) -> Result<bss_products_sdk::models::Sku, ApprovalError> {
    repo::find_sku(tx, scope, tenant, id)
        .await
        .map_err(store_err)?
        .ok_or_else(|| invalid("NOT_FOUND", "id", id.to_string()))
}

/// The products kinds share one engine boundary without erasing the transaction runner.
#[toolkit_macros::domain_model]
#[derive(Clone)]
pub(crate) enum Subject {
    Publish(publish::SkuPublish),
    Change(change::SkuChange),
    Retire(retire::SkuRetire),
}
#[async_trait::async_trait]
impl<'a> bss_approval::ApprovalSubject<DbTx<'a>> for Subject {
    fn kind(&self) -> &'static str {
        match self {
            Self::Publish(s) => s.kind(),
            Self::Change(s) => s.kind(),
            Self::Retire(s) => s.kind(),
        }
    }
    fn ref_type(&self) -> &'static str {
        match self {
            Self::Publish(s) => s.ref_type(),
            Self::Change(s) => s.ref_type(),
            Self::Retire(s) => s.ref_type(),
        }
    }
    async fn collect(
        &self,
        tx: &DbTx<'a>,
        ids: &[Uuid],
    ) -> Result<Vec<bss_approval::ItemRef>, ApprovalError> {
        match self {
            Self::Publish(s) => s.collect(tx, ids).await,
            Self::Change(s) => s.collect(tx, ids).await,
            Self::Retire(s) => s.collect(tx, ids).await,
        }
    }
    async fn validate_submit(
        &self,
        tx: &DbTx<'a>,
        items: &[bss_approval::ItemRef],
    ) -> Result<(), ApprovalError> {
        match self {
            Self::Publish(s) => s.validate_submit(tx, items).await,
            Self::Change(s) => s.validate_submit(tx, items).await,
            Self::Retire(s) => s.validate_submit(tx, items).await,
        }
    }
    async fn lock(
        &self,
        tx: &DbTx<'a>,
        unit: Uuid,
        items: &[bss_approval::ItemRef],
    ) -> Result<(), ApprovalError> {
        match self {
            Self::Publish(s) => s.lock(tx, unit, items).await,
            Self::Change(s) => s.lock(tx, unit, items).await,
            Self::Retire(s) => s.lock(tx, unit, items).await,
        }
    }
    fn snapshot(
        &self,
        items: &[bss_approval::ItemRef],
        date: Option<time::Date>,
    ) -> serde_json::Value {
        match self {
            Self::Publish(s) => s.snapshot(items, date),
            Self::Change(s) => s.snapshot(items, date),
            Self::Retire(s) => s.snapshot(items, date),
        }
    }
    async fn apply(
        &self,
        tx: &DbTx<'a>,
        unit: &bss_approval::Unit,
        items: &[bss_approval::ItemRef],
    ) -> Result<(), ApprovalError> {
        match self {
            Self::Publish(s) => s.apply(tx, unit, items).await,
            Self::Change(s) => s.apply(tx, unit, items).await,
            Self::Retire(s) => s.apply(tx, unit, items).await,
        }
    }
    async fn unlock(
        &self,
        tx: &DbTx<'a>,
        unit: &bss_approval::Unit,
        items: &[bss_approval::ItemRef],
        approved: bool,
    ) -> Result<(), ApprovalError> {
        match self {
            Self::Publish(s) => s.unlock(tx, unit, items, approved).await,
            Self::Change(s) => s.unlock(tx, unit, items, approved).await,
            Self::Retire(s) => s.unlock(tx, unit, items, approved).await,
        }
    }
}
