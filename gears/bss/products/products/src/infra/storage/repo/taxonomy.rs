//! The taxonomy and content store — the category tree's create, rename and
//! re-parent with the uniqueness race decided by the index, plus the three
//! content planes 02 owns beside it: category assignments, attribute
//! definitions and values, and the ungoverned metadata map (`design/02` §3.1
//! `inst-tx-name-in-parent`, §4.1; **P-D-06**, **P-D-47**, **P-D-50**,
//! **P-D-88**).
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
use sea_orm::sea_query::{Expr, ExprTrait, OnConflict};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureDeleteExt, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use super::{TERMINAL_HEAD_STATES, driver_failure};
use crate::domain::error::DomainError;
use crate::domain::taxonomy::{
    AssignmentRole, CategoryState, DefinitionState, DeleteCensus, REGISTRY_SEEDED_BY, RetireCensus,
    StaleCategoryToken, TaxonomyMutation, WELL_KNOWN_SEEDS,
};
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    attribute_definition, attribute_value, category, metadata, product, product_category, sku,
};

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

// -- Category assignments (`products_product_category`) --
//
// `dod-category-assignment-table` makes this table the **single source of
// truth** for membership, and puts both of its guarantees in indexes rather
// than in application code:
//
// - `uq_products_product_category` -- one Product cannot hold one category
//   twice, in one role or in two;
// - `uq_products_product_category_primary` -- at most one `primary` per
//   Product, a partial index `WHERE role = 'primary'`.
//
// So this module never reads to check either one. It writes and reads the
// violation back, exactly as [`classify_category_write`] does for the tree,
// and for the same reason: a read-then-write check answers from a snapshot
// two writers share.

/// One row of a Product's assignment set, read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryAssignment {
    /// The category the Product is filed under.
    pub category_id: Uuid,
    /// `primary` or `secondary`, parsed fail-closed.
    pub role: AssignmentRole,
    /// When the assignment was written.
    pub assigned_at: DateTime<Utc>,
}

/// Which index refused an assignment write.
///
/// **This is not an error code, deliberately.** §7 row 17 records that four
/// refusals in this feature *"have no code"*, and the primary/secondary
/// duplicate is one of them. Minting one here would answer that row from the
/// storage layer; returning a named outcome leaves the code to the door once
/// its owner assigns one, and still lets a caller tell the two conflicts
/// apart today.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum AssignmentWrite {
    /// Every row in the set landed.
    Applied,
    /// A second `primary` — `uq_products_product_category_primary`.
    PrimaryConflict,
    /// The same category named twice — `uq_products_product_category`.
    DuplicateCategory,
}

/// Replace a Product's whole assignment set, in the caller's transaction.
///
/// The save door writes content rows *"in the same transaction"* as the head
/// (`dod-save-door`, **P-D-46**), so this takes a [`DBRunner`] and never opens
/// one of its own: a set written on a runner of its own would survive a
/// rolled-back save.
///
/// **Replace rather than merge**, because the payload is the set. A merge
/// would leave a category the operator removed from the payload still filed,
/// and the door has no second call with which to notice.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure. A uniqueness refusal is
/// **not** an error: it is an [`AssignmentWrite`] the caller classifies, for
/// the reason that type's doc gives.
pub async fn replace_category_assignments(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
    assignments: &[(Uuid, AssignmentRole)],
    now: DateTime<Utc>,
) -> Result<AssignmentWrite, RepoError> {
    product_category::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product_category::Column::TenantId.eq(tenant_id))
                .add(product_category::Column::ProductId.eq(product_id)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("clear the assignments of {product_id}"), e))?;

    for (category_id, role) in assignments {
        let model = product_category::ActiveModel {
            tenant_id: Set(tenant_id),
            product_id: Set(product_id),
            category_id: Set(*category_id),
            role: Set(role.as_str().to_owned()),
            assigned_at: Set(now),
        };
        let outcome = product_category::Entity::insert(model.clone())
            .secure()
            .scope_with_model(scope, &model)
            .map_err(|e| driver_failure(format!("assignment scope of {tenant_id}"), e))?
            .exec(runner)
            .await;
        if let Err(e) = outcome {
            let failure = driver_failure(
                format!("assign category {category_id} to product {product_id}"),
                e,
            );
            return match classify_assignment_write(&failure) {
                Some(conflict) => Ok(conflict),
                None => Err(failure),
            };
        }
    }
    Ok(AssignmentWrite::Applied)
}

/// One Product's assignment set, ordered by category id.
///
/// Ordered so two reads of one unchanged set are the same list. This is the
/// **read**'s determinism and answers nothing about §7 row 9, which is about
/// the frozen-content sort key `01-foundation` §4.3 and **P-D-29** state.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure; [`RepoError::CorruptRow`] on
/// a `role` outside the roster the `CHECK` admits.
pub async fn category_assignments(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
) -> Result<Vec<CategoryAssignment>, RepoError> {
    let rows = product_category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product_category::Column::TenantId.eq(tenant_id))
                .add(product_category::Column::ProductId.eq(product_id)),
        )
        .order_by(product_category::Column::CategoryId, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the assignments of {product_id}"), e))?;
    rows.into_iter()
        .map(|row| {
            let role = AssignmentRole::parse(&row.role).ok_or_else(|| {
                RepoError::CorruptRow(format!(
                    "products_product_category.role `{}` on {}/{}",
                    row.role, row.product_id, row.category_id
                ))
            })?;
            Ok(CategoryAssignment {
                category_id: row.category_id,
                role,
                assigned_at: row.assigned_at,
            })
        })
        .collect()
}

