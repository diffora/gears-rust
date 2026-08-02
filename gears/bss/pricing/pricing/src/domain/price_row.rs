//! The authored **price row** — the Slice-3-owned shape
//! (`design/03-price-structure.md`).
//!
//! This is what an operator authors and what the Slice-3 rules judge: the
//! explicit `modelKind`, the per-kind money placement, the tier bands under the
//! `[fromQty, toQty)` convention, the package block fields, and the usage
//! evaluation policy (`billingGranularity`, `tierAggregationWindow`, the D-44
//! level-aggregation triple). The `chargeKind` rides along from the canonical
//! scope key because half the shape rules are a function of it.
//!
//! **Per-kind optionality is modelled honestly.** Nearly every field is an
//! `Option`, and none of them is defaulted here. A price row is *authored*
//! before it is *published*, so the type has to be able to hold a draft that is
//! not yet publishable — the rules in [`crate::domain::rules`] are what make a
//! field required for a kind, and a type that made `amount_minor` mandatory
//! would have made `AMOUNT_PLACEMENT_INVALID` unreachable for the one kind it
//! exists to catch.
//!
//! **This type computes no charge.** It carries structure; Tariffs evaluates it.
//!
//! ## Structural compatibility with the conformance corpus
//!
//! The corpus (`bss_fixtures::model::Snapshot`) is the shape the joint golden
//! fixtures freeze, and the publish validator compares against it. This type is
//! deliberately field-for-field compatible with it — same names, same meaning —
//! but it is a **separate type**: the corpus model sits behind the crate's
//! `corpus` feature, which a gear does not enable (the production surface is
//! `ModelKind` + `Registry`), and the corpus `Snapshot` additionally carries
//! `currency`, `proration_basis` and the Slice-10 reservation fields that are
//! not Slice-3-owned.
//!
//! [`ModelKind`] itself is **reused** from `bss_fixtures` rather than mirrored.
//! One vocabulary for the kind enum means the fixture gate's lookup is total by
//! construction, and a sixth kind cannot appear on one side only.

use std::fmt;

use toolkit_macros::domain_model;

pub use bss_fixtures::ModelKind;

use crate::domain::money::MinorAmount;
use crate::domain::scope_key::ChargeKind;

/// How a non-usage `per_unit` row obtains its quantity.
///
/// Usage rows never carry this: their `Q` comes from the meter, and offering an
/// authored quantity beside a metered one would be two answers to "how much was
/// consumed" (`inst-mk-required`, 2026-07-28 review fix).
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum QuantitySource {
    /// Subscriptions supplies the seat count at rating time.
    SubscriptionSeatCount,
    /// The author states a fixed quantity, carried in
    /// [`PriceRow::manual_quantity`].
    Manual,
}

impl QuantitySource {
    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SubscriptionSeatCount => "subscription_seat_count",
            Self::Manual => "manual",
        }
    }
}

impl fmt::Display for QuantitySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The quantization applied to a metered quantity before it is priced.
///
/// It is **not** a period bound: it says what one billable unit is, not over
/// what stretch the units accumulate. That is why a `package` row needs
/// [`TierAggregationWindow`] as well (`inst-pk-window`, D-58).
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BillingGranularity {
    /// Seconds.
    PerSecond,
    /// Minutes.
    PerMinute,
    /// Hours. The counterpart of [`AggregationGranularity::Hour`] (D-77).
    PerHour,
    /// Days. The counterpart of [`AggregationGranularity::Day`] (D-77).
    PerDay,
    /// No quantization: the raw metered unit is the billable unit.
    WholeUnit,
}

impl BillingGranularity {
    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PerSecond => "per_second",
            Self::PerMinute => "per_minute",
            Self::PerHour => "per_hour",
            Self::PerDay => "per_day",
            Self::WholeUnit => "whole_unit",
        }
    }
}

impl fmt::Display for BillingGranularity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// When the tier counter `Q` resets — and, on a `package` row, the window over
/// which `used` accumulates **before** block round-up (D-58).
///
/// The catalog persists the enum and nothing more: the reset semantics are
/// Tariffs-owned.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TierAggregationWindow {
    /// The calendar month.
    CalendarMonth,
    /// The subscription's invoice period.
    InvoicePeriod,
    /// Never resets.
    SubscriptionLifetime,
    /// Each event stands alone.
    PerEvent,
}

