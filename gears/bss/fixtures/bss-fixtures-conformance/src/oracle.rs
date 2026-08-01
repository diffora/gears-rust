//! The reference implementation of the pricing `PRD.md` 17.2 formula matrix.
//!
//! This exists so "green" can be earned before the rating gear exists: it stands
//! in for Tariffs. When rating's evaluator lands it must reproduce the same
//! corpus; disagreement reddens the corpus rather than overriding either side.
//!
//! Kept deliberately small and readable - it is audited by reading, and it is
//! the only thing standing between the corpus and a shared misreading of 17.2.

use crate::traits::{CorpusEvaluator, EvalError, EvalInput, Evaluated};
use bss_fixtures::{
    AggregationFunction, Band, BandTop, Family, ModelKind, ProrationBasis, ReservationFlavor,
    Runtime, Snapshot,
};

pub struct ReferenceOracle;

impl CorpusEvaluator for ReferenceOracle {
    fn evaluate(&self, input: &EvalInput<'_>) -> Result<Evaluated, EvalError> {
        let snap = input.snapshot;

        // Gauge samples mean the case is asking for a granule fold.
        if !input.runtime.samples.is_empty() {
            return fold(snap, input.runtime);
        }

        // A period in the given means the case is asking about apportionment,
        // not price. The two questions never mix in one assertion.
        if input.given.period_start.is_some() {
            let basis = snap
                .proration_basis
                .ok_or(EvalError::MissingField("proration_basis"))?;
            return prorate(basis, input.given);
        }

        // A reservation reshapes what the bands see, so it resolves before them.
        if let Some(flavor) = snap.reservation_flavor {
            return reservation(snap, input.runtime, input.given.q, flavor);
        }

        let q = input.given.q;

        let charge = match snap.model_kind {
            ModelKind::Graduated => graduated(&snap.bands, q)?,
            ModelKind::Volume => volume(&snap.bands, q)?,
            ModelKind::Package => {
                let size = snap
                    .package_size
                    .ok_or(EvalError::MissingField("package_size"))?;
                let price = snap
                    .package_price_minor
                    .ok_or(EvalError::MissingField("package_price_minor"))?;
                package(size, price, q)?
            }
            ModelKind::PerUnit => {
                let unit = snap
                    .amount_minor
                    .ok_or(EvalError::MissingField("amount_minor"))?;
                quantity(q)? * unit
            }
            // The quantity is deliberately unread: a flat row is owed in full
            // whatever was consumed. Multiplying here would silently turn it
            // into `per_unit`.
            ModelKind::Flat => snap
                .amount_minor
                .ok_or(EvalError::MissingField("amount_minor"))?,
        };

        Ok(Evaluated::Charge(charge))
    }

    fn supported_families(&self) -> Vec<Family> {
        vec![
            Family::TierBoundary,
            Family::Package,
            Family::PerUnit,
            Family::Flat,
            Family::Proration,
            Family::SupersessionContinuity,
            Family::LevelAggregation,
            Family::Reserved,
        ]
    }
}

/// Widens a quantity into the money domain, refusing rather than wrapping.
fn quantity(q: u64) -> Result<i64, EvalError> {
    i64::try_from(q).map_err(|_| EvalError::QuantityOutOfRange(q))
}

/// Marginal: each band prices only the units falling inside it.
///
/// # Errors
///
/// Returns [`EvalError::QuantityOutOfRange`] if a band's unit count exceeds the
/// money domain.
pub(crate) fn graduated(bands: &[Band], q: u64) -> Result<i64, EvalError> {
    let mut total: i64 = 0;
    for b in bands {
        let top = match b.to_qty {
            BandTop::Open => q,
            BandTop::Closed(t) => t.min(q),
        };
        if top <= b.from_qty {
            continue;
        }
        total += quantity(top - b.from_qty)? * b.unit_amount_minor;
    }
    Ok(total)
}

/// Variant A: the band holding `q` sets one rate for the whole quantity.
/// Variant B (a per-tier flat fee) is not authorable in the catalog.
///
/// # Errors
///
/// Returns [`EvalError::NoBandCoversQuantity`] if no band covers `q`, rather
/// than inventing a rate.
pub(crate) fn volume(bands: &[Band], q: u64) -> Result<i64, EvalError> {
    let band = bands
        .iter()
        .find(|b| {
            q >= b.from_qty
                && match b.to_qty {
                    BandTop::Open => true,
                    BandTop::Closed(t) => q < t,
                }
        })
        .ok_or(EvalError::NoBandCoversQuantity(q))?;

    Ok(quantity(q)? * band.unit_amount_minor)
}

