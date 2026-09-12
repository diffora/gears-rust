//! The authoring/publish client (`PRD` §9.1 `interface-authoring-publish`,
//! `inst-sdk-surface`'s first row): create, save and publish a Product or a
//! SKU, with **the idempotency key, the `If-Match` revision and the
//! intent semantics as part of the contract** — breaking = major, as §9.1's
//! change policy says.
//!
//! # Why the contract is spelled in preconditions
//!
//! A registry write is never bare. Every mutating door on this surface
//! speaks two preconditions, and the SDK carries both **typed** rather than
//! as headers a caller might forget:
//!
//! - [`Precondition::if_match`] — the `internal_revision` the caller last
//!   read, which the door compares against the head row as its first
//!   statement (`design/01` `inst-fd-etag`). A save or publish without it is
//!   refused `VALIDATION`; a stale one is `STALE_REVISION`, and the right
//!   answer to that is to re-read and retry, never to loop. A create carries
//!   none — there is no row yet.
//! - [`Precondition::idempotency_key`] — the caller's key
//!   (`inst-fd-idempotency`): the same key with the same body **replays** the
//!   stored success (`HeadReceipt::replayed`), the same key with a different
//!   body is `IDEMPOTENCY_CONFLICT`, a key whose first attempt is still running
//!   is `IDEMPOTENCY_KEY_IN_FLIGHT`. A refusal is never replayed (P-D-38): a
//!   retry on the same key after a refusal **runs** and gets a fresh verdict.
//!
//! The **intent** half of the §9.1 sentence is the resolution intent a
//! consumer declares to `design/06`'s resolver (`browse` vs `posted`); it
//! belongs to the read side and is spelled on the catalog-version surface,
//! not repeated here.
//!
//! # One binding, two transports
//!
//! The default deployment resolves this trait from `ClientHub` in-process
//! (P-D-15) and the binding runs **the door itself** — the same phases, the
//! same gate, the same audit row — so an SDK write and a REST write with the
//! same key are one key. The REST doors are the out-of-process binding.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-sdk-surface:p1

use std::collections::BTreeMap;

use async_trait::async_trait;
use toolkit_canonical_errors::CanonicalError;
use toolkit_security::SecurityContext;
use uuid::Uuid;

use crate::models::LifecycleState;

/// The two preconditions every write carries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Precondition {
    /// The `internal_revision` the caller last read. Required on save and
    /// publish; ignored on create.
    pub if_match: Option<i64>,
    /// The caller's idempotency key. Optional (P-D-34's skip): a write
    /// without one is never replayed and never conflicts.
    pub idempotency_key: Option<String>,
}

/// A field value in a save — the wire's JSON scalars, without the wire.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldValue {
    /// A string field.
    Text(String),
    /// A boolean field (`sellable`).
    Bool(bool),
    /// An integer field.
    Integer(i64),
    /// An explicit clear: the field set to `null`.
    Null,
}

/// The fields a save writes, keyed by the wire field name
/// (`name`, `productCode`, `skuType`, `sellable`, `planTier`,
/// `meteringUnit`, `usageTypeRef`, `taxCategoryRef`, `glCodeRef`, …). A key
/// the head door does not admit is refused `VALIDATION`; a bucket-i or
/// bucket-ii field after first publish is `ILLEGAL_FIELD_MUTATION`.
pub type SaveFields = BTreeMap<String, FieldValue>;

/// A new Product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewProduct {
    /// A caller-supplied id, or `None` for a server-minted one.
    pub id: Option<Uuid>,
    /// The owning brand.
    pub brand_id: Uuid,
    /// The human name, tenant-unique under `normalized(name)`.
    pub name: String,
    /// The optional operator-facing code, reserved atomically at create.
    pub product_code: Option<String>,
    /// Region scope; `None` is unrestricted.
    pub region_scope: Option<String>,
    /// Brand scope; `None` is unrestricted.
    pub brand_scope: Option<String>,
}

