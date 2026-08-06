//! The **canonical scope key** and its eight axes.
//!
//! One key serves four jobs at once (`design/01-foundation.md` §4.1): row
//! uniqueness, supersession scoping, `PriceWindow` non-overlap, and window
//! ownership. That is why it is a type rather than a tuple assembled per call
//! site — every one of those four rules has to agree, to the axis, about what
//! "the same key" means.
//!
//! ```text
//! (planId, currency, region, priceOverlay, phase, priceEligibility, chargeKind, cohort)
//! ```
//!
//! It extends the manifest's `(plan, currency, region, priceOverlay)` key
//! **additively** with `phase`, `priceEligibility`, `chargeKind` (ADR
//! `cpt-cf-bss-pricing-adr-canonical-scope-key`) and `cohort` (ADR
//! `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis`), and supersedes it for
//! normative purposes.
//!
//! Axis defaults: `priceOverlay = base`, `priceEligibility = all_subscriptions`,
//! `cohort = none`. `phase` also has a default — the plan's terminal phase id —
//! but it is **data, not a constant**, so it is not a `Default` impl on
//! [`PhaseId`]; see that type.
//!
//! # The design set has nine and ten axes here, and this type still has eight
//!
//! **D-196 was decided by the product owner on 2026-08-06** and is **not yet
//! built**: the canonical scope key gains `(meter, dimensionKey)` **on
//! `chargeKind = 'usage'` rows**, because every usage line of one plan in one
//! market otherwise renders one key and the second is refused
//! `DUPLICATE_SCOPE_KEY` at save — which made D-103's confirmed multi-meter plan
//! unstorable while the meter-line index, `MeterInjectivity` and
//! `plan_rules::cycle_shape_tests` all assumed it storable.
//!
//! Until the clauses D-196 lists are paid, **every "eight axes" in this crate is
//! this type's truth and no longer the design set's** (`design/01-foundation.md`
//! §4.1). The pairing rule to build is an implication and not a biconditional —
//! a meter implies `usage`, while a usage row with no meter stays admissible —
//! and the rendering is to keep fixed arity at ten segments. The physical half
//! is §3.7's: `meter` is nullable and NULLs are distinct inside a `UNIQUE`, so
//! the two scope-key indexes key over `COALESCE(meter, '')`, measured rather
//! than assumed.

use std::fmt;

use chrono::{DateTime, Utc};
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::instant;
use crate::domain::money::CurrencyCode;
use crate::domain::validation::ValidationReport;

/// Rule code for a `cohort` / `priceEligibility` disagreement.
///
/// The design set states the rule (`design/01-foundation.md` §4.1) without
/// naming a code; this is the machine-readable discriminator the gear reports
/// it under, in the shape the rest of the code set uses.
pub const COHORT_ELIGIBILITY_MISMATCH: &str = "COHORT_ELIGIBILITY_MISMATCH";

/// A plan identifier — the first axis, and the aggregate every other axis
/// discriminates within.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlanId(Uuid);

impl PlanId {
    /// Wrap a plan id.
    #[must_use]
    pub const fn new(id: Uuid) -> Self {
        Self(id)
    }

