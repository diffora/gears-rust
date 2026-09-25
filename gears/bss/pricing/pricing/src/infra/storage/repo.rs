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
pub mod plan_item_repo;
pub mod plan_repo;
pub mod plan_revision_repo;
pub mod price_book_entry_repo;
pub mod price_repo;
pub mod reference_op_repo;
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
    } else if message.contains("pricing_price_book_entry_key") {
        Some("ENTRY_KEY_TAKEN")
    } else if message.contains("pricing_price_approved_start") {
        Some("WINDOW_OVERLAP")
    } else if message.contains("pricing_price_price_book_entry_id_version_no_key")
        || message.contains("pricing_price.price_book_entry_id, pricing_price.version_no")
    {
        Some("PRICE_VERSION_TAKEN")
    } else if message.contains("pricing_dimension_key_pkey")
        || message.contains("pricing_dimension_key.tenant_id, pricing_dimension_key.key")
    {
        Some("DIM_KEY_TAKEN")
    } else {
        plan_unique_code(message)
    }
}
/// The phase 3 keys. Postgres names the index or constraint; `SQLite` names the columns, which a
/// partial index shares with its siblings: the two single-column revision indexes read alike there
/// and are told apart by `plan_revision_repo`, which knows the state it wrote. Longer column
/// lists are matched before the single column they begin with.
fn plan_unique_code(message: &str) -> Option<&'static str> {
    if message.contains("pricing_plan_code")
        || message.contains("pricing_plan.tenant_id, pricing_plan.code")
    {
        Some("PLAN_CODE_TAKEN")
    } else if message.contains("pricing_plan_revision_no")
        || message.contains("pricing_plan_revision.plan_id, pricing_plan_revision.rev_no")
    {
        Some("REVISION_NO_TAKEN")
    } else if message.contains("pricing_plan_revision_open") {
        Some("REVISION_DRAFT_EXISTS")
    } else if message.contains("pricing_plan_revision_published") {
        Some("REVISION_PUBLISHED_EXISTS")
    } else if message.contains("pricing_plan_item_sku")
        || message.contains("pricing_plan_item.revision_id, pricing_plan_item.sku_id")
    {
        Some("ITEM_SKU_TAKEN")
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
/// Refuse a lock that names no approval unit of the tenant.
async fn unit_exists(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &toolkit_db::secure::AccessScope,
    tenant: uuid::Uuid,
    unit: uuid::Uuid,
    context: &str,
) -> Result<(), RepoError> {
    let parent = approval_repo::find_unit(runner, scope, tenant, unit)
        .await
        .map_err(|error| {
            error.db_err().map_or_else(
                || RepoError::Db(error.to_string()),
                |source| RepoError::Driver {
                    context: context.to_owned(),
                    source: source.clone(),
                },
            )
        })?;
    if parent.is_none() {
        return Err(RepoError::Conflict {
            code: "UNIT_NOT_FOUND",
        });
    }
    Ok(())
}
fn matched(rows: u64, code: &'static str) -> Result<(), RepoError> {
    if rows == 1 {
        Ok(())
    } else {
        Err(RepoError::Conflict { code })
    }
}
