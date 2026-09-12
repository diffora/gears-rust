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

/// The four recognized sets (`design/03` §4's roster, pinned by no `CHECK` per
/// P-D-92 — the DDL pins non-emptiness only, so this enum is the roster's
/// enforcement site).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SetKind {
    /// Metering units — `dod-unit-*`'s set.
    MeteringUnit,
    /// Tax categories — an accounting-code set.
    TaxCategory,
    /// GL codes — the other accounting-code set.
    GlCode,
    /// Plan tiers — the one set with its own grant, event and refusal code.
    PlanTier,
}

impl SetKind {
    /// The stored and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MeteringUnit => "metering_unit",
            Self::TaxCategory => "tax_category",
            Self::GlCode => "gl_code",
            Self::PlanTier => "plan_tier",
        }
    }

    /// Parse a path segment or stored value, `None` outside the roster.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "metering_unit" => Some(Self::MeteringUnit),
            "tax_category" => Some(Self::TaxCategory),
            "gl_code" => Some(Self::GlCode),
            "plan_tier" => Some(Self::PlanTier),
            _ => None,
        }
    }

    /// The refusal a blocked removal raises — the one place the generic
    /// machinery answers per kind, because the design gives each family its
    /// own code (`UNIT_DELIST_BLOCKED`, `PLAN_TIER_RETIRE_BLOCKED`,
    /// `ACCOUNTING_CODE_DELIST_BLOCKED`).
    #[must_use]
    pub fn delist_blocked(self, detail: String) -> DomainError {
        match self {
            Self::MeteringUnit => DomainError::UnitDelistBlocked(detail),
            Self::PlanTier => DomainError::PlanTierRetireBlocked(detail),
            Self::TaxCategory | Self::GlCode => DomainError::AccountingCodeDelistBlocked(detail),
        }
    }
}

impl SetKind {
    /// The `products_sku` column whose value names a member of this set —
    /// the holder population a removal counts, **uniform across all four
    /// kinds** (`dod-recognized-set-mechanics`; P-D-146). Until 03's columns
    /// landed (P-D-145) only `metering_unit` had a carrier and the other
    /// three guards were necessarily off.
    #[must_use]
    pub const fn carrier_column(self) -> &'static str {
        match self {
            Self::MeteringUnit => "metering_unit",
            Self::PlanTier => "plan_tier",
            Self::TaxCategory => "tax_category_ref",
            Self::GlCode => "gl_code_ref",
        }
    }
}

/// The two columns whose change is **finance-material**
/// (`dod-finance-materiality`; `design/03` §4 puts both accounting codes in
/// bucket iii as Finance's). `plan_tier` is Product's, not Finance's, and is
/// deliberately absent.
pub const FINANCE_MATERIAL_COLUMNS: [&str; 2] = ["tax_category_ref", "gl_code_ref"];

/// Whether a publish that touched `touched` is finance-material — the operand
/// `dod-finance-predicate` was blocked on while the columns did not exist
/// (**P-D-146**). The submit door ORs this with the caller's own flag, so a
/// caller can still declare a change finance-material for a reason the
/// registry cannot see, but can no longer declare a code change *not* to be.
///
/// @cpt-dod:cpt-cf-bss-products-dod-finance-materiality:p1
#[must_use]
pub fn is_finance_material(touched: &[String]) -> bool {
    touched
        .iter()
        .any(|column| FINANCE_MATERIAL_COLUMNS.contains(&column.as_str()))
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

/// What the collector said a `usageTypeRef` is bound to, frozen beside the
/// version row at publish (`dod-binding-snapshot`, **P-D-134** row 6,
/// **P-D-146**).
///
/// Provenance, not content: the snapshot lives in its own nullable column on
/// `products_entity_version`, outside the digested rendering, so
/// `DIGEST_VERSION` does not move with it and a re-verification of the digest
/// never reads it. The three fields are the three the definition of done names; the
/// collector's own types are flattened to strings here so the domain owes the
/// collector SDK nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageTypeBinding {
    /// The resolved usage type's GTS id, as the collector spells it.
    pub gts_id: String,
    /// `counter` or `gauge`.
    pub kind: String,
    /// The metadata keys the usage type declares.
    pub metadata_fields: Vec<String>,
}

