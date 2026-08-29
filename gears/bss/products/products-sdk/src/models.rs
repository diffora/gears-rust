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
}

/// The lifecycle state of a catalog entity.
///
/// The edge list is the Foundation's and is enforced both in the application
/// and by the head-row trigger whitelist. `Retired` and `Discarded` are
/// terminal at the physical layer: no admitted update writes a
/// `lifecycle_state` out of either.
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

/// A SKU as a consumer reads it.
///
/// The capability columns a SKU carries — typing, `sellable`, `PlanTier`, the
/// accounting codes, the metering unit — are not here. They belong to the
/// features that own their rules, and a consumer reads them from those.
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
}
