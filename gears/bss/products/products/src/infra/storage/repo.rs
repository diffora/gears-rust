//! Repositories for `products_product`, `products_sku`,
//! `products_entity_version`, `products_identity_ref` and
//! `products_audit_log` (`design/01-foundation.md` §4.1, §4.2, §4.3, §4.4;
//! `design/10-retention-erasure.md` `inst-im-map`).
//!
//! Phase 1's `products_product`/`products_sku` functions close no Definition
//! of Done; they are the enabler every door above them needs, so they carry
//! exactly two operations per table — insert one row, read one back by id —
//! and nothing a later phase would need to undo. In particular there is
//! **no** `ON CONFLICT` handling here: `uq_products_sku_code`'s
//! reservation-by-insert semantics belong to `dod-code-reservation`, and
//! giving a duplicate-key insert typed conflict handling now would be scope
//! taken from that phase. A duplicate insert surfacing as
//! [`RepoError::Driver`] is the correct behaviour for this phase — the create
//! door that will call this repository does not exist yet to act on a finer
//! answer.
//!
//! Phase 2 Slice C adds [`resolve_actor_ref`], which closes the code half of
//! `dod-actor-ref`. Slice D, the phase's last, adds [`AuditEntry`] and the
//! three writing disciplines that consume the `actor_ref` it resolves —
//! [`write_refusal_audit`], [`write_eventless_act_audit`] and
//! [`write_elevated_read_audit`] — closing `dod-audit-trail`
//! (`design/01-foundation.md` §4.4).
//!
//! Phase 5 Slice C1 adds [`claim_idempotency_key`], the mechanism half of
//! `dod-idempotency-store` (`design/01-foundation.md` §3.2). Wiring it into
//! the create doors — computing the payload hash, deciding
//! `IDEMPOTENCY_CONFLICT` against an [`IdempotencyClaim::Answered`] or an
//! [`IdempotencyClaim::InFlight`] hash mismatch, and raising
//! `IDEMPOTENCY_KEY_IN_FLIGHT` on a matching in-flight hit or on
//! [`IdempotencyClaim::TakeoverRaceLost`] — landed in the slice after it.
//!
//! [`answer_idempotency_key`] is the other end of that mechanism and is what
//! makes the store useful for the case it exists for: without it a committed
//! create leaves its key `claimed` forever, and the client's own in-window
//! retry is refused `IDEMPOTENCY_KEY_IN_FLIGHT` instead of replaying the
//! original `201`. It is deliberately shaped like its claim: no transaction
//! of its own, the caller's runner, and a write that either moves a row it
//! finds `claimed` or reports that it found none — never a silent zero-row
//! success (`inst-fd-idem-claim-write`, **P-D-29**).
//!
//! Phase 6 adds the four writes the publish and discard transactions are
//! made of, and nothing above them: [`insert_entity_version`] freezes one
//! version row, [`publish_product_head`] and [`publish_sku_head`] carry the
//! whole of a publish in **one** guarded `UPDATE` each, and
//! [`discard_product_head`] and [`discard_sku_head`] carry a discard in one.
//! The doors that call them, the canonical rendering they are handed and the
//! digest over it are all a later slice's; this module stores bytes and
//! moves counters. All six join the caller's transaction, for the reason
//! below, and the publish pair depends on it in a way the others do not: the
//! head-row guard admits a `published_version` bump only where the matching
//! frozen row already exists (`m20260829_000002_create_products_product`), so
//! a freeze committed on a runner of its own would both trip the guard's
//! ordering and outlive a rolled-back publish.
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
//!
//! @cpt-dod:cpt-cf-bss-products-dod-audit-trail:p1
//! @cpt-dod:cpt-cf-bss-products-dod-idempotency-store:p1

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::{Expr, ExprTrait, OnConflict, SimpleExpr};
use sea_orm::{ColumnTrait, Condition, DbErr, EntityTrait, FromQueryResult};
use serde_json::Value as JsonValue;
use toolkit_db::secure::{
    AccessScope, DBRunner, ScopeError, SecureDeleteExt, SecureEntityExt, SecureInsertExt,
    SecureUpdateExt,
};
use uuid::Uuid;

use bss_products_sdk::models::LifecycleState;

use crate::domain::deprecation::Provenance;
use crate::domain::error::DomainError;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{
    approval, audit_log, entity_version, idempotency, identity_ref, product, product_category, sku,
};

/// A statement's failure, with `sea-orm`'s own error kept unchanged.
///
/// # This function opens nothing and touches no runner
///
/// It is a constructor, called from a `map_err` on a statement the caller's
/// transaction has already run. Every statement in this module is a
/// `SecureORM` one, so every one of them fails as a [`ScopeError`], and this
/// is the single place that reads which kind it is.
///
/// `ScopeError::Db` wraps the driver error the statement raised, and that
/// inner error is exactly what a retry classifier needs, so it is unwrapped
/// and preserved as [`RepoError::Driver`]. Preserving it is the whole point:
/// `toolkit_db::contention::is_retryable_contention` classifies only
/// `DbErr::Exec` and `DbErr::Query`, so a contention failure rendered to a
/// string on the way out of this module reaches the caller as a bare 500
/// where the doors promise a retry.
///
/// The other three variants are the scope layer refusing to build or run the
/// statement at all: no driver error exists, nothing about them is transient,
/// and they stay [`RepoError::Db`].
fn driver_failure(context: String, source: ScopeError) -> RepoError {
    match source {
        ScopeError::Db(source) => RepoError::Driver { context, source },
        ScopeError::Invalid(_) | ScopeError::TenantNotInScope { .. } | ScopeError::Denied(_) => {
            RepoError::Db(format!("{context}: {source}"))
        }
    }
}

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
    /// The clone's immediate source and the frozen version its content was
    /// read at (`None` for an ordinary create; version `None` under a set
    /// source means the source was read at its head — P-D-76). Create-only:
    /// the head guard refuses any later write of the pair.
    pub cloned_from: Option<Uuid>,
    pub cloned_from_version: Option<i64>,
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
    /// The clone's immediate source and the frozen version its content was
    /// read at (P-D-76; `None`/`None` for a non-clone).
    pub cloned_from: Option<Uuid>,
    pub cloned_from_version: Option<i64>,
    /// Why this entity is `deprecated`, or `None` where it is not — the
    /// operand `dod-provenance-reversal` reads to decide which children a
    /// parent's un-deprecation revives. `None` on a `deprecated` row is a
    /// row this gear deprecated through neither path, and the reversal rule
    /// leaves it alone rather than guessing.
    pub deprecation_provenance: Option<Provenance>,
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
    /// The clone's immediate source and the frozen version its content was
    /// read at (`None` for an ordinary create; version `None` under a set
    /// source means the source was read at its head — P-D-76). Create-only:
    /// the head guard refuses any later write of the pair.
    pub cloned_from: Option<Uuid>,
    pub cloned_from_version: Option<i64>,
    /// 03's type profile — required at create (P-D-145).
    pub sku_type: String,
    /// `inst-cl-sellable`'s default is `true`.
    pub sellable: bool,
    /// The tier the create assigns — `standard` when the caller names none.
    pub plan_tier: String,
    pub tax_category_ref: Option<String>,
    pub gl_code_ref: Option<String>,
    /// The meter pair, set at create only by the clone (`design/11` §3.1's
    /// *Metering declaration: Copy*, P-D-154); the create door leaves both
    /// `None` and the save door writes them.
    pub metering_unit: Option<String>,
    /// The other half of the pair.
    pub usage_type_ref: Option<String>,
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
    /// Whether this SKU's composition is still unresolved
    /// (`design/01-foundation.md` §4.2, **P-D-35**). System-owned: the head
    /// table's guard admits a write of it only in the same statement as a
    /// `published_version` bump, so it moves on a publish or not at all.
    /// `products_product` carries no twin — `bundle` is a value of the
    /// SKU-only `type` column.
    pub composition_pending: bool,
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
    /// The clone's immediate source and the frozen version its content was
    /// read at (P-D-76; `None`/`None` for a non-clone).
    pub cloned_from: Option<Uuid>,
    pub cloned_from_version: Option<i64>,
    /// The successor named at retirement initiation, or `None`.
    ///
    /// **The row carried this column and this struct did not**, which made
    /// `04`'s lead-window re-announcement drop it. The retire door emits
    /// `SkuRetired` with `replaced_by: request.replaced_by`; a publish inside
    /// the window re-emitted the same event with `None`, and consumers key on
    /// `(skuId, effectiveAt)` and take the latest (**P-D-20**, **P-D-48**), so
    /// an unrelated publish **erased** the successor from every consumer's
    /// view. Reported by strand C, which could not confirm it because there
    /// was no field here to read.
    ///
    /// Write-once per retirement by the row's own trigger — `null` to
    /// non-null at initiation, non-null to `null` at the governed cancel
    /// (**P-D-49**) — so a re-announcement can only ever repeat what the
    /// initiation announced.
    pub replaced_by_sku_id: Option<Uuid>,
    /// Why this entity is `deprecated`, or `None` where it is not — the
    /// operand `dod-provenance-reversal` reads to decide which children a
    /// parent's un-deprecation revives. `None` on a `deprecated` row is a
    /// row this gear deprecated through neither path, and the reversal rule
    /// leaves it alone rather than guessing.
    pub deprecation_provenance: Option<Provenance>,
    /// The declared metering unit — 03's `MeterDeclaration`, atomic with
    /// `usage_type_ref` (the paired `CHECK`). Bucket-ii: written by the save
    /// door only while `published_version = 0`, frozen into version content
    /// at publish.
    pub metering_unit: Option<String>,
    /// The declaration's usage-type reference — the pair's other half.
    pub usage_type_ref: Option<String>,
    /// The ceremony that admitted the last bucket-ii correction after first
    /// publish (P-D-129) — `None` until 07's `CorrectionDoor` writes one.
    /// Not version content: the head guard's door identity, like
    /// `composition_pending`.
    pub correction_ref: Option<Uuid>,
    /// 03's classification columns (P-D-145): the type profile, the sellable
    /// flag, the tier and the two Finance codes.
    pub sku_type: Option<String>,
    pub sellable: bool,
    pub plan_tier: Option<String>,
    pub tax_category_ref: Option<String>,
    pub gl_code_ref: Option<String>,
}

/// Insert one `products_product` row and read it back as authored
/// (`dod-create-doors`).
///
/// # Errors
/// [`RepoError::Driver`] on a `CHECK`/uniqueness violation the
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
        cloned_from: Set(new.cloned_from),
        cloned_from_version: Set(new.cloned_from_version),
        // Slice 04's columns: a create never names them.
        deprecation_provenance: Set(None),
    };

    let row = product::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("product {} scope", new.product_id), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| driver_failure(format!("insert product {}", new.product_id), e))?;

    into_product_record(row)
}

/// Read one Product by id, within `tenant_id`'s scope.
///
/// Answers `Ok(None)` both when no such row exists and when a row exists but
/// lies outside `scope`. Deliberately the same answer either way: a
/// repository that answered "forbidden" for a row belonging to another
/// tenant would confirm that the row exists, which is the existence leak the
/// SQL-level scoping is there to close — the catalog is commercially
/// sensitive, so absence is what a foreign scope sees.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure; [`RepoError::CorruptRow`] when
/// the stored `lifecycle_state` is outside the enumeration
/// [`LifecycleState`] parses.
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
        .map_err(|e| driver_failure(format!("read product {product_id}"), e))?;

    row.map(into_product_record).transpose()
}

/// One clone of an entity, as the lineage column records it: the clone's id
/// and the source version it read (`None` for a head read of a draft).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloneRef {
    /// The clone.
    pub entity_id: Uuid,
    /// `cloned_from_version`: the frozen version read, or `None` for a draft.
    pub cloned_from_version: Option<i64>,
}

/// The clones whose `cloned_from` names `source` — the reverse lineage lookup
/// `design/11` §2 promised when it justified having no clone event by the
/// field being *"queryable"* (`dod-clone-lineage`, P-D-152). Same kind only:
/// a clone is always of its own kind (P-D-72).
///
/// # Errors
///
/// [`RepoError`] on a storage failure below the domain.
///
/// @cpt-dod:cpt-cf-bss-products-dod-clone-lineage:p3
pub async fn clones_of(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    kind: VersionedEntityKind,
    source: Uuid,
) -> Result<Vec<CloneRef>, RepoError> {
    let mut out = match kind {
        VersionedEntityKind::Product => product::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(product::Column::TenantId.eq(tenant_id))
                    .add(product::Column::ClonedFrom.eq(source)),
            )
            .all(runner)
            .await
            .map_err(|e| driver_failure(format!("read clones of product {source}"), e))?
            .into_iter()
            .map(|row| CloneRef {
                entity_id: row.product_id,
                cloned_from_version: row.cloned_from_version,
            })
            .collect::<Vec<_>>(),
        VersionedEntityKind::Sku => sku::Entity::find()
            .secure()
            .scope_with(scope)
            .filter(
                Condition::all()
                    .add(sku::Column::TenantId.eq(tenant_id))
                    .add(sku::Column::ClonedFrom.eq(source)),
            )
            .all(runner)
            .await
            .map_err(|e| driver_failure(format!("read clones of sku {source}"), e))?
            .into_iter()
            .map(|row| CloneRef {
                entity_id: row.sku_id,
                cloned_from_version: row.cloned_from_version,
            })
            .collect::<Vec<_>>(),
    };
    out.sort_by_key(|c| c.entity_id);
    Ok(out)
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
        cloned_from: row.cloned_from,
        cloned_from_version: row.cloned_from_version,
        deprecation_provenance: parse_provenance(
            row.deprecation_provenance.as_deref(),
            "products_product",
            row.product_id,
        )?,
    })
}

/// Parse a stored `deprecation_provenance`, or refuse the row.
///
/// A value outside `direct|cascaded` is a [`RepoError::CorruptRow`] on the
/// same terms as an unparseable `lifecycle_state`: the column is the operand
/// `dod-provenance-reversal` reads, and defaulting it would decide a
/// reversal from a value nothing wrote.
fn parse_provenance(
    stored: Option<&str>,
    table: &str,
    entity_id: Uuid,
) -> Result<Option<Provenance>, RepoError> {
    match stored {
        None => Ok(None),
        Some(raw) => Provenance::parse(raw).map(Some).ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "{table}.deprecation_provenance `{raw}` on entity {entity_id}"
            ))
        }),
    }
}

