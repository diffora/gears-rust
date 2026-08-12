//! What a mass-repricing run **selects** — the row set, and the one refusal that
//! stands between an operator and a run that would do nothing
//! (`design/12-operator-efficiency.md` §3 `inst-mr-api`, §3 `inst-mp-grandfathered`,
//! §5's `RUN_SELECTOR_EMPTY`; D-134, D-307).
//!
//! # The selector is the canonical key's axes, and it is deliberately no new
//! vocabulary
//!
//! `inst-mr-api` names *"`run_id`, selector, adjustment, changeover instant"* and
//! defines none of the four. The selector is settled here as **any subset of the
//! canonical scope key's axes**: a run selects published rows whose key matches
//! every axis the selector names and ignores the axes it does not. That reading
//! was chosen over a query language, a saved-search id and a row-id list for
//! three reasons that are about what the rest of the design set already says:
//!
//! 1. *"A currency segment"* — §2's own example of what a run acts on — is then
//!    literally one axis, `currency`, and needs no translation.
//! 2. D-134 makes the run's transaction unit the **plan**. A selector built from
//!    the key has `plan_id` in it, so the per-plan grouping the apply owes falls
//!    out of a column rather than out of a second grouping rule.
//! 3. A row-id list would be a selector the run cannot re-evaluate and cannot
//!    explain; the key's axes are the vocabulary the operator already authored
//!    the rows under, so a run is describable in the same words as the catalog.
//!
//! Nine axes and not ten: `priceOverlay` is not selectable because every row this
//! gear authors carries `base` and
//! [`PriceOverlay`](crate::domain::scope_key::PriceOverlay) has no second variant
//! — offering it would offer a choice the authoring plane does not have, which is
//! the same argument [`ScopeKey::new`](crate::domain::scope_key::ScopeKey::new)
//! makes for leaving it out of its own parameters.
//!
//! # `None` is *unconstrained*, and that is not the same word as `Cohort::None`
//!
//! Every axis is an `Option`, and the `Option`'s `None` means **the run does not
//! constrain this axis**. On the cohort axis that sits next to a domain value
//! spelled `None` as well, and the two are different facts: `cohort: None` selects
//! rows of every generation *and* rows of none, while `cohort: Some(Cohort::None)`
//! would select only rows that retain nobody.
//!
//! The wire surface can express only the first, because it spells the axis the way
//! [`ScopeKeyRequest`](crate::api::rest::prices::ScopeKeyRequest) does — an
//! optional instant — and `null` there is already spent on "unconstrained". That
//! costs nothing: `check_cohort_eligibility` makes `cohort != none` **if and only
//! if** `priceEligibility == existing_grandfathered`, so "the rows that retain
//! nobody" is exactly `price_eligibility != existing_grandfathered` and is
//! reachable through the eligibility axis. The type is still built to carry the
//! distinction, because the store's column does.
//!
//! # Grandfathered rows are excluded **structurally**, and this type is where
//!
//! `inst-mp-grandfathered` has two clauses. The first — *"repricing selectors
//! structurally exclude `existing_grandfathered` rows"* — is decidable from the
//! selector alone and is enforced here, through [`RunSelector::admits_grandfathered`]:
//! a selector that does not name the eligibility axis excludes that class, because
//! a grandfathered row is immutable in price (Foundation §4.3) and a run that
//! quietly repriced one would break a promise made to a subscriber.
//!
//! The second clause — *"an explicit attempt to include one fails **that row**
//! with a per-row validation error, never a silent skip"* — is deliberately **not**
//! enforced by dropping those rows from the expansion. Dropping them is precisely
//! the silent skip the clause forbids. So a selector that names
//! `existing_grandfathered` outright still expands over them, they are frozen into
//! the journal like every other selected row, and the per-row refusal is owed by
//! the **apply**, which is the only place a journal row can be moved to `failed`
//! (the table's insert trigger refuses any birth state but `pending`, D-261). That
//! debt is named in [`crate::api::rest::repricing_runs`] rather than left for a
//! reader to infer.

