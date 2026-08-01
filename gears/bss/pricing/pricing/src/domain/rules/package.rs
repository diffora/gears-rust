//! Package (block) pricing rules (`cpt-cf-bss-pricing-algo-package`).
//!
//! A `package` row prices in whole blocks: `blocks = ceil(used / package_size)`,
//! `charge = blocks * package_price_minor`. The catalog authors the three fields
//! and computes none of that — but the fields have to be structurally sound and
//! the accumulation window has to exist, because block math is **non-linear in
//! the window**.

use bss_fixtures::ModelKind;
use toolkit_macros::domain_model;

use crate::domain::price_row::PriceRow;
use crate::domain::rules::{EVAL_POLICY_MISSING, PACKAGE_FIELDS_INVALID};
use crate::domain::validation::{ValidationReport, ValidationRule};

/// `inst-pk-fields` — block fields in range, and structurally exclusive with
/// tier bands.
///
/// `package_size > 0` because a block of zero units is a division by zero
/// downstream, and a row that carries both bands and block fields states two
/// incompatible formulas for one price — Tariffs would have to choose.
///
/// `package_price_minor >= 0` is guaranteed by construction: the amount type
/// refuses a negative, so there is nothing left for this rule to check. Presence
/// of the two fields on a `package` row is `inst-mk-required`'s (it owns the
/// per-kind required set); this rule judges the values and the exclusivity.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct PackageFields;

impl ValidationRule<PriceRow> for PackageFields {
    fn name(&self) -> &'static str {
        "inst-pk-fields"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        let Some(kind) = subject.model_kind else {
            return;
        };
        if !matches!(kind, ModelKind::Package) {
            if subject.package_size.is_some() || subject.package_price_minor.is_some() {
                report.violate(
                    PACKAGE_FIELDS_INVALID,
                    subject.subject(),
                    "package_size / package_price_minor are authorable only on a package row",
                );
            }
            return;
        }
        if subject.package_size == Some(0) {
            report.violate(
                PACKAGE_FIELDS_INVALID,
                subject.subject(),
                "package_size must be greater than 0: a block of no units prices nothing",
            );
        }
        if !subject.bands.is_empty() {
            report.violate(
                PACKAGE_FIELDS_INVALID,
                subject.subject(),
                format!(
                    "a package row carries {} tier band(s): block pricing and tier bands are \
                     structurally exclusive, and a row holding both states two formulas",
                    subject.bands.len()
                ),
            );
        }
    }
}

/// `inst-pk-window` — a `package` row must carry `tierAggregationWindow` (D-58).
///
/// Not tier bookkeeping borrowed for a kind that has no tiers: the window is
/// what `used` accumulates over **before** the block round-up. 150 units in a
/// month folds to `ceil(150/100) = 2` blocks under `invoice_period` and to 30
/// blocks under a daily fold — a fifteen-fold spread on the same published row.
///
/// `billingGranularity` does not supply it. It quantizes the quantity; it does
/// not bound a period — the same asymmetry the PRD already resolves for
/// `volume`, whose single rate applies to the total `Q` *within* the window.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct PackageWindow;

impl ValidationRule<PriceRow> for PackageWindow {
    fn name(&self) -> &'static str {
        "inst-pk-window"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        if subject.model_kind != Some(ModelKind::Package) || !subject.is_usage() {
            return;
        }
        if subject.tier_aggregation_window.is_none() {
            report.violate(
                EVAL_POLICY_MISSING,
                subject.subject(),
                "a package row must carry tierAggregationWindow \
                 (calendar_month | invoice_period | subscription_lifetime | per_event): \
                 it is the window `used` accumulates over before the block round-up, and \
                 billingGranularity does not bound a period",
            );
        }
    }
}

#[cfg(test)]
#[path = "package_tests.rs"]
mod package_tests;