    /// The underlying uuid.
    #[must_use]
    pub const fn get(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for PlanId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A phase identifier — the `phase` axis, **always id-typed** (D-19).
///
/// The axis was once a union of per-plan uuids and one reserved literal
/// `evergreen`; D-19 collapsed it to a uuid in every case. A phased plan's rows
/// carry its authored terminal `phase_id`; a non-phased or one-time plan gets
/// **one implicit terminal phase row** (kind `evergreen`) auto-created at plan
/// creation, and its id is the default. The literal `evergreen` survives only
/// as the phase *kind*, never as a value of this axis.
///
/// There is deliberately **no `Default`**. The default phase is *the plan's*
/// terminal phase id, which is per-plan data; a nil-uuid default would be a
/// phase id resolving to no phase row at all, and phase coverage, supersession
/// and duplicate-key comparisons would all silently compare rows against a
/// phase nobody authored.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhaseId(Uuid);

impl PhaseId {
    /// Wrap a phase id.
    #[must_use]
    pub const fn new(id: Uuid) -> Self {
        Self(id)
    }

    /// The underlying uuid.
    #[must_use]
    pub const fn get(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for PhaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A pricing region — the market axis, validated against the tenant region
/// taxonomy by Slice 4, not here.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Region(String);

impl Region {
    /// Wrap a region value.
    ///
    /// # Errors
    ///
    /// [`DomainError::InvalidRequest`] when the value is blank — an empty axis
    /// value is not "no region", it is a key component that cannot be compared.
    pub fn new(value: &str) -> Result<Self, DomainError> {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(DomainError::InvalidRequest(
                "region axis value must not be blank".to_owned(),
            ));
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// The region value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The `priceOverlay` axis.
///
/// Rows authored by this gear always carry [`PriceOverlay::Base`], which is why
/// that is the only variant. Partner / orgTier / brand overlays are **separate
/// `PriceOverlay` rows** with their own scope, precedence and dating, joined to
/// base rows by Tariffs at evaluation — they are never a value of this axis on
/// an authored price row. The axis stays in the key because the manifest key it
/// extends carries it and because a future non-base authored row would have to
/// be a distinct key, not an overwrite.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PriceOverlay {
    /// The base price plane — every row this gear authors.
    #[default]
    Base,
}

impl PriceOverlay {
    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
        }
    }
}

impl fmt::Display for PriceOverlay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `priceEligibility` axis: which subscriptions a row may price.
///
/// Three classes, not two. When more than one holds an active window on the
/// same remaining axes, Tariffs selects **most-specific-wins** —
/// `existing_grandfathered` > `new_subscriptions_only` > `all_subscriptions`
/// (PRD §1.4, `design/07-pricewindow-linkage.md` W3). The variants are declared
/// in that order **reversed**, least specific first, so the derived [`Ord`]
/// ranks them exactly as the class order does and no later code can build a
/// second, disagreeing ranking out of this type.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PriceEligibility {
    /// Every subscription on the key. The default, and the class a routine
    /// supersession stays inside.
    #[default]
    AllSubscriptions,
    /// Priced only for subscriptions created on or after the row's cutover.
    /// Existing subscriptions are never re-bound to it and keep their prior
    /// snapshot (PRD AC #59). Like [`PriceEligibility::AllSubscriptions`] and
    /// unlike a grandfathered generation it carries `cohort = none`: the cohort
    /// axis discriminates *retained* generations, and this class retains
    /// nobody.
    NewSubscriptionsOnly,
    /// A grandfathered generation: an immutable copy retained for subscribers
    /// who were on the key when a cutover closed it. Always paired with a
    /// non-`none` [`Cohort`].
    ExistingGrandfathered,
}

impl PriceEligibility {
    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AllSubscriptions => "all_subscriptions",
            Self::NewSubscriptionsOnly => "new_subscriptions_only",
            Self::ExistingGrandfathered => "existing_grandfathered",
        }
    }
}

impl fmt::Display for PriceEligibility {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `chargeKind` axis: which component of the plan a row prices.
///
/// It is in the key because a single plan legitimately carries several
/// components **at once**: a hybrid plan holds a `recurring` **and** a `usage`
/// row (optionally a `one_time_setup` row) on one `planId`, all on the same
/// currency, region and phase. Without this axis those rows would collide on
/// the duplicate-key index and the second one would be rejected as a duplicate
/// of the first. With it they are **distinct keys**, each with its own windows
/// and its own supersession chain.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ChargeKind {
    /// A recurring subscription charge.
    Recurring,
    /// A metered usage charge.
    Usage,
    /// A one-off charge that is not a setup fee.
    OneTime,
    /// A one-off setup fee, distinguished from [`ChargeKind::OneTime`] so a
    /// hybrid plan can carry both without them colliding on one key.
    OneTimeSetup,
}