impl TierAggregationWindow {
    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CalendarMonth => "calendar_month",
            Self::InvoicePeriod => "invoice_period",
            Self::SubscriptionLifetime => "subscription_lifetime",
            Self::PerEvent => "per_event",
        }
    }
}

impl fmt::Display for TierAggregationWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The D-40 third window: which period **qualifies** the rate tier.
///
/// Orthogonal to both [`TierAggregationWindow`] (counter reset) and
/// [`BillingGranularity`] (billing cadence): `trailing_period` qualifies the tier
/// from the prior period's total and locks it for this one.
///
/// [`TierQualificationWindow::Current`] is the **default** the PRD states, so a
/// row that authors nothing qualifies from the current window.
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TierQualificationWindow {
    /// Qualify from the current window's own running total. The default.
    #[default]
    Current,
    /// Qualify from the prior period's total, then lock.
    TrailingPeriod,
}

impl TierQualificationWindow {
    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::TrailingPeriod => "trailing_period",
        }
    }
}

impl fmt::Display for TierQualificationWindow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How `Q` is derived from the measures in the window (D-44).
///
/// [`AggregationFunction::Sum`] is the **default**: a row that authors nothing
/// is a `sum` row. The two non-`sum` functions are granule *folds* and are what
/// make a row "level-shaped"; the fold itself is Rating's (T-D-17).
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AggregationFunction {
    /// The plain window sum of normalised measures. The default; not a fold.
    #[default]
    Sum,
    /// The maximum sample in each granule.
    Peak,
    /// The step-integral of the level over each granule.
    TimeWeighted,
}

impl AggregationFunction {
    /// Is this the default window sum — i.e. does the row have no granule fold?
    #[must_use]
    pub const fn is_sum(self) -> bool {
        matches!(self, Self::Sum)
    }

    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sum => "sum",
            Self::Peak => "peak",
            Self::TimeWeighted => "time_weighted",
        }
    }
}

impl fmt::Display for AggregationFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The granule a non-`sum` window is cut into (D-44).
///
/// [`AggregationGranularity::Hour`] is the default. Each value has exactly one
/// legal [`BillingGranularity`] counterpart (D-77); see
/// [`AggregationGranularity::billing_counterpart`].
#[domain_model]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AggregationGranularity {
    /// One-hour granules. The default.
    #[default]
    Hour,
    /// One-day granules.
    Day,
}

impl AggregationGranularity {
    /// The one [`BillingGranularity`] this granule may pair with (D-77).
    ///
    /// The pairing is what stops `inst-tb-units` and `inst-la-units` naming
    /// different units for the same band — a factor-of-24 error at the band
    /// edge that every other stated check passes.
    #[must_use]
    pub const fn billing_counterpart(self) -> BillingGranularity {
        match self {
            Self::Hour => BillingGranularity::PerHour,
            Self::Day => BillingGranularity::PerDay,
        }
    }

    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hour => "hour",
            Self::Day => "day",
        }
    }
}

impl fmt::Display for AggregationGranularity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// What happens to an unused included allowance at the end of a period (D-45).
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RolloverPolicy {
    /// Unused allowance expires. A purely row-local lever.
    None,
    /// Unused allowance carries forward. Compiles into a **plan-scoped**,
    /// revision-immutable grant row, which is why a supersession may not change
    /// it (D-129).
    Carry,
}

impl RolloverPolicy {
    /// Does this policy compile into a plan-scoped grant?
    #[must_use]
    pub const fn is_carry(self) -> bool {
        matches!(self, Self::Carry)
    }

    /// The persisted / wire token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Carry => "carry",
        }
    }
}

impl fmt::Display for RolloverPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// `includedAllowance {quantity, rolloverPolicy}` (D-45).
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct IncludedAllowance {
    /// The included quantity, in the row's billable unit.
    pub quantity: u64,
    /// What happens to the unused remainder.
    pub rollover_policy: RolloverPolicy,
}

/// The upper bound of a tier band.
///
/// An enum rather than an `Option<u64>` so it reads the way the corpus and the
/// column do — `NULL` / `"open"` is a *state* of the band, not an absent value —
/// and so the D-17 top-band rule has something to match on.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BandTop {
    /// No upper bound. The top band is always this (D-17).
    Open,
    /// The exclusive upper bound.
    Closed(u64),
}