/// Repeating block: `blocks = ceil(used / packageSize)`, charge = blocks x price.
/// Distinct from Volume Variant B (a per-tier flat fee), which is not authorable.
///
/// # Errors
///
/// Returns [`EvalError::ZeroPackageSize`] for a zero size, which would otherwise
/// panic in `div_ceil`.
pub(crate) fn package(size: u64, price_minor: i64, q: u64) -> Result<i64, EvalError> {
    if size == 0 {
        return Err(EvalError::ZeroPackageSize);
    }
    Ok(quantity(q.div_ceil(size))? * price_minor)
}

/// `Q` = Σ granule folds (D-44 / rating T-D-17).
///
/// The window is cut into granules of the row's `aggregation_granularity`.
/// `peak` folds each granule to its maximum sample; `time_weighted` folds it to
/// the step-integral of the level, with the last level held across granule
/// boundaries for up to `max_hold_granules`. Beyond the hold the level reads 0 —
/// deliberately the customer-favourable floor, and provisional, because a
/// backfilled sample re-folds the affected granules.
///
/// The integral accumulates in level-seconds and converts to the billable unit
/// only when the division is exact. Rounding is Billing's, not this seam's, so a
/// fold that does not land on a whole unit is refused rather than rounded.
///
/// # Errors
///
/// Returns [`EvalError`] if the window or the level triple is absent, or if a
/// fold does not divide exactly into the billable unit.
fn fold(snap: &Snapshot, rt: &Runtime) -> Result<Evaluated, EvalError> {
    let granularity = snap
        .aggregation_granularity
        .ok_or(EvalError::MissingField("aggregation_granularity"))?;
    let max_hold = snap
        .max_hold_granules
        .ok_or(EvalError::MissingField("max_hold_granules"))?;
    let function = snap
        .aggregation_function
        .ok_or(EvalError::MissingField("aggregation_function"))?;
    let window_start = rt
        .window_start
        .ok_or(EvalError::MissingGiven("window_start"))?;
    let window_end = rt.window_end.ok_or(EvalError::MissingGiven("window_end"))?;

    if window_end <= window_start {
        return Err(EvalError::DegeneratePeriod);
    }

    let step = granularity.seconds();
    let span = seconds(window_start, window_end);
    let granule_count = span.div_euclid(step);

    let mut samples = rt.samples.clone();
    samples.sort_by_key(|s| s.at);

    let mut total: u64 = 0;
    for g in 0..granule_count {
        let g_start =
            window_start + chrono::Duration::seconds(i64::try_from(g * step).unwrap_or(0));
        let g_end = g_start + chrono::Duration::seconds(i64::try_from(step).unwrap_or(0));

        let in_granule: Vec<_> = samples
            .iter()
            .filter(|s| s.at >= g_start && s.at < g_end)
            .collect();

        // The level carried in from earlier, if the hold still reaches here.
        let carried = held_level(&samples, window_start, g, step, max_hold);

        total += match function {
            // The peak of the *level* over the granule, not of the samples in
            // it. The level carried in from an earlier sample was really the
            // level until the first in-granule observation, so it competes.
            //
            // The doc sentence "peak = max sample in the granule" is shorthand.
            // Read literally it is gameable: a level held at 100 all hour folds
            // to 100 while unsampled, but to 1 the moment a single sample of 1
            // arrives - adding an observation would cut the bill a hundredfold.
            // It would also split `peak` from `time_weighted`, which integrates
            // the same level and does honour the hold.
            AggregationFunction::Peak => in_granule
                .iter()
                .map(|s| s.level)
                .chain(carried)
                .max()
                .unwrap_or(0),
            AggregationFunction::TimeWeighted => {
                integral(&in_granule, carried, g_start, g_end, step)?
            }
            // `sum` is not a fold; a sum row never reaches here.
            AggregationFunction::Sum => {
                return Err(EvalError::SumIsNotAFold);
            }
        };
    }

    Ok(Evaluated::Fold { q: total })
}

/// The level held into a granule from the most recent earlier sample, if that
/// sample is within `max_hold` granules. Beyond the hold the level reads 0.
fn held_level(
    samples: &[bss_fixtures::GaugeSample],
    window_start: chrono::DateTime<chrono::Utc>,
    g_index: u64,
    step: u64,
    max_hold: u64,
) -> Option<u64> {
    let g_start = window_start + chrono::Duration::seconds(i64::try_from(g_index * step).ok()?);
    let last = samples.iter().rfind(|s| s.at < g_start)?;
    // Distance in *granules*, not seconds: a sample late in the previous granule
    // is one granule away, however few seconds separate it from the boundary.
    let last_index = seconds(window_start, last.at).div_euclid(step);
    if g_index.saturating_sub(last_index) <= max_hold {
        Some(last.level)
    } else {
        None
    }
}