impl ChargeKind {
    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recurring => "recurring",
            Self::Usage => "usage",
            Self::OneTime => "one_time",
            Self::OneTimeSetup => "one_time_setup",
        }
    }
}

impl fmt::Display for ChargeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The `cohort` axis: the grandfathering **generation** discriminator.
///
/// It is in the key because grandfathering is repeatable. Every cutover creates
/// a **new** generation on **its own key**, identified by the UTC instant the
/// cutover happened. Repricing a key three times with per-cohort retention
/// therefore produces three retained generations on three distinct keys, each
/// with its own window — so their windows can all be active at once without
/// ever violating the non-overlap rule, which is enforced per key. Were the
/// cohort not in the key, the second cutover would land on the first
/// generation's key and either be rejected as a duplicate or overwrite a row
/// that some subscriber is still priced from.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cohort {
    /// Not a grandfathered row. The default.
    #[default]
    None,
    /// A grandfathered generation, stamped with the UTC cutover instant that
    /// created it. Tariffs selects within the `existing_grandfathered` class by
    /// matching this instant.
    Generation(DateTime<Utc>),
}

impl Cohort {
    /// Is this the classless (non-grandfathered) axis value?
    #[must_use]
    pub const fn is_none(self) -> bool {
        matches!(self, Self::None)
    }

    /// The cutover instant, when there is one.
    #[must_use]
    pub fn generation(self) -> Option<DateTime<Utc>> {
        match self {
            Self::None => Option::None,
            Self::Generation(at) => Some(at),
        }
    }
}

impl fmt::Display for Cohort {
    /// Epoch milliseconds, and **lossless** because of it: [`ScopeKey::new`]
    /// refuses a generation below the quantum (D-144), so this rendering can
    /// never be the place an instant quietly loses precision on its way into a
    /// key that is then matched for equality.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::None => f.write_str("none"),
            Self::Generation(at) => write!(f, "{}", at.timestamp_millis()),
        }
    }
}

/// The `cohort` / `priceEligibility` biconditional:
/// **`cohort != none` if and only if `priceEligibility == existing_grandfathered`**.
///
/// Both directions matter and they fail differently. A cohort on a
/// non-grandfathered row mints a key no resolution class ever selects, so the
/// row is published and never priced from. A grandfathered row without a cohort
/// lands on the `all_subscriptions` successor's own key, where it is either
/// rejected as a duplicate or — worse — becomes an immutable row occupying the
/// key the next reprice needs.
///
/// It is a **checked function** rather than a type-level guarantee because the
/// two axes are separate key columns: they are read back from the database as
/// two independent values, so the pairing has to be re-established on every
/// rehydration, not only at first construction. This is the entry point that
/// path calls; [`ScopeKey::new`] calls it too.
///
/// The rejection is carried as a [`DomainError::ValidationFailed`] envelope
/// with one violation rather than a bespoke variant, so the rule has exactly
/// one representation whether it is hit here or as a rule registered in the
/// publish pipeline — the error mapping renders the code either way, and the
/// taxonomy does not grow a variant per rule.
///
/// # Errors
///
/// [`DomainError::ValidationFailed`] carrying a single
/// [`COHORT_ELIGIBILITY_MISMATCH`] violation when the two axes disagree.
pub fn check_cohort_eligibility(
    price_eligibility: PriceEligibility,
    cohort: Cohort,
) -> Result<(), DomainError> {
    let grandfathered = matches!(price_eligibility, PriceEligibility::ExistingGrandfathered);
    let generational = !cohort.is_none();
    if grandfathered == generational {
        return Ok(());
    }
    let mut report = ValidationReport::default();
    report.violate(
        COHORT_ELIGIBILITY_MISMATCH,
        format!("{price_eligibility}/{cohort}"),
        "cohort is set if and only if priceEligibility is existing_grandfathered",
    );
    Err(DomainError::ValidationFailed(report))
}