/// Insert one `products_sku` row and read it back as authored
/// (`dod-create-doors`).
///
/// # Errors
/// [`RepoError::Driver`] on the `fk_products_sku_product`
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
        // The column's own default, written explicitly because this literal is
        // exhaustive: a create raises nothing (P-D-35 names the publish door
        // on a `bundle` as the flag's only raiser), and the head table's guard
        // refuses any later write of it that does not also bump
        // `published_version`.
        composition_pending: Set(false),
        region_scope: Set(new.region_scope),
        brand_scope: Set(new.brand_scope),
        created_by: Set(new.created_by),
        created_at: Set(new.created_at),
        updated_at: Set(new.created_at),
        cloned_from: Set(new.cloned_from),
        cloned_from_version: Set(new.cloned_from_version),
        // Slice 04's columns: a create never names them.
        deprecation_provenance: Set(None),
        replaced_by_sku_id: Set(None),
        metering_unit: Set(new.metering_unit),
        usage_type_ref: Set(new.usage_type_ref),
        // The meter pair arrives through the save door (bucket-ii, admitted
        // while `published_version = 0`) — or, since P-D-154, copied by the
        // clone from its source; the create door itself hands `None` for both.
        // Set above, with the head columns.
        // P-D-129's door identity: only the correction re-publish writes it.
        correction_ref: Set(None),
        // 03's classification (P-D-145): the type and tier the create judged,
        // the flag's default, the two Finance codes as given.
        sku_type: Set(Some(new.sku_type)),
        sellable: Set(new.sellable),
        plan_tier: Set(Some(new.plan_tier)),
        tax_category_ref: Set(new.tax_category_ref),
        gl_code_ref: Set(new.gl_code_ref),
    };

    let row = sku::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("sku {} scope", new.sku_id), e))?
        .exec_with_returning(runner)
        .await
        .map_err(|e| driver_failure(format!("insert sku {}", new.sku_id), e))?;

    into_sku_record(row)
}

/// Read one SKU by id, within `tenant_id`'s scope.
///
/// Answers `Ok(None)` both when no such row exists and when a row exists but
/// lies outside `scope`, for [`find_product`]'s reason.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure; [`RepoError::CorruptRow`] when
/// the stored `lifecycle_state` is outside the enumeration
/// [`LifecycleState`] parses.
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
        .map_err(|e| driver_failure(format!("read sku {sku_id}"), e))?;

    row.map(into_sku_record).transpose()
}

/// Read every **non-terminal** SKU under one Product, within `tenant_id`'s
/// scope — the parent-child containment check's operand
/// (`fr-parent-child-integrity`, `design/01-foundation.md` §4.1).
///
/// # Why terminal children are excluded in the statement, not by the caller
///
/// `retired` and `discarded` children are out of use: a narrowing of the
/// parent cannot orphan a row nothing can transact against, and no door will
/// ever ask them to be contained again — the head-write guard refuses every
/// write to a terminal head, so the state they were left in is the state
/// they keep. Filtering them here rather than in the caller keeps the rule's
/// operand a property of the read: a second caller cannot forget the
/// exclusion and turn a tidy retirement into a refusal on an unrelated save.
/// [`TERMINAL_HEAD_STATES`] is the same roster both save statements pin, so
/// the two cannot drift.
///
/// # `runner` MUST already be the caller's own transaction
///
/// This function opens none, exactly as [`find_product`] and [`find_sku`] do
/// not. Its one caller is the save door's containment phase, which runs
/// inside the mutation transaction (P-D-42 puts the idempotency claim there
/// and every later phase with it), so the children this reads are the
/// children the `UPDATE` a few statements later commits against.
///
/// Ordered by `sku_code`, so a parent with several offending children is
/// refused naming the same one on every run rather than whichever the
/// driver happened to return first.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure; [`RepoError::CorruptRow`] when
/// a stored `lifecycle_state` is outside the enumeration [`LifecycleState`]
/// parses.
/// Every non-`discarded` SKU of one Product, in `sku_code` order — the
/// family clone's child census (P-D-79): children in any of C1's four
/// states clone, and a `discarded` child is not part of the family at all.
///
/// [`find_non_terminal_skus_of_product`]'s filter is deliberately not
/// reused: that one also drops `retired` rows, and a retired child is
/// exactly the revival case the clone exists for.
pub async fn find_skus_of_product(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
) -> Result<Vec<SkuRecord>, RepoError> {
    let rows = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::ProductId.eq(product_id))
                .add(sku::Column::LifecycleState.ne(LifecycleState::Discarded.as_str())),
        )
        .order_by(sku::Column::SkuCode, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the SKUs of product {product_id}"), e))?;

    rows.into_iter().map(into_sku_record).collect()
}

pub async fn find_non_terminal_skus_of_product(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
) -> Result<Vec<SkuRecord>, RepoError> {
    let rows = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::ProductId.eq(product_id))
                .add(sku::Column::LifecycleState.is_not_in(TERMINAL_HEAD_STATES)),
        )
        .order_by(sku::Column::SkuCode, sea_orm::Order::Asc)
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read the SKUs of product {product_id}"), e))?;

    rows.into_iter().map(into_sku_record).collect()
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
        composition_pending: row.composition_pending,
        region_scope: row.region_scope,
        brand_scope: row.brand_scope,
        created_by: row.created_by,
        created_at: row.created_at,
        updated_at: row.updated_at,
        cloned_from: row.cloned_from,
        cloned_from_version: row.cloned_from_version,
        replaced_by_sku_id: row.replaced_by_sku_id,
        deprecation_provenance: parse_provenance(
            row.deprecation_provenance.as_deref(),
            "products_sku",
            row.sku_id,
        )?,
        metering_unit: row.metering_unit,
        usage_type_ref: row.usage_type_ref,
        correction_ref: row.correction_ref,
        sku_type: row.sku_type,
        sellable: row.sellable,
        plan_tier: row.plan_tier,
        tax_category_ref: row.tax_category_ref,
        gl_code_ref: row.gl_code_ref,
    })
}

/// Resolve `principal_ref` to its `actor_ref` through the identity-ref map
/// (`design/10-retention-erasure.md` `inst-im-map`), minting one on the
/// principal's first appearance with no live ref and emitting no event for
/// the mint (`dod-actor-ref`).
///
/// # `runner` MUST already be the caller's own transaction — never the door's
///
/// This function takes `runner: &impl DBRunner` and opens no transaction of
/// its own, for the reason this module's doc gives for every function here:
/// the toolkit's transaction-bypass guard refuses `Db::conn()` inside an
/// already open transaction, so a repository that acquired its own
/// connection could not be called from inside one at all. That leaves the
/// transaction discipline entirely the **caller's** obligation, and it is a
/// stricter one than the doors below this repository ever ask for: `runner`
/// MUST be a transaction of its own, committed independently, and MUST NEVER
/// be the door's own transaction (P-D-26). The reason is a refusal: a
/// refused act rolls the door's transaction back while its audit row commits
/// independently and requires an `actor_ref` to attribute to, so this
/// resolution has to survive the rollback that the refusal it may precede
/// causes. Resolution therefore MUST run before the authorization gate and
/// before any phase that can refuse. A ref minted for an act that is then
/// refused is not a bug; it is exactly what `last_seen_at` on that ref
/// should record.
///
/// # `last_seen_at` is advanced by resolution, never only by the mint
///
/// A live ref for `(tenant_id, principal_ref)` — `tombstoned_at IS NULL` —
/// has its `last_seen_at` advanced to `now` on every call that finds it;
/// `first_seen_at` never moves again after the mint. An earlier version of
/// this rule advanced the column only when minting, which pinned it to
/// `first_seen_at` forever and let age-based erasure tombstone an active
/// employee mid-employment — a recorded failure this shape exists to avoid,
/// not a hypothetical.
///
/// "First appearance" means first appearance of a principal with **no live
/// ref**, not first appearance ever: a principal acting again after its ref
/// was tombstoned mints a **fresh, different** `actor_ref`. A tombstoned ref
/// is retired permanently — every append-only record keeps the `actor_ref`
/// it was stamped with, so re-minting a retired key would make render-time
/// joins show the new identity against historical rows. One active ref per
/// `(tenant, principal)` is enforced physically by the partial unique index
/// `uq_products_identity_ref_active`, not by this function.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error. The
/// driver failures include the `uq_products_identity_ref_active` violation a race between two
/// resolutions of the same never-before-seen principal would raise — this
/// phase reports that race undifferentiated, as [`insert_product`]'s own doc
/// does for its own uniqueness index.
pub async fn resolve_actor_ref(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    principal_ref: &str,
    now: DateTime<Utc>,
) -> Result<Uuid, RepoError> {
    let live = identity_ref::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(identity_ref::Column::TenantId.eq(tenant_id))
                .add(identity_ref::Column::PrincipalRef.eq(principal_ref))
                .add(identity_ref::Column::TombstonedAt.is_null()),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("resolve actor ref, tenant {tenant_id}"), e))?;

    if let Some(row) = live {
        // The liveness predicate is repeated here, and it is not redundant.
        // The row was read live a statement ago, but slice 10's erasure
        // tombstones the map entry from a transaction of its own, and this
        // function's own contract puts it in a separate transaction too — so
        // an erasure can commit between the read and this write. Without the
        // predicate the advance would touch a row erasure had just retired
        // and the ref would be returned as live, stamping a freshly committed
        // act with a pseudonym that is supposed to be permanently dead. A
        // tombstoned ref is retired permanently: nothing joins
        // `products_audit_log.actor_ref` back to this map, so no other guard
        // would catch it.
        let advanced = identity_ref::Entity::update_many()
            .secure()
            .scope_with(scope)
            .col_expr(identity_ref::Column::LastSeenAt, Expr::value(now))
            .filter(
                Condition::all()
                    .add(identity_ref::Column::TenantId.eq(tenant_id))
                    .add(identity_ref::Column::ActorRef.eq(row.actor_ref))
                    .add(identity_ref::Column::TombstonedAt.is_null())
                    // **Never advance the stamp backwards.** Two of a
                    // principal's first requests race: the winner mints the row
                    // at its own `now`, and the loser — whose `now` is earlier —
                    // then wrote `last_seen_at < first_seen_at` and broke
                    // `chk_products_identity_ref_seen_order`, answering a `500`
                    // on an ordinary create (benidorm, 2026-09-06). An
                    // out-of-order advance now matches no row, and the branch
                    // below reads the winner's row rather than minting a second.
                    .add(identity_ref::Column::FirstSeenAt.lte(now)),
            )
            .exec(runner)
            .await
            .map_err(|e| {
                driver_failure(
                    format!("advance last_seen_at for actor {}", row.actor_ref),
                    e,
                )
            })?;

        // Zero rows means the race above was lost. That is not an error: it
        // is exactly the state the mint path is for — a principal with no
        // live ref — so fall through to it and mint a fresh one, which is
        // what the design requires of a principal acting after its erasure.
        if advanced.rows_affected > 0 {
            return Ok(row.actor_ref);
        }
        // Zero rows has two causes and only one of them is a vanished row: the
        // clamp above also declines an out-of-order advance. The row we read is
        // still this principal's actor, so answer it rather than minting a
        // second ref for the same principal.
        if row.first_seen_at > now {
            return Ok(row.actor_ref);
        }
    }

    // Random, never derived from `principal_ref`: a v5 uuid over the principal
    // would let anyone holding the principal id recompute the ref, which is
    // precisely the re-identification the pseudonymous map exists to prevent.
    let actor_ref = Uuid::new_v4();
    let model = identity_ref::ActiveModel {
        tenant_id: Set(tenant_id),
        actor_ref: Set(actor_ref),
        principal_ref: Set(principal_ref.to_owned()),
        identity_payload: Set(None),
        tombstoned_at: Set(None),
        first_seen_at: Set(now),
        last_seen_at: Set(now),
    };

    identity_ref::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("actor ref {actor_ref} scope"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("mint actor ref, tenant {tenant_id}"), e))?;

    Ok(actor_ref)
}

/// What a [`AuditEntry::Refusal`] names in place of a subject id.
///
/// A refusal raised after the subject was minted names it directly; a
/// refusal raised **before** the mint — `DUPLICATE_NAME` and
/// `DUPLICATE_CODE` are the ordinary cases (`design/01-foundation.md` §4.4)
/// — has no id yet to carry, so it carries the attempted natural key
/// instead: the `name`, `sku_code` or `product_code` the caller supplied. An
/// audit row must never name an id that identifies nothing, which is the
/// reason this is an enum rather than two nullable fields either of which a
/// caller could leave both unset.
#[derive(Clone, Debug)]
pub enum RefusalSubject {
    /// The refusal happened after the subject was minted.
    Minted {
        /// The subject's id.
        subject_id: Uuid,
        /// The subject's revision at the time of the act, where the door
        /// has one to give.
        subject_revision: Option<i64>,
    },
    /// The refusal happened before the mint: the attempted `name`,
    /// `sku_code` or `product_code`.
    Attempted(String),
}