/// A new SKU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSku {
    /// A caller-supplied id, or `None` for a server-minted one.
    pub id: Option<Uuid>,
    /// The parent Product.
    pub product_id: Uuid,
    /// The operator-facing code, reserved atomically at create.
    pub sku_code: String,
    /// Region scope; `None` is unrestricted.
    pub region_scope: Option<String>,
    /// Brand scope; `None` is unrestricted.
    pub brand_scope: Option<String>,
    /// `product` | `service` | `bundle`; may be left for a later save.
    pub sku_type: Option<String>,
    /// Whether the SKU is offered on its own; the door defaults it `true`.
    pub sellable: Option<bool>,
    /// The `PlanTier` member, by stable code.
    pub plan_tier: Option<String>,
    /// The tax category code.
    pub tax_category_ref: Option<String>,
    /// The GL code.
    pub gl_code_ref: Option<String>,
}

/// What a write answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadReceipt {
    /// The entity acted on (minted on create).
    pub entity_id: Uuid,
    /// The head's `internal_revision` after the act — the next `If-Match`.
    pub internal_revision: i64,
    /// The head's lifecycle state after the act.
    pub lifecycle_state: LifecycleState,
    /// The head's published version after the act (`0` before first
    /// publish).
    pub published_version: i64,
    /// `true` when the answer is the stored success of an earlier attempt
    /// on the same idempotency key; nothing was written.
    pub replayed: bool,
}

/// The authoring/publish contract, resolved from `ClientHub`.
///
/// The tenant is the caller's own (`ctx`), never a parameter: a registry
/// write is always in the writer's tenant. Every method returns
/// [`CanonicalError`]; the codes a caller may see are the
/// [`crate::errors::ErrorCode`] vocabulary's.
#[async_trait]
pub trait Authoring: Send + Sync {
    /// Create a Product as a `draft`.
    ///
    /// # Errors
    /// `DUPLICATE_NAME`, `DUPLICATE_CODE`, `VALIDATION`, the idempotency
    /// refusals, or the authorization gate's canonical projection.
    async fn create_product(
        &self,
        ctx: &SecurityContext,
        product: NewProduct,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError>;

    /// Save fields on a Product head.
    ///
    /// # Errors
    /// `STALE_REVISION`, `ILLEGAL_FIELD_MUTATION`, `ENTITY_TERMINAL`,
    /// `CONTENT_PII_BLOCKED`, `VALIDATION`, the idempotency refusals, or the
    /// authorization gate's canonical projection.
    async fn save_product(
        &self,
        ctx: &SecurityContext,
        product_id: Uuid,
        fields: SaveFields,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError>;

    /// Publish a Product head under the governance gate.
    ///
    /// # Errors
    /// `APPROVAL_REQUIRED` when the gate holds a ceremony open,
    /// `INCOMPLETE_ENTITY`, `PRIMARY_CATEGORY_REQUIRED`, `STALE_REVISION`,
    /// `ENTITY_TERMINAL`, or the authorization gate's canonical projection.
    async fn publish_product(
        &self,
        ctx: &SecurityContext,
        product_id: Uuid,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError>;

    /// Create a SKU as a `draft` under its parent.
    ///
    /// # Errors
    /// `DUPLICATE_CODE`, `PARENT_TERMINAL`, `SCOPE_NOT_CONTAINED`,
    /// `SKU_TYPE_UNKNOWN`, `VALIDATION`, the idempotency refusals, or the
    /// authorization gate's canonical projection.
    async fn create_sku(
        &self,
        ctx: &SecurityContext,
        sku: NewSku,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError>;

    /// Save fields on a SKU head.
    ///
    /// # Errors
    /// As [`Authoring::save_product`], plus the classification refusals
    /// (`UNRECOGNIZED_UNIT`, `PLAN_TIER_UNKNOWN`, `METER_DECLARATION_INCOMPLETE`).
    async fn save_sku(
        &self,
        ctx: &SecurityContext,
        sku_id: Uuid,
        fields: SaveFields,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError>;

    /// Publish a SKU head under the governance gate.
    ///
    /// # Errors
    /// As [`Authoring::publish_product`], plus `PARENT_NOT_PUBLISHED`,
    /// `BUNDLE_OVERRIDE_REQUIRED` and the classification refusals.
    async fn publish_sku(
        &self,
        ctx: &SecurityContext,
        sku_id: Uuid,
        precondition: Precondition,
    ) -> Result<HeadReceipt, CanonicalError>;
}