use toolkit_macros::domain_model;

use crate::domain::money::{CurrencyCode, MinorAmount, RateMinor};
use crate::domain::overlay::{Adjustment, Magnitude};
use crate::domain::price_record::PriceRecord;
use crate::domain::price_row::ModelKind;
use crate::domain::scope_key::{
    ChargeKind, Cohort, DimensionKey, Meter, PhaseId, PlanId, PriceEligibility, Region,
};

/// §5's refusal for a run whose selector matched nothing (architectural 422,
/// rendered 400 — Foundation §3.3).
///
/// Declared by the design set and referenced here rather than minted. It is the
/// run's one *pre-commit* refusal, and it exists because the alternative is worse
/// than an error: a run with an empty row set is `completed` the instant it opens
/// — its completion predicate is *no `pending` rows remain* and there are none —
/// so an operator who mistyped a region would be told their mass adjustment
/// **succeeded**.
pub const RUN_SELECTOR_EMPTY: &str = "RUN_SELECTOR_EMPTY";

/// Which published rows a run acts on: any subset of the canonical key's axes.
///
/// Every field absent selects the tenant's whole published catalog minus the
/// grandfathered class. That is a legal run — *"reprice everything"* is a real
/// operator act and §5 declares no refusal for it — and it is the reason
/// [`RunSelector::is_unconstrained`] exists: a surface that wants to report how
/// wide a run reaches can ask, without this type deciding on the design set's
/// behalf that wide is wrong.
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunSelector {
    /// Axis 1. The one D-134 groups the apply's transactions by.
    pub plan_id: Option<PlanId>,
    /// Axis 2 — §2's *"a currency segment"*, spelled as one axis.
    pub currency: Option<CurrencyCode>,
    /// Axis 3.
    pub region: Option<Region>,
    /// Axis 5 (axis 4, `priceOverlay`, is not selectable — see the module doc).
    pub phase: Option<PhaseId>,
    /// Axis 6. Absent excludes `existing_grandfathered`; see
    /// [`RunSelector::admits_grandfathered`].
    pub price_eligibility: Option<PriceEligibility>,
    /// Axis 7.
    pub charge_kind: Option<ChargeKind>,
    /// Axis 8. `None` is *unconstrained*, which is not [`Cohort::None`].
    pub cohort: Option<Cohort>,
    /// Axis 9 (D-196's usage line, first half).
    pub meter: Option<Meter>,
    /// Axis 10. `Some(DimensionKey::none())` selects the undimensioned rows,
    /// whose column holds the empty-tuple sentinel rather than a `NULL`.
    pub dimension_key: Option<DimensionKey>,
}

impl RunSelector {
    /// Does this selector name no axis at all?
    ///
    /// A caller's question, not a rule: a run over the whole published catalog is
    /// legal (see the type doc), and what a surface does with the answer — log it,
    /// echo it on the run's report — is the surface's call.
    #[must_use]
    pub const fn is_unconstrained(&self) -> bool {
        self.plan_id.is_none()
            && self.currency.is_none()
            && self.region.is_none()
            && self.phase.is_none()
            && self.price_eligibility.is_none()
            && self.charge_kind.is_none()
            && self.cohort.is_none()
            && self.meter.is_none()
            && self.dimension_key.is_none()
    }

    /// Does this selector reach the `existing_grandfathered` class?
    ///
    /// `inst-mp-grandfathered` clause 1, and the whole of it: **only** a selector
    /// that names the eligibility axis as `existing_grandfathered` does. An absent
    /// eligibility axis excludes the class rather than including it with the
    /// others, which is the one place in this type where an unconstrained axis is
    /// not the same as "every value".
    ///
    /// It is asymmetric on purpose. Every other axis left absent widens the run;
    /// this one left absent narrows it, because the class it would otherwise add
    /// is immutable in price (Foundation §4.3) and an operator repricing "all EUR
    /// rows" has not asked to break a retention promise. Naming the class is how
    /// they ask — and what they then get is the *per-row* refusal the apply owes,
    /// never a set that silently shrank.
    #[must_use]
    pub const fn admits_grandfathered(&self) -> bool {
        matches!(
            self.price_eligibility,
            Some(PriceEligibility::ExistingGrandfathered)
        )
    }
}

