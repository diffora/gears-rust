//! Repositories for `products_product` and `products_sku` (`design/01-foundation.md` §4.1, §4.2).
//!
//! Phase 1 of the gear's plan: it closes no Definition of Done. It is the
//! enabler every door above it needs, so it carries exactly two operations per
//! table — insert one row, read one back by id — and nothing a later phase
//! would need to undo. In particular there is **no** `ON CONFLICT` handling
//! here: `uq_products_sku_code`'s reservation-by-insert semantics belong to
//! `dod-code-reservation`, and giving a duplicate-key insert typed conflict
//! handling now would be scope taken from that phase. A duplicate insert
//! surfacing as [`RepoError::Db`] is the correct behaviour for this phase —
//! the create door that will call this repository does not exist yet to act
//! on a finer answer.
//!
//! # Free functions, not a provider-holding struct
//!
//! Every write this gear will make joins a multi-row transaction: the create
//! door writes the entity row and its creation outbox row in one transaction
//! (`dod-create-doors`), publish writes the version row and the head update in
//! one, and an audit row commits inside whichever mutation it governs. The
//! toolkit's transaction-bypass guard refuses `Db::conn()` inside an already
//! open transaction, so a repository that owned its own connection could not
//! be called from any of those callers — it would not merely take the wrong
//! runner, it could not run at all on the only path those doors will use. The
//! sibling pricing gear's `pin_frontier_repo` module doc states the identical
//! rule for the identical reason. These functions therefore take the caller's
//! `runner: &impl DBRunner` and never acquire one of their own. A
//! provider-holding struct gets added when, and only when, a caller needs one
//! with no transaction open — no caller does yet.

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

use bss_products_sdk::models::LifecycleState;

use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{product, sku};

/// The row an insert of `products_product` supplies.
///
/// Distinct from [`product::ActiveModel`]: the lifecycle and version columns
/// are not caller inputs. Every created Product starts `draft`,
/// `internal_revision = 1`, `published_version = 0`
/// (`dod-create-doors`), so this repository sets them rather than trusting a
/// caller to.
#[derive(Clone, Debug)]
pub struct NewProduct {
    /// Server-minted by the create door, never caller-supplied.
    pub product_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The brand the Product belongs to.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored.
    pub name: String,
    /// NFKC, full casefold, whitespace-collapsed — computed by the caller so
    /// both engines store identical bytes.
    pub name_normalized: String,
    /// The optional external mapping code.
    pub product_code: Option<String>,
    /// The region value set from the payload, or empty for unrestricted.
    pub region_scope: String,
    /// The brand value set from the payload, or empty for unrestricted.
    pub brand_scope: String,
    /// The pseudonymous ref of whoever created the row.
    pub created_by: String,
    /// The commit instant; `updated_at` starts equal to it.
    pub created_at: DateTime<Utc>,
}

/// A Product as this repository hands it back.
///
/// Distinct from [`product::Model`]: `lifecycle_state` is carried as the
/// SDK's [`LifecycleState`] rather than the raw column string, because every
/// caller of this repository reasons about the enum and never about the
/// token a driver returned.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProductRecord {
    /// The row's own id.
    pub product_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The brand the Product belongs to.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored.
    pub name: String,
    /// The normalized form the uniqueness index compares.
    pub name_normalized: String,
    /// The optional external mapping code.
    pub product_code: Option<String>,
    /// Where the entity sits in the lifecycle machine.
    pub lifecycle_state: LifecycleState,
    /// Moves on every admitted write.
    pub internal_revision: i64,
    /// Moves only on publish.
    pub published_version: i64,
    /// The region value set. Empty means unrestricted.
    pub region_scope: String,
    /// The brand value set. Empty means unrestricted.
    pub brand_scope: String,
    /// The pseudonymous ref of whoever created the row.
    pub created_by: String,
    /// The commit instant.
    pub created_at: DateTime<Utc>,
    /// The instant of the row's last admitted write.
    pub updated_at: DateTime<Utc>,
}

/// The row an insert of `products_sku` supplies.
///
/// Distinct from [`sku::ActiveModel`], for [`NewProduct`]'s reason: the
/// lifecycle and version columns are this repository's to set, not the
/// caller's.
#[derive(Clone, Debug)]
pub struct NewSku {
    /// Server-minted by the create door, never caller-supplied.
    pub sku_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The parent Product.
    pub product_id: Uuid,
    /// Tenant-unique among non-discarded rows, reserved by the insert itself.
    pub sku_code: String,
    /// The region value set, contained in the parent's.
    pub region_scope: String,
    /// The brand value set, contained in the parent's.
    pub brand_scope: String,
    /// The pseudonymous ref of whoever created the row.
    pub created_by: String,
    /// The commit instant; `updated_at` starts equal to it.
    pub created_at: DateTime<Utc>,
}

/// A SKU as this repository hands it back.
///
/// Distinct from [`sku::Model`], for [`ProductRecord`]'s reason.
#[allow(clippy::struct_field_names)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SkuRecord {
    /// The row's own id.
    pub sku_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The parent Product.
    pub product_id: Uuid,
    /// Tenant-unique among non-discarded rows.
    pub sku_code: String,
    /// Where the entity sits in the lifecycle machine.
    pub lifecycle_state: LifecycleState,
    /// Moves on every admitted write.
    pub internal_revision: i64,
    /// Moves only on publish.
    pub published_version: i64,
    /// The region value set. Empty means unrestricted.
    pub region_scope: String,
    /// The brand value set. Empty means unrestricted.
    pub brand_scope: String,
    /// The pseudonymous ref of whoever created the row.
    pub created_by: String,
    /// The commit instant.
    pub created_at: DateTime<Utc>,
    /// The instant of the row's last admitted write.
    pub updated_at: DateTime<Utc>,
}