/// The canonical scope key.
///
/// Fields are private and the constructor validates, so a key that violates the
/// cohort biconditional cannot be handed to the duplicate-key index or the
/// window rules. Axis order below is the normative order and is the order
/// [`fmt::Display`] renders in.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ScopeKey {
    plan_id: PlanId,
    currency: CurrencyCode,
    region: Region,
    price_overlay: PriceOverlay,
    phase: PhaseId,
    price_eligibility: PriceEligibility,
    charge_kind: ChargeKind,
    cohort: Cohort,
}

impl ScopeKey {
    /// Build a validated key for a row this gear authors.
    ///
    /// `price_overlay` is not a parameter: rows authored here always carry
    /// [`PriceOverlay::Base`], and partner / orgTier / brand overlays are
    /// separate overlay rows rather than a value of this axis. Passing it would
    /// offer a choice the authoring plane does not have.
    ///
    /// `phase` is required rather than defaulted: the default is the plan's
    /// terminal phase id, which only the caller holding the plan can resolve
    /// (D-19).
    ///
    /// # Errors
    ///
    /// [`DomainError::ValidationFailed`] when `price_eligibility` and `cohort`
    /// disagree; see [`check_cohort_eligibility`].
    /// [`DomainError::TimestampPrecisionExceeded`] when a generational `cohort`
    /// carries precision below the millisecond quantum (D-144) — this axis is
    /// matched for **equality** against an instant a different gear produced, so
    /// an unquantized value would build a key nobody can find rather than a key
    /// that is wrong.
    pub fn new(
        plan_id: PlanId,
        currency: CurrencyCode,
        region: Region,
        phase: PhaseId,
        price_eligibility: PriceEligibility,
        charge_kind: ChargeKind,
        cohort: Cohort,
    ) -> Result<Self, DomainError> {
        check_cohort_eligibility(price_eligibility, cohort)?;
        if let Some(generation) = cohort.generation() {
            instant::check_quantum("cohort", generation)?;
        }
        Ok(Self {
            plan_id,
            currency,
            region,
            price_overlay: PriceOverlay::Base,
            phase,
            price_eligibility,
            charge_kind,
            cohort,
        })
    }

    /// Axis 1 — the plan.
    #[must_use]
    pub const fn plan_id(&self) -> PlanId {
        self.plan_id
    }

    /// Axis 2 — the currency.
    #[must_use]
    pub const fn currency(&self) -> &CurrencyCode {
        &self.currency
    }

    /// Axis 3 — the pricing region.
    #[must_use]
    pub const fn region(&self) -> &Region {
        &self.region
    }

    /// Axis 4 — the overlay plane (always `base` on an authored row).
    #[must_use]
    pub const fn price_overlay(&self) -> PriceOverlay {
        self.price_overlay
    }

    /// Axis 5 — the phase.
    #[must_use]
    pub const fn phase(&self) -> PhaseId {
        self.phase
    }

    /// Axis 6 — the eligibility class.
    #[must_use]
    pub const fn price_eligibility(&self) -> PriceEligibility {
        self.price_eligibility
    }

    /// Axis 7 — the charge component.
    #[must_use]
    pub const fn charge_kind(&self) -> ChargeKind {
        self.charge_kind
    }

    /// Axis 8 — the grandfathering generation.
    #[must_use]
    pub const fn cohort(&self) -> Cohort {
        self.cohort
    }
}

impl fmt::Display for ScopeKey {
    /// The canonical rendering: eight axes, normative order, one separator.
    /// This is the string a `DUPLICATE_SCOPE_KEY` rejection names, so it has to
    /// be stable and complete — a rendering that dropped an axis would report a
    /// collision between two rows that do not actually share a key.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.plan_id,
            self.currency,
            self.region,
            self.price_overlay,
            self.phase,
            self.price_eligibility,
            self.charge_kind,
            self.cohort
        )
    }
}

#[cfg(test)]
#[path = "scope_key_tests.rs"]
mod scope_key_tests;