/// One row, with `adjustment` applied to whichever field holds its money.
///
/// **Two callers, one arithmetic.** [`crate::api::rest::repricing_runs::run_materiality`]
/// projects a row's amount in memory, for the comparison a reviewer is being
/// asked to approve — never persisted. [`crate::infra::repricing::apply_run_in`]
/// projects the identical row the identical way to build the successor content
/// it actually writes. A run's materiality verdict and its apply must agree on
/// what the adjustment *does* to a row, so the arithmetic has exactly one
/// owner rather than two copies an edit could drift apart — moved here,
/// `pub(crate)`, the moment the second caller appeared.
///
/// `markup`/`discount` with a `percent_bp` magnitude is currency-neutral and
/// always resolves: `base ± round_half_even(base*bp, 10_000)`. **Not
/// truncating**, and not a decision invented for this function: the crate
/// denies `clippy::integer_division` (workspace `Cargo.toml`), and while
/// `materiality::delta::AmountMove::reaches_percent` sidesteps that by
/// cross-multiplying — it only ever answers a comparison, never a value — this
/// function has to produce an actual minor-unit amount, and no comparison
/// avoids that. `bss_ledger::domain::money_math::round_half_even` is this
/// crate family's own precedent for exactly that shape (half-to-even,
/// `div_euclid`/`rem_euclid` rather than `/`, so the lint that flagged
/// truncation here never fires on it either); it is `pub(crate)` to that gear
/// and not importable, so [`round_half_even`] below reproduces the identical
/// algorithm rather than a different one invented in place. **The direction is
/// symmetric and the tie-break is not always visible**: half-to-even rounds a
/// `.5` remainder to whichever neighbour is even, so two adjustments of equal
/// distance from a whole minor unit can round in opposite directions — a
/// markup and a discount of the same `bp` are therefore not exact mirrors of
/// one another at the boundary, which is arithmetic rather than an
/// inconsistency. A `markup`/`discount` with an `amount` magnitude, or a
/// `fixed` line, needs `currency`'s entry in the adjustment's `AmountSet`.
///
/// A result that would go negative is **floored at zero**: a price cannot go
/// negative, and a floor is itself a real, large move a threshold should see —
/// not the absence of one.
///
/// # `flat`/`per_unit`/`package`'s one field that cannot signal "not computable"
///
/// `flat`/`per_unit`'s `amount_minor` and `package`'s `package_price_minor` are
/// both `Option<MinorAmount>`, so a currency the adjustment cannot resolve is
/// left `None` — [`crate::domain::materiality::delta::row_delta`] already reads
/// an absent amount as `NotComputable("amount_minor" | "package_price_minor")`,
/// D-115 clause (3)'s existing "no delta, so material regardless of threshold"
/// fallback, not a new rule minted here.
///
/// # `graduated`/`volume`'s bands go through [`project_rate`], not [`project_amount`]
///
/// `TierBand::unit_price_rate` is a `RateMinor` (D-311, 2026-08-11) — a
/// multiplier at 10⁻⁹-minor-unit precision, not a `MinorAmount` — because a
/// metered rate routinely prices below the currency's minor unit (S3 at
/// `$0.023`/GB, Lambda at `$0.0000166667`/GB-second) and a `MinorAmount`
/// cannot express that at all. Wrapping [`project_amount`]'s own result in a
/// `RateMinor` would reintroduce the exact defect D-311 removed: `0.0150` and
/// `0.0110` both truncate to the same minor unit, so two different ladders
/// would land on one after a reprice. [`project_rate`] applies the identical
/// percentage formula at the rate's own scale instead, so a sub-minor-unit
/// rate stays sub-minor-unit; see its own doc for the amount/fixed magnitudes,
/// which it refuses to apply to a rate at all rather than guessing what they
/// would mean there. A band [`project_rate`] refuses is left at its
/// **published** rate, on the same "not this projection's job to decide"
/// reading, and [`crate::infra::repricing::apply_rows_in`] is the one place
/// that silence becomes a refusal instead — see [`adjusts_rate`].
pub(crate) fn project_row(mut row: PriceRecord, adjustment: &Adjustment) -> PriceRecord {
    let currency = row.scope_key.currency().clone();
    match row.row.model_kind {
        Some(ModelKind::Flat | ModelKind::PerUnit) => {
            row.row.amount_minor = row
                .row
                .amount_minor
                .and_then(|amount| project_amount(amount.get(), adjustment, &currency));
        }
        Some(ModelKind::Package) => {
            row.row.package_price_minor = row
                .row
                .package_price_minor
                .and_then(|amount| project_amount(amount.get(), adjustment, &currency));
        }
        Some(ModelKind::Graduated | ModelKind::Volume) => {
            for band in &mut row.row.bands {
                if let Some(projected) = project_rate(band.unit_price_rate, adjustment) {
                    band.unit_price_rate = projected;
                }
            }
        }
        None => {}
    }
    row
}