/// The stored state of each named category, for the save door's subject.
///
/// Answers only the ids it was given, so a caller can tell *"this id resolved
/// to nothing"* from *"this id resolved to a retired node"* — the two are
/// different refusals (`CategoryResolvableRule` and `CategoryNotRetiredRule`)
/// and a read that silently dropped the unresolvable ones would collapse them
/// into one.
///
/// One statement for the whole set rather than one per id: a save naming four
/// categories would otherwise take four round trips inside the mutation
/// transaction, and the four could disagree with each other under a peer's
/// retire.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure; [`RepoError::CorruptRow`] on a
/// `state` outside the roster the `CHECK` admits.
pub async fn category_states(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    ids: &[Uuid],
) -> Result<Vec<(Uuid, CategoryState)>, RepoError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows = category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(category::Column::TenantId.eq(tenant_id))
                .add(category::Column::CategoryId.is_in(ids.to_vec())),
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("states of {} categories", ids.len()), e))?;
    rows.into_iter()
        .map(|row| {
            let state = CategoryState::parse(&row.state).ok_or_else(|| {
                RepoError::CorruptRow(format!(
                    "products_category.state `{}` on {}",
                    row.state, row.category_id
                ))
            })?;
            Ok((row.category_id, state))
        })
        .collect()
}

/// Name which assignment index refused a write, or `None` where the failure
/// is not a uniqueness one.
///
/// # The two engines say different things, and this was measured
///
/// The first version of this function assumed both engines name the index.
/// **`SQLite` does not.** It names the *columns*, and
/// `a_second_primary_is_refused_by_the_partial_index` reddened with the
/// engine's own text:
///
/// ```text
/// UNIQUE constraint failed: products_product_category.tenant_id, products_product_category.product_id
/// ```
///
/// So there are two message shapes to read, not one:
///
/// | | partial primary index | table-level `UNIQUE` |
/// |---|---|---|
/// | Postgres | `"uq_products_product_category_primary"` | `"uq_products_product_category"` |
/// | `SQLite` | `…tenant_id, …product_id` | `…tenant_id, …product_id, …category_id` |
///
/// # Both orderings are load-bearing, for the same reason
///
/// In each column of that table the primary form's text is a **prefix** of
/// the other's: `uq_products_product_category_primary` contains
/// `uq_products_product_category`, and the three-column list contains the
/// two-column one. So the narrower test must come first on each engine, or
/// every second primary reads as a duplicate category — which is exactly what
/// `the_primary_conflict_is_not_read_as_a_duplicate` holds, by asserting the
/// two outcomes against **each other** rather than each alone.
fn classify_assignment_write(error: &RepoError) -> Option<AssignmentWrite> {
    let message = error.to_string().to_ascii_lowercase();

    // Postgres: the constraint is named. `_primary` first -- see the doc.
    if message.contains("uq_products_product_category_primary") {
        return Some(AssignmentWrite::PrimaryConflict);
    }
    if message.contains("uq_products_product_category")
        || message.contains("products_product_category_pkey")
    {
        return Some(AssignmentWrite::DuplicateCategory);
    }

    // `SQLite`: the columns are named. Gate on the failure being a uniqueness
    // one first, so a foreign-key or CHECK message that happens to carry the
    // table name is never read as a conflict.
    if !(message.contains("unique constraint") || message.contains("duplicate key")) {
        return None;
    }
    // `category_id` first: the three-column list contains the two-column one.
    if message.contains("products_product_category.category_id") {
        return Some(AssignmentWrite::DuplicateCategory);
    }
    if message.contains("products_product_category.product_id") {
        return Some(AssignmentWrite::PrimaryConflict);
    }
    None
}

// -- Attribute definitions (`products_attribute_definition`) --

/// One definition row as the doors and the validators read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeDefinitionRecord {
    /// The row's own id.
    pub definition_id: Uuid,
    /// The tenant-unique key an author names the attribute by.
    pub key: String,
    /// The declared shape. **A `String`, not an enum**: no document
    /// enumerates the admitted types and the DDL pins non-emptiness only
    /// (P-D-74's shape, `design/02` §6). Closing it here would author that
    /// answer.
    pub value_type: String,
    /// Whether values carry locale coordinates.
    pub localized: bool,
    /// The region visibility set, stored in P-D-39's rendering — **the empty
    /// string is unrestricted, not empty**. Read it through
    /// [`crate::domain::containment::ResolvedScope::parse`]; a containment
    /// test written as membership alone hides every unrestricted row.
    pub region_scope: String,
    /// The brand visibility set, same reading.
    pub brand_scope: String,
    /// `active`, `deprecated` or `removed`, parsed fail-closed.
    pub state: DefinitionState,
    /// The well-known marker, `None` for an operator-added definition. A
    /// seeded definition is deprecatable and never removable.
    pub seeded_by: Option<String>,
}

/// One new definition, as the store needs it.
#[derive(Clone, Debug)]
pub struct NewAttributeDefinition<'a> {
    /// The tenant.
    pub tenant_id: Uuid,
    /// The row's own id.
    pub definition_id: Uuid,
    /// Unique per tenant — `uq_products_attribute_definition_key` decides
    /// the race, not a prior read.
    pub key: &'a str,
    /// See [`AttributeDefinitionRecord::value_type`] on why this is a string.
    pub value_type: &'a str,
    /// Whether values carry locale coordinates.
    pub localized: bool,
    /// P-D-39 rendering: `""` is unrestricted.
    pub region_scope: &'a str,
    /// P-D-39 rendering: `""` is unrestricted.
    pub brand_scope: &'a str,
    /// `Some("registry")` for a well-known seed, `None` otherwise.
    pub seeded_by: Option<&'a str>,
}

