//! Tenant-scoped registry repositories and typed storage errors.

use crate::infra::storage::RepoError;
use toolkit_db::secure::ScopeError;

pub mod approval_repo;
pub mod audit_repo;
pub mod category_repo;
pub mod idempotency_repo;
pub mod reference_repo;
pub mod sku_repo;
pub mod version_repo;

pub use approval_repo::*;
pub use audit_repo::*;
pub use category_repo::*;
pub use idempotency_repo::*;
pub use reference_repo::*;
pub use sku_repo::*;
pub use version_repo::*;

/// Preserve the driver error for contention classification.
#[must_use]
pub fn driver_failure(context: String, source: ScopeError) -> RepoError {
    match source {
        ScopeError::Db(source) => RepoError::Driver { context, source },
        other => RepoError::Db(format!("{context}: {other}")),
    }
}

/// Result of a conditional head write; no match means no mutation occurred.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeadWrite<T> {
    Written(T),
    Unmatched,
}

/// Map unique violations on both named `PostgreSQL` indexes and `SQLite` columns.
fn map_unique(context: String, e: ScopeError) -> RepoError {
    if !e.is_unique_violation() {
        return driver_failure(context, e);
    }
    let s = e.to_string();
    let code = if s.contains("uq_products_sku_code") || s.contains("products_sku.code") {
        "SKU_CODE_TAKEN"
    } else if s.contains("uq_products_sku_name") || s.contains("products_sku.name") {
        "SKU_NAME_TAKEN"
    } else if s.contains("uq_products_category_code") || s.contains("products_category.code") {
        "CATEGORY_CODE_TAKEN"
    } else if s.contains("uq_products_sku_version_date")
        || s.contains("products_sku_version.effective_from")
    {
        "VERSION_DATE_TAKEN"
    } else if s.contains("uq_products_sku_reference_live")
        || s.contains("products_sku_reference.owner_gear")
    {
        "REFERENCE_EXISTS"
    } else {
        return driver_failure(context, e);
    };
    RepoError::Db(code.to_owned())
}