/// One tier band's rate under `adjustment`, at the rate's own 10⁻⁹-minor-unit
/// scale (D-311) — [`project_row`]'s band arithmetic, and deliberately **not**
/// a call to [`project_amount`].
///
/// # Why only a percentage computes
///
/// `markup`/`discount` with a `percent_bp` magnitude is scale-invariant:
/// `round_half_even(rate * bp, 10_000)` is the identical formula
/// [`project_amount`] applies at the minor-unit scale, unchanged, at the
/// rate's finer nano-minor scale — so a sub-minor-unit rate, the entire
/// reason [`RateMinor`] exists, stays sub-minor-unit through a reprice rather
/// than being rounded through a coarser unit on its way to the successor.
///
/// # Why a currency amount does not — a decision, not a gap
///
/// `markup`/`discount` with an `amount` magnitude, and a `fixed` line, are
/// both defined in **currency minor units**, and neither is applied to a
/// rate here, computable currency or not:
///
/// * `fixed` sets every band it touches to **one** literal value. Applied to
///   a ladder that would collapse every band to the identical rate, which
///   defeats the reason the row has bands at all — not a rounding question,
///   a shape question, and the answer is refuse rather than flatten a tiered
///   row into a `flat` one under a `graduated` label.
/// * `amount` markup/discount is a flat minor-unit delta. A rate has no
///   minor-unit floor to receive it against, and "the same delta on every
///   band" is only one of several policies a flat currency shift could mean
///   for a ladder (proportional to the band index is another) — a choice
///   this mechanism has no basis to make silently on an operator's behalf.
///
/// Both arms return `None` unconditionally. [`project_row`] reads that as
/// "leave this band at its published rate" — the same reading an
/// unresolvable currency already gets on `flat`/`per_unit`/`package` — and
/// [`crate::infra::repricing::apply_rows_in`] is where that silence becomes
/// an actual refusal of the row, via [`adjusts_rate`], because an apply that
/// commits a successor identical to its predecessor while marking the row
/// `applied` is the coercion this function declines to perform quietly.
fn project_rate(base: RateMinor, adjustment: &Adjustment) -> Option<RateMinor> {
    let nano = base.nano_minor();
    let projected: i128 = match adjustment {
        Adjustment::Markup(Magnitude::PercentBp(bp)) => {
            i128::from(nano) + round_half_even(i128::from(nano) * i128::from(*bp), 10_000)
        }
        Adjustment::Discount(Magnitude::PercentBp(bp)) => {
            i128::from(nano) - round_half_even(i128::from(nano) * i128::from(*bp), 10_000)
        }
        Adjustment::Markup(Magnitude::Amount(_))
        | Adjustment::Discount(Magnitude::Amount(_))
        | Adjustment::Fixed(_) => return None,
    };
    i64::try_from(projected.max(0))
        .ok()
        .and_then(|nano| RateMinor::from_nano_minor(nano).ok())
}