/// One row `products_audit_log` will hold, shaped so an illegal combination
/// cannot be constructed (`design/01-foundation.md` §4.4).
///
/// The design names three classes and each shapes its row differently: a
/// struct of `Option` fields here would let a caller build, say, a refusal
/// that also carries a `session_id`, which no refusal may — this enum makes
/// that combination not compile rather than merely undocumented.
///
/// # What this table does not hold
///
/// Under P-D-21, only acts that emit no broker event, in these three
/// classes. A committed mutation that **does** emit writes no row here — its
/// outbox event is the record. And `AUDIT_UNAVAILABLE` itself has no row of
/// its own: by construction it *is* the row that could not be written, so
/// the class would otherwise carry a member it can never satisfy (P-D-34).
/// It is recorded out-of-band, as log and metric, never through this type or
/// its private inserter.
///
/// # This phase produces a `DomainError`, not an HTTP response
///
/// [`write_refusal_audit`] and [`write_elevated_read_audit`] answer
/// [`DomainError::AuditUnavailable`] on a failed write. Turning that into the
/// RFC 9457 503 the design promises is Phase 3's, once the gear's
/// capabilities widen from `[db]` to `[db, rest]`; no Problem mapping is
/// built here.
#[derive(Clone, Debug)]
pub enum AuditEntry {
    /// A door's refusal. Carries the refusal's `error_code` — a column
    /// rather than free text because §3.1 makes the code the attribution
    /// channel — and the subject the refusal names, one way or the other.
    /// Never carries a `session_id`.
    Refusal {
        /// The refusal's stable wire code, e.g. `DUPLICATE_NAME`.
        error_code: String,
        /// The subject the refusal names.
        subject: RefusalSubject,
    },
    /// A committed act the design declares emits no broker event. Carries
    /// the subject it acted on. Carries neither `error_code` nor
    /// `session_id`.
    EventlessAct {
        /// The subject's id — always minted by the time this class is
        /// written, since the act already committed.
        subject_id: Uuid,
        /// The subject's revision at the time of the act, where the door
        /// has one to give.
        subject_revision: Option<i64>,
    },
    /// A committed eventless act whose subject has no minted uuid — the
    /// freeze ledger's `(catalog_version_id, participant)` pair is the
    /// first: the pair rides `attempted_key` exactly as a pre-mint
    /// refusal's subject does, and `subject_id` stays `NULL`. Carries
    /// neither `error_code` nor `session_id`.
    KeyedAct {
        /// The act's subject, rendered as the door's own key string.
        attempted_key: String,
    },
    /// A read served under break-glass elevation. Carries the elevation's
    /// `session_id` — 05 audits every elevated access with it. Carries no
    /// `error_code`.
    ElevatedRead {
        /// The break-glass session under which the read was served.
        session_id: Uuid,
        /// The subject read, where the read named one.
        subject_id: Option<Uuid>,
        /// The subject's revision at the time of the read, where the door
        /// has one to give.
        subject_revision: Option<i64>,
    },
    /// An accepted act under a ceremony — the row carries `ceremony_ref`
    /// (`07`'s break-glass lanes, `dod-reference-audit`).
    CeremonyAct {
        subject_id: Uuid,
        subject_revision: Option<i64>,
        ceremony_ref: Uuid,
    },
}

/// The fields every audit-row class carries, whatever its shape
/// (`design/01-foundation.md` §4.4).
#[derive(Clone, Debug)]
pub struct AuditCommon {
    /// Server-minted by the caller, never re-derived here.
    pub audit_id: Uuid,
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// The pseudonymous ref of whoever, or whatever refused act, this row
    /// attributes to. Never a direct operator identity.
    pub actor_ref: Uuid,
    /// The audit action token.
    pub action: String,
    /// The kind of thing the entry's subject names.
    pub subject_kind: String,
    /// A free-text reason, where the door supplies one.
    pub reason: Option<String>,
    /// Ties related rows together across a single request, where one
    /// exists.
    ///
    /// The value is the W3C trace id `infra::events::correlation_id` reads
    /// off the ambient span — 32 hex characters, rendered so it stays
    /// grep-equal to the access log, the span and the error envelope. The
    /// column shipped `uuid`, which could hold none of them, so every caller
    /// passed `None` and this doc carried the two shapes the repair could
    /// take. **P-D-118 chose `text`** (2026-09-03) and the migration landed
    /// in place on 2026-09-04; the door writers fill it now. A background act
    /// — the GC, the runner — still writes `None`, because it has no request:
    /// that is a fact about the act, not a hole. Minting a value per row
    /// would be worse than `None`, filling the column with values that
    /// correlate nothing while reading as though they did.
    pub correlation_id: Option<String>,
    /// The commit instant; the operand `10-retention-erasure`'s
    /// `RetentionClock` reads. Taken as a parameter rather than read from
    /// `Utc::now()`, matching [`resolve_actor_ref`].
    pub written_at: DateTime<Utc>,
}

/// Insert one `products_audit_log` row. Private: every public writer below
/// goes through this one function so the three classes differ in the row
/// they build, never in how the insert is performed.
///
/// Writes `seal_state = "unsealed"` and leaves `chain_id`, `seq`,
/// `prev_hash` and `row_hash` `NULL` on every call, unconditionally — in v1
/// and after the platform sealing capability activates alike (P-D-08), so
/// the unproven era stays queryable rather than inferred from a deployment
/// date. This gear computes no hash and runs no verification job; no
/// argument to this function can make it write a sealed row.
async fn insert_audit_row(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    entry: AuditEntry,
) -> Result<(), RepoError> {
    let audit_id = common.audit_id;
    let mut ceremony: Option<Uuid> = None;

    let (subject_id, subject_revision, error_code, attempted_key, session_id) = match entry {
        AuditEntry::Refusal {
            error_code,
            subject,
        } => match subject {
            RefusalSubject::Minted {
                subject_id,
                subject_revision,
            } => (
                Some(subject_id),
                subject_revision,
                Some(error_code),
                None,
                None,
            ),
            RefusalSubject::Attempted(key) => (None, None, Some(error_code), Some(key), None),
        },
        AuditEntry::EventlessAct {
            subject_id,
            subject_revision,
        } => (Some(subject_id), subject_revision, None, None, None),
        AuditEntry::KeyedAct { attempted_key } => (None, None, None, Some(attempted_key), None),
        AuditEntry::CeremonyAct {
            subject_id,
            subject_revision,
            ceremony_ref,
        } => {
            ceremony = Some(ceremony_ref);
            (Some(subject_id), subject_revision, None, None, None)
        }
        AuditEntry::ElevatedRead {
            session_id,
            subject_id,
            subject_revision,
        } => (subject_id, subject_revision, None, None, Some(session_id)),
    };

    let model = audit_log::ActiveModel {
        audit_id: Set(common.audit_id),
        tenant_id: Set(common.tenant_id),
        actor_ref: Set(common.actor_ref),
        action: Set(common.action),
        subject_kind: Set(common.subject_kind),
        subject_id: Set(subject_id),
        subject_revision: Set(subject_revision),
        error_code: Set(error_code),
        attempted_key: Set(attempted_key),
        reason: Set(common.reason),
        correlation_id: Set(common.correlation_id),
        written_at: Set(common.written_at),
        session_id: Set(session_id),
        // P-D-129's column; `07`'s ceremony lanes write it through
        // `AuditEntry::CeremonyAct` (P-D-147), every other row is `None`.
        ceremony_ref: Set(ceremony),
        seal_state: Set("unsealed".to_owned()),
        chain_id: Set(None),
        seq: Set(None),
        prev_hash: Set(None),
        row_hash: Set(None),
    };

    audit_log::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| driver_failure(format!("audit row {audit_id} scope"), e))?
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("insert audit row {audit_id}"), e))?;

    Ok(())
}

/// Write a refusal's audit row (`dod-audit-trail`).
///
/// # `runner` MUST be a transaction of its own, and MUST NEVER be the door's
///
/// A refusal's row commits in its own transaction, independently of the
/// refused mutation, and is a **precondition of answering the caller**
/// (owner's call, 2026-08-27). The runner handed to this function must
/// already be a transaction of its own, committed independently, and must
/// **never** be the door's own transaction — the door's is precisely the
/// transaction a refusal rolls back, and a row written against it would be
/// rolled back with it, which is the one failure `nfr-availability-audit`'s
/// "100% write-path audit" forbids.
///
/// # Errors
/// [`DomainError::AuditUnavailable`] when the row cannot be written. The
/// caller MUST answer this, and MUST NOT report the domain refusal it had
/// otherwise reached: a refusal the caller learns about and the registry
/// does not is exactly what the "100% write-path audit" NFR forbids. This
/// phase produces the `DomainError` only — the RFC 9457 503 mapping is Phase
/// 3's, once the gear's capabilities widen from `[db]` to `[db, rest]`.
pub async fn write_refusal_audit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    error_code: String,
    subject: RefusalSubject,
) -> Result<(), DomainError> {
    let audit_id = common.audit_id;

    insert_audit_row(
        runner,
        scope,
        common,
        AuditEntry::Refusal {
            error_code,
            subject,
        },
    )
    .await
    .map_err(|e| DomainError::AuditUnavailable(format!("refusal audit row {audit_id}: {e}")))
}

/// Write a committed eventless act's audit row (`dod-audit-trail`).
///
/// # `runner` MUST be the door's own mutation transaction
///
/// A committed eventless act's row commits **inside the guarded mutation's
/// transaction** (P-D-08 S3 as amended by P-D-31) — the runner handed to
/// this function must be the door's own. The act and its record stand or
/// fall together: if the row cannot be written, the mutation's own
/// transaction fails to commit along with it, which is what
/// `nfr-availability-audit`'s "100% write-path audit" asks for on the
/// success path. There is no separate `AUDIT_UNAVAILABLE` carve-out here —
/// unlike the other two disciplines, a write-time failure here is simply the
/// mutation's own transaction failing.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error.
/// An accepted act's audit row that **carries the ceremony reference** —
/// `07`'s break-glass correction and its dead-producer retirement
/// (`dod-reference-audit`: *"the same value `products_correction_override`
/// stores, so the ceremony and the evidence are joinable from either side"*).
/// The row is otherwise [`write_eventless_act_audit`]'s.
///
/// # Errors
///
/// [`RepoError`] on a driver failure.
///
/// @cpt-dod:cpt-cf-bss-products-dod-reference-audit:p1
pub async fn write_ceremony_act_audit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    subject_id: Uuid,
    subject_revision: Option<i64>,
    ceremony_ref: Uuid,
) -> Result<(), RepoError> {
    insert_audit_row(
        runner,
        scope,
        common,
        AuditEntry::CeremonyAct {
            subject_id,
            subject_revision,
            ceremony_ref,
        },
    )
    .await
}

pub async fn write_eventless_act_audit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    subject_id: Uuid,
    subject_revision: Option<i64>,
) -> Result<(), RepoError> {
    insert_audit_row(
        runner,
        scope,
        common,
        AuditEntry::EventlessAct {
            subject_id,
            subject_revision,
        },
    )
    .await
}

/// Write a keyed eventless act's audit row — the ack and release doors'
/// record (`dod-clone-audit`'s inverse posture: these acts emit no broker
/// event by design, so the audit row IS the record — `dod-ack-door`).
///
/// # Errors
///
/// [`RepoError`] as [`insert_audit_row`] raises it.
pub async fn write_keyed_act_audit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    attempted_key: String,
) -> Result<(), RepoError> {
    insert_audit_row(
        runner,
        scope,
        common,
        AuditEntry::KeyedAct { attempted_key },
    )
    .await
}

/// Write an elevated read's audit row (`dod-audit-trail`).
///
/// # `runner` MUST be a transaction of its own, and is a precondition of serving the read
///
/// An elevated read's row commits in its own transaction and is a
/// **precondition of serving the read** (P-D-34) — a read has no mutation
/// transaction to join, so unlike [`write_eventless_act_audit`] there is
/// none for this runner to be. The runner handed to this function must be a
/// transaction of its own, committed independently before the read is
/// served: an elevated read the registry did not record is exactly what
/// break-glass auditing exists to prevent.
///
/// # Errors
/// [`DomainError::AuditUnavailable`] when the row cannot be written. The
/// caller MUST answer this and MUST serve nothing, as [`write_refusal_audit`]
/// requires of a refusal. This phase produces the `DomainError` only — the
/// RFC 9457 503 mapping is Phase 3's.
pub async fn write_elevated_read_audit(
    runner: &impl DBRunner,
    scope: &AccessScope,
    common: AuditCommon,
    session_id: Uuid,
    subject_id: Option<Uuid>,
    subject_revision: Option<i64>,
) -> Result<(), DomainError> {
    let audit_id = common.audit_id;

    insert_audit_row(
        runner,
        scope,
        common,
        AuditEntry::ElevatedRead {
            session_id,
            subject_id,
            subject_revision,
        },
    )
    .await
    .map_err(|e| DomainError::AuditUnavailable(format!("elevated read audit row {audit_id}: {e}")))
}

/// What [`claim_idempotency_key`] found `(tenant_id, endpoint, client_key)`
/// in, shaped so the three outcomes the caller must tell apart cannot be
/// conflated into one error type (`design/01-foundation.md` §3.2,
/// `dod-idempotency-store`).
///
/// A returned enum rather than an error for the non-error cases: only
/// [`InFlight`](Self::InFlight) is a refusal the caller executes nothing
/// under, and even that refusal is the door's to raise as
/// `IDEMPOTENCY_KEY_IN_FLIGHT` — this repository raises no `DomainError` of
/// its own, since the existing taxonomy already carries the two idempotency
/// codes this slice's outcomes map to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IdempotencyClaim {
    /// No live row existed for the key, or the held row was expired and this
    /// call took it over. The caller now holds the key and proceeds with the
    /// guarded mutation.
    Claimed,
    /// A row exists in `answered` state and is live. `payload_hash` is the
    /// hash the answer was recorded against — the caller compares it with
    /// its own to tell an identical replay from `IDEMPOTENCY_CONFLICT`
    /// (`inst-fd-idem-conflict`), a comparison this repository does not make
    /// because it was never handed the incoming request to compare.
    /// `response_status`/`response_body` are the replay itself, and it is
    /// self-contained (**P-D-29**): nothing else needs to be read to serve
    /// it.
    Answered {
        /// The digest the stored answer was recorded against.
        payload_hash: Vec<u8>,
        /// The status the original caller was told.
        response_status: i32,
        /// The body the original caller was told.
        response_body: JsonValue,
    },
    /// A live, unexpired `claimed` row already holds the key. The caller
    /// refuses `IDEMPOTENCY_KEY_IN_FLIGHT` **when the payloads agree** and
    /// `IDEMPOTENCY_CONFLICT` when they do not: a payload mismatch "stays
    /// `IDEMPOTENCY_CONFLICT` in either state"
    /// (`design/01-foundation.md` §3.2 `inst-fd-idem-claim-inflight`,
    /// `inst-fd-idem-conflict`), so the in-flight refusal is reserved for
    /// the duplicate that is genuinely the same request. This repository
    /// writes nothing to the row on either reading — the comparison is the
    /// caller's, for [`Answered`](Self::Answered)'s reason: this layer was
    /// never handed the incoming request.
    InFlight {
        /// The digest the live `claimed` row was recorded against.
        payload_hash: Vec<u8>,
        /// The composite act's parent handle, if the holding act stamped one
        /// (P-D-79): the family clone's committed-but-unanswered claim
        /// carries its new parent here, and the same-key retry resumes from
        /// it. `None` for every single-entity door's claim.
        entity_ref: Option<Uuid>,
    },
    /// This call lost the expired-key takeover race (**P-D-49**): another
    /// caller's compare-and-swap moved the row off the stamp this one read.
    ///
    /// Distinct from [`InFlight`](Self::InFlight) because **no digest
    /// comparison is owed here and none is possible**. The loser "may even
    /// carry a different payload from the winner, and is still refused
    /// in-flight rather than for the mismatch, since this transaction never
    /// compared the two" (§3.2 `inst-fd-idem-retention`, **P-D-49**): the
    /// row this call read was the *expired* holder's, and the payload now
    /// under the key is the winner's, which this transaction never saw.
    /// Answering `IDEMPOTENCY_CONFLICT` from a hash this call never read
    /// would be a fabricated verdict. The caller refuses
    /// `IDEMPOTENCY_KEY_IN_FLIGHT`, having executed nothing.
    TakeoverRaceLost,
}

