//! Approval kinds and immutable proposal content shared by the subjects.
#![allow(
    dead_code,
    reason = "Task 9 subjects are wired into production REST doors in Task 10"
)]
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
