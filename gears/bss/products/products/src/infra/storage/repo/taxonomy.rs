//! The category store — create, rename and re-parent, each with the
//! uniqueness race decided by the index (`design/02` §3.1
//! `inst-tx-name-in-parent`, §4.1; **P-D-50**, **P-D-88**).
//!
//! # The race is the index's, and that is a rule rather than an optimisation
//!
//! `inst-tx-name-in-parent` and `dod-name-in-parent` both put the decision on
//! `(tenant_id, parent_id, normalized(name))` **rather than a read-then-write
//! check**. So none of these functions reads for a collision first: they
//! write, and [`classify_category_write`] reads the violation back. A
//! pre-read would answer from a snapshot two writers can share, and the
//! probe that proves the difference is a concurrent one.
//!
//! **Two indexes, not one, and the second is not redundant.** A plain
//! `UNIQUE (tenant_id, parent_id, name_normalized)` treats every `NULL`
//! parent as distinct, so it cannot constrain the roots at all — P-D-88 arm
//! 1's `uq_products_category_root_name … WHERE parent_id IS NULL` is what
//! does. Both are classified to the same code, because a caller cannot tell
//! and should not have to.
//!
//! # `mutation_seq` counts acts, not row writes
//!
//! P-D-50 makes it the live-value door's `If-Match` operand, so a rename and
//! a re-parent each bump it by one and a write that touches two columns still
//! bumps once.
//!
//! # These functions hold no lock
//!
//! `inst-tc-writer-lock` serializes taxonomy mutations per tenant, and the
//! lock is the **provider's** (`DBProvider::lock`) — a `DBRunner` is sealed
//! and cannot issue one. So the lock lives one layer up, in
//! `crate::infra::taxonomy`, and every caller here must already hold it:
//! `inst-tx-walk` puts the walk *"inside the write transaction, under the
//! per-tenant taxonomy writer lock"*, and a re-parent's cycle verdict is only
//! trustworthy under it.

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use super::driver_failure;
use crate::domain::error::DomainError;
use crate::domain::taxonomy::TaxonomyMutation;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::category;

/// One new category, as the store needs it.
#[derive(Clone, Debug)]
pub struct NewCategory<'a> {
    /// The tenant.
    pub tenant_id: Uuid,
    /// The node's own id.
    pub category_id: Uuid,
    /// `None` is a root.
    pub parent_id: Option<Uuid>,
    /// The operator's name, as typed.
    pub name: &'a str,
    /// `domain::name::normalize`'s output — computed application-side,
    /// because the index compares this column and the engine has no NFKC.
    pub name_normalized: &'a str,
}

/// Create one category, letting the index decide the name race.
///
/// # Errors
///
/// [`DomainError::DuplicateCategoryName`] when the normalized name is taken
/// inside the parent — or among the roots. [`RepoError`] on a storage or
/// scope failure.
pub async fn insert_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewCategory<'_>,
    now: DateTime<Utc>,
) -> Result<Result<(), DomainError>, RepoError> {
    let model = category::ActiveModel {
        tenant_id: Set(new.tenant_id),
        category_id: Set(new.category_id),
        parent_id: Set(new.parent_id),
        name: Set(new.name.to_owned()),
        name_normalized: Set(new.name_normalized.to_owned()),
        state: Set("active".to_owned()),
        mutation_seq: Set(0),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let outcome = category::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("category scope of {}", new.tenant_id), e))?
        .exec(runner)
        .await;
    match outcome {
        Ok(_) => Ok(Ok(())),
        Err(e) => classify_category_write(
            new.name_normalized,
            new.parent_id,
            TaxonomyMutation::Create,
            driver_failure(format!("create category {}", new.category_id), e),
        ),
    }
}