fn into_definition(
    row: attribute_definition::Model,
) -> Result<AttributeDefinitionRecord, RepoError> {
    let state = DefinitionState::parse(&row.state).ok_or_else(|| {
        RepoError::CorruptRow(format!(
            "products_attribute_definition.state `{}` on {}",
            row.state, row.definition_id
        ))
    })?;
    Ok(AttributeDefinitionRecord {
        definition_id: row.definition_id,
        key: row.key,
        value_type: row.value_type,
        localized: row.localized,
        region_scope: row.region_scope,
        brand_scope: row.brand_scope,
        state,
        seeded_by: row.seeded_by,
    })
}

/// Insert one `active` definition, letting the key index decide the race.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, the tenant-unique key
/// conflict included — the door classifies it, this store does not widen a
/// driver error into a domain refusal.
pub async fn insert_attribute_definition(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewAttributeDefinition<'_>,
    now: DateTime<Utc>,
) -> Result<AttributeDefinitionRecord, RepoError> {
    let model = attribute_definition::ActiveModel {
        tenant_id: Set(new.tenant_id),
        definition_id: Set(new.definition_id),
        key: Set(new.key.to_owned()),
        value_type: Set(new.value_type.to_owned()),
        localized: Set(new.localized),
        region_scope: Set(new.region_scope.to_owned()),
        brand_scope: Set(new.brand_scope.to_owned()),
        state: Set(DefinitionState::Active.as_str().to_owned()),
        seeded_by: Set(new.seeded_by.map(str::to_owned)),
        created_at: Set(now),
        updated_at: Set(now),
    };
    attribute_definition::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("definition scope of {}", new.tenant_id), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("define attribute {}", new.key), e))?;
    into_definition(attribute_definition::Model {
        tenant_id: new.tenant_id,
        definition_id: new.definition_id,
        key: new.key.to_owned(),
        value_type: new.value_type.to_owned(),
        localized: new.localized,
        region_scope: new.region_scope.to_owned(),
        brand_scope: new.brand_scope.to_owned(),
        state: DefinitionState::Active.as_str().to_owned(),
        seeded_by: new.seeded_by.map(str::to_owned),
        created_at: now,
        updated_at: now,
    })
}

/// Read one definition by its tenant-unique key, or `None`.
///
/// **Every state, including `removed`.** A tombstone is a row the resolver
/// still has to see — a value on a terminal head keeps resolving past its
/// definition's removal (`inst-de-edge-remove`) — so filtering the roster
/// here would make that unreachable. Set membership is the caller's
/// judgement, over [`AttributeDefinitionRecord::state`].
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure; [`RepoError::CorruptRow`] on
/// a state outside the roster.
pub async fn attribute_definition_by_key(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    key: &str,
) -> Result<Option<AttributeDefinitionRecord>, RepoError> {
    let row = attribute_definition::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(attribute_definition::Column::TenantId.eq(tenant_id))
                .add(attribute_definition::Column::Key.eq(key)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read attribute definition {key}"), e))?;
    row.map(into_definition).transpose()
}

/// One tenant's whole definition roster, ordered by key. **A pure read.**
///
/// It seeded the well-known five until **P-D-104**, which moved that off the
/// read path: a lazy read-through means a `GET` writes, a read-only replica
/// breaks, and the first reader of a tenant pays a write it did not ask for.
/// [`seed_well_known_definitions`] is the writer now, and the door calls it on
/// the write path.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure; [`RepoError::CorruptRow`] on
/// a state outside the roster.
pub async fn attribute_definitions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<AttributeDefinitionRecord>, RepoError> {
    let rows = attribute_definition::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(attribute_definition::Column::TenantId.eq(tenant_id)))
        .order_by(attribute_definition::Column::Key, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the definition roster of {tenant_id}"), e))?;
    rows.into_iter().map(into_definition).collect()
}

/// Materialise [`crate::domain::taxonomy::WELL_KNOWN_SEEDS`] for one tenant
/// that has no definition rows at all (**P-D-100**, as amended by
/// **P-D-104**).
///
/// # One writer, on a write path
///
/// `products_attribute_definition` is per-tenant, so `dod-well-known-seeds`'
/// five are five rows **per tenant**. P-D-100 first split the work into a
/// migration for tenants present at deploy and a read-through for the rest;
/// P-D-104 withdrew both halves of that split, on two measurements. The
/// migration arm is **unbuildable** — seeding a per-tenant store needs a list
/// of tenants and no gear's schema has a tenant registry, and no migration in
/// the workspace inserts a row at all. And it was redundant: the condition
/// below is *"this tenant has no seed rows"*, never *"this tenant is new"*, so
/// one writer always reached a pre-deploy tenant just as readily. The
/// old-versus-new split was reading a distinction the condition never made.
///
/// # Empty is the trigger, and it is not "the five are missing"
///
/// A tenant that has **deprecated** one of the five is not re-seeded.
/// Re-materialising a definition an operator deliberately moved out of the way
/// would undo their act, and the state flip is the only removal there is — so
/// they would have no way left to say no. The caller passes the roster it has
/// already read, so the common path costs no extra statement.
///
/// # Idempotent under a race, by the index
///
/// Two concurrent first writes both see an empty roster and both insert.
/// `uq_products_attribute_definition_key` admits one of each key, so the loser
/// takes a conflict — swallowed here, because a tenant whose seeds already
/// exist is the outcome the caller wanted. Each row is inserted on its own so
/// one key's conflict does not lose the other four.
///
/// # Errors
///
/// [`RepoError`] on any storage failure that is **not** the key conflict —
/// this function does not widen a driver error into a success.
/// @cpt-dod:cpt-cf-bss-products-dod-well-known-seeds:p1
pub async fn seed_well_known_definitions(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    for seed in WELL_KNOWN_SEEDS {
        let outcome = insert_attribute_definition(
            runner,
            scope,
            NewAttributeDefinition {
                tenant_id,
                definition_id: Uuid::now_v7(),
                key: seed.key,
                value_type: seed.value_type,
                localized: seed.localized,
                region_scope: "",
                brand_scope: "",
                seeded_by: Some(REGISTRY_SEEDED_BY),
            },
            now,
        )
        .await;
        if let Err(error) = outcome {
            let message = error.to_string().to_ascii_lowercase();
            let conflict = message.contains("unique constraint")
                || message.contains("duplicate key")
                || message.contains("uq_products_attribute_definition_key");
            if !conflict {
                return Err(error);
            }
        }
    }
    Ok(())
}

/// One state flip: the state the caller's `GovernedLiveOp` read, and the
/// state to move to.
///
/// Bundled for the reason `recognized::StateFlip` gives — two loose
/// [`DefinitionState`] arguments could be transposed at a call site with
/// nothing to notice.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DefinitionFlip {
    /// The state the caller read — the staleness pin.
    pub expected: DefinitionState,
    /// The target state.
    pub to: DefinitionState,
}