/// The composite primary key of `products_idempotency`, as a filter. One
/// spelling, so no statement in this module can address a row by fewer axes
/// than the key actually has.
fn idempotency_key_of(tenant_id: Uuid, endpoint: &str, client_key: &str) -> Condition {
    Condition::all()
        .add(idempotency::Column::TenantId.eq(tenant_id))
        .add(idempotency::Column::Endpoint.eq(endpoint))
        .add(idempotency::Column::ClientKey.eq(client_key))
}

/// Claim `(tenant_id, endpoint, client_key)` for `payload_hash`, or report
/// why it could not be claimed (`design/01-foundation.md` §3.2,
/// `dod-idempotency-store`; **P-D-42**, **P-D-49**, **P-D-38**).
///
/// # This function opens no transaction — `runner` MUST already be the
/// # guarded mutation's own
///
/// **The claim `INSERT` is the gate, not a lookup** (**P-D-42**): this
/// function writes the claim row with an `INSERT ... ON CONFLICT DO NOTHING`
/// and reads back only when the conflict actually fired, so between the
/// attempt and the read nothing is ever left free for a second caller to
/// also see as available. There is no separate reservation step and no
/// `in_flight_until` column — the primary key is the whole of the gate. For
/// that gate to do its job, `runner` MUST be the caller's own guarded
/// mutation's transaction: joining it is what makes a rollback free the key
/// automatically, with no release step of its own, and it is a stricter
/// obligation than [`resolve_actor_ref`]'s, which asks for a transaction of
/// its own instead. A `runner` that is not the mutation's transaction breaks
/// the one property this whole mechanism exists to provide.
///
/// `now` and `expires_at` are both caller-supplied, matching
/// [`resolve_actor_ref`]'s own discipline: this function reasons about no
/// clock but the one its caller hands it.
///
/// # The expired-key takeover is a compare-and-swap on `expires_at` itself
/// # (**P-D-49**)
///
/// Nothing holds an expired row between this function's own conflict check
/// and its takeover `UPDATE`, so two duplicates racing on one expired key can
/// both clear the check and both read the same expired row. Without a
/// predicate both would be told they claimed it and the guarded mutation
/// would run twice under one key — precisely the failure this mechanism
/// exists to prevent. The takeover `UPDATE` therefore carries `WHERE
/// expires_at = <the value this call read>`; exactly one racer's `UPDATE`
/// matches, and **a zero-row result is the lost race, not success** — it is
/// answered [`IdempotencyClaim::TakeoverRaceLost`], never
/// [`IdempotencyClaim::Claimed`], and the loser may even carry a different
/// payload from the winner and is still refused in-flight rather than for
/// the mismatch, since this transaction never compared the two — which is
/// why that outcome is a variant of its own and carries no digest for the
/// caller to compare.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error. The failures include a
/// conflicting row this call cannot read back inside its own transaction —
/// the store contradicting itself, not a foreign tenant's row, since the
/// insert was already validated against the same scope.
/// [`RepoError::CorruptRow`] when the held row's `state` is outside
/// `claimed`/`answered`, or when an `answered` row is missing one of its
/// paired response columns — both refused by
/// `chk_products_idempotency_response_group` at write time, so reaching
/// either here means a row was written around this gear.
#[allow(
    clippy::too_many_arguments,
    reason = "the composite key is three columns and the claim needs the digest, the \
              instant and the fresh expiry beside them, matching the sibling pricing \
              gear's identically-shaped `IdempotencyGate::claim`; bundling them into a \
              parameter struct would hide the key's own shape behind a type nothing \
              else uses"
)]
pub async fn claim_idempotency_key(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    endpoint: &str,
    client_key: &str,
    payload_hash: &[u8],
    now: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> Result<IdempotencyClaim, RepoError> {
    let model = idempotency::ActiveModel {
        tenant_id: Set(tenant_id),
        endpoint: Set(endpoint.to_owned()),
        client_key: Set(client_key.to_owned()),
        state: Set("claimed".to_owned()),
        payload_hash: Set(payload_hash.to_vec()),
        response_status: Set(None),
        response_body: Set(None),
        expires_at: Set(expires_at),
        // A fresh claim carries no parent handle; a composite door stamps
        // one afterwards, in this same transaction (P-D-79).
        entity_ref: Set(None),
    };

    let on_conflict = OnConflict::columns([
        idempotency::Column::TenantId,
        idempotency::Column::Endpoint,
        idempotency::Column::ClientKey,
    ])
    .do_nothing()
    .to_owned();

    match idempotency::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| {
            driver_failure(
                format!("idempotency claim {tenant_id}/{endpoint}/{client_key} scope"),
                e,
            )
        })?
        .on_conflict_raw(on_conflict)
        .exec(runner)
        .await
    {
        Ok(_) => return Ok(IdempotencyClaim::Claimed),
        // The key is already held; the conflict swallowed the insert.
        Err(ScopeError::Db(DbErr::RecordNotInserted)) => {}
        Err(e) => {
            return Err(driver_failure(
                format!("idempotency claim {tenant_id}/{endpoint}/{client_key}"),
                e,
            ));
        }
    }

    let held = idempotency::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(idempotency_key_of(tenant_id, endpoint, client_key))
        .one(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("read held idempotency claim {tenant_id}/{endpoint}/{client_key}"),
                e,
            )
        })?
        .ok_or_else(|| {
            RepoError::Db(format!(
                "idempotency claim {tenant_id}/{endpoint}/{client_key} conflicted but is \
                 not readable in the same transaction"
            ))
        })?;

    if now > held.expires_at {
        return take_over_expired_idempotency_claim(runner, scope, &held, payload_hash, expires_at)
            .await;
    }

    match held.state.as_str() {
        "answered" => {
            let (Some(response_status), Some(response_body)) =
                (held.response_status, held.response_body)
            else {
                return Err(RepoError::CorruptRow(format!(
                    "products_idempotency {tenant_id}/{endpoint}/{client_key} answered \
                     with an incomplete response"
                )));
            };
            Ok(IdempotencyClaim::Answered {
                payload_hash: held.payload_hash,
                response_status,
                response_body,
            })
        }
        "claimed" => Ok(IdempotencyClaim::InFlight {
            payload_hash: held.payload_hash,
            entity_ref: held.entity_ref,
        }),
        other => Err(RepoError::CorruptRow(format!(
            "products_idempotency.state `{other}` on {tenant_id}/{endpoint}/{client_key}"
        ))),
    }
}

/// Take an expired claim over: `payload_hash`, no response, and a fresh
/// `expires_at`, under a predicate matching the row's own claim stamp as
/// [`claim_idempotency_key`] read it (**P-D-49**).
///
/// That predicate is the whole of the race protection this function
/// provides: nothing holds an expired row between the caller's conflict
/// check and this `UPDATE`, so two duplicates can both find the row expired
/// and both arrive here carrying the same `held.expires_at`. Without the
/// predicate both `UPDATE` statements would match, both callers would be told they
/// claimed the key, and the guarded mutation would run twice under one key.
/// With it, exactly one `UPDATE` matches; **the other affects zero rows,
/// which this function treats as the lost race, never as success** —
/// [`IdempotencyClaim::TakeoverRaceLost`], the one outcome that carries no
/// digest, because the payload now under the key is the winner's and this
/// transaction never read it.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error.
async fn take_over_expired_idempotency_claim(
    runner: &impl DBRunner,
    scope: &AccessScope,
    held: &idempotency::Model,
    payload_hash: &[u8],
    new_expires_at: DateTime<Utc>,
) -> Result<IdempotencyClaim, RepoError> {
    let result = idempotency::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            idempotency::Column::State,
            Expr::value("claimed".to_owned()),
        )
        .col_expr(
            idempotency::Column::PayloadHash,
            Expr::value(payload_hash.to_vec()),
        )
        .col_expr(
            idempotency::Column::ResponseStatus,
            Expr::value(None::<i32>),
        )
        .col_expr(
            idempotency::Column::ResponseBody,
            Expr::value(None::<JsonValue>),
        )
        .col_expr(idempotency::Column::ExpiresAt, Expr::value(new_expires_at))
        // The taken-over claim is a fresh act's: a stale parent handle from
        // the crashed holder must not leak into it (P-D-79).
        .col_expr(idempotency::Column::EntityRef, Expr::value(None::<Uuid>))
        .filter(
            idempotency_key_of(held.tenant_id, &held.endpoint, &held.client_key)
                .add(idempotency::Column::ExpiresAt.eq(held.expires_at)),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!(
                    "take over expired idempotency claim {}/{}/{}",
                    held.tenant_id, held.endpoint, held.client_key
                ),
                e,
            )
        })?;

    // Zero rows means the takeover race above was lost: another caller's
    // `UPDATE` already moved `expires_at` off the stamp this call read, so
    // the `WHERE` clause matches nothing left. Reporting that as `Claimed`
    // is exactly the defect this compare-and-swap exists to prevent — it
    // would tell two callers they both hold a key only one of them does.
    // It is not `InFlight` either: that outcome carries the held digest for
    // the caller to compare, and the digest now under the key is the
    // winner's, which this transaction never read (P-D-49).
    if result.rows_affected == 0 {
        return Ok(IdempotencyClaim::TakeoverRaceLost);
    }
    Ok(IdempotencyClaim::Claimed)
}

/// What [`answer_idempotency_key`] found the key in when it tried to write
/// the answer — an outcome the caller acts on, not an error this repository
/// picks a class for.
///
/// A returned enum, for the reason [`IdempotencyClaim`] is one: this layer
/// cannot tell the two callers of a missed answer apart. A door answering a
/// claim it took on this very runner moments ago meets
/// [`NotHeld`](Self::NotHeld) only if the store contradicts itself, and must
/// fail its mutation over it; a future lane answering a claim taken
/// elsewhere may legitimately find the key expired out from under it and
/// taken over by another caller (**P-D-49**), which is not a fault at all.
/// Raising a [`RepoError`] here would force the second reading into the
/// first, and a `Result<(), _>` that swallowed the zero-row case would be
/// exactly the silent success the claim's own compare-and-swap exists to
/// prevent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdempotencyAnswer {
    /// The row was `claimed` and is now `answered`, carrying both response
    /// columns.
    Recorded,
    /// No `claimed` row matched the key: it was never claimed, it was
    /// already answered, or it was taken over by another caller. **Nothing
    /// was written**, and the caller decides what that means for it.
    NotHeld,
}

/// Answer a held claim: move `(tenant_id, endpoint, client_key)` from
/// `claimed` to `answered`, recording the status and body the caller is
/// about to return (`design/01-foundation.md` §3.2
/// `inst-fd-idem-claim-write`, **P-D-29**; `dod-idempotency-store`).
///
/// # This function opens no transaction — `runner` MUST be the same one the
/// # claim and the mutation ran on
///
/// `inst-fd-idem-claim-write` is explicit that claim, mutation and answer
/// **commit together or not at all**. This function therefore takes the
/// caller's runner like every other function in this module, and that runner
/// MUST be the very transaction [`claim_idempotency_key`] and the guarded
/// mutation already ran on. Answering on a runner of its own would reopen
/// the gap P-D-42 closed from the other side: an answer that committed while
/// the mutation rolled back would replay a `201` for an act that never
/// happened, and a mutation that committed while the answer failed would
/// leave the key `claimed` over a committed act — the very defect this
/// function exists to remove.
///
/// # Both response columns are written in one statement
///
/// `chk_products_idempotency_response_group` admits `answered` only with
/// `response_status` **and** `response_body` non-null, so the state and both
/// columns move in a single `UPDATE`; a two-statement version could not
/// exist, since either half alone violates the `CHECK` at write time. The
/// stored pair is the whole of a replay (**P-D-29**): a bare reference to
/// the created entity could not reproduce the original status, and a refusal
/// has no entity to reference at all.
///
/// # A zero-row result is reported, never swallowed
///
/// The `UPDATE` carries `WHERE state = 'claimed'` beside the key, so it
/// cannot overwrite an answer another caller already recorded, and it
/// answers [`IdempotencyAnswer::NotHeld`] rather than
/// [`IdempotencyAnswer::Recorded`] when nothing matched — see that enum's
/// own doc for why the outcome is returned rather than raised.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error. A key that is
/// simply not held is **not** an error; it is
/// [`IdempotencyAnswer::NotHeld`].
pub async fn answer_idempotency_key(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    endpoint: &str,
    client_key: &str,
    response_status: i32,
    response_body: JsonValue,
) -> Result<IdempotencyAnswer, RepoError> {
    let result = idempotency::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            idempotency::Column::State,
            Expr::value("answered".to_owned()),
        )
        .col_expr(
            idempotency::Column::ResponseStatus,
            Expr::value(Some(response_status)),
        )
        .col_expr(
            idempotency::Column::ResponseBody,
            Expr::value(Some(response_body)),
        )
        .filter(
            idempotency_key_of(tenant_id, endpoint, client_key)
                .add(idempotency::Column::State.eq("claimed")),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("answer idempotency claim {tenant_id}/{endpoint}/{client_key}"),
                e,
            )
        })?;

    // Zero rows is a real answer, not a no-op to shrug at: no `claimed` row
    // matched, so the response this call was handed was never recorded and
    // the caller must not proceed as though it had been.
    if result.rows_affected == 0 {
        return Ok(IdempotencyAnswer::NotHeld);
    }
    Ok(IdempotencyAnswer::Recorded)
}

