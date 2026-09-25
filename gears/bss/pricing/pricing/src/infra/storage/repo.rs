//! Repositories accept any scoped transaction or connection runner.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-scoped-repositories:p1
use super::RepoError;
use toolkit_db::secure::ScopeError;
pub mod approval_repo;
pub mod audit_repo;
pub mod book_repo;
pub mod dimension_repo;
pub mod idempotency_repo;
pub mod price_repo;
pub mod reference_op_repo;
pub mod row_repo;
pub mod settings_repo;
/// Preserve the driver's variant for serializable retries.
#[must_use]
pub fn driver_failure(context: String, error: ScopeError) -> RepoError {
    match error {
        ScopeError::Db(source) => RepoError::Driver { context, source },
        other => RepoError::Db(format!("{context}: {other}")),
    }
}
/// Identify named Postgres constraints and `SQLite` unique column/index diagnostics.
#[must_use]
pub fn unique_code(message: &str) -> Option<&'static str> {
    if message.contains("pricing_price_book_tenant_id_code_key")
        || message.contains("pricing_price_book.tenant_id, pricing_price_book.code")
    {
        Some("BOOK_CODE_TAKEN")
    } else if message.contains("pricing_price_key") {
        Some("PRICE_KEY_TAKEN")
    } else if message.contains("pricing_price_row_approved_start") {
        Some("WINDOW_OVERLAP")
    } else if message.contains("pricing_price_row_price_id_version_no_key")
        || message.contains("pricing_price_row.price_id, pricing_price_row.version_no")
    {
        Some("ROW_VERSION_TAKEN")
    } else if message.contains("pricing_dimension_key_pkey")
        || message.contains("pricing_dimension_key.tenant_id, pricing_dimension_key.key")
    {
        Some("DIM_KEY_TAKEN")
    } else {
        None
    }
}
fn map_unique(context: String, error: ScopeError) -> RepoError {
    if error.is_unique_violation()
        && let Some(code) = unique_code(&error.to_string())
    {
        return RepoError::Conflict { code };
    }
    driver_failure(context, error)
}
fn matched(rows: u64, code: &'static str) -> Result<(), RepoError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(RepoError::Conflict { code })
    }
}
