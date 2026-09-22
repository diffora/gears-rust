//! The recognized sets' rules — the kind roster, the member state machine,
//!
//! @cpt-dod:cpt-cf-bss-products-dod-seeded-members:p1
//! set membership, and what a meter declaration may name
//! (`design/03` §3.1, `features/sku-classification.md`
//! `state-recognized-set`, `dod-recognized-set-mechanics`,
//! `dod-unit-recognition`).
//!
//! # The set is a judgement, not a state
//!
//! `state` stores three values, and *"in the set"* is a rule over them: the
//! `active` and `deprecated` rows are the set, a `removed` row is a tombstone
//! outside it. Two different questions read that rule differently — an
//! **existing** carrier keeps resolving against a `deprecated` member, while
//! a **new** declaration is refused — so this module answers the questions
//! ([`declaration_verdict`]) rather than exporting a boolean the callers
//! would each re-interpret.
//!
//! # The state machine is four edges and a refusal
//!
//! `active → deprecated` (blocks new declarations, existing carriers keep
//! resolving), `deprecated → removed` (only once unreferenced, and never for
//! a seeded member), and the two re-listing edges `deprecated → active` and
//! `removed → active` — safe precisely because the identity never changed.
//! **`active → removed` is refused**: the whole safety property of
//! de-listing is that deprecation blocks new declarations first
//! (`inst-rm-append-only`). There is no DELETE and no `member_code` UPDATE
//! in any state — the shipped guard refuses `member_code` by name (with
//! `tenant_id`, `set_kind`, `seeded_by` and `created_at`), which makes
//! semantic immutability a schema property and `dod-unit-immutable`'s "the
//! absence of the door is the enforcement" literally true. §4 words that
//! guard as a whitelist admitting two columns; what ships is the complement
//! enumeration, so `updated_at` is writable and a later column is admitted
//! by default — `design/03` §6's open question, not a settled reading.

use super::error::DomainError;

/// The two recognized sets (`design/03` §4's roster, pinned by no `CHECK` per
/// P-D-92 — the DDL pins non-emptiness only, so this enum is the roster's
/// enforcement site).
///
/// **It was four until P-D-169.** The two accounting sets — tax categories and
/// GL codes — left with the columns that carried them: `PRD` §2.1 says billing
/// descriptors are *"owned elsewhere and **MUST NOT** be re-specified here"*,
/// and the two overrides of that sentence were both withdrawn (the
/// tax-ownership amendment of 2026-09-10, and the measurement that pricing's
/// `pricing_gl_code_taxonomy` is the tenant's GL vocabulary).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetKind {
    /// Metering units — `dod-unit-*`'s set.
    MeteringUnit,
    /// Plan tiers — the set with its own grant, event and refusal code.
    PlanTier,
}

impl SetKind {
    /// The stored and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MeteringUnit => "metering_unit",
            Self::PlanTier => "plan_tier",
        }
    }

    /// Parse a path segment or stored value, `None` outside the roster.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "metering_unit" => Some(Self::MeteringUnit),
            "plan_tier" => Some(Self::PlanTier),
            _ => None,
        }
    }

    /// The refusal a blocked removal raises — the one place the generic
    /// machinery answers per kind, because the design gives each family its
    /// own code (`UNIT_DELIST_BLOCKED`, `PLAN_TIER_RETIRE_BLOCKED`).
    #[must_use]
    pub fn delist_blocked(self, detail: String) -> DomainError {
        match self {
            Self::MeteringUnit => DomainError::UnitDelistBlocked(detail),
            Self::PlanTier => DomainError::PlanTierRetireBlocked(detail),
        }
    }
}

impl SetKind {
    /// The `products_sku` column whose value names a member of this set —
    /// the holder population a removal counts, **uniform across both kinds**
    /// (`dod-recognized-set-mechanics`; P-D-146).
    #[must_use]
    pub const fn carrier_column(self) -> &'static str {
        match self {
            Self::MeteringUnit => "metering_unit",
            Self::PlanTier => "plan_tier",
        }
    }
}

/// One member's stored state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberState {
    /// In the set; admits new declarations.
    Active,
    /// In the set; refuses new declarations, existing carriers keep
    /// resolving.
    Deprecated,
    /// The tombstone outside the set. The row survives so no published row
    /// ever names a member that has ceased to exist.
    Removed,
}