/// Rename one category. The name moves, the parent does not.
///
/// # Errors
///
/// [`DomainError::DuplicateCategoryName`] on a collision inside the existing
/// parent; [`RepoError`] on a storage or scope failure.
pub async fn rename_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
    name: &str,
    name_normalized: &str,
    now: DateTime<Utc>,
) -> Result<Result<CategoryWrite, DomainError>, RepoError> {
    let outcome = category::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(category::Column::Name, Expr::value(name.to_owned()))
        .col_expr(
            category::Column::NameNormalized,
            Expr::value(name_normalized.to_owned()),
        )
        .col_expr(
            category::Column::MutationSeq,
            Expr::col(category::Column::MutationSeq).add(1),
        )
        .col_expr(category::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(category::Column::TenantId.eq(tenant_id))
                .add(category::Column::CategoryId.eq(category_id)),
        )
        .exec(runner)
        .await;
    match outcome {
        Ok(result) => Ok(Ok(CategoryWrite::from_rows(result.rows_affected))),
        Err(e) => classify_category_write(
            name_normalized,
            None,
            TaxonomyMutation::Rename,
            driver_failure(format!("rename category {category_id}"), e),
        )
        .map(|inner| inner.map(|()| CategoryWrite::Applied)),
    }
}

/// Re-parent one category. The parent moves, the name does not — and the
/// name is re-checked anyway, because it lands in a new sibling set.
///
/// The caller **must** have run `domain::taxonomy::cycle_verdict` over the
/// new parent's ancestor chain, read under the same lock and transaction:
/// nothing physical can catch a cycle, a `CHECK` seeing one row.
///
/// # Errors
///
/// [`DomainError::DuplicateCategoryName`] when the node's existing name is
/// taken inside the new parent; [`RepoError`] on a storage or scope failure.
pub async fn reparent_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
    new_parent: Option<Uuid>,
    now: DateTime<Utc>,
) -> Result<Result<CategoryWrite, DomainError>, RepoError> {
    let outcome = category::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(category::Column::ParentId, Expr::value(new_parent))
        .col_expr(
            category::Column::MutationSeq,
            Expr::col(category::Column::MutationSeq).add(1),
        )
        .col_expr(category::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(category::Column::TenantId.eq(tenant_id))
                .add(category::Column::CategoryId.eq(category_id)),
        )
        .exec(runner)
        .await;
    match outcome {
        Ok(result) => Ok(Ok(CategoryWrite::from_rows(result.rows_affected))),
        Err(e) => classify_category_write(
            "",
            new_parent,
            TaxonomyMutation::Reparent,
            driver_failure(format!("reparent category {category_id}"), e),
        )
        .map(|inner| inner.map(|()| CategoryWrite::Applied)),
    }
}

/// Whether the guarded write matched a row.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum CategoryWrite {
    /// The row moved.
    Applied,
    /// No row matched the filter — the node is not in the caller's scope, or
    /// not in this tenant. Not an error here: the door decides whether that
    /// is a 404, the way the head doors do.
    Unmatched,
}

impl CategoryWrite {
    const fn from_rows(rows: u64) -> Self {
        if rows == 0 {
            Self::Unmatched
        } else {
            Self::Applied
        }
    }
}

/// Read one category's parent, for the caller's ancestor walk.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn category_parents(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<(Uuid, Option<Uuid>)>, RepoError> {
    let rows = category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(category::Column::TenantId.eq(tenant_id)))
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("category tree of {tenant_id}"), e))?;
    Ok(rows
        .into_iter()
        .map(|r| (r.category_id, r.parent_id))
        .collect())
}

/// Read the uniqueness violation back off the engine.
///
/// **Both indexes map to one code.** `uq_products_category_name_in_parent`
/// and P-D-88 arm 1's root-name partial constrain the same rule at different
/// parents, and a caller cannot act differently on the two.
///
/// A violation that is neither is returned as the storage failure it is —
/// this classifier does not widen a driver error into a domain refusal.
fn classify_category_write(
    name_normalized: &str,
    parent_id: Option<Uuid>,
    mutation: TaxonomyMutation,
    error: RepoError,
) -> Result<Result<(), DomainError>, RepoError> {
    let message = error.to_string().to_ascii_lowercase();
    let unique = message.contains("unique constraint")
        || message.contains("duplicate key")
        || message.contains("uq_products_category_name_in_parent")
        || message.contains("uq_products_category_root_name");
    if !unique {
        return Err(error);
    }
    let where_ = match parent_id {
        Some(parent) => format!("inside parent {parent}"),
        None => "among the roots".to_owned(),
    };
    Ok(Err(DomainError::DuplicateCategoryName(format!(
        "a category already carries this normalized name {where_} (the {} re-checked it, and the \
         index decided the race, not a read: {name_normalized})",
        match mutation {
            TaxonomyMutation::Create => "create",
            TaxonomyMutation::Rename => "rename",
            TaxonomyMutation::Reparent => "re-parent",
        }
    ))))
}
