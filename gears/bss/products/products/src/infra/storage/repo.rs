//! Repositories for `products_product`, `products_sku`, `products_identity_ref`
//! and `products_audit_log` (`design/01-foundation.md` §4.1, §4.2, §4.4;
//! `design/10-retention-erasure.md` `inst-im-map`).
//!
//! Phase 1's `products_product`/`products_sku` functions close no Definition
//! of Done; they are the enabler every door above them needs, so they carry
//! exactly two operations per table — insert one row, read one back by id —
//! and nothing a later phase would need to undo. In particular there is
//! **no** `ON CONFLICT` handling here: `uq_products_sku_code`'s
//! reservation-by-insert semantics belong to `dod-code-reservation`, and
//! giving a duplicate-key insert typed conflict handling now would be scope
//! taken from that phase. A duplicate insert surfacing as [`RepoError::Db`]
//! is the correct behaviour for this phase — the create door that will call
//! this repository does not exist yet to act on a finer answer.
//!
//! Phase 2 Slice C adds [`resolve_actor_ref`], which closes the code half of
//! `dod-actor-ref`. Slice D, the phase's last, adds [`AuditEntry`] and the
//! three writing disciplines that consume the `actor_ref` it resolves —
//! [`write_refusal_audit`], [`write_eventless_act_audit`] and
//! [`write_elevated_read_audit`] — closing `dod-audit-trail`
//! (`design/01-foundation.md` §4.4).
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
//! @cpt-cf-bss-products-dod-audit-trail

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{
    AccessScope, DBRunner, SecureEntityExt, SecureInsertExt, SecureUpdateExt,
};
use uuid::Uuid;

use bss_products_sdk::models::LifecycleState;

use crate::domain::error::DomainError;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::{audit_log, identity_ref, product, sku};

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
/// lies outside `scope`. Deliberately the same answer either way: a
/// repository that answered "forbidden" for a row belonging to another
/// tenant would confirm that the row exists, which is the existence leak the
/// SQL-level scoping is there to close — the catalog is commercially
/// sensitive, so absence is what a foreign scope sees.
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
/// lies outside `scope`, for [`find_product`]'s reason.
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
/// [`RepoError::Db`] on a scope failure or a storage failure, including the
/// `uq_products_identity_ref_active` violation a race between two
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
        .map_err(|e| RepoError::Db(format!("resolve actor ref, tenant {tenant_id}: {e}")))?;

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
                    .add(identity_ref::Column::TombstonedAt.is_null()),
            )
            .exec(runner)
            .await
            .map_err(|e| {
                RepoError::Db(format!(
                    "advance last_seen_at for actor {}: {e}",
                    row.actor_ref
                ))
            })?;

        // Zero rows means the race above was lost. That is not an error: it
        // is exactly the state the mint path is for — a principal with no
        // live ref — so fall through to it and mint a fresh one, which is
        // what the design requires of a principal acting after its erasure.
        if advanced.rows_affected > 0 {
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
        .map_err(|e| RepoError::Db(format!("actor ref {actor_ref} scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("mint actor ref, tenant {tenant_id}: {e}")))?;

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
    pub correlation_id: Option<Uuid>,
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
        seal_state: Set("unsealed".to_owned()),
        chain_id: Set(None),
        seq: Set(None),
        prev_hash: Set(None),
        row_hash: Set(None),
    };

    audit_log::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .map_err(|e| RepoError::Db(format!("audit row {audit_id} scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| RepoError::Db(format!("insert audit row {audit_id}: {e}")))?;

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
/// [`RepoError::Db`] on a scope failure or a storage failure.
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

#[cfg(test)]
#[path = "repo_tests.rs"]
mod repo_tests;
