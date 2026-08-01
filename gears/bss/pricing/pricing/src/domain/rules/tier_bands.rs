//! Tier-band rules (`cpt-cf-bss-pricing-algo-tier-bands`).
//!
//! The band set is judged as a **geometry** under the half-open `[from, to)`
//! convention: ascending, non-overlapping, contiguous, no zero-width band, and
//! an always-open top. Every quantity an operator can sell must land in exactly
//! one band — "sold but unrateable" is the failure mode all of it exists to make
//! impossible.

use toolkit_macros::domain_model;

use crate::domain::price_row::{BandTop, PriceRow, TierBand};
use crate::domain::rules::{
    EVAL_POLICY_MISSING, TIER_BAND_EMPTY, TIER_BAND_PRICE_INCREASE, TIER_BANDS_GAP,
    TIER_BANDS_OVERLAP, TIER_TOP_CLOSED,
};
use crate::domain::validation::{ValidationReport, ValidationRule};

/// `inst-tb-order` — ordering, overlap, contiguity, zero-width bands, and the
/// non-volume-discount advisory.
///
/// Runs over whatever band set the row carries, whatever its kind: bands on a
/// kind that may not have them are a *placement* fault (`inst-mk-forbidden` /
/// `inst-pk-fields`), but a malformed set is still malformed, and reporting its
/// geometry in the same pass is what lets the author fix both at once.
///
/// Geometry is judged over the band set **sorted by `from_qty`**, not over the
/// order the author happened to write. The persisted band table is keyed
/// `(price_id, from_qty)` and carries no ordinal, so authoring order does not
/// survive a round-trip through the store: judging on it would let one row
/// validate at save and fail the identical re-validation at publish, which is
/// the one thing the pre-check and the commit-time re-run must never do.
///
/// The ascending requirement then needs no separate check: under `[from, to)` a
/// band that starts at or below its predecessor's start also starts below its
/// predecessor's end, which is an overlap.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct BandGeometry;

impl ValidationRule<PriceRow> for BandGeometry {
    fn name(&self) -> &'static str {
        "inst-tb-order"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        for band in &subject.bands {
            if band.is_zero_width() {
                report.violate(
                    TIER_BAND_EMPTY,
                    subject.subject(),
                    format!(
                        "band {band} covers no quantity: to_qty must be strictly above from_qty"
                    ),
                );
            }
        }
        let mut ordered = subject.bands.clone();
        ordered.sort_by_key(|band| band.from_qty);
        for (previous, next) in ordered.iter().zip(ordered.iter().skip(1)) {
            check_adjacency(subject, *previous, *next, report);
            if next.unit_price_minor > previous.unit_price_minor {
                report.warn(
                    TIER_BAND_PRICE_INCREASE,
                    subject.subject(),
                    format!(
                        "band {next} prices a unit at {} against {} in {previous}: the ladder \
                         rises rather than discounts",
                        next.unit_price_minor, previous.unit_price_minor
                    ),
                );
            }
        }
    }
}

/// The two ways a consecutive pair can fail: they share quantity, or they leave
/// one unpriced.
fn check_adjacency(
    row: &PriceRow,
    previous: TierBand,
    next: TierBand,
    report: &mut ValidationReport,
) {
    let Some(top) = previous.to_qty.closed_at() else {
        report.violate(
            TIER_BANDS_OVERLAP,
            row.subject(),
            format!(
                "band {previous} is open-topped but is not the last band; {next} lies inside it"
            ),
        );
        return;
    };
    if next.from_qty < top {
        report.violate(
            TIER_BANDS_OVERLAP,
            row.subject(),
            format!("band {next} starts inside {previous}: quantities are priced twice"),
        );
    } else if next.from_qty > top {
        report.violate(
            TIER_BANDS_GAP,
            row.subject(),
            format!(
                "quantities [{top}, {}) fall between {previous} and {next} and are priced by \
                 neither",
                next.from_qty
            ),
        );
    }
}

