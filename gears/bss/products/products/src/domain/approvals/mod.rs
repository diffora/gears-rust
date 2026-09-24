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