impl MemberState {
    /// The stored and wire spelling — the migration's CHECK roster.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Deprecated => "deprecated",
            Self::Removed => "removed",
        }
    }

    /// Parse a stored value, `None` outside the roster.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "deprecated" => Some(Self::Deprecated),
            "removed" => Some(Self::Removed),
            _ => None,
        }
    }
}

/// The state-machine edges, exactly `state-recognized-set`'s four.
///
/// # Errors
///
/// [`DomainError::IllegalTransition`] for every pair outside them —
/// `active → removed` deliberately included.
pub fn member_edge(from: MemberState, to: MemberState) -> Result<(), DomainError> {
    let admitted = matches!(
        (from, to),
        (MemberState::Active, MemberState::Deprecated)
            | (MemberState::Deprecated, MemberState::Removed)
            | (
                MemberState::Deprecated | MemberState::Removed,
                MemberState::Active
            )
    );
    if admitted {
        Ok(())
    } else {
        Err(DomainError::IllegalTransition {
            from: from.as_str().to_owned(),
            to: to.as_str().to_owned(),
        })
    }
}

/// The three acts the set's write doors perform, as a `GovernedLiveOp`
/// submission may declare them (**P-D-171**).
///
/// # Why a declared token and not a route census
///
/// A `governed_live_op` approval names its subject (`recognized_set/{set_kind}/
/// {member_code}`) and **not** the act, so the submit door cannot tell a
/// relabel from a removal by looking at the subject alone — which is exactly
/// why the `min(N, 1)` exception `design/05` §4 registers for a
/// `display_label` change had no operand to read and went unenforced from
/// 2026-09-03 (P-D-121 row 17) to P-D-170's *Owed* item. The op payload is
/// the operand: **P-D-120 row 14** already makes `content_snapshot` *"the op
/// payload"* for every non-entity subject, and `api::rest::bulk`'s lifecycle
/// rows already render one as `{"op": …}`. This type names the tokens that
/// rendering may carry for this slice.
///
/// # The roster is closed, and an unknown token is not one of them
///
/// [`Self::parse`] answers `None` outside the three, and the submit door
/// treats `None` as *"declares no op"* — today's judgement, which is
/// material. A token that could be invented into the discount would make the
/// exception a caller's to claim by spelling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberOp {
    /// `POST …/config/vocabularies/{class}/values` — the governed add.
    Add,
    /// `POST …/members/{memberCode}/transitions` — one state-machine edge.
    Transition,
    /// `POST …/members/{memberCode}/label` — the `display_label` rename, and
    /// the only one of the three the exception reaches.
    Relabel,
}

impl MemberOp {
    /// The whole roster, in door order.
    ///
    /// A fourth op cannot land silently: [`Self::token`] and
    /// [`Self::is_display_label_rename`] are exhaustive matches, and
    /// `every_member_op_is_in_the_roster` matches every variant and asserts
    /// `ALL` carries it.
    pub const ALL: [Self; 3] = [Self::Add, Self::Transition, Self::Relabel];

    /// The token a submission declares, in the owning slice's own vocabulary
    /// — `02` spells its two `category.{op}` and `attribute_definition.{op}`,
    /// and this is `03`'s half of the same convention
    /// (`domain::live_op`'s own module doc names the shape).
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Add => "recognized_set.add",
            Self::Transition => "recognized_set.transition",
            Self::Relabel => "recognized_set.label",
        }
    }

    /// Parse a declared token, `None` outside the roster.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.token() == value)
    }

    /// Whether this op is the `display_label` change `design/05` §4 excepts
    /// from input (d)'s registration (**P-D-121** row 17).
    ///
    /// Exhaustive rather than a one-name `matches!`, so a fourth op has to
    /// say which side of the exception it is on.
    #[must_use]
    pub const fn is_display_label_rename(self) -> bool {
        match self {
            Self::Relabel => true,
            Self::Add | Self::Transition => false,
        }
    }
}

/// Whether the recognized-and-active check runs (**P-D-121** row 8).
///
/// A carried-forward value is judged by the state it had when declared;
/// only a **new or changed** declaration is judged against the current set.
/// `first_publish` is a new declaration even when the draft already carried
/// the value — the PRD treats the first publish that way.
#[must_use]
pub fn declaration_is_new(
    previous: Option<&str>,
    incoming: Option<&str>,
    first_publish: bool,
) -> bool {
    first_publish || previous != incoming
}