impl BandTop {
    /// Is the band unbounded above?
    #[must_use]
    pub const fn is_open(self) -> bool {
        matches!(self, Self::Open)
    }

    /// The exclusive upper bound, when there is one.
    #[must_use]
    pub const fn closed_at(self) -> Option<u64> {
        match self {
            Self::Open => None,
            Self::Closed(top) => Some(top),
        }
    }
}

impl fmt::Display for BandTop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => f.write_str("open"),
            Self::Closed(top) => write!(f, "{top}"),
        }
    }
}

/// One tier band: the half-open quantity range `[from_qty, to_qty)` and the unit
/// price that applies inside it.
///
/// **Band quantities are billable units — the units that exist *after*
/// [`BillingGranularity`] quantization.** A `per_hour` row's bands count hours,
/// never raw seconds; a `per_day` row's bands count days. This is normative
/// (`inst-tb-units`) and it is the sentence that stops the catalog and Tariffs
/// diverging about what `from_qty = 100` means. On a non-`sum` (level) row the
/// granule fold *is* the quantization, so the billable unit is
/// `level unit x granule` (GB-hours at `hour`, GB-days at `day`) and the rule
/// holds unchanged — which is exactly why D-77 pins the granularity pairing.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TierBand {
    /// Inclusive lower bound, in billable units.
    pub from_qty: u64,
    /// Exclusive upper bound, in billable units; [`BandTop::Open`] on the top
    /// band.
    pub to_qty: BandTop,
    /// The unit price inside the band. `0` is valid — a free first band is a
    /// normal way to author "N included".
    pub unit_price_minor: MinorAmount,
}

impl TierBand {
    /// A closed band.
    #[must_use]
    pub const fn closed(from_qty: u64, to_qty: u64, unit_price_minor: MinorAmount) -> Self {
        Self {
            from_qty,
            to_qty: BandTop::Closed(to_qty),
            unit_price_minor,
        }
    }

    /// An open-topped band.
    #[must_use]
    pub const fn open(from_qty: u64, unit_price_minor: MinorAmount) -> Self {
        Self {
            from_qty,
            to_qty: BandTop::Open,
            unit_price_minor,
        }
    }

    /// Does this band cover no quantity at all (`to_qty <= from_qty`)?
    ///
    /// A band that covers nothing is not harmlessly ignorable: it makes the band
    /// set's contiguity ambiguous and it is always an authoring mistake.
    #[must_use]
    pub const fn is_zero_width(&self) -> bool {
        match self.to_qty {
            BandTop::Open => false,
            BandTop::Closed(top) => top <= self.from_qty,
        }
    }
}

impl fmt::Display for TierBand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}, {})", self.from_qty, self.to_qty)
    }
}

/// The authored price row: the Slice-3 shape, plus the `chargeKind` axis it is
/// judged against.
///
/// Fields are public and unvalidated on purpose. The row is the *subject* the
/// [`crate::domain::validation::ValidationPipeline`] runs over, and a
/// constructor that refused an unpublishable combination would move the rules
/// into the type — where they could not be enumerated into one aggregate report,
/// which is the whole point of the fail-closed pipeline.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceRow {
    /// Scope-key axis 7. Not authored on the row itself — it comes from the key
    /// — but it is carried here because the kind matrix, the evaluation-policy
    /// placement rules and the supersession guard are all functions of it.
    pub charge_kind: ChargeKind,
    /// The explicit model kind. `None` is the authoring state
    /// `MODEL_KIND_MISSING` rejects; there is no implicit default at rating time.
    pub model_kind: Option<ModelKind>,
    /// The single amount on `flat`, the unit price on `per_unit`, and **NULL**
    /// on `graduated` / `volume` / `package`, whose money lives in the band or
    /// package column — so no row ever carries two competing prices.
    pub amount_minor: Option<MinorAmount>,
    /// The authored tier bands, in authored order. Empty on a non-tiered row.
    ///
    /// Authored bands only: the D-45 allowance compile is a projection and never
    /// rewrites this set (D-130).
    pub bands: Vec<TierBand>,
    /// Units per block; `package` only, `> 0`.
    pub package_size: Option<u64>,
    /// Price per block; `package` only.
    pub package_price_minor: Option<MinorAmount>,
    /// Where a **non-usage** `per_unit` row's quantity comes from.
    pub quantity_source: Option<QuantitySource>,
    /// The fixed quantity, required when
    /// [`QuantitySource::Manual`] is authored.
    pub manual_quantity: Option<u64>,
    /// The published metering unit a usage row prices.
    pub meter: Option<String>,
    /// The dimension discriminator on the `(meter, dimensionKey)` line.
    ///
    /// Not an `Option`: the column is `NOT NULL DEFAULT ''` and the empty string
    /// is the **empty-tuple sentinel**, so the Slice-2 injectivity index collides
    /// undimensioned rows instead of treating them as distinct `NULL`s.
    pub dimension_key: String,
    /// The billable-unit quantization; required on every usage row.
    pub billing_granularity: Option<BillingGranularity>,
    /// The counter-reset window; required on tiered and `package` usage rows.
    pub tier_aggregation_window: Option<TierAggregationWindow>,
    /// The D-40 tier-qualification window; optional on a tiered row.
    pub tier_qualification_window: Option<TierQualificationWindow>,
    /// How `Q` is derived. `None` means [`AggregationFunction::Sum`].
    pub aggregation_function: Option<AggregationFunction>,
    /// The fold granule. `None` means [`AggregationGranularity::Hour`] on a
    /// non-`sum` row, and is the only legal value on a `sum` row.
    pub aggregation_granularity: Option<AggregationGranularity>,
    /// The `hold_last` bound, in granules; required on non-`sum` rows, forbidden
    /// otherwise. No default — the sampling-gap bound is a commercial statement.
    pub max_hold_granules: Option<u64>,
    /// The D-45 included allowance.
    pub included_allowance: Option<IncludedAllowance>,
}