/// Flip one definition's state, pinned at the state the caller expected.
///
/// **This is the only path to `removed`**, which is what
/// `dod-attribute-definition-table` requires: *"the `removed` value MUST be
/// reachable only as a state flip; no migration or door may delete a row"*.
/// The store offers no delete for this table at all, and the table's own
/// `BEFORE DELETE` trigger refuses one on both engines rather than trusting
/// that.
///
/// The pin makes the live-op's staleness rule physical: a peer's flip between
/// the caller's read and this statement leaves `rows_affected = 0` and the
/// door answers `STALE_LIVE_OP` rather than absorbing the race.
///
/// **Which pairs are legal is not decided here.** `inst-de-edge-*` and the
/// seeded-definition refusal are `dod-definition-lifecycle`'s; this statement
/// writes the pair it is given.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure. A vanished or moved row is
/// `Ok(false)`, the caller's to classify.
pub async fn flip_definition_state(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    definition_id: Uuid,
    flip: DefinitionFlip,
    now: DateTime<Utc>,
) -> Result<bool, RepoError> {
    let DefinitionFlip { expected, to } = flip;
    let result = attribute_definition::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            attribute_definition::Column::State,
            Expr::value(to.as_str()),
        )
        .col_expr(attribute_definition::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(attribute_definition::Column::TenantId.eq(tenant_id))
                .add(attribute_definition::Column::DefinitionId.eq(definition_id))
                .add(attribute_definition::Column::State.eq(expected.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("flip definition {definition_id}"), e))?;
    Ok(result.rows_affected == 1)
}

// -- Attribute values (`products_attribute_value`) --

/// One value's full coordinate.
///
/// **`entity_kind` is a `&str` and stays one.** §7 row 20 asks *"what
/// `entity_kind` values does each table admit"* and the migration pinned the
/// column to non-emptiness for exactly that reason; an enum here would answer
/// the row from the storage layer. The kinds in flight today are `product`,
/// `sku` and `category`, and the third is why the question is live: a
/// category's values are the **live state itself**, with no freeze-copy,
/// while Product and SKU rows hold the current head only.
///
/// The three locale coordinates ship `NOT NULL` with `""` as the stated
/// absence (**P-D-88** arm 2), so the key is total and the global coordinate
/// is spelled `("", "", "")`. What `global` *means* to the resolver is §6's,
/// not this type's.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AttributeCoordinate<'a> {
    /// `product`, `sku` or `category` — open, see the type doc.
    pub entity_kind: &'a str,
    /// The owning row's id in whichever table `entity_kind` names.
    pub entity_id: Uuid,
    /// Which definition the value answers to.
    pub definition_id: Uuid,
    /// `""` is absent, not null.
    pub locale: &'a str,
    /// `""` is absent, not null.
    pub region: &'a str,
    /// `""` is absent, not null.
    pub brand: &'a str,
}

/// One stored value, read back with its whole coordinate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttributeValueRecord {
    /// The owning kind, as stored.
    pub entity_kind: String,
    /// The owning row.
    pub entity_id: Uuid,
    /// The definition this value answers to.
    pub definition_id: Uuid,
    /// `""` is absent.
    pub locale: String,
    /// `""` is absent.
    pub region: String,
    /// `""` is absent.
    pub brand: String,
    /// The value itself.
    pub value: String,
    /// When it was last written.
    pub updated_at: DateTime<Utc>,
}

/// Write one value at its coordinate, overwriting whatever stood there.
///
/// An upsert on the full seven-column key rather than a read-then-write, so
/// two authors racing on one coordinate produce one row and a last-writer
/// value instead of a driver conflict the door would have to translate. The
/// definition FK is real and refuses a value against a definition the tenant
/// never declared.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, the definition FK included.
pub async fn upsert_attribute_value(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    coordinate: AttributeCoordinate<'_>,
    value: &str,
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let model = attribute_value::ActiveModel {
        tenant_id: Set(tenant_id),
        entity_kind: Set(coordinate.entity_kind.to_owned()),
        entity_id: Set(coordinate.entity_id),
        definition_id: Set(coordinate.definition_id),
        locale: Set(coordinate.locale.to_owned()),
        region: Set(coordinate.region.to_owned()),
        brand: Set(coordinate.brand.to_owned()),
        value: Set(value.to_owned()),
        updated_at: Set(now),
    };
    let on_conflict = OnConflict::columns([
        attribute_value::Column::TenantId,
        attribute_value::Column::EntityKind,
        attribute_value::Column::EntityId,
        attribute_value::Column::DefinitionId,
        attribute_value::Column::Locale,
        attribute_value::Column::Region,
        attribute_value::Column::Brand,
    ])
    .update_columns([
        attribute_value::Column::Value,
        attribute_value::Column::UpdatedAt,
    ])
    .to_owned();

    attribute_value::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("attribute value scope of {tenant_id}"), e))?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!(
                    "write attribute value {}/{} on {}",
                    coordinate.definition_id, coordinate.locale, coordinate.entity_id
                ),
                e,
            )
        })?;
    Ok(())
}