/// Release a `claimed` key that will not be answered — the scheduled lane's
/// deferral (P-D-157): a held run consumed nothing, so its claim row goes,
/// and the next sweep claims the same `(lane, transition)` afresh. Only a
/// `claimed` row matches; an `answered` one is a terminal record and stays.
///
/// # Errors
///
/// [`RepoError`] on a driver failure.
pub async fn release_idempotency_claim(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    endpoint: &str,
    client_key: &str,
) -> Result<u64, RepoError> {
    let result = idempotency::Entity::delete_many()
        .secure()
        .scope_with(scope)
        .filter(
            idempotency_key_of(tenant_id, endpoint, client_key)
                .add(idempotency::Column::State.eq("claimed")),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("release idempotency claim {tenant_id}/{endpoint}/{client_key}"),
                e,
            )
        })?;
    Ok(result.rows_affected)
}

/// One `tenant_id` from a DISTINCT discovery projection — the sweeps'
/// shared row shape.
#[derive(FromQueryResult)]
struct TenantIdRow {
    /// The discovered tenant.
    tenant_id: Uuid,
}

// The four non-foundation aggregates live in per-aggregate submodules,
// re-exported flat so every caller keeps addressing `repo::*` unchanged.
/// Whether a Product holds a `primary` category assignment
/// (`inst-tx-primary-at-publish`'s operand).
///
/// The publish door reads this before its pipeline opens, because
/// `ValidationRule::evaluate` is synchronous and cannot reach another table.
///
/// # Errors
///
/// [`RepoError`] as the read raises it.
pub async fn has_primary_category(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
) -> Result<bool, RepoError> {
    let found = product_category::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product_category::Column::TenantId.eq(tenant_id))
                .add(product_category::Column::ProductId.eq(product_id))
                .add(product_category::Column::Role.eq("primary")),
        )
        .one(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("read the primary category assignment of product {product_id}"),
                e,
            )
        })?;
    Ok(found.is_some())
}

/// Flip the subject's **open** approval record to `superseded`, returning its
/// id where one was open (`dod-supersede`).
///
/// The partial `UNIQUE (tenant_id, subject_kind, subject_ref) WHERE state IN
/// ('pending','satisfied')` admits at most one open record, and the read below
/// names it — but the **`UPDATE` carries the open-state predicate too**, and
/// that is the part that makes the concurrency claim true rather than merely
/// stated. An earlier revision filtered the write by id alone: two concurrent
/// frozen-content writes both read the open row, the winner finalized it, and
/// the loser's write hit the append-only trigger, so a **legal** act answered
/// 500. With the predicate the loser matches zero rows and reports `None`.
///
/// **Nothing is re-submitted.** `inst-gv-supersede` requires re-submission to
/// be *"an explicit human act ... never automatic — auto-resubmit would pin
/// content nobody re-read"*, so this function writes exactly one row and
/// creates none.
///
/// # Errors
///
/// [`RepoError`] as the statement raises it.
pub async fn supersede_open_approval(
    runner: &impl DBRunner,
    _door_scope: &AccessScope,
    tenant_id: Uuid,
    subject: &crate::domain::governance::GateSubject,
    now: DateTime<Utc>,
) -> Result<Option<Uuid>, RepoError> {
    // Tenant-scoped for `gate_candidates`' reason: `products_approval`'s
    // `resource_col` is `approval_id`, so a door scope pinned to the entity it
    // gates would supersede **nothing** — silently, since a supersession that
    // matches no row is not an error. See that function's note (benidorm,
    // 2026-09-06).
    let scope = &AccessScope::for_tenant(tenant_id);
    let open = approval::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(tenant_id))
                .add(approval::Column::SubjectKind.eq(subject.kind.as_str()))
                .add(approval::Column::SubjectRef.eq(subject.reference.clone()))
                .add(approval::Column::State.is_in(["pending", "satisfied"])),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read the open approval of {tenant_id}"), e))?;
    let Some(open) = open else {
        return Ok(None);
    };
    let approval_id = open.approval_id;
    let outcome = approval::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            approval::Column::State,
            Expr::value("superseded".to_owned()),
        )
        .col_expr(approval::Column::FinalizedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(tenant_id))
                .add(approval::Column::ApprovalId.eq(approval_id))
                // The open-state predicate belongs HERE, not only on the read
                // above: without it a racer that finalized the record between
                // the two statements is overwritten, and the append-only
                // trigger refuses the write — a legal act dying on a 500.
                .add(approval::Column::State.is_in(["pending", "satisfied"])),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("supersede approval {approval_id}"), e))?;
    if outcome.rows_affected == 0 {
        // A peer finalized it first. Superseding is idempotent from the
        // caller's side: the record is closed either way, and the frozen
        // -content write that triggered this is legal regardless.
        return Ok(None);
    }
    Ok(Some(approval_id))
}

mod bulk;
mod governance;
mod increment;
mod lifecycle;
mod pii_allowlist;
mod read_models;
mod recognized;
mod reference;
mod retention;
mod retention_gc;
mod taxonomy;
mod versions;

pub use bulk::*;
pub use governance::*;
pub use increment::*;
pub use lifecycle::*;
pub use pii_allowlist::*;
pub use read_models::*;
pub use recognized::*;
pub use reference::*;
pub use retention::*;
pub use retention_gc::*;
pub use taxonomy::*;
pub use versions::*;

/// Stamp the composite act's parent handle onto a `claimed` key (P-D-79).
///
/// # `runner` MUST be the transaction that took the claim
///
/// The stamp shares the claim's atomicity contract: claim `INSERT`, parent
/// row and stamp commit together or not at all, which is what lets a
/// same-key retry read a committed-but-unanswered claim as *in progress —
/// resume from `entity_ref`* rather than as a half-written record. Stamping
/// on a runner of its own would open the window this column exists to
/// close.
///
/// # Errors
///
/// [`RepoError::Db`] when no `claimed` row matched — the caller took the
/// claim on this very transaction, so a missing row means the store
/// contradicts itself, exactly [`answer_idempotency_key`]'s `NotHeld`
/// posture at that call site.
pub async fn stamp_idempotency_entity_ref(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    endpoint: &str,
    client_key: &str,
    entity_ref: Uuid,
) -> Result<(), RepoError> {
    let result = idempotency::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            idempotency::Column::EntityRef,
            Expr::value(Some(entity_ref)),
        )
        .filter(
            idempotency_key_of(tenant_id, endpoint, client_key)
                .add(idempotency::Column::State.eq("claimed")),
        )
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("stamp idempotency entity_ref {tenant_id}/{endpoint}/{client_key}"),
                e,
            )
        })?;

    if result.rows_affected == 0 {
        return Err(RepoError::Db(format!(
            "idempotency key {client_key} on {endpoint} was claimed by this transaction but no \
             claimed row remained to stamp"
        )));
    }
    Ok(())
}

/// Which head table a frozen row belongs to.
///
/// `products_entity_version.entity_kind` is a closed roster on both engines
/// (`chk_products_entity_version_entity_kind`), so it is carried as an
/// enumeration here rather than as a caller-supplied string: a third kind is
/// a migration, never a typo at a call site.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VersionedEntityKind {
    /// A `products_product` head.
    Product,
    /// A `products_sku` head.
    Sku,
}

impl VersionedEntityKind {
    /// The stored token, matching the `CHECK` roster and the head-row
    /// guard's own literal.
    fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Sku => "sku",
        }
    }
}

/// The row a freeze supplies to `products_entity_version`
/// (`design/01-foundation.md` §4.3).
///
/// [`Self::content`] and [`Self::content_digest`] are **inputs**, not
/// computed here: the canonical rendering and the `SHA-256` over it are the
/// publish door's, and this repository stores the bytes it is handed. Any
/// re-rendering on the way to storage would put the digest and slice 10's
/// byte-for-byte restore drill on different bytes, which is the one property
/// the column exists to have.
#[derive(Clone, Debug)]
pub struct NewEntityVersion {
    /// Owning tenant.
    pub tenant_id: Uuid,
    /// Which head table [`Self::entity_id`] names.
    pub entity_kind: VersionedEntityKind,
    /// The head row's own id.
    pub entity_id: Uuid,
    /// The version being frozen, `>= 1`.
    pub published_version: i64,
    /// The canonical rendering itself, exactly the bytes
    /// [`Self::content_digest`] was computed over.
    pub content: String,
    /// `SHA-256` over [`Self::content`] as handed in, computed by the door.
    pub content_digest: Vec<u8>,
    /// The digest scheme [`Self::content_digest`] was computed under.
    pub digest_version: i32,
    /// The authorizing `ApprovalRecord`'s id on a yes verdict, where the
    /// gate that mints one has run.
    pub approval_ref: Option<Uuid>,
    /// The pseudonymous ref of whoever published.
    pub actor_ref: Uuid,
    /// The publish instant.
    pub published_at: DateTime<Utc>,
    /// What the metering `usageTypeRef` resolved to at publish, as
    /// `UsageTypeBinding::snapshot_json` renders it — **provenance beside the
    /// content, outside the digest** (`dod-binding-snapshot`, P-D-134 row 6,
    /// P-D-146). `None` for a Product row and for a SKU with no meter.
    pub binding_snapshot: Option<String>,
}

/// What a guarded head-row `UPDATE` found: the outcome the caller acts on,
/// never a silent zero-row success.
///
/// A returned enum for [`IdempotencyAnswer`]'s reason: this layer cannot
/// tell the readings of a zero-row result apart, and only the door can. On a
/// publish, [`Self::Unmatched`] means the head no longer carries the expected
/// `internal_revision` — `STALE_REVISION` — or it is terminal, or it lies
/// outside `scope`. On a discard it means the same, plus the row not being
/// legal to discard. A door that needs to tell those apart re-reads the head
/// through [`find_product`]/[`find_sku`] to pick the refusal code; that read
/// decides only which message is returned, never whether the write landed,
/// so it carries none of the race a read-then-write would.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadWrite {
    /// The `UPDATE` matched its row and the write landed.
    Applied,
    /// No row matched the filter. **Nothing was written.**
    Unmatched,
}

/// What `07`'s correction door hands the publish statement
/// (`dod-correction-republish`, **P-D-41**'s "optional third argument"): the
/// bucket-ii column(s) and their new values, plus the `correction_ref` that
/// is the version row's physical door identity (**P-D-129** row 6). Written
/// **in the bump statement**, never on their own — the row-image trigger
/// refuses either half alone after first publish.
#[derive(Debug, Clone)]
pub struct CorrectionWrite {
    /// The fresh reference this correction is known by; the same value the
    /// override row and the audit row carry on a break-glass lane.
    pub correction_ref: Uuid,
    /// The bucket-ii columns and their new values (`None` clears).
    pub columns: Vec<(sku::Column, Option<String>)>,
}

/// Freeze one version row: insert `products_entity_version` for
/// `(tenant_id, entity_kind, entity_id, published_version)`
/// (`design/01-foundation.md` §4.3, `inst-fd-publish-freeze`).
///
/// # This function opens no transaction — `runner` MUST be the publish's own
///
/// It takes the caller's runner like every other function in this module,
/// and that runner MUST be the very transaction the head-row `UPDATE` runs
/// on. This is not a preference: the head-row guard admits a
/// `published_version` bump **only where the matching frozen row already
/// exists**, so the freeze has to be visible to the bump — and a freeze that
/// committed on a runner of its own would leave a version row behind when
/// the publish rolled back, after which the guard would admit a later bump
/// to a version no committed act ever produced. Freeze first, bump second,
/// one transaction (`inst-fd-publish-txn`).
///
/// # The digest is an input, and this function computes nothing
///
/// `content` and `content_digest` arrive already computed. Rendering the
/// content canonically and hashing it is the publish door's work — it is the
/// door that knows the post-act image (**P-D-33**) — and this repository
/// deliberately imports no canonicalizer: bytes re-rendered on the way to
/// storage are no longer guaranteed to be the bytes the digest was taken
/// over, which is the single property slice 10's restore drill depends on.
/// `digest_version` is likewise stored as handed in, so that "a digest-version
/// bump, not a silent change" stays checkable from the row alone
/// (**P-D-29**, **P-D-33**).
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error. The driver failures include a
/// duplicate key — the same version of the same entity frozen twice — and
/// the `chk_products_entity_version_published_version` /
/// `chk_products_entity_version_digest_version` lower bounds. The table is
/// append-only with no `UPDATE` path at all, so a re-freeze is a refusal
/// rather than an overwrite.
/// Read the newest frozen version row of one entity, or `None` for a
/// never-published one.
///
/// The clone door's read (`inst-cn-door`): a `retired`, `published` or
/// `deprecated` source reads its entity content from the **last frozen
/// version** — never a head's pending edits, which would leak in-flight
/// unapproved content — and `cloned_from_version` records exactly the version
/// this returned (P-D-76).
pub async fn latest_entity_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: VersionedEntityKind,
    entity_id: Uuid,
) -> Result<Option<(i64, String)>, RepoError> {
    let row = entity_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(entity_version::Column::TenantId.eq(tenant_id))
                .add(entity_version::Column::EntityKind.eq(entity_kind.as_str()))
                .add(entity_version::Column::EntityId.eq(entity_id)),
        )
        .order_by(
            entity_version::Column::PublishedVersion,
            sea_orm::Order::Desc,
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read latest frozen version of {entity_id}"), e))?;

    Ok(row.map(|row| (row.published_version, row.content)))
}