impl PriceRow {
    /// An otherwise-empty row on `charge_kind` carrying `model_kind`.
    ///
    /// Every optional field starts absent and `dimension_key` starts at the
    /// empty-tuple sentinel, which is the authored state of a row nobody has
    /// filled in yet — not a publishable one.
    #[must_use]
    pub fn new(charge_kind: ChargeKind, model_kind: Option<ModelKind>) -> Self {
        Self {
            charge_kind,
            model_kind,
            amount_minor: None,
            bands: Vec::new(),
            package_size: None,
            package_price_minor: None,
            quantity_source: None,
            manual_quantity: None,
            meter: None,
            dimension_key: String::new(),
            billing_granularity: None,
            tier_aggregation_window: None,
            tier_qualification_window: None,
            aggregation_function: None,
            aggregation_granularity: None,
            max_hold_granules: None,
            included_allowance: None,
        }
    }

    /// Is this a metered usage row?
    #[must_use]
    pub const fn is_usage(&self) -> bool {
        matches!(self.charge_kind, ChargeKind::Usage)
    }

    /// Is the kind one whose money lives in tier bands?
    #[must_use]
    pub const fn is_tiered(&self) -> bool {
        matches!(
            self.model_kind,
            Some(ModelKind::Graduated | ModelKind::Volume)
        )
    }

    /// The row's effective derivation function: what it authored, else the
    /// [`AggregationFunction::Sum`] default.
    ///
    /// Every level rule reads the row through here rather than through the raw
    /// `Option`, because "authored nothing" and "authored `sum`" are the same
    /// row and must not validate differently.
    #[must_use]
    pub fn effective_aggregation_function(&self) -> AggregationFunction {
        self.aggregation_function.unwrap_or_default()
    }

    /// The row's effective fold granule: what it authored, else the
    /// [`AggregationGranularity::Hour`] default.
    #[must_use]
    pub fn effective_aggregation_granularity(&self) -> AggregationGranularity {
        self.aggregation_granularity.unwrap_or_default()
    }

    /// The row's effective tier-qualification window: what it authored, else
    /// the [`TierQualificationWindow::Current`] default the PRD states
    /// ("`current` (default) | `trailing_period`").
    ///
    /// Read through here for the same reason as the two above: an unauthored
    /// window and an authored `current` are the same row, and comparing the raw
    /// `Option`s across a supersession would reject a change that changed
    /// nothing.
    #[must_use]
    pub fn effective_tier_qualification_window(&self) -> TierQualificationWindow {
        self.tier_qualification_window.unwrap_or_default()
    }

    /// Does this row carry a granule fold (a non-`sum` derivation)?
    #[must_use]
    pub fn is_level(&self) -> bool {
        !self.effective_aggregation_function().is_sum()
    }