/// `inst-tb-first` — the band set starts at the row's quantity origin.
///
/// A tiered row must carry at least one band and the first must start at
/// `from_qty = 0`; anything else leaves `[0, first)` unpriced, which is the same
/// hole `TIER_BANDS_GAP` names between two bands, at the origin. The design set
/// states the rule without naming a code, and this is the one that already means
/// "a quantity no band prices".
///
/// **A `$0` first band is valid.** The rule reads bounds and never price: a free
/// opening band is a normal way to author "N included", and since D-45 the
/// preferred authoring is the first-class `includedAllowance` whose compile
/// projects exactly this shape.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct BandOrigin;

impl ValidationRule<PriceRow> for BandOrigin {
    fn name(&self) -> &'static str {
        "inst-tb-first"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        if !subject.is_tiered() {
            return;
        }
        let Some(first) = subject.bands.first() else {
            report.violate(
                TIER_BANDS_GAP,
                subject.subject(),
                "a graduated / volume row must carry at least one tier band; \
                 with none, no quantity is priced at all",
            );
            return;
        };
        if first.from_qty != 0 {
            report.violate(
                TIER_BANDS_GAP,
                subject.subject(),
                format!(
                    "the first band is {first}: quantities [0, {}) are priced by no band",
                    first.from_qty
                ),
            );
        }
    }
}

/// `inst-tb-top` — the top band is always open (D-17).
///
/// "Price undefined above X" is never the commercial intent. Capping a quantity
/// is an entitlement **quota** that Subscriptions enforces, a per-period fee cap
/// is Tariffs Future scope, and a different price above X is simply another
/// band. So any quantity is always rateable on a tiered row, and a closed top
/// fails publish rather than quietly creating a sellable quantity nothing
/// prices.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct BandTopOpen;

impl ValidationRule<PriceRow> for BandTopOpen {
    fn name(&self) -> &'static str {
        "inst-tb-top"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        let Some(last) = subject.bands.last() else {
            return;
        };
        if let BandTop::Closed(top) = last.to_qty {
            report.violate(
                TIER_TOP_CLOSED,
                subject.subject(),
                format!(
                    "the top band {last} closes at {top}: quantities at or above it would be \
                     sold and unrateable. Capping belongs to entitlement quotas, not to prices"
                ),
            );
        }
    }
}

/// `inst-tb-window` — the usage evaluation policy a row cannot be evaluated
/// without.
///
/// Two requirements under one code, exactly as the design set groups them.
/// **Every** usage row carries `billingGranularity`: it is what one billable
/// unit *is*, and band bounds are expressed in those units. A **tiered** usage
/// row additionally carries `tierAggregationWindow`: it is when the counter
/// resets, and a band set without it prices an unbounded accumulation.
///
/// The `package` case — which needs the same window for a different reason, to
/// bound the accumulation *before* block round-up — is `inst-pk-window`'s, so
/// one missing window is reported once.
///
/// The value sets (`per_second | per_minute | per_hour | per_day | whole_unit`
/// and `calendar_month | invoice_period | subscription_lifetime | per_event`)
/// need no check here: they are the enums, so an unknown value cannot be
/// constructed.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct UsageEvaluationPolicy;

impl ValidationRule<PriceRow> for UsageEvaluationPolicy {
    fn name(&self) -> &'static str {
        "inst-tb-window"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        if !subject.is_usage() {
            return;
        }
        if subject.billing_granularity.is_none() {
            report.violate(
                EVAL_POLICY_MISSING,
                subject.subject(),
                "every usage row must carry billingGranularity \
                 (per_second | per_minute | per_hour | per_day | whole_unit): \
                 it is the unit the bands are counted in",
            );
        }
        if subject.is_tiered() && subject.tier_aggregation_window.is_none() {
            report.violate(
                EVAL_POLICY_MISSING,
                subject.subject(),
                "a tiered usage row must carry tierAggregationWindow \
                 (calendar_month | invoice_period | subscription_lifetime | per_event): \
                 without it the tier counter never resets",
            );
        }
    }
}

#[cfg(test)]
#[path = "tier_bands_tests.rs"]
mod tier_bands_tests;