/// What a **new** meter declaration may name (`inst-mt-recognized`,
/// `dod-unit-recognition`).
///
/// `member` is the stored row, or `None` where the set never carried the
/// code. A `removed` tombstone answers the same refusal as an unknown code,
/// because it is outside the set; a `deprecated` member is its own refusal,
/// because the operator can act on it (re-list, or pick another unit) and
/// the code says which situation they are in.
///
/// # Errors
///
/// [`DomainError::UnrecognizedUnit`] or [`DomainError::UnitDeprecated`].
pub fn declaration_verdict(unit: &str, member: Option<MemberState>) -> Result<(), DomainError> {
    match member {
        Some(MemberState::Active) => Ok(()),
        Some(MemberState::Deprecated) => Err(DomainError::UnitDeprecated(format!(
            "metering unit `{unit}` is deprecated: existing published carriers keep resolving, \
             and a new declaration must name an active unit"
        ))),
        Some(MemberState::Removed) | None => Err(DomainError::UnrecognizedUnit(format!(
            "metering unit `{unit}` is not in the recognized set: the path to a new unit is the \
             recognized-set door's governed add, never an inline mint"
        ))),
    }
}

/// **The two types are the SDK port's since 2026-09-22.** They were declared
/// here while the port was in-crate; moving the port to `products-sdk` so
/// another module can register a catalog moved its operands with it, and a
/// second declaration of the same three fields is how a picker comes to
/// disagree with the gate about what a usage type is. Every
/// `crate::domain::recognized::UsageType*` path keeps resolving.
pub use bss_products_sdk::usage_types::{UsageTypeAnswer, UsageTypeBinding};

/// What the catalog said a `usageTypeRef` is bound to, in the stored form
/// frozen beside the version row at publish (`dod-binding-snapshot`,
/// **P-D-134** row 6, **P-D-146**): one JSON object, keys in alphabetical
/// order, the metadata keys sorted — so two publishes of the same binding
/// store the same bytes.
///
/// **Provenance, not content**: the snapshot lives in its own nullable column
/// on `products_entity_version`, outside the digested rendering, so
/// `DIGEST_VERSION` does not move with it and a re-verification of the digest
/// never reads it. The three fields are the three the definition of done
/// names; the catalog's own types are flattened to strings at the port so the
/// domain owes the collector SDK nothing.
///
/// This paragraph moved here with the types (2026-09-22): it argues about the
/// **snapshot**, which is this function's business, not about the shape of the
/// binding, which is now the port's.
///
/// A free function rather than an inherent method, because the type is the
/// SDK's now and this rendering is the **domain's** business: the snapshot is
/// what `binding_snapshot` freezes and what a re-verification reads, and the
/// port has no opinion about it.
#[must_use]
pub fn binding_snapshot_json(binding: &UsageTypeBinding) -> String {
    let mut fields = binding.metadata_fields.clone();
    fields.sort();
    serde_json::json!({
        "gts_id": binding.gts_id,
        "kind": binding.kind,
        "metadata_fields": fields,
    })
    .to_string()
}

/// Map a pre-transaction resolve onto the publish refusal
/// (**P-D-121** row 19). The validators phase receives the answer and
/// never calls out. A resolved answer hands back the binding the publish
/// freezes beside the version row (`dod-binding-snapshot`).
///
/// # Errors
///
/// [`DomainError::UsageTypeUnresolved`] or [`DomainError::UsageTypeUnavailable`].
pub fn judge_usage_type(
    answer: UsageTypeAnswer,
    usage_type_ref: &str,
) -> Result<UsageTypeBinding, DomainError> {
    match answer {
        UsageTypeAnswer::Resolved(binding) => Ok(binding),
        UsageTypeAnswer::Unresolved => Err(DomainError::UsageTypeUnresolved(format!(
            "usageTypeRef `{usage_type_ref}` did not resolve in the collector"
        ))),
        UsageTypeAnswer::Unavailable => Err(DomainError::UsageTypeUnavailable(format!(
            "the usage-type collector did not answer for `{usage_type_ref}`"
        ))),
    }
}

/// The atomic-pair rule (`inst-mt-atomic-pair`, `dod-meter-atomic`): the
/// resulting row carries `metering_unit` and `usage_type_ref` together or
/// not at all. The paired `CHECK` refuses the same shape at the physical
/// layer; this is the door's half, with the code the taxonomy names.
///
/// # Errors
///
/// [`DomainError::MeterDeclarationIncomplete`].
pub fn meter_pair_complete(
    metering_unit: Option<&str>,
    usage_type_ref: Option<&str>,
) -> Result<(), DomainError> {
    if metering_unit.is_some() == usage_type_ref.is_some() {
        return Ok(());
    }
    let (present, absent) = if metering_unit.is_some() {
        ("metering_unit", "usage_type_ref")
    } else {
        ("usage_type_ref", "metering_unit")
    };
    Err(DomainError::MeterDeclarationIncomplete(format!(
        "a MeterDeclaration is atomic: {present} arrived without {absent}, and the pair travels \
         together or not at all"
    )))
}