/// Every value one entity carries, ordered by the whole coordinate.
///
/// The order is total over the four coordinate columns, so one unchanged set
/// reads back the same way twice. That is this read's own determinism and is
/// **not** an answer to §7 row 9, which is about the frozen-content sort key
/// **P-D-29** and `01-foundation` §4.3 state in the same words — a different
/// site, whose amendment is a register change.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn attribute_values_of(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<Vec<AttributeValueRecord>, RepoError> {
    let rows = attribute_value::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(attribute_value::Column::TenantId.eq(tenant_id))
                .add(attribute_value::Column::EntityKind.eq(entity_kind))
                .add(attribute_value::Column::EntityId.eq(entity_id)),
        )
        .order_by(attribute_value::Column::DefinitionId, sea_orm::Order::Asc)
        .order_by(attribute_value::Column::Locale, sea_orm::Order::Asc)
        .order_by(attribute_value::Column::Region, sea_orm::Order::Asc)
        .order_by(attribute_value::Column::Brand, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the values of {entity_kind} {entity_id}"), e))?;
    Ok(rows
        .into_iter()
        .map(|row| AttributeValueRecord {
            entity_kind: row.entity_kind,
            entity_id: row.entity_id,
            definition_id: row.definition_id,
            locale: row.locale,
            region: row.region,
            brand: row.brand,
            value: row.value,
            updated_at: row.updated_at,
        })
        .collect())
}

/// Remove one value at its coordinate. `false` where no row stood there.
///
/// A value row is content and carries no tombstone requirement — the
/// no-delete rule (**P-D-47**) is the **definition** table's, whose removal
/// is a state flip so that these rows are never orphaned.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn delete_attribute_value(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    coordinate: AttributeCoordinate<'_>,
) -> Result<bool, RepoError> {
    let result = attribute_value::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(attribute_value::Column::TenantId.eq(tenant_id))
                .add(attribute_value::Column::EntityKind.eq(coordinate.entity_kind))
                .add(attribute_value::Column::EntityId.eq(coordinate.entity_id))
                .add(attribute_value::Column::DefinitionId.eq(coordinate.definition_id))
                .add(attribute_value::Column::Locale.eq(coordinate.locale))
                .add(attribute_value::Column::Region.eq(coordinate.region))
                .add(attribute_value::Column::Brand.eq(coordinate.brand)),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!(
                    "remove attribute value {} on {}",
                    coordinate.definition_id, coordinate.entity_id
                ),
                e,
            )
        })?;
    Ok(result.rows_affected == 1)
}

// -- The metadata map (`products_metadata`) --
//
// Ungoverned, and **outside frozen version content** (**P-D-06**): a write
// here bumps no revision and lands in no `products_entity_version` row. The
// caps `METADATA_LIMIT` names are the door's, read from configuration --
// §7 row 2 records that neither the key count nor the value length has a
// value anywhere yet, so nothing is enforced here.

/// One metadata entry, read back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MetadataEntry {
    /// The map key.
    pub key: String,
    /// The value.
    pub value: String,
    /// When the key was first written.
    pub created_at: DateTime<Utc>,
    /// When it was last overwritten.
    pub updated_at: DateTime<Utc>,
}

/// Write one metadata key, overwriting whatever stood there.
///
/// `created_at` is set on insert and **left alone on overwrite**, so the
/// column keeps meaning "when this key first appeared" rather than silently
/// becoming a second copy of `updated_at`.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn upsert_metadata(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
    entry: (&str, &str),
    now: DateTime<Utc>,
) -> Result<(), RepoError> {
    let (key, value) = entry;
    let model = metadata::ActiveModel {
        tenant_id: Set(tenant_id),
        entity_kind: Set(entity_kind.to_owned()),
        entity_id: Set(entity_id),
        key: Set(key.to_owned()),
        value: Set(value.to_owned()),
        created_at: Set(now),
        updated_at: Set(now),
    };
    let on_conflict = OnConflict::columns([
        metadata::Column::TenantId,
        metadata::Column::EntityKind,
        metadata::Column::EntityId,
        metadata::Column::Key,
    ])
    .update_columns([metadata::Column::Value, metadata::Column::UpdatedAt])
    .to_owned();

    metadata::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("metadata scope of {tenant_id}"), e))?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("write metadata {key} on {entity_id}"), e))?;
    Ok(())
}

/// One entity's whole metadata map, ordered by key.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn metadata_of(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
) -> Result<Vec<MetadataEntry>, RepoError> {
    let rows = metadata::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(metadata::Column::TenantId.eq(tenant_id))
                .add(metadata::Column::EntityKind.eq(entity_kind))
                .add(metadata::Column::EntityId.eq(entity_id)),
        )
        .order_by(metadata::Column::Key, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| {
            driver_failure(format!("read the metadata of {entity_kind} {entity_id}"), e)
        })?;
    Ok(rows
        .into_iter()
        .map(|row| MetadataEntry {
            key: row.key,
            value: row.value,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}

/// Remove one metadata key. `false` where no row stood there.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn delete_metadata_key(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: &str,
    entity_id: Uuid,
    key: &str,
) -> Result<bool, RepoError> {
    let result = metadata::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(metadata::Column::TenantId.eq(tenant_id))
                .add(metadata::Column::EntityKind.eq(entity_kind))
                .add(metadata::Column::EntityId.eq(entity_id))
                .add(metadata::Column::Key.eq(key)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("remove metadata {key} on {entity_id}"), e))?;
    Ok(result.rows_affected == 1)
}