/// Insert one `products_product` row and read it back as authored
/// (`dod-create-doors`).
///
/// # Errors
/// [`RepoError::Db`] on a scope failure or a `CHECK`/uniqueness violation the
/// database refuses the insert for — including a duplicate `(tenant_id,
/// brand_id, name_normalized)` or a duplicate `product_code`, which this
/// phase reports undifferentiated because no caller yet exists to act on a
/// finer answer.
pub async fn insert_product(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewProduct,
) -> Result<ProductRecord, RepoError> {
    let model = product::ActiveModel {
        product_id: Set(new.product_id),
        tenant_id: Set(new.tenant_id),
        brand_id: Set(new.brand_id),
        name: Set(new.name),
        name_normalized: Set(new.name_normalized),
        product_code: Set(new.product_code),
        lifecycle_state: Set(LifecycleState::Draft.as_str().to_owned()),
        internal_revision: Set(1),
        published_version: Set(0),
        region_scope: Set(new.region_scope),
        brand_scope: Set(new.brand_scope),
        created_by: Set(new.created_by),
        created_at: Set(new.created_at),
        updated_at: Set(new.created_at),
    };

    let row = product::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| RepoError::Db(format!("product {} scope: {e}", new.product_id)))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| RepoError::Db(format!("insert product {}: {e}", new.product_id)))?;

    into_product_record(row)
}

/// Read one Product by id, within `tenant_id`'s scope.
///
/// Answers `Ok(None)` both when no such row exists and when a row exists but
/// outside `scope` — see [`RepoError::NotFound`]'s doc for why those two
/// cases are one answer here as well.
///
/// # Errors
/// [`RepoError::Db`] on a storage failure; [`RepoError::CorruptRow`] when the
/// stored `lifecycle_state` is outside the enumeration [`LifecycleState`]
/// parses.
pub async fn find_product(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
) -> Result<Option<ProductRecord>, RepoError> {
    let row = product::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::ProductId.eq(product_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read product {product_id}: {e}")))?;

    row.map(into_product_record).transpose()
}

/// Read a stored `products_product` row into this repository's vocabulary.
fn into_product_record(row: product::Model) -> Result<ProductRecord, RepoError> {
    let lifecycle_state = LifecycleState::parse(&row.lifecycle_state).ok_or_else(|| {
        RepoError::CorruptRow(format!(
            "products_product.lifecycle_state `{}` on product {}",
            row.lifecycle_state, row.product_id
        ))
    })?;

    Ok(ProductRecord {
        product_id: row.product_id,
        tenant_id: row.tenant_id,
        brand_id: row.brand_id,
        name: row.name,
        name_normalized: row.name_normalized,
        product_code: row.product_code,
        lifecycle_state,
        internal_revision: row.internal_revision,
        published_version: row.published_version,
        region_scope: row.region_scope,
        brand_scope: row.brand_scope,
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// Insert one `products_sku` row and read it back as authored
/// (`dod-create-doors`).
///
/// # Errors
/// [`RepoError::Db`] on a scope failure, the `fk_products_sku_product`
/// foreign key, or a duplicate `(tenant_id, sku_code)` — `sku_code`'s
/// reservation-by-insert (`dod-code-reservation`) is the index's job in this
/// phase; this repository does not yet type the conflict.
pub async fn insert_sku(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewSku,
) -> Result<SkuRecord, RepoError> {
    let model = sku::ActiveModel {
        sku_id: Set(new.sku_id),
        tenant_id: Set(new.tenant_id),
        product_id: Set(new.product_id),
        sku_code: Set(new.sku_code),
        lifecycle_state: Set(LifecycleState::Draft.as_str().to_owned()),
        internal_revision: Set(1),
        published_version: Set(0),
        region_scope: Set(new.region_scope),
        brand_scope: Set(new.brand_scope),
        created_by: Set(new.created_by),
        created_at: Set(new.created_at),
        updated_at: Set(new.created_at),
    };

    let row = sku::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| RepoError::Db(format!("sku {} scope: {e}", new.sku_id)))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| RepoError::Db(format!("insert sku {}: {e}", new.sku_id)))?;

    into_sku_record(row)
}

/// Read one SKU by id, within `tenant_id`'s scope.
///
/// Answers `Ok(None)` both when no such row exists and when a row exists but
/// outside `scope`, for [`find_product`]'s reason.
///
/// # Errors
/// [`RepoError::Db`] on a storage failure; [`RepoError::CorruptRow`] when the
/// stored `lifecycle_state` is outside the enumeration [`LifecycleState`]
/// parses.
pub async fn find_sku(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
) -> Result<Option<SkuRecord>, RepoError> {
    let row = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::SkuId.eq(sku_id)),
        )
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read sku {sku_id}: {e}")))?;

    row.map(into_sku_record).transpose()
}

/// Read a stored `products_sku` row into this repository's vocabulary.
fn into_sku_record(row: sku::Model) -> Result<SkuRecord, RepoError> {
    let lifecycle_state = LifecycleState::parse(&row.lifecycle_state).ok_or_else(|| {
        RepoError::CorruptRow(format!(
            "products_sku.lifecycle_state `{}` on sku {}",
            row.lifecycle_state, row.sku_id
        ))
    })?;

    Ok(SkuRecord {
        sku_id: row.sku_id,
        tenant_id: row.tenant_id,
        product_id: row.product_id,
        sku_code: row.sku_code,
        lifecycle_state,
        internal_revision: row.internal_revision,
        published_version: row.published_version,
        region_scope: row.region_scope,
        brand_scope: row.brand_scope,
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

#[cfg(test)]
#[path = "repo_tests.rs"]
mod repo_tests;