pub async fn insert_entity_version(
    runner: &impl DBRunner,
    scope: &AccessScope,
    new: NewEntityVersion,
) -> Result<(), RepoError> {
    let model = entity_version::ActiveModel {
        tenant_id: Set(new.tenant_id),
        entity_kind: Set(new.entity_kind.as_str().to_owned()),
        entity_id: Set(new.entity_id),
        published_version: Set(new.published_version),
        content: Set(new.content),
        content_digest: Set(new.content_digest),
        digest_version: Set(new.digest_version),
        approval_ref: Set(new.approval_ref),
        actor_ref: Set(new.actor_ref),
        published_at: Set(new.published_at),
        binding_snapshot: Set(new.binding_snapshot),
    };

    entity_version::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| {
            driver_failure(
                format!(
                    "freeze {} {} v{} scope",
                    new.entity_kind.as_str(),
                    new.entity_id,
                    new.published_version
                ),
                e,
            )
        })?
        .exec(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!(
                    "freeze {} {} v{}",
                    new.entity_kind.as_str(),
                    new.entity_id,
                    new.published_version
                ),
                e,
            )
        })?;

    Ok(())
}

/// The head states a publish is admitted from: a `draft` for its first
/// publish, a `published` or `deprecated` head for version N+1
/// (`inst-fd-publish-pin`).
///
/// Carried in the `UPDATE`'s own filter rather than checked by a prior read,
/// for the reason [`discard_product_head`] gives at greater length.
///
/// # Two enforcements stand between a terminal head and a version N+1, and both are wanted
///
/// The **first** is physical. `m20260829_000002_create_products_product.rs`
/// and `m20260829_000003_create_products_sku.rs` each raise on a bump off a
/// terminal head, on both engines — Postgres as
///
/// ```sql
/// IF NEW.published_version IS DISTINCT FROM OLD.published_version
///    AND OLD.lifecycle_state IN ('retired', 'discarded')
/// THEN
///   RAISE EXCEPTION 'products_product: a published_version bump is not admitted on a terminal head';
/// END IF;
/// ```
///
/// and `SQLite` as `trg_products_product_published_version_terminal` /
/// `trg_products_sku_published_version_terminal`, whose `WHEN` is the same
/// pair of conditions spelled `IS NOT` / `IN`. The **second** is this
/// filter.
///
/// An earlier version of this doc called the filter "the only thing
/// standing between a terminal head and a version N+1 nobody may publish".
/// That held when it was written and stopped holding later in the same
/// phase, when both migrations gained the clause quoted above. It is
/// corrected here rather than deleted because the stale wording invites the
/// opposite error: a reader who reconstructs the pairing from this file
/// alone could strike the trigger clause as redundant, and the trigger is
/// the half that does not depend on any caller composing the right `WHERE`.
///
/// Neither half subsumes the other. The trigger refuses the write however
/// it is issued — a future door, a migration, a repair script — and is
/// therefore the invariant. The filter is what makes the refusal *usable*:
/// a terminal head simply falls outside the `UPDATE`'s match set, so the
/// door gets `rows_affected == 0` and classifies it through the ordinary
/// `Unmatched` path into `ENTITY_TERMINAL`, instead of a raised database
/// exception it would have to parse a driver message to tell apart from a
/// genuine storage failure.
///
/// Note also that the neighbouring head-row guard clauses do **not** cover
/// this case on their own: a bump on a `retired` head changes no
/// `lifecycle_state`, so the edge clause never fires, and the guard's
/// `published_version` clause is satisfied by the frozen version row alone.
/// That is why the terminal clause had to be added to the trigger as a
/// clause of its own.
const PUBLISHABLE_HEAD_STATES: [&str; 3] = [
    LifecycleState::Draft.as_str(),
    LifecycleState::Published.as_str(),
    LifecycleState::Deprecated.as_str(),
];

/// Publish a Product head: one `UPDATE` carrying the version bump, the
/// revision bump, the `draft -> published` edge and `updated_at`
/// (`inst-fd-publish-freeze`, `inst-fd-publish-bump`).
///
/// # This function opens no transaction, and the freeze MUST precede it on
/// # the same runner
///
/// [`insert_entity_version`] for `published_version + 1` must already have
/// run on this very `runner`: the head-row guard admits the bump only where
/// that row exists, so the reverse order trips the guard on every publish
/// and a separately committed freeze survives a rolled-back publish.
///
/// # Exactly one statement, and why that is load-bearing
///
/// `inst-fd-publish-bump` requires `internal_revision` to move **once**, and
/// the guard bumps nothing itself — it *refuses* any `UPDATE` whose
/// `internal_revision` is not `OLD + 1`, "on every admitted UPDATE, without
/// exception". Two statements would therefore move the revision twice for
/// one act, and the `ETag` a client holds would skip a value the door never
/// returned. So the version bump, the revision bump, the state and
/// `updated_at` all ride one `UPDATE`.
///
/// # The edge is decided by the row image, not by the caller
///
/// `lifecycle_state` is written through a `CASE` that maps `draft` to
/// `published` and leaves every other value as it stands. A re-publish from
/// a `published` or `deprecated` head takes no edge, and writing
/// `'published'` unconditionally would flip a `deprecated` head back —
/// a state change the transition door owns and the two-person ceremony
/// governs. Deciding it in the statement rather than from a prior read also
/// keeps the decision on the row image the write actually lands on.
///
/// # What this statement does not yet write, and who owes it
///
/// Two columns `inst-fd-publish-txn` puts in this same `UPDATE` are absent
/// here, deliberately and not silently:
///
/// - **`composition_pending`** (§4.2, **P-D-32**) — **this** function will
///   never write it on any wave, and the reason is the schema rather than a
///   schedule: it is the Product publish, and `products_product` has no such
///   column, because `bundle` is a value of the SKU-only `type` column (§4.2).
///   The column, its guard clause and now its write are all
///   [`publish_sku_head`]'s, which carries the flag as a parameter; see that
///   function's own doc for the one narrowing still owed there.
/// - **A corrected bucket-ii value** (`inst-fd-publish-correction`,
///   **P-D-41**) — the door that supplies one is slice 07's `CorrectionDoor`,
///   which has no caller here to hand it in. §4.2 admits a bucket-ii write
///   only in the same statement as a `published_version` bump, so when 07
///   lands, its value must be carried by **this** statement rather than by a
///   second one, on the "once" argument above.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error. The driver failures include
/// the head-row guard's refusal of a bump whose frozen version row is missing —
/// which is a raised refusal, not a zero-row result. A stale revision is
/// **not** an error; it is [`HeadWrite::Unmatched`].
pub async fn publish_product_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
    expected_internal_revision: i64,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let next_state: SimpleExpr = Expr::case(
        Expr::col(product::Column::LifecycleState).eq(LifecycleState::Draft.as_str()),
        Expr::val(LifecycleState::Published.as_str()),
    )
    .finally(Expr::col(product::Column::LifecycleState))
    .into();

    let result = product::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            product::Column::PublishedVersion,
            Expr::col(product::Column::PublishedVersion).add(Expr::val(1_i64)),
        )
        .col_expr(
            product::Column::InternalRevision,
            Expr::col(product::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(product::Column::LifecycleState, next_state)
        .col_expr(product::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::ProductId.eq(product_id))
                .add(product::Column::InternalRevision.eq(expected_internal_revision))
                .add(product::Column::LifecycleState.is_in(PUBLISHABLE_HEAD_STATES)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("publish product {product_id}"), e))?;

    // Zero rows is an answer, not a no-op: the head no longer carries the
    // revision the door pinned its approval to, or it is terminal, or it is
    // another tenant's. Reporting it as success would tell the door its
    // version landed while the head still carries someone else's content,
    // and the frozen row it wrote a statement earlier would be the only
    // trace of a publish that never happened.
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Publish a SKU head, in one `UPDATE`, on [`publish_product_head`]'s terms
/// exactly — same freeze-first ordering, same single-statement rule, same
/// `CASE`-decided edge — **plus `composition_pending`**, which only this twin
/// has a column for.
///
/// # `composition_pending` rides this statement and can ride no other
///
/// `inst-fd-publish-freeze`: *"On a `bundle` SKU that same `UPDATE` also
/// carries `composition_pending` — set where this publish carried the
/// uncomposed-bundle override, cleared where it did not"* (§4.2, **P-D-32**).
/// `composition_pending` is a `products_sku` column and no other table has
/// one, and `m20260829_000003_create_products_sku`'s guard admits a change to
/// it **only in the same statement as a `published_version` bump** — so this
/// `UPDATE`, the single statement that bumps the version, is the one place in
/// the gear the flag can move at all. A second statement would be refused by
/// the trigger, and would also break `inst-fd-publish-bump`'s "once" on the
/// way there.
///
/// `composition_pending` is therefore a **parameter**, not a value this
/// function derives: it is the door's gate verdict
/// (`domain::governance::GateAuthorization::uncomposed_bundle_override`) that
/// says whether this act carried the override, and a repository that guessed
/// would be a second answer to a question the ceremony already answered.
/// Writing the same value the row already holds is not a change and does not
/// trip the guard (`IS DISTINCT FROM`), so the "cleared where it did not"
/// half costs nothing on a row that was never raised.
///
/// # What is still owed here, and it is a narrowing rather than a write
///
/// The instruction scopes the clause to a **`bundle`** SKU. `bundle` is a
/// value of the `type` column, which is **slice 03's** and does not exist on
/// `products_sku` at this commit, so there is no operand this statement could
/// test. What is built is the clause with its subject widened to every SKU:
/// set from the override, cleared without it. That is exactly right wherever
/// the override is granted — and the override is itself a bundle-composition
/// ceremony's, so a non-`bundle` SKU has nothing to carry one — but it is
/// **not** the narrowing the instruction states, and it is recorded as owed to
/// slice 03 rather than reported as present. When `type` lands, the condition
/// joins this statement's `col_expr` as a `CASE`, not as a caller-side `if`,
/// for the same reason the edge's `CASE` is here: the row image the write
/// lands on is what must be tested.
///
/// The bucket-ii correction is owed by both twins alike; see
/// [`publish_product_head`].
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error. The driver failures include
/// the head-row guard's refusal of a bump whose frozen version row is missing. A
/// stale revision is [`HeadWrite::Unmatched`], not an error.
#[allow(clippy::too_many_arguments)]
pub async fn publish_sku_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    expected_internal_revision: i64,
    composition_pending: bool,
    now: DateTime<Utc>,
    correction: Option<&CorrectionWrite>,
) -> Result<HeadWrite, RepoError> {
    let next_state: SimpleExpr = Expr::case(
        Expr::col(sku::Column::LifecycleState).eq(LifecycleState::Draft.as_str()),
        Expr::val(LifecycleState::Published.as_str()),
    )
    .finally(Expr::col(sku::Column::LifecycleState))
    .into();

    let mut update = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            sku::Column::PublishedVersion,
            Expr::col(sku::Column::PublishedVersion).add(Expr::val(1_i64)),
        )
        .col_expr(
            sku::Column::InternalRevision,
            Expr::col(sku::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(sku::Column::LifecycleState, next_state)
        // In this statement and no other: the head-row guard admits a change
        // to the flag only alongside the `published_version` bump above.
        .col_expr(
            sku::Column::CompositionPending,
            Expr::value(composition_pending),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now));
    // The correction rides the bump statement (P-D-41, P-D-129 row 6): the
    // bucket-ii column(s) and `correction_ref` move in the same `UPDATE` as
    // `published_version`, which is the only form the row-image trigger
    // admits after first publish.
    if let Some(correction) = correction {
        update = update.col_expr(
            sku::Column::CorrectionRef,
            Expr::value(Some(correction.correction_ref)),
        );
        for (column, value) in &correction.columns {
            update = update.col_expr(*column, Expr::value(value.clone()));
        }
    }
    let result = update
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::SkuId.eq(sku_id))
                .add(sku::Column::InternalRevision.eq(expected_internal_revision))
                .add(sku::Column::LifecycleState.is_in(PUBLISHABLE_HEAD_STATES)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("publish sku {sku_id}"), e))?;

    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Deprecate one Product head — `published → deprecated`, stamping