// -- The retire and delete guard's census (`inst-tx-retire-guard`) --

/// Read the census a retire or delete is judged against.
///
/// # The two reads are one statement each, and the join is a subquery
///
/// `dod-retire-delete-guard` requires the guard read *"the referencing
/// Product's lifecycle state"* and **not** *"the mere presence of a
/// `products_product_category` row"*. So the Product read is the outer query
/// and the link table is a subquery inside it: the row that decides is the
/// Product's, and a link row held by a discarded draft contributes nothing
/// because its Product never enters the outer result.
///
/// Doing it as two round trips instead — link rows, then their Products —
/// would be wrong in one direction and not the other. A Product **discarded**
/// between the reads is harmless: the second read sees it terminal and the
/// guard correctly lets the retire through. A Product **published** between
/// them is not: its link row did not exist for the first read, so the guard
/// would answer *"unreferenced"* about catalog that now references the node.
/// One statement has no window.
///
/// # `sample + 1`
///
/// Each half is bounded at `sample + 1` so
/// [`crate::domain::taxonomy::retire_verdict`] can say *"at least N"* without
/// a second counting statement whose total could disagree with its own
/// exemplars, which is the failure [`super::member_holders`] records
/// on the sibling guard.
///
/// # The scope this reads under must be the tenant's whole scope
///
/// Both reads are scoped, as every read in this crate is. That is safe only
/// because a narrower scope would hide holders and make the guard answer
/// *"unreferenced"* about a node that is referenced — a fail-**open**
/// direction, and the one direction a delete guard must not fail in. Every
/// door in this gear builds `AccessScope::for_tenant`, so the scope is
/// tenant-wide at every call site that exists; a future narrower scope would
/// need this read exempted rather than merely reviewed.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn retire_census(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
    sample: u64,
) -> Result<RetireCensus, RepoError> {
    let holders_of_category = sea_orm::sea_query::Query::select()
        .column(product_category::Column::ProductId)
        .from(product_category::Entity)
        .and_where(Expr::col(product_category::Column::TenantId).eq(tenant_id))
        .and_where(Expr::col(product_category::Column::CategoryId).eq(category_id))
        .to_owned();

    let referencing_products: Vec<String> = product::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::LifecycleState.is_not_in(TERMINAL_HEAD_STATES))
                .add(Expr::col(product::Column::ProductId).in_subquery(holders_of_category)),
        )
        .order_by(product::Column::Name, sea_orm::Order::Asc)
        .limit(sample + 1)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("holders of category {category_id}"), e))?
        .into_iter()
        .map(|row| row.name)
        .collect();

    let active_children: Vec<String> = category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(category::Column::TenantId.eq(tenant_id))
                .add(category::Column::ParentId.eq(category_id))
                .add(category::Column::State.eq(ACTIVE_CATEGORY_STATE)),
        )
        .order_by(category::Column::Name, sea_orm::Order::Asc)
        .limit(sample + 1)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("children of category {category_id}"), e))?
        .into_iter()
        .map(|row| row.name)
        .collect();

    Ok(RetireCensus {
        referencing_products,
        active_children,
        sample_bound: usize::try_from(sample).unwrap_or(usize::MAX),
    })
}

/// The stored `active` token, the one half of
/// `chk_products_category_state`'s two-value roster this module writes and
/// filters on.
///
/// A literal rather than a parsed enum because the category machine has two
/// states and no edge back from `retired` (`inst-ce-terminal`), so nothing
/// here needs to reason over the roster — only to name the live half.
const ACTIVE_CATEGORY_STATE: &str = "active";

/// Flip one category to `retired`, pinned at `active`.
///
/// The pin is the same mechanism [`flip_definition_state`] uses: a peer that
/// retired the node between the caller's census and this statement leaves
/// `rows_affected = 0`, so the caller answers a staleness refusal rather than
/// reporting a retire it did not perform. `mutation_seq` advances because a
/// retire is an act on the row and P-D-50 counts acts.
///
/// **The guard is not here.** [`retire_census`] reads and
/// [`crate::domain::taxonomy::retire_verdict`] judges; this statement writes
/// what it is told, under the caller's lock.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure. A vanished or already-retired
/// node is `Ok(CategoryWrite::Unmatched)`.
pub async fn retire_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
    now: DateTime<Utc>,
) -> Result<CategoryWrite, RepoError> {
    let result = category::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            category::Column::State,
            Expr::value(CategoryState::Retired.as_str()),
        )
        .col_expr(
            category::Column::MutationSeq,
            Expr::col(category::Column::MutationSeq).add(1),
        )
        .col_expr(category::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(category::Column::TenantId.eq(tenant_id))
                .add(category::Column::CategoryId.eq(category_id))
                .add(category::Column::State.eq(ACTIVE_CATEGORY_STATE)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("retire category {category_id}"), e))?;
    Ok(CategoryWrite::from_rows(result.rows_affected))
}

/// Delete one retired category row — the single physical removal this feature
/// performs (`inst-ce-terminal`).
///
/// Filtered on `state = 'retired'`, so the machine's own order is physical:
/// `active -> retired -> (row deleted)`, with no path that deletes a live
/// node. The children half of *"retired + empty + unreferenced"* is also the
/// parent foreign key's, which refuses the delete while any child row still
/// points at it — including a **retired** child, which
/// [`retire_census`] deliberately does not count. The two are not in conflict:
/// the census decides whether the act is *admitted*, the FK decides whether it
/// is *possible*, and a retired child makes it inadmissible to nobody and
/// impossible to the engine. The caller retires and deletes depth-first.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure, the parent FK's refusal
/// included — a caller that skipped the census meets the engine instead.
pub async fn delete_retired_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
) -> Result<CategoryWrite, RepoError> {
    let result = category::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(category::Column::TenantId.eq(tenant_id))
                .add(category::Column::CategoryId.eq(category_id))
                .add(category::Column::State.eq(CategoryState::Retired.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("delete category {category_id}"), e))?;
    Ok(CategoryWrite::from_rows(result.rows_affected))
}

