//! Transport-agnostic read models.
//!
//! These are the shapes a consumer sees. They deliberately carry the
//! Foundation's identity, lifecycle and version columns and nothing a capability
//! slice owns: a consumer that needs a SKU's `PlanTier` or its metering unit is
//! reading a capability's surface, not this one.

use uuid::Uuid;

/// Which of the two catalog entities a row is.
///
/// The two share one lifecycle machine and one head-row guard, so the code that
/// operates on either is generic over this rather than duplicated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntityKind {
    /// A `Product` — the parent a SKU hangs from.
    Product,
    /// A `SKU` — the sellable unit a Product contains.
    Sku,
}

impl EntityKind {
    /// The stable wire spelling, which is also the value the `entity_kind`
    /// column and the event body core carry.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Sku => "sku",
        }
    }

    /// Parse a stored or wire value.
    ///
    /// Returns `None` for anything outside the two-member roster rather than
    /// defaulting — a row whose `entity_kind` is neither is not a Product.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "product" => Some(Self::Product),
            "sku" => Some(Self::Sku),
            _ => None,
        }
    }
}

/// The lifecycle state of a catalog entity.
///
/// The edge list is the Foundation's and is enforced both in the application
/// and by the head-row trigger whitelist. `Retired` and `Discarded` are
/// terminal at the physical layer: no admitted update writes a
/// `lifecycle_state` out of either.
///
/// # The wire subset, which is smaller than this enum
///
/// All five states exist here; **the consumer-facing wire vocabulary is two**:
/// `published` and `deprecated` (P-D-66, pinned in `schema-pin.toml`). Browse
/// serves only those; `draft` is never served and `retired` is history-only,
/// reachable through the versions surface and the by-id read's explicit state
/// opt-in (P-D-70). A consumer that treats this field as an open string is
/// choosing display tolerance over the pin's guard, which is its own risk to
/// carry.
///
/// @cpt-dod:cpt-cf-bss-products-dod-status-vocabulary:p1
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LifecycleState {
    /// Authored, never published. The only state a discard is admitted from.
    Draft,
    /// Published at least once; `published_version` is above zero.
    Published,
    /// Published and marked for eventual retirement; still admits children.
    Deprecated,
    /// Terminal. Holds its name and its codes.
    Retired,
    /// Terminal. Releases its name and its codes.
    Discarded,
}

impl LifecycleState {
    /// The stable wire spelling, which is also the value the `lifecycle_state`
    /// column carries and the `CHECK` constraint admits.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Deprecated => "deprecated",
            Self::Retired => "retired",
            Self::Discarded => "discarded",
        }
    }

    /// Whether no admitted edge leaves this state.
    ///
    /// A head write on a terminal row is refused `ENTITY_TERMINAL` by the
    /// application and by the trigger whitelist independently; this is the
    /// application's half of that answer.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Retired | Self::Discarded)
    }

    /// Parse a stored or wire value.
    ///
    /// Returns `None` for anything outside the roster rather than defaulting,
    /// which is the fail-closed posture the gear takes everywhere: an
    /// unrecognised state is a corrupt row, not a draft.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft" => Some(Self::Draft),
            "published" => Some(Self::Published),
            "deprecated" => Some(Self::Deprecated),
            "retired" => Some(Self::Retired),
            "discarded" => Some(Self::Discarded),
            _ => None,
        }
    }
}

/// A Product as a consumer reads it.
/// The field names repeat the type name — `product_id`, `sku_code` — and that
/// is deliberate. Each one is the name the database column carries **and** the
/// name the wire contract uses, so shortening them here would make this crate
/// the only place in the gear with a third spelling of the same field.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Product {
    /// Server-minted, immutable, never operator-supplied.
    pub product_id: Uuid,
    /// The tenant every row is scoped to.
    pub tenant_id: Uuid,
    /// The brand the Product belongs to. An operand of the uniqueness index,
    /// which is why it is a required payload field rather than derived from
    /// the caller's claims.
    pub brand_id: Uuid,
    /// The operator-facing name, as authored.
    pub name: String,
    /// The optional external mapping code. When unset, product-level external
    /// mapping is by `product_id` alone.
    pub product_code: Option<String>,
    /// Where the entity sits in the lifecycle machine.
    pub lifecycle_state: LifecycleState,
    /// Moves on every admitted write; backs optimistic concurrency.
    pub internal_revision: i64,
    /// Moves only on publish; the only counter a consumer may pin to.
    pub published_version: i64,
}