    /// How a finding locates this row for the author.
    ///
    /// The row has no identity of its own at this layer — a price id is assigned
    /// by the persistence path, and the S3 shape is exactly the authored fields —
    /// so the subject is the coordinates that make the finding readable: the
    /// charge component and the kind under judgement.
    #[must_use]
    pub fn subject(&self) -> String {
        let kind = self.model_kind.map_or("(no model kind)", model_kind_wire);
        format!("{}/{kind}", self.charge_kind)
    }
}

/// The **unit- and counter-determining fields** two rows of one metered line
/// disagree on, in the spelling the design set names them.
///
/// Seven fields — `model_kind`, `billingGranularity`, `aggregationFunction`,
/// `aggregationGranularity`, `tierAggregationWindow`, `tierQualificationWindow`
/// and `package_size` (the D-82 / D-98 list, extended by D-122). Empty means the
/// two rows meter, derive and price the same way, and whatever else differs
/// between them is a **price** lever.
///
/// ## One list, because two guards ask the same question
///
/// The tier counter `Q` is keyed `(subscription, meter, dimensionKey, window)`
/// and belongs to the subscription's usage history rather than to any row. Two
/// mechanisms hand a subscriber from one row to another **without** resetting
/// it, and each has its own rule saying the denomination must not move across
/// the handover:
///
/// - **supersession** ([`SupersessionPair::mismatched_unit_fields`](crate::domain::rules::supersession::SupersessionPair::mismatched_unit_fields),
///   `inst-tb-supersession-units`), which adds `meter`, `dimensionKey` and the
///   `carry`-conditioned `included_allowance` to this list;
/// - **phase conversion**
///   ([`PhaseOverrideUnits`](crate::domain::plan_rules::phase_graph::PhaseOverrideUnits),
///   `inst-ph-override-units`, D-89), where a `per_hour` trial row converting
///   into a `per_day` evergreen row applies an hours-denominated `Q` to
///   day-denominated bands.
///
/// Both are the D-77 factor-of-24 class arriving through a different door, so
/// the list is written **once**. Two hand-maintained copies is exactly how that
/// class re-enters: the D-127 lesson is that a guard must not be able to differ
/// by which mechanism asked, and a list that can drift is a guard that differs.
///
/// The comparison is **symmetric** — it answers *which fields moved*, not which
/// way — so a caller passes its two rows in whatever order reads chronologically
/// at the call site.
///
/// `aggregationFunction`, `aggregationGranularity` and `tierQualificationWindow`
/// are read through the `effective_*` accessors and never through the raw
/// `Option`s: "authored nothing" and "authored the default" are the same row,
/// and comparing the `Option`s would report a change on a pair that changed
/// nothing at all.
#[must_use]
pub fn unit_determining_mismatch(before: &PriceRow, after: &PriceRow) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if before.model_kind != after.model_kind {
        changed.push("model_kind");
    }
    if before.billing_granularity != after.billing_granularity {
        changed.push("billingGranularity");
    }
    if before.effective_aggregation_function() != after.effective_aggregation_function() {
        changed.push("aggregationFunction");
    }
    if before.effective_aggregation_granularity() != after.effective_aggregation_granularity() {
        changed.push("aggregationGranularity");
    }
    if before.tier_aggregation_window != after.tier_aggregation_window {
        changed.push("tierAggregationWindow");
    }
    if before.effective_tier_qualification_window() != after.effective_tier_qualification_window() {
        changed.push("tierQualificationWindow");
    }
    if before.package_size != after.package_size {
        changed.push("package_size");
    }
    changed
}

/// The corpus / wire spelling of a model kind.
///
/// A free function rather than an inherent method because [`ModelKind`] belongs
/// to `bss_fixtures`. Exhaustive on purpose: a sixth kind cannot enter the enum
/// without this match being extended, and the spelling has to match the fixture
/// registry byte for byte — `PerUnit` is not a string that appears in
/// `registry.toml`, and it is not a token any rejection may render.
#[must_use]
pub const fn model_kind_wire(kind: ModelKind) -> &'static str {
    match kind {
        ModelKind::Flat => "flat",
        ModelKind::PerUnit => "per_unit",
        ModelKind::Graduated => "graduated",
        ModelKind::Volume => "volume",
        ModelKind::Package => "package",
    }
}

#[cfg(test)]
#[path = "price_row_tests.rs"]
mod price_row_tests;