/// Read the census a **physical delete** is judged against (**P-D-116 row 21**).
///
/// Presence, not state: every `products_product_category` row naming the node
/// counts, whatever its Product's lifecycle state, and every child row counts,
/// whatever its state. That is the opposite reading from [`retire_census`]
/// and deliberately so -- the retire guard must not lock a category behind a
/// discarded draft's link row, and the delete must not leave that row pointing
/// at nothing in the table the design calls the single source of truth. Both
/// statements are one query each, bounded at `sample + 1`, under the caller's
/// lock, for [`retire_census`]'s reasons.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn delete_census(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
    sample: u64,
) -> Result<DeleteCensus, RepoError> {
    let holders_of_category = sea_orm::sea_query::Query::select()
        .column(product_category::Column::ProductId)
        .from(product_category::Entity)
        .and_where(Expr::col(product_category::Column::TenantId).eq(tenant_id))
        .and_where(Expr::col(product_category::Column::CategoryId).eq(category_id))
        .to_owned();

    let linked_products: Vec<String> = product::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(Expr::col(product::Column::ProductId).in_subquery(holders_of_category)),
        )
        .order_by(product::Column::Name, sea_orm::Order::Asc)
        .limit(sample + 1)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("link rows of category {category_id}"), e))?
        .into_iter()
        .map(|row| format!("{} ({})", row.name, row.lifecycle_state))
        .collect();

    let children: Vec<String> = category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(category::Column::TenantId.eq(tenant_id))
                .add(category::Column::ParentId.eq(category_id)),
        )
        .order_by(category::Column::Name, sea_orm::Order::Asc)
        .limit(sample + 1)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("child rows of category {category_id}"), e))?
        .into_iter()
        .map(|row| format!("{} ({})", row.name, row.state))
        .collect();

    Ok(DeleteCensus {
        linked_products,
        children,
        sample_bound: usize::try_from(sample).unwrap_or(usize::MAX),
    })
}

/// Everything still carrying a value for one definition -- the removal
/// operand `dod-definition-lifecycle` defines.
///
/// # Three statements, because the key is polymorphic
///
/// `entity_kind` spans three tables, which is the same reason
/// `products_attribute_value` carries no foreign key to the owning entity:
/// *"Three kinds live in three tables, so no single FK can cover the
/// coordinate"*. So this reads Products, SKUs and categories separately and
/// concatenates. **A head created between two of the three reads is not
/// seen** -- the window is real and is why the caller runs this inside the
/// apply transaction of a human-paced, approved `GovernedLiveOp` rather than
/// on a hot path.
///
/// # What counts, and what does not
///
/// - **Products and SKUs**: non-terminal heads only. `dod-definition-lifecycle`
///   names the *"non-terminal head"* as the operand, and the `DoD`'s own probe
///   requires removal be **admitted** while only a frozen version carries a
///   value -- so a discarded or retired head's row must not block.
/// - **Categories**: `active` ones count. A category has no lifecycle state
///   and its values are the live state itself, so there is no terminal reading
///   available; `design/02` §6 records that the removal guard *"counts an
///   active category as a value-carrying head"*, which is what this does.
///
/// The `entity_kind` tokens are literals here rather than a parsed roster,
/// because §7 row 20 is the live question of what that column admits and an
/// enum would answer it. What this function claims is narrower and true
/// whatever the roster becomes: *these three kinds*, read this way.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn definition_value_holders(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    definition_id: Uuid,
    sample: u64,
) -> Result<Vec<String>, RepoError> {
    let carriers = |kind: &str| {
        sea_orm::sea_query::Query::select()
            .column(attribute_value::Column::EntityId)
            .from(attribute_value::Entity)
            .and_where(Expr::col(attribute_value::Column::TenantId).eq(tenant_id))
            .and_where(Expr::col(attribute_value::Column::DefinitionId).eq(definition_id))
            .and_where(Expr::col(attribute_value::Column::EntityKind).eq(kind))
            .to_owned()
    };

    let mut holders: Vec<String> = product::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::LifecycleState.is_not_in(TERMINAL_HEAD_STATES))
                .add(Expr::col(product::Column::ProductId).in_subquery(carriers("product"))),
        )
        .order_by(product::Column::Name, sea_orm::Order::Asc)
        .limit(sample + 1)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("product carriers of {definition_id}"), e))?
        .into_iter()
        .map(|row| row.name)
        .collect();

    holders.extend(
        sku::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(sku::Column::TenantId.eq(tenant_id))
                    .add(sku::Column::LifecycleState.is_not_in(TERMINAL_HEAD_STATES))
                    .add(Expr::col(sku::Column::SkuId).in_subquery(carriers("sku"))),
            )
            .order_by(sku::Column::SkuCode, sea_orm::Order::Asc)
            .limit(sample + 1)
            .all(runner)
            .await
            .map_err(|e| driver_failure(format!("sku carriers of {definition_id}"), e))?
            .into_iter()
            .map(|row| row.sku_code),
    );

    holders.extend(
        category::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(category::Column::TenantId.eq(tenant_id))
                    .add(category::Column::State.eq(ACTIVE_CATEGORY_STATE))
                    .add(Expr::col(category::Column::CategoryId).in_subquery(carriers("category"))),
            )
            .order_by(category::Column::Name, sea_orm::Order::Asc)
            .limit(sample + 1)
            .all(runner)
            .await
            .map_err(|e| driver_failure(format!("category carriers of {definition_id}"), e))?
            .into_iter()
            .map(|row| row.name),
    );

    Ok(holders)
}