#[cfg(test)]
#[path = "recognized_tests.rs"]
mod recognized_tests;

// ------------------------------------------------------------------ 03 P-D-145

/// The closed set `inst-cl-type-profile` names, and the required-field set
/// each type carries at publish.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkuType {
    /// A commercial offer, realized by a plan. The only role a plan's own SKU
    /// may carry.
    Offer,
    /// A billable constituent of an offer, priced by a charge line.
    Component,
    /// Composition is pricing's; a bundle is commercially incomplete by
    /// design.
    Bundle,
}

impl SkuType {
    /// The wire tokens, in the order the design lists them.
    pub const ALL: [Self; 3] = [Self::Offer, Self::Component, Self::Bundle];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Offer => "offer",
            Self::Component => "component",
            Self::Bundle => "bundle",
        }
    }

    /// Parse a wire token; `None` for anything outside the closed set.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

/// `SKU_TYPE_UNKNOWN`: a present value outside the closed set, or — at
/// publish — no value at all (the create door refuses absence at the shape
/// phase, P-D-121 row 13, so this arm is reached only by a head written past
/// that door).
///
/// # Errors
///
/// [`DomainError::SkuTypeUnknown`].
pub fn type_profile(raw: Option<&str>) -> Result<SkuType, DomainError> {
    match raw {
        Some(value) => SkuType::parse(value).ok_or_else(|| {
            DomainError::SkuTypeUnknown(format!(
                "sku_type `{value}` is outside the closed set (offer, component, bundle)"
            ))
        }),
        None => Err(DomainError::SkuTypeUnknown(
            "sku_type is absent: a SKU publishes under one of offer, component or bundle"
                .to_owned(),
        )),
    }
}

/// `inst-pt-assign`'s verdict on a tier the head carries: unknown or
/// `removed` fails `PLAN_TIER_UNKNOWN`; a **new** assignment of a `deprecated`
/// tier fails `PLAN_TIER_DEPRECATED` while existing published carriers stay
/// valid — the caller says whether the assignment is new.
///
/// # Errors
///
/// [`DomainError::PlanTierUnknown`], [`DomainError::PlanTierDeprecated`].
pub fn tier_verdict(
    tier: &str,
    member: Option<MemberState>,
    new_assignment: bool,
) -> Result<(), DomainError> {
    match member {
        Some(MemberState::Active) => Ok(()),
        Some(MemberState::Deprecated) if !new_assignment => Ok(()),
        Some(MemberState::Deprecated) => Err(DomainError::PlanTierDeprecated(format!(
            "plan tier `{tier}` is deprecated: existing published carriers keep it, and a new \
             assignment must name an active tier"
        ))),
        Some(MemberState::Removed) | None => Err(DomainError::PlanTierUnknown(format!(
            "plan tier `{tier}` is not in the tenant's PlanTier set: the path to a new tier is \
             the recognized-set door's governed add"
        ))),
    }
}

/// The tier a create assigns when the caller names none: the seeded
/// `standard` (P-D-131 row 11 — mandatory on every SKU, so an empty tier would
/// make the first publish impossible).
pub const DEFAULT_PLAN_TIER: &str = "standard";

/// The platform baseline each set is seeded with on a tenant's **first write
/// that could need one** (P-D-104, P-D-121 row 10): the four units PRD §17.1
/// names and the `standard` tier (P-D-131 row 11, the one half of that
/// decision P-D-169 did not withdraw).
#[must_use]
pub const fn seed_roster(kind: SetKind) -> &'static [(&'static str, Option<&'static str>)] {
    match kind {
        SetKind::MeteringUnit => &[
            ("vCPU-hours", None),
            ("GB-storage", None),
            ("GB-egress", None),
            ("request-count", None),
        ],
        SetKind::PlanTier => &[(DEFAULT_PLAN_TIER, Some("Standard"))],
    }
}

/// The `seeded_by` token the baseline rows carry.
pub const SEEDED_BY_PLATFORM: &str = "platform";