/// Step-integral of the level over one granule, in the billable unit.
fn integral(
    in_granule: &[&bss_fixtures::GaugeSample],
    carried: Option<u64>,
    g_start: chrono::DateTime<chrono::Utc>,
    g_end: chrono::DateTime<chrono::Utc>,
    step: u64,
) -> Result<u64, EvalError> {
    let mut level_seconds: u64 = 0;
    let mut cursor = g_start;
    let mut level = carried.unwrap_or(0);

    for s in in_granule {
        level_seconds += level * seconds(cursor, s.at);
        cursor = s.at;
        level = s.level;
    }
    level_seconds += level * seconds(cursor, g_end);

    if level_seconds.is_multiple_of(step) {
        Ok(level_seconds.div_euclid(step))
    } else {
        Err(EvalError::NonIntegralFold {
            level_seconds,
            step,
        })
    }
}

/// Reservation pricing (`inst-rv-tier-q`, `inst-rv-level`, D-53, D-139).
///
/// `capacity` bills the allocation whatever the usage and never touches `Q`;
/// under D-139 the charge accrues per covered granule, because `reservedRate`
/// is denominated in the row's billable unit and is therefore money per
/// granule rather than a period charge.
///
/// `consumption` prices the matched quantity at the reserved rate and bands the
/// **remainder from zero** — the matched quantity leaves the tier counter
/// entirely rather than offsetting it.
///
/// # Errors
///
/// Returns [`EvalError`] when the rate, the allocation or the coverage is
/// absent.
fn reservation(
    snap: &Snapshot,
    rt: &Runtime,
    q: u64,
    flavor: ReservationFlavor,
) -> Result<Evaluated, EvalError> {
    let rate = snap
        .reserved_rate_minor
        .ok_or(EvalError::MissingField("reserved_rate_minor"))?;
    let reserved = rt
        .reserved_quantity
        .ok_or(EvalError::MissingGiven("reserved_quantity"))?;

    let charge = match flavor {
        ReservationFlavor::Capacity => {
            let granules = rt
                .covered_granules
                .ok_or(EvalError::MissingGiven("covered_granules"))?;
            // The quantity is deliberately unread: capacity is never reduced by
            // absent usage.
            quantity(reserved)? * rate * quantity(granules)?
        }
        ReservationFlavor::Consumption => {
            let matched = q.min(reserved);
            let remainder = q - matched;
            quantity(matched)? * rate + graduated(&snap.bands, remainder)?
        }
    };

    Ok(Evaluated::Charge(charge))
}

const SECONDS_PER_DAY: u64 = 86_400;

/// Whole seconds between two instants; never negative.
fn seconds(a: chrono::DateTime<chrono::Utc>, b: chrono::DateTime<chrono::Utc>) -> u64 {
    u64::try_from((b - a).num_seconds().max(0)).unwrap_or(0)
}

/// The chargeable share of a period, as an exact integer ratio.
///
/// Value glosses are the pricing glossary's, verbatim: `calendar_days_30` is a
/// fixed 30-day month with the day count capped at 30; `whole_unit` performs no
/// sub-period proration; `none` means no proration at all — full-period charge.
///
/// Deliberately returns no money. Rating emits prorated components at full
/// intermediate precision and never rounds; Billing rounds. A prorated minor
/// amount does not exist at this seam.
///
/// # Errors
///
/// Returns [`EvalError::MissingGiven`] if the period is not fully specified, or
/// [`EvalError::DegeneratePeriod`] if it is empty or inverted.
fn prorate(basis: ProrationBasis, given: &bss_fixtures::Given) -> Result<Evaluated, EvalError> {
    let period_start = given
        .period_start
        .ok_or(EvalError::MissingGiven("period_start"))?;
    let period_end = given
        .period_end
        .ok_or(EvalError::MissingGiven("period_end"))?;
    // The chargeable stretch defaults to the whole period.
    let from = given.from.unwrap_or(period_start);
    let to = given.to.unwrap_or(period_end);

    if period_end <= period_start || to < from {
        return Err(EvalError::DegeneratePeriod);
    }

    let days = |a: chrono::DateTime<chrono::Utc>, b: chrono::DateTime<chrono::Utc>| -> u64 {
        seconds(a, b).div_euclid(SECONDS_PER_DAY)
    };

    Ok(match basis {
        ProrationBasis::CalendarDaysActual => Evaluated::Units {
            charged: days(from, to),
            in_basis: days(period_start, period_end),
        },
        // Fixed 30-day month; the day count is capped at 30 so a 31-day month
        // never bills 31/30 of itself.
        ProrationBasis::CalendarDays30 => Evaluated::Units {
            charged: days(from, to).min(30),
            in_basis: 30,
        },
        ProrationBasis::BySecond => Evaluated::Units {
            charged: seconds(from, to),
            in_basis: seconds(period_start, period_end),
        },
        // Neither apportions: the whole period is charged whatever the stretch.
        ProrationBasis::WholeUnit | ProrationBasis::None => Evaluated::Units {
            charged: 1,
            in_basis: 1,
        },
    })
}

#[cfg(test)]
#[path = "oracle_tests.rs"]
mod oracle_tests;