impl UsageTypeBinding {
    /// The stored form: one JSON object, keys in alphabetical order, the
    /// metadata keys sorted — so two publishes of the same binding store the
    /// same bytes.
    #[must_use]
    pub fn snapshot_json(&self) -> String {
        let mut fields = self.metadata_fields.clone();
        fields.sort();
        serde_json::json!({
            "gts_id": self.gts_id,
            "kind": self.kind,
            "metadata_fields": fields,
        })
        .to_string()
    }
}

/// The collector's three answers (`dod-usage-type-resolution`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageTypeAnswer {
    /// The ref resolved, to this binding. Validators receive it and never
    /// call out.
    Resolved(UsageTypeBinding),
    /// The collector answered not-found.
    Unresolved,
    /// The collector was unreachable or unwired (**P-D-131**).
    Unavailable,
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
    Product,
    Service,
    /// Composition is pricing's; a bundle is commercially incomplete by design
    /// and requires neither accounting code.
    Bundle,
}

impl SkuType {
    /// The wire tokens, in the order the design lists them.
    pub const ALL: [Self; 3] = [Self::Product, Self::Service, Self::Bundle];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Service => "service",
            Self::Bundle => "bundle",
        }
    }

    /// Parse a wire token; `None` for anything outside the closed set.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }

    /// Whether the type profile demands both accounting codes at publish.
    #[must_use]
    pub const fn requires_accounting_codes(self) -> bool {
        matches!(self, Self::Product | Self::Service)
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
                "sku_type `{value}` is outside the closed set (product, service, bundle)"
            ))
        }),
        None => Err(DomainError::SkuTypeUnknown(
            "sku_type is absent: a SKU publishes under one of product, service or bundle"
                .to_owned(),
        )),
    }
}

/// `ACCOUNTING_CODE_REQUIRED` at publish, naming the missing field: `product`
/// and `service` require both codes, `bundle` neither (`inst-cl-type-profile`).
///
/// # Errors
///
/// [`DomainError::AccountingCodeRequired`].
pub fn required_codes_present(
    kind: SkuType,
    tax_category_ref: Option<&str>,
    gl_code_ref: Option<&str>,
) -> Result<(), DomainError> {
    if !kind.requires_accounting_codes() {
        return Ok(());
    }
    let missing: Vec<&str> = [
        ("tax_category_ref", tax_category_ref),
        ("gl_code_ref", gl_code_ref),
    ]
    .into_iter()
    .filter(|(_, value)| value.is_none_or(|code| code.trim().is_empty()))
    .map(|(field, _)| field)
    .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(DomainError::AccountingCodeRequired(format!(
        "a `{}` SKU publishes with both accounting codes; missing: {}",
        kind.as_str(),
        missing.join(", ")
    )))
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

/// `inst-ac-codes`' verdict on one accounting code (`tax_category_ref` or
/// `gl_code_ref`, named in `field`): one code per refusal serving both fields.
///
/// # Errors
///
/// [`DomainError::AccountingCodeUnknown`], [`DomainError::AccountingCodeDeprecated`].
pub fn accounting_code_verdict(
    field: &str,
    code: &str,
    member: Option<MemberState>,
    new_assignment: bool,
) -> Result<(), DomainError> {
    match member {
        Some(MemberState::Active) => Ok(()),
        Some(MemberState::Deprecated) if !new_assignment => Ok(()),
        Some(MemberState::Deprecated) => Err(DomainError::AccountingCodeDeprecated(format!(
            "{field} `{code}` is deprecated: existing published carriers keep it, and a new \
             assignment must name an active code"
        ))),
        Some(MemberState::Removed) | None => Err(DomainError::AccountingCodeUnknown(format!(
            "{field} `{code}` is not in Finance's recognized set: the path to a new code is the \
             recognized-set door's governed add"
        ))),
    }
}

/// The tier a create assigns when the caller names none: the seeded
/// `standard` (P-D-131 row 11 — mandatory on every SKU, so an empty tier would
/// make the first publish impossible).
pub const DEFAULT_PLAN_TIER: &str = "standard";

/// The platform baseline each set is seeded with on a tenant's **first write
/// that could need one** (P-D-104, P-D-121 row 10): the four units PRD §17.1
/// names, the `standard` tier (P-D-131 row 11), and **nothing** for Finance's
/// two sets — their roster is Finance's to fill through the governed door.
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
        SetKind::TaxCategory | SetKind::GlCode => &[],
    }
}

/// The `seeded_by` token the baseline rows carry.
pub const SEEDED_BY_PLATFORM: &str = "platform";