/// `deprecation_provenance` in the **same statement**
/// (`inst-lc-deprecate`, `dod-deprecation-provenance`).
///
/// # The stamp and the transition are one `UPDATE`, and that is not a choice
///
/// The head guard's row-image predicate admits
/// `deprecation_provenance` *only* where `lifecycle_state` changes in the
/// same statement (**P-D-34**, installed in `m20260829_000002`/`000003`), so
/// two statements would be refused by the trigger — the second one, with a
/// message about the provenance, on a row the first had already moved. The
/// pairing is therefore physical, not conventional.
///
/// # Filters, and what each one refuses
///
/// `lifecycle_state = 'published'` is the edge's own precondition, and it is
/// what makes an already-`deprecated` head answer [`HeadWrite::Unmatched`]
/// rather than taking a second stamp — `domain::deprecation::stamp_for`
/// gives the same answer application-side, and this filter is why a caller
/// that skipped it still cannot re-stamp. `internal_revision` is the
/// caller's `If-Match` pin (**P-D-33**).
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. An inadmissible deprecation
/// is [`HeadWrite::Unmatched`], not an error — the door has already read the
/// head and can name which precondition failed.
pub async fn deprecate_product_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
    expected_internal_revision: i64,
    provenance: Provenance,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let result = product::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            product::Column::LifecycleState,
            Expr::value(LifecycleState::Deprecated.as_str()),
        )
        .col_expr(
            product::Column::DeprecationProvenance,
            Expr::value(provenance.as_str()),
        )
        .col_expr(
            product::Column::InternalRevision,
            Expr::col(product::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(product::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::ProductId.eq(product_id))
                .add(product::Column::InternalRevision.eq(expected_internal_revision))
                .add(product::Column::LifecycleState.eq(LifecycleState::Published.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("deprecate product {product_id}"), e))?;

    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Cascade one child SKU of a deprecating parent — `published → deprecated`
/// stamped [`Provenance::Cascaded`], **pinned at the revision the
/// classification read** (`inst-lc-deprecate`, `dod-deprecation-cascade`).
///
/// # Why per child, and why the revision is pinned
///
/// The disposition per child state is `domain::deprecation::disposition_for`'s
/// to decide, and the listing of skipped drafts is what the operator sees —
/// so the door reads the children inside its own transaction, classifies
/// them, and calls this once per child classified `Deprecate`. The pin
/// mirrors the parent's own semantics (**P-D-33**: an act runs against the
/// image it was decided on): with it, the `SkuDeprecated` event's
/// `internal_revision` is the pinned value plus one **as committed by this
/// write**, never arithmetic on a row a concurrent save may have moved — and
/// a child that moved between the classification and this statement answers
/// [`HeadWrite::Unmatched`], failing the whole mutation closed
/// (`01 inst-fd-fail-closed`) as a retryable refusal rather than committing
/// a half-cascade or announcing a revision the row never held.
///
/// The state filter stays as well, for [`deprecate_product_head`]'s reason:
/// an already-`deprecated` child must never take a second stamp, whatever
/// the caller classified.
///
/// # Errors
///
/// [`RepoError::Driver`] on a storage failure. A moved child is
/// [`HeadWrite::Unmatched`], the caller's to refuse.
pub async fn cascade_deprecate_child(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    expected_internal_revision: i64,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let result = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            sku::Column::LifecycleState,
            Expr::value(LifecycleState::Deprecated.as_str()),
        )
        .col_expr(
            sku::Column::DeprecationProvenance,
            Expr::value(Provenance::Cascaded.as_str()),
        )
        .col_expr(
            sku::Column::InternalRevision,
            Expr::col(sku::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::SkuId.eq(sku_id))
                .add(sku::Column::InternalRevision.eq(expected_internal_revision))
                .add(sku::Column::LifecycleState.eq(LifecycleState::Published.as_str())),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("cascade the deprecation onto sku {sku_id}"), e))?;

    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Discard a never-published draft Product: `lifecycle_state = 'discarded'`,
/// one revision bump, `updated_at` (`inst-fd-discard`).
///
/// # This function opens no transaction — `runner` is the discard act's
///
/// Like every function in this module it joins the caller's transaction, so
/// the state write, the act's audit row's governing mutation and its outbox
/// row commit together or not at all.
///
/// # The legality lives in the `WHERE` clause, not in a prior read
///
/// `inst-fd-discard` admits the act only from `draft` with
/// `published_version = 0`. Both conditions are carried by the statement's
/// own filter, beside the expected `internal_revision`, so the **database**
/// judges the row image the write lands on. A read-then-write would be a
/// race: a concurrent publish between the read and the write would leave
/// this statement discarding a head that is published by the time it runs,
/// and the head-row guard would admit it — `published -> discarded` is not an
/// edge, so the guard would refuse *that* one, but `draft -> discarded` on a
/// head that published and was somehow still `draft` is exactly the shape no
/// guard can catch. The reservation and the claim in this same module make
/// the identical argument for the identical reason.
///
/// # The reservations release by this same write, with no second statement
///
/// `uq_products_product_name` and `uq_products_product_code` are both partial
/// on `lifecycle_state <> 'discarded'`, so the row leaves both indexes the
/// moment this `UPDATE` commits: the name and the `productCode` are free for
/// the next holder. There is no release statement here because there is
/// nothing left to release — a separate one would have no rows to touch.
/// This is why a discarded draft releases its name while a `retired` entity
/// keeps it: the predicate names `discarded` alone.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error. A row that is
/// not legal to discard, or that no longer carries the expected revision, is
/// **not** an error; it is [`HeadWrite::Unmatched`] — see that enum's doc for
/// how a door tells the readings apart.
pub async fn discard_product_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
    expected_internal_revision: i64,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let result = product::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            product::Column::LifecycleState,
            Expr::value(LifecycleState::Discarded.as_str()),
        )
        .col_expr(
            product::Column::InternalRevision,
            Expr::col(product::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(product::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::ProductId.eq(product_id))
                .add(product::Column::InternalRevision.eq(expected_internal_revision))
                .add(product::Column::LifecycleState.eq(LifecycleState::Draft.as_str()))
                .add(product::Column::PublishedVersion.eq(0_i64)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("discard product {product_id}"), e))?;

    // Zero rows is an answer: the head was published, already terminal,
    // moved under the door's pinned revision, or belongs to another tenant.
    // Reporting it as success would emit `ProductDiscarded` for a row that
    // is still live and still holding its name.
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Discard a never-published draft SKU, on [`discard_product_head`]'s terms
/// exactly.
///
/// The reservation released here is `uq_products_sku_code` — the `skuCode`
/// reservation itself (`dod-code-reservation`), partial on the same
/// `lifecycle_state <> 'discarded'` predicate, so it releases by this write
/// and by no separate statement.
///
/// # Errors
/// [`RepoError::Driver`] on a storage failure, or [`RepoError::Db`] on a
/// scope refusal that raised no driver error. An
/// inadmissible discard is [`HeadWrite::Unmatched`], not an error.
pub async fn discard_sku_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    expected_internal_revision: i64,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    let result = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            sku::Column::LifecycleState,
            Expr::value(LifecycleState::Discarded.as_str()),
        )
        .col_expr(
            sku::Column::InternalRevision,
            Expr::col(sku::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now))
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::SkuId.eq(sku_id))
                .add(sku::Column::InternalRevision.eq(expected_internal_revision))
                .add(sku::Column::LifecycleState.eq(LifecycleState::Draft.as_str()))
                .add(sku::Column::PublishedVersion.eq(0_i64)),
        )
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("discard sku {sku_id}"), e))?;

    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// The two head states no head write is admitted on at all
/// (`inst-fd-terminal`, **P-D-25** widened by **P-D-32**).
///
/// Both save statements below carry it in their own `WHERE` clause. The door
/// asks `transition::check_head_write` first and answers `ENTITY_TERMINAL`
/// from the row it read; this copy is the one that decides, because a
/// neighbouring transition can retire the head between that read and this
/// write and only the statement judges the image the write lands on. Exactly
/// the argument [`discard_product_head`] makes for its own filter.
pub(crate) const TERMINAL_HEAD_STATES: [&str; 2] = [
    LifecycleState::Retired.as_str(),
    LifecycleState::Discarded.as_str(),
];

/// A `name` save, with the normalization that has to travel with it.
///
/// One field rather than two, and that is the whole reason the type exists:
/// `name` and `name_normalized` are one bucket-iii field in §4.1 — the second
/// being *"the same field's index operand"*, the operand
/// `uq_products_product_name` enforces on — so a statement that moved one
/// without the other would leave the uniqueness index keyed to a name the row
/// no longer carries, and nothing downstream would notice until the next
/// author collided with a name nobody holds. Two `Option` fields would have
/// admitted exactly that pair of states; this one admits neither.
///
/// The normalization itself is `crate::domain::name::normalize`'s and is
/// computed by the door, for the reason [`insert_entity_version`] states about
/// its own digest: this module renders nothing it stores.
#[derive(Clone, Debug)]
pub struct SavedName {
    /// The operator-facing name, as authored.
    pub value: String,
    /// `crate::domain::name::normalize(value)`, as the door computed it.
    pub normalized: String,
}

/// A save of a nullable text column: the caller either named a value or asked
/// for the column to be cleared.
///
/// An enum rather than the `Option<Option<String>>` the two states would
/// otherwise need, which is both a denied lint and — more to the point —
/// a shape whose two `None`s a reader has to hold apart by position. Only
/// `products_product.product_code` is nullable among the columns either save
/// statement writes, so this is its type and no other's.
#[derive(Clone, Debug)]
pub enum NullableText {
    /// Write this value.
    Set(String),
    /// Write `NULL`, releasing whatever reservation the old value held —
    /// `uq_products_product_code` is partial on `product_code IS NOT NULL`,
    /// so a cleared code leaves the index by this write and by no other.
    Clear,
}

/// The columns one Product save writes, each `None` where the request did not
/// carry the field.
///
/// Every member is a **bucket-i or bucket-iii** column of §4.1 and nothing
/// else: the mechanical columns this statement moves — `internal_revision`
/// and `updated_at` — are the statement's own and are not caller inputs, and
/// row identity is admitted in no `UPDATE` at all (**P-D-34**), so neither
/// class has a field here to be set from. Which bucket each column is in, and
/// what the door may do with it, is `crate::domain::bucket`'s answer and not
/// this module's; what is here is the set of columns a routed save can reach.
#[derive(Clone, Debug, Default)]
pub struct ProductHeadSave {
    /// Bucket i (§4.1): the brand the row belongs to.
    pub brand_id: Option<Uuid>,
    /// Bucket i: the external mapping code, clearable.
    pub product_code: Option<NullableText>,
    /// Bucket iii: the name and its index operand, inseparable
    /// ([`SavedName`]).
    pub name: Option<SavedName>,
    /// Bucket iii, in both directions.
    pub region_scope: Option<String>,
    /// Bucket iii, in both directions.
    pub brand_scope: Option<String>,
    /// Whether the same act writes **`02`'s content rows** — a category
    /// assignment set or an attribute value — beside this head.
    ///
    /// # It widens [`empty_save`]'s operand rather than bypassing its guard
    ///
    /// That guard's own words are *"no statement in this module bumps
    /// `internal_revision` without writing a content column"*, and its reason
    /// is that *"a bare bump is a write with no content that still invalidates
    /// every `ETag` a client holds."* Both stand. What changed is where this
    /// gear's content lives: when the guard was written the only content was
    /// this table's columns, and `design/02` C2 now makes attribute values
    /// **entity content** in as many words — *"they ride the owning entity's
    /// internal revision, freeze into its published versions"*.
    ///
    /// So a save naming only `categories` is not a bare bump. It is a content
    /// write whose content is in another table, and the revision **must** move:
    /// values riding a revision that did not change would leave two different
    /// content states sharing one `ETag`, which is the concurrency the ride
    /// exists to give them.
    ///
    /// `false` by `Default`, so every caller that predates `02`'s content is
    /// unchanged and the guard keeps refusing a genuinely empty save.
    pub content_moved: bool,
}

impl ProductHeadSave {
    /// Whether this save carries a **bucket-i** column, and therefore whether
    /// [`save_product_head`]'s filter must also pin `published_version = 0`.
    ///
    /// Derived from the fields rather than passed in beside them: a `bool`
    /// argument saying which buckets a value carries is a second answer to a
    /// question the value itself already answers, and the two could disagree.
    const fn touches_structural(&self) -> bool {
        self.brand_id.is_some() || self.product_code.is_some()
    }

    /// Whether the save names no column at all.
    ///
    /// The door refuses an empty save before reaching this module; the check
    /// is here so that a later caller cannot turn an empty payload into a
    /// bare `internal_revision` bump — a write with no content that would
    /// still invalidate every `ETag` a client holds.
    const fn is_empty(&self) -> bool {
        self.brand_id.is_none()
            && self.product_code.is_none()
            && self.name.is_none()
            && self.region_scope.is_none()
            && self.brand_scope.is_none()
            && !self.content_moved
    }
}

/// [`ProductHeadSave`]'s SKU twin. Three differences, all the schema's: there
/// is no `name` — `products_sku` has no such column, which is why a `name`
/// field arriving for a SKU is a registry miss rather than a routed save —
/// and `product_id` is the **parent link** and bucket-i (§4.1, the owner's
/// call of 2026-08-27), where on the Product the identically named column is
/// the primary key and is admitted in no `UPDATE` at all.
#[derive(Clone, Debug, Default)]
pub struct SkuHeadSave {
    /// Bucket i: the code `uq_products_sku_code` reserves.
    pub sku_code: Option<String>,
    /// Bucket i: the parent link.
    pub product_id: Option<Uuid>,
    /// Bucket iii, in both directions.
    pub region_scope: Option<String>,
    /// Bucket iii, in both directions.
    pub brand_scope: Option<String>,
    /// Bucket ii: half of the atomic `MeterDeclaration` — admitted via this
    /// door only while `published_version = 0` (P-D-41); after first publish
    /// the write belongs to slice 07's correction act.
    pub metering_unit: Option<String>,
    /// Bucket ii: the declaration's other half, on identical terms.
    pub usage_type_ref: Option<String>,
    /// 03's classification fields (P-D-145): `sku_type` is bucket ii, the
    /// other four bucket iii.
    pub sku_type: Option<String>,
    pub sellable: Option<bool>,
    pub plan_tier: Option<String>,
    pub tax_category_ref: Option<String>,
    pub gl_code_ref: Option<String>,
    /// Whether the same act writes **`02`'s content rows** — a category
    /// assignment set or an attribute value — beside this head.
    ///
    /// # It widens [`empty_save`]'s operand rather than bypassing its guard
    ///
    /// That guard's own words are *"no statement in this module bumps
    /// `internal_revision` without writing a content column"*, and its reason
    /// is that *"a bare bump is a write with no content that still invalidates
    /// every `ETag` a client holds."* Both stand. What changed is where this
    /// gear's content lives: when the guard was written the only content was
    /// this table's columns, and `design/02` C2 now makes attribute values
    /// **entity content** in as many words — *"they ride the owning entity's
    /// internal revision, freeze into its published versions"*.
    ///
    /// So a save naming only `categories` is not a bare bump. It is a content
    /// write whose content is in another table, and the revision **must** move:
    /// values riding a revision that did not change would leave two different
    /// content states sharing one `ETag`, which is the concurrency the ride
    /// exists to give them.
    ///
    /// `false` by `Default`, so every caller that predates `02`'s content is
    /// unchanged and the guard keeps refusing a genuinely empty save.
    pub content_moved: bool,
}

impl SkuHeadSave {
    /// See [`ProductHeadSave::touches_structural`].
    const fn touches_structural(&self) -> bool {
        self.sku_code.is_some() || self.product_id.is_some()
    }