// -- The category live-value door's store half (`inst-av-category-branch`,
//    **P-D-50**) --

/// Write one category display value under the row's act token.
///
/// # The token is spent by the same statement that advances it
///
/// **P-D-50** makes `products_category.mutation_seq` the door's `If-Match`
/// operand and an **act counter**: it advances when a door commits an act on
/// the row and for nothing else, because an approval subject built from an act
/// identity must render the same subject on the approved retry. So the counter
/// bump carries the caller's expected value in its own `WHERE` clause. A peer
/// act between the caller's read and this statement leaves
/// `rows_affected = 0`, and the caller answers
/// [`crate::domain::taxonomy::StaleCategoryToken`] rather than absorbing the
/// race — the read-then-write window a separate check would leave open is
/// exactly what the token exists to close.
///
/// The value write runs **after** the token is won, on the caller's runner, so
/// the pair is one transaction if the caller opened one. `inst-av-category-branch`
/// requires the event in that same transaction too; this function does not
/// enqueue it — the **door** does, on the same runner after the value writes
/// (`events::enqueue_taxonomy`, `CategoryDisplayUpdated` on the category's own
/// id with the token this statement won, since 2026-09-03).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure. A lost token is
/// `Ok(Err(StaleCategoryToken))`, not an error: the act was judged and
/// Take the category's act token, or refuse the caller's precondition.
///
/// A **compare-and-set**, not a read then a write: the `UPDATE` carries
/// `mutation_seq = expected_seq` in its own `WHERE`, so two patches quoting
/// one token cannot both be admitted. A door that read the token, judged it
/// and then wrote would leave exactly the window `STALE_CATEGORY_TOKEN`
/// exists to close.
///
/// One bump per **act** and never per row write (**P-D-50**): a patch
/// carrying six coordinates moves the token by one, which is what makes the
/// token quotable by a caller that wrote six values and read one number back.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure. A `Ok(Err(..))` is the
/// caller's precondition failing, which is a refusal and not an error.
pub async fn bump_category_mutation_seq(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
    expected_seq: i64,
    now: DateTime<Utc>,
) -> Result<Result<i64, StaleCategoryToken>, RepoError> {
    let taken = category::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            category::Column::MutationSeq,
            Expr::col(category::Column::MutationSeq).add(1),
        )
        .col_expr(category::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(category::Column::TenantId.eq(tenant_id))
                .add(category::Column::CategoryId.eq(category_id))
                .add(category::Column::MutationSeq.eq(expected_seq)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("category token of {category_id}"), e))?;

    if taken.rows_affected == 0 {
        // Read the row back to report what the token actually stands at. A
        // vanished row answers the same refusal with the sentinel: the
        // caller's precondition did not hold either way, and a 404 is the
        // door's to prefer once it re-reads.
        let found = category_mutation_seq(runner, scope, tenant_id, category_id)
            .await?
            .unwrap_or(-1);
        return Ok(Err(StaleCategoryToken {
            expected: expected_seq,
            found,
        }));
    }
    Ok(Ok(expected_seq.saturating_add(1)))
}

/// refused, which is a domain answer.
pub async fn write_category_display_value(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
    expected_seq: i64,
    value: (Uuid, &str),
    now: DateTime<Utc>,
) -> Result<Result<i64, StaleCategoryToken>, RepoError> {
    let (definition_id, text) = value;
    // One CAS, shared with the live-value door: two copies of a
    // compare-and-set are two chances for one of them to decay into a read
    // then a write, which is the window `STALE_CATEGORY_TOKEN` exists to
    // close.
    let bumped =
        match bump_category_mutation_seq(runner, scope, tenant_id, category_id, expected_seq, now)
            .await?
        {
            Ok(seq) => seq,
            Err(stale) => return Ok(Err(stale)),
        };

    upsert_attribute_value(
        runner,
        scope,
        tenant_id,
        AttributeCoordinate {
            entity_kind: CATEGORY_ENTITY_KIND,
            entity_id: category_id,
            definition_id,
            locale: "",
            region: "",
            brand: "",
        },
        text,
        now,
    )
    .await?;

    Ok(Ok(bumped))
}

/// The `entity_kind` a category's own values carry.
///
/// A literal, not a parsed roster: §7 row 20 is the live question of what that
/// column admits, and an enum would answer it. What this constant claims is
/// only that *this* kind is spelled this way.
const CATEGORY_ENTITY_KIND: &str = "category";

/// Read one category's act counter — the token a door hands back as an
/// `ETag`.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn category_mutation_seq(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    category_id: Uuid,
) -> Result<Option<i64>, RepoError> {
    Ok(category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(category::Column::TenantId.eq(tenant_id))
                .add(category::Column::CategoryId.eq(category_id)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("category seq of {category_id}"), e))?
        .map(|row| row.mutation_seq))
}

#[cfg(test)]
#[path = "taxonomy_tests.rs"]
mod taxonomy_tests;

/// Every category of a tenant as `(id, parent, name)` — the projector's
/// operand for rendering browse paths (`inst-rp-consume`, `inst-rp-reparent`).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn category_nodes(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
) -> Result<Vec<(Uuid, Option<Uuid>, String)>, RepoError> {
    let rows = category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(category::Column::TenantId.eq(tenant_id)))
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("category nodes of {tenant_id}"), e))?;
    Ok(rows
        .into_iter()
        .map(|row| (row.category_id, row.parent_id, row.name))
        .collect())
}
