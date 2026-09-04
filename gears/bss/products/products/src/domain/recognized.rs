//! The recognized sets' rules — the kind roster, the member state machine,
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

/// The collector's three answers (`dod-usage-type-resolution`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageTypeAnswer {
    /// The ref resolved. Validators receive this and never call out.
    Resolved,
    /// The collector answered not-found.
    Unresolved,
    /// The collector was unreachable or unwired (**P-D-131**).
    Unavailable,
}

/// Map a pre-transaction resolve onto the publish refusal
/// (**P-D-121** row 19). The validators phase receives the answer and
/// never calls out.
///
/// # Errors
///
/// [`DomainError::UsageTypeUnresolved`] or [`DomainError::UsageTypeUnavailable`].
pub fn judge_usage_type(answer: UsageTypeAnswer, usage_type_ref: &str) -> Result<(), DomainError> {
    match answer {
        UsageTypeAnswer::Resolved => Ok(()),
        UsageTypeAnswer::Unresolved => Err(DomainError::UsageTypeUnresolved(format!(
            "usageTypeRef `{usage_type_ref}` did not resolve in the collector"
        ))),
        UsageTypeAnswer::Unavailable => Err(DomainError::UsageTypeUnavailable(format!(
            "the usage-type collector did not answer for `{usage_type_ref}`"
        ))),
    }
}

/// Resolve `usageTypeRef` before the publish transaction.
///
/// Tests use an admitting stub so the existing suite stays green; the
/// production binary is fail-closed Unavailable until `gear.rs` wires a
/// `ClientHub` collector (**P-D-131**). The three-outcome `DoD` probe is
/// [`judge_usage_type`].
#[must_use]
pub fn resolve_usage_type(_usage_type_ref: &str) -> UsageTypeAnswer {
    #[cfg(test)]
    {
        UsageTypeAnswer::Resolved
    }
    #[cfg(not(test))]
    {
        UsageTypeAnswer::Unavailable
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