/// A SKU's commercial type — `design/03`'s closed set (`inst-cl-type`).
///
/// Three members, closed: a fourth type is a design change, not a value. A
/// consumer that treats the wire field as an open string is choosing display
/// tolerance over the pin's guard, as with [`LifecycleState`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkuType {
    /// A sellable good with both accounting codes required at publish.
    Product,
    /// A sellable service, the same code profile as a product.
    Service,
    /// Commercially incomplete by design: no codes required, and its publish
    /// carries plan-price's composition state (`composition_pending`).
    Bundle,
}

impl SkuType {
    /// The stored and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Service => "service",
            Self::Bundle => "bundle",
        }
    }

    /// Parse the stored spelling; anything else is not a type.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "product" => Some(Self::Product),
            "service" => Some(Self::Service),
            "bundle" => Some(Self::Bundle),
            _ => None,
        }
    }
}

/// A SKU as a consumer reads it.
///
/// The classification columns ride here **from day one**
/// (`design/03` `dod-sdk-read-shape`, **P-D-146**): the type, `sellable`,
/// the tier, the meter pair and the two accounting codes. `03` owns their
/// rules; this shape carries their values so a consumer never has to reach
/// past the SDK for them. Pricing's `CatalogSku` still lacks three of these
/// (`03` §7 row 4, `12-consumer-contracts`'), and carrying them here keeps
/// that fix additive on the consumer side.
/// The field names repeat the type name — `product_id`, `sku_code` — and that
/// is deliberate. Each one is the name the database column carries **and** the
/// name the wire contract uses, so shortening them here would make this crate
/// the only place in the gear with a third spelling of the same field.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sku {
    /// Server-minted, immutable, never operator-supplied.
    pub sku_id: Uuid,
    /// The tenant every row is scoped to.
    pub tenant_id: Uuid,
    /// The parent Product. Re-parenting is refused after first publish.
    pub product_id: Uuid,
    /// Tenant-unique, reserved atomically at create, immutable after first
    /// publish.
    pub sku_code: String,
    /// Where the entity sits in the lifecycle machine.
    pub lifecycle_state: LifecycleState,
    /// Moves on every admitted write; backs optimistic concurrency.
    pub internal_revision: i64,
    /// Moves only on publish; the only counter a consumer may pin to.
    pub published_version: i64,
    /// `design/03`'s closed commercial type.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-sdk-read-shape:p1
    pub sku_type: SkuType,
    /// Whether the SKU is offered for sale; defaults `true` at create
    /// (`dod-sellable`). A flip is a bucket-iii save the next publish freezes.
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-sellable:p1
    pub sellable: bool,
    /// Whether a `bundle` still awaits its composition signal
    /// (`PRD` AC #36's adoption block operand, P-D-35's column). Always
    /// `false` on a non-bundle. The read shape's ninth `CatalogSku`-superset
    /// member; the tenth, `name`, is the parent Product's — a SKU carries no
    /// display name of its own, `sku_code` being its operator-facing one
    /// (P-D-151).
    ///
    /// @cpt-dod:cpt-cf-bss-products-dod-catalogsku-shape:p1
    pub composition_pending: bool,
    /// The `PlanTier` member the SKU is assigned to, by stable code.
    pub plan_tier: Option<String>,
    /// The metering unit — half of the meter pair, both or neither present.
    pub metering_unit: Option<String>,
    /// The usage-collector type the meter binds to — the other half.
    pub usage_type_ref: Option<String>,
    /// The tax category code; required at publish for `product`/`service`.
    pub tax_category_ref: Option<String>,
    /// The GL code; required at publish for `product`/`service`.
    pub gl_code_ref: Option<String>,
}