/// Does `adjustment` compute a real rate mutation for a `graduated`/`volume`
/// row's bands (D-311), or does every band stay at its published rate under
/// it ([`project_rate`])?
///
/// `pub(crate)` for [`crate::infra::repricing::apply_rows_in`]. Materiality's
/// own projection (`api::rest::repricing_runs::run_materiality`) is content
/// to read a [`project_rate`] `None` as a silent zero-delta on that band —
/// the same "no delta, so material regardless of threshold" fallback D-115
/// clause (3) already gives an unresolvable currency, an accepted, named gap
/// for a reviewer's comparison. The apply is not a projection for a reviewer
/// to weigh; it is the write, and a row this mechanism cannot honestly
/// reprice must refuse rather than commit a successor identical to its
/// predecessor while the journal reports the row `applied`.
#[must_use]
pub(crate) const fn adjusts_rate(adjustment: &Adjustment) -> bool {
    matches!(
        adjustment,
        Adjustment::Markup(Magnitude::PercentBp(_)) | Adjustment::Discount(Magnitude::PercentBp(_))
    )
}

/// `base`'s minor-unit amount under `adjustment`, in `currency` — [`project_row`]'s
/// arithmetic, factored out because every model kind applies the identical
/// formula to a different field.
///
/// `None` when an `amount`/`fixed` magnitude has no entry for `currency`; see
/// [`project_row`]'s doc for what a caller does with that.
///
/// Widens to `i128` for the `percent_bp` arms' multiply-then-round, the same
/// guard [`round_half_even`] itself documents: `base` and `bp` are each
/// bounded by `i64`, and their product is not.
fn project_amount(
    base: i64,
    adjustment: &Adjustment,
    currency: &CurrencyCode,
) -> Option<MinorAmount> {
    let projected: i128 = match adjustment {
        Adjustment::Markup(Magnitude::PercentBp(bp)) => {
            i128::from(base) + round_half_even(i128::from(base) * i128::from(*bp), 10_000)
        }
        Adjustment::Discount(Magnitude::PercentBp(bp)) => {
            i128::from(base) - round_half_even(i128::from(base) * i128::from(*bp), 10_000)
        }
        Adjustment::Markup(Magnitude::Amount(set)) => {
            i128::from(base) + i128::from(set.get(currency)?)
        }
        Adjustment::Discount(Magnitude::Amount(set)) => {
            i128::from(base) - i128::from(set.get(currency)?)
        }
        Adjustment::Fixed(set) => i128::from(set.get(currency)?),
    };
    i64::try_from(projected.max(0))
        .ok()
        .and_then(|minor| MinorAmount::new(minor).ok())
}

/// Round `numer/denom` (`denom` must be positive) to the nearest integer, ties
/// to even — half-to-even, "banker's rounding".
///
/// Reproduces `bss_ledger::domain::money_math::round_half_even` rather than
/// importing it (that function is `pub(crate)` to a different gear): this
/// crate denies `clippy::integer_division`, and `div_euclid`/`rem_euclid` are
/// method calls rather than the `/` operator the lint matches, which is what
/// lets this stay exact about ties without an `#[allow]` on money arithmetic.
/// `i128` throughout because [`project_amount`]'s `numer` is already a product
/// of two `i64`s.
fn round_half_even(numer: i128, denom: i128) -> i128 {
    debug_assert!(denom > 0, "denominator must be positive");
    let q = numer.div_euclid(denom);
    let r = numer.rem_euclid(denom); // 0 <= r < denom
    // Compare `r` to `denom - r` rather than `2 * r` to `denom`, so a large
    // remainder cannot overflow — `bss_ledger`'s own reason for the same
    // comparison, reproduced along with the algorithm it guards.
    let complement = denom - r;
    if r < complement {
        q
    } else if r > complement {
        q + 1
    } else if q % 2 == 0 {
        // exact half: round to even
        q
    } else {
        q + 1
    }
}

#[cfg(test)]
#[path = "repricing_tests.rs"]
mod repricing_tests;