    /// Whether the save touches the correctable bucket — which shares
    /// bucket-i's `published_version = 0` admission window at this door
    /// (`design/01` §4.2, P-D-41), so the filter arm below is one predicate
    /// with two member sets.
    const fn touches_correctable(&self) -> bool {
        self.metering_unit.is_some() || self.usage_type_ref.is_some() || self.sku_type.is_some()
    }

    /// See [`ProductHeadSave::is_empty`].
    const fn is_empty(&self) -> bool {
        self.sku_code.is_none()
            && self.product_id.is_none()
            && self.region_scope.is_none()
            && self.brand_scope.is_none()
            && self.metering_unit.is_none()
            && self.usage_type_ref.is_none()
            && self.sku_type.is_none()
            && self.sellable.is_none()
            && self.plan_tier.is_none()
            && self.tax_category_ref.is_none()
            && self.gl_code_ref.is_none()
            && !self.content_moved
    }
}

/// The failure a save carrying no column at all is answered with.
///
/// [`RepoError::Db`] — this gear's internal channel — and **not** a
/// [`DomainError`]: the door refuses an empty payload `VALIDATION` naming the
/// body, so a caller cannot reach this message, and a request that did reach
/// it would be reporting the gear's own defect rather than the caller's. The
/// backstop exists so that "no statement in this module bumps
/// `internal_revision` without writing a content column" holds against
/// callers this module has not met yet — a bare bump is a write with no
/// content that still invalidates every `ETag` a client holds.
fn empty_save() -> RepoError {
    RepoError::Db(
        "a save must name at least one column: a bare internal_revision bump is not a save"
            .to_owned(),
    )
}

/// Save a Product head: **one** `UPDATE` carrying the routed columns, the
/// revision bump and `updated_at` (`inst-fd-transition-bump`,
/// `cpt-cf-bss-products-dod-save-door`).
///
/// # Exactly one statement, and no version row
///
/// The head-row guard bumps nothing itself — it *refuses* any `UPDATE` whose
/// `internal_revision` is not `OLD + 1`, on every admitted `UPDATE` without
/// exception — so a save split across two statements would move the revision
/// twice for one act and the `ETag` a client holds would skip a value the
/// door never returned. That is [`publish_product_head`]'s argument, and it
/// reaches a save for the same reason.
///
/// A save writes **no** `products_entity_version` row and moves
/// `published_version` not at all: the head is the authoring surface in every
/// non-terminal state (`inst-fd-transition-guard`), and the version row is
/// the publish act's. There is therefore no freeze to precede this statement
/// and no ordering constraint of the kind [`publish_product_head`] carries.
///
/// # The legality is in the `WHERE` clause
///
/// Two conditions ride the filter beside the pinned revision, and the
/// database is the copy that decides:
///
/// - **Non-terminal**, always ([`TERMINAL_HEAD_STATES`]).
/// - **`published_version = 0`**, and only where the save carries a
///   **bucket-i** column ([`ProductHeadSave::touches_structural`]). §4.1
///   admits identity writes before first publish and refuses them after
///   (`inst-fd-bucket-i-refusal`); §4.1 admits a bucket-iii write on any
///   non-terminal head, published or not — a published Product **can** be
///   renamed. Pinning `published_version = 0` unconditionally would refuse
///   exactly that rename, and pinning it never would leave the identity rule
///   to the door's own read, which a concurrent publish can invalidate
///   between the read and this write.
///
/// The physical trigger states both rules a third time
/// (`trg_products_product_bucket_i`, `trg_products_product_bucket_iii`), and
/// the difference matters: the trigger *raises*, so a save that reached it
/// would be an operator-facing 500. This filter answers
/// [`HeadWrite::Unmatched`] instead, which the door turns into the governed
/// refusal that names the caller's own field.
///
/// # Errors
/// [`RepoError::Db`] where `save` names no column at all — the gear's own
/// internal channel and not a [`DomainError`], for the reason the backstop
/// [`empty_save`] states — and on a scope refusal that raised no driver
/// error. [`RepoError::Driver`] on a storage failure, which for this
/// statement includes **either unique index's** collision
/// (`uq_products_product_name` on a renamed row, `uq_products_product_code`
/// on a re-coded one); the door reads which from the driver's own text, as
/// the create door does. A stale revision, a terminal head, or a bucket-i
/// write after first publish is **not** an error; it is
/// [`HeadWrite::Unmatched`].
pub async fn save_product_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_id: Uuid,
    expected_internal_revision: i64,
    save: &ProductHeadSave,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    if save.is_empty() {
        return Err(empty_save());
    }

    let mut statement = product::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            product::Column::InternalRevision,
            Expr::col(product::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(product::Column::UpdatedAt, Expr::value(now));

    if let Some(brand_id) = save.brand_id {
        statement = statement.col_expr(product::Column::BrandId, Expr::value(brand_id));
    }
    if let Some(code) = save.product_code.as_ref() {
        let value = match code {
            NullableText::Set(code) => Expr::value(code.clone()),
            NullableText::Clear => Expr::value(Option::<String>::None),
        };
        statement = statement.col_expr(product::Column::ProductCode, value);
    }
    if let Some(name) = save.name.as_ref() {
        statement = statement
            .col_expr(product::Column::Name, Expr::value(name.value.clone()))
            .col_expr(
                product::Column::NameNormalized,
                Expr::value(name.normalized.clone()),
            );
    }
    if let Some(region_scope) = save.region_scope.as_ref() {
        statement = statement.col_expr(
            product::Column::RegionScope,
            Expr::value(region_scope.clone()),
        );
    }
    if let Some(brand_scope) = save.brand_scope.as_ref() {
        statement = statement.col_expr(
            product::Column::BrandScope,
            Expr::value(brand_scope.clone()),
        );
    }

    let mut filter = Condition::all()
        .add(product::Column::TenantId.eq(tenant_id))
        .add(product::Column::ProductId.eq(product_id))
        .add(product::Column::InternalRevision.eq(expected_internal_revision))
        .add(product::Column::LifecycleState.is_not_in(TERMINAL_HEAD_STATES));
    if save.touches_structural() {
        filter = filter.add(product::Column::PublishedVersion.eq(0_i64));
    }

    let result = statement
        .filter(filter)
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("save product {product_id}"), e))?;

    // Zero rows is an answer, not a no-op: the head no longer carries the
    // pinned revision, it has gone terminal, it has been published under a
    // bucket-i save, or it is another tenant's. Reporting it as success would
    // answer `200` and an `ETag` for a revision the row never took.
    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// Save a SKU head, on [`save_product_head`]'s terms exactly — one statement,
/// no version row, the same two conditions in the filter, and the same
/// reading of `Unmatched`.
///
/// The one difference is the column set ([`SkuHeadSave`]): no `name`, and
/// `product_id` as the bucket-i parent link rather than as row identity.
/// A re-parenting save is therefore admitted before first publish and refused
/// after it by the same `published_version = 0` clause the code rides, which
/// is what the owner's call of 2026-08-27 asks for — *"re-parenting changes
/// whose SKU it is, not how it is described"*.
///
/// # Errors
/// As [`save_product_head`], with one unique index rather than two:
/// `uq_products_sku_code` is the only one `products_sku` carries.
pub async fn save_sku_head(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_id: Uuid,
    expected_internal_revision: i64,
    save: &SkuHeadSave,
    now: DateTime<Utc>,
) -> Result<HeadWrite, RepoError> {
    if save.is_empty() {
        return Err(empty_save());
    }

    let mut statement = sku::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            sku::Column::InternalRevision,
            Expr::col(sku::Column::InternalRevision).add(Expr::val(1_i64)),
        )
        .col_expr(sku::Column::UpdatedAt, Expr::value(now));

    if let Some(sku_code) = save.sku_code.as_ref() {
        statement = statement.col_expr(sku::Column::SkuCode, Expr::value(sku_code.clone()));
    }
    if let Some(product_id) = save.product_id {
        statement = statement.col_expr(sku::Column::ProductId, Expr::value(product_id));
    }
    if let Some(region_scope) = save.region_scope.as_ref() {
        statement = statement.col_expr(sku::Column::RegionScope, Expr::value(region_scope.clone()));
    }
    if let Some(brand_scope) = save.brand_scope.as_ref() {
        statement = statement.col_expr(sku::Column::BrandScope, Expr::value(brand_scope.clone()));
    }
    if let Some(metering_unit) = save.metering_unit.as_ref() {
        statement = statement.col_expr(
            sku::Column::MeteringUnit,
            Expr::value(metering_unit.clone()),
        );
    }
    if let Some(usage_type_ref) = save.usage_type_ref.as_ref() {
        statement = statement.col_expr(
            sku::Column::UsageTypeRef,
            Expr::value(usage_type_ref.clone()),
        );
    }
    if let Some(sku_type) = save.sku_type.as_ref() {
        statement = statement.col_expr(sku::Column::SkuType, Expr::value(sku_type.clone()));
    }
    if let Some(sellable) = save.sellable {
        statement = statement.col_expr(sku::Column::Sellable, Expr::value(sellable));
    }
    if let Some(plan_tier) = save.plan_tier.as_ref() {
        statement = statement.col_expr(sku::Column::PlanTier, Expr::value(plan_tier.clone()));
    }
    if let Some(tax_category_ref) = save.tax_category_ref.as_ref() {
        statement = statement.col_expr(
            sku::Column::TaxCategoryRef,
            Expr::value(tax_category_ref.clone()),
        );
    }
    if let Some(gl_code_ref) = save.gl_code_ref.as_ref() {
        statement = statement.col_expr(sku::Column::GlCodeRef, Expr::value(gl_code_ref.clone()));
    }

    let mut filter = Condition::all()
        .add(sku::Column::TenantId.eq(tenant_id))
        .add(sku::Column::SkuId.eq(sku_id))
        .add(sku::Column::InternalRevision.eq(expected_internal_revision))
        .add(sku::Column::LifecycleState.is_not_in(TERMINAL_HEAD_STATES));
    if save.touches_structural() || save.touches_correctable() {
        filter = filter.add(sku::Column::PublishedVersion.eq(0_i64));
    }

    let result = statement
        .filter(filter)
        .exec(runner)
        .await
        .map_err(|e| driver_failure(format!("save sku {sku_id}"), e))?;

    if result.rows_affected == 0 {
        return Ok(HeadWrite::Unmatched);
    }
    Ok(HeadWrite::Applied)
}

/// A SKU by its `skuCode` — C5's SKU identity, the promotion resolver's
/// second lookup after the exported id (`dod-promotion-resolver`, P-D-127
/// row 4).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_sku_by_code(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    sku_code: &str,
) -> Result<Option<SkuRecord>, RepoError> {
    let row = sku::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(sku::Column::TenantId.eq(tenant_id))
                .add(sku::Column::SkuCode.eq(sku_code)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read sku by code {sku_code}"), e))?;
    row.map(into_sku_record).transpose()
}

/// A Product by its `productCode` — C5's first Product identity after the
/// exported id.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_product_by_code(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    product_code: &str,
) -> Result<Option<ProductRecord>, RepoError> {
    let row = product::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::ProductCode.eq(product_code)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read product by code {product_code}"), e))?;
    row.map(into_product_record).transpose()
}

/// A Product by `(brandId, normalized name)` — C5's fallback identity for a
/// Product carrying no code.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn find_product_by_brand_and_name(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    brand_id: Uuid,
    name_normalized: &str,
) -> Result<Option<ProductRecord>, RepoError> {
    let row = product::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(product::Column::TenantId.eq(tenant_id))
                .add(product::Column::BrandId.eq(brand_id))
                .add(product::Column::NameNormalized.eq(name_normalized)),
        )
        .one(runner)
        .await
        .map_err(|e| driver_failure(format!("read product by name under {brand_id}"), e))?;
    row.map(into_product_record).transpose()
}

/// One frozen version's content by number — the export's entity half
/// (`inst-bk-export`): the bytes the manifest entry names, never the head.
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn entity_version_at(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: VersionedEntityKind,
    entity_id: Uuid,
    published_version: i64,
) -> Result<Option<String>, RepoError> {
    let row = entity_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(entity_version::Column::TenantId.eq(tenant_id))
                .add(entity_version::Column::EntityKind.eq(entity_kind.as_str()))
                .add(entity_version::Column::EntityId.eq(entity_id))
                .add(entity_version::Column::PublishedVersion.eq(published_version)),
        )
        .one(runner)
        .await
        .map_err(|e| {
            driver_failure(
                format!("read frozen version {published_version} of {entity_id}"),
                e,
            )
        })?;
    Ok(row.map(|row| row.content))
}

/// One frozen version as the history timeline renders it (`inst-rh-timeline`).
#[derive(Debug, Clone)]
pub struct FrozenVersionRow {
    pub published_version: i64,
    pub content: String,
    pub approval_ref: Option<Uuid>,
    pub actor_ref: Uuid,
    pub published_at: DateTime<Utc>,
}

/// Every frozen version of one entity, oldest first — the request-time read
/// over `products_entity_version` the timeline is (P-D-150).
///
/// # Errors
///
/// [`RepoError`] on a storage or scope failure.
pub async fn entity_versions_of(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    entity_kind: VersionedEntityKind,
    entity_id: Uuid,
) -> Result<Vec<FrozenVersionRow>, RepoError> {
    let rows = entity_version::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(entity_version::Column::TenantId.eq(tenant_id))
                .add(entity_version::Column::EntityKind.eq(entity_kind.as_str()))
                .add(entity_version::Column::EntityId.eq(entity_id)),
        )
        .order_by(
            entity_version::Column::PublishedVersion,
            sea_orm::Order::Asc,
        )
        .all(runner)
        .await
        .map_err(|e| driver_failure(format!("read frozen versions of {entity_id}"), e))?;
    Ok(rows
        .into_iter()
        .map(|row| FrozenVersionRow {
            published_version: row.published_version,
            content: row.content,
            approval_ref: row.approval_ref,
            actor_ref: row.actor_ref,
            published_at: row.published_at,
        })
        .collect())
}

#[cfg(test)]
#[path = "repo_tests.rs"]
mod repo_tests;
