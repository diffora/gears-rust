//! Model-kind rules (`cpt-cf-bss-pricing-algo-model-kind`).
//!
//! Four rules: the kind is explicit, its required fields are present, its
//! forbidden fields are absent, and the kind is legal on the row's charge
//! component.
//!
//! The three field rules all start by reading `model_kind` and returning when it
//! is absent. That is deliberate rather than defensive: `inst-mk-explicit` has
//! already recorded `MODEL_KIND_MISSING`, and re-deriving a per-kind field set
//! from a kind nobody authored would fill the report with consequences of the
//! one fault the author has to fix first.

use bss_fixtures::ModelKind;
use toolkit_macros::domain_model;

use crate::domain::price_row::{PriceRow, QuantitySource, model_kind_wire};
use crate::domain::rules::{
    AMOUNT_PLACEMENT_INVALID, EVAL_POLICY_MISPLACED, MODEL_KIND_CHARGEKIND_MISMATCH,
    MODEL_KIND_MISSING, PACKAGE_FIELDS_INVALID, QUANTITY_SOURCE_MISSING,
};
use crate::domain::validation::{ValidationReport, ValidationRule};

/// `inst-mk-explicit` — the kind is authored, never inferred.
///
/// There is no implicit default at rating time, so "tiered (unspecified)" is not
/// a publishable row: a kind the catalog leaves open is a formula Tariffs would
/// have to guess.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct ExplicitModelKind;

impl ValidationRule<PriceRow> for ExplicitModelKind {
    fn name(&self) -> &'static str {
        "inst-mk-explicit"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        if subject.model_kind.is_none() {
            report.violate(
                MODEL_KIND_MISSING,
                subject.subject(),
                "modelKind must be one of flat | per_unit | graduated | volume | package; \
                 no implicit default exists at rating time",
            );
        }
    }
}

/// `inst-mk-required` — the fields a kind cannot be evaluated without.
///
/// Owns the **amount-placement matrix**: `amount_minor` is where `flat` keeps
/// its single amount and where `per_unit` keeps its unit price, and it MUST be
/// NULL on `graduated` / `volume` (money lives in the band column) and on
/// `package` (money lives in `package_price_minor`) — so no row ever carries two
/// competing prices.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct KindRequiredFields;

impl ValidationRule<PriceRow> for KindRequiredFields {
    fn name(&self) -> &'static str {
        "inst-mk-required"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        let Some(kind) = subject.model_kind else {
            return;
        };
        let wire = model_kind_wire(kind);
        check_amount_placement(subject, kind, report);

        if matches!(kind, ModelKind::PerUnit) && !subject.is_usage() {
            check_quantity_source(subject, report);
        }

        if matches!(kind, ModelKind::Package)
            && (subject.package_size.is_none() || subject.package_price_minor.is_none())
        {
            report.violate(
                PACKAGE_FIELDS_INVALID,
                subject.subject(),
                format!(
                    "a {wire} row must carry both package_size and package_price_minor; \
                     the block size and the block price are what it prices with"
                ),
            );
        }
    }
}

/// The per-kind home of the row's money.
fn check_amount_placement(row: &PriceRow, kind: ModelKind, report: &mut ValidationReport) {
    let wire = model_kind_wire(kind);
    let carries_amount = matches!(kind, ModelKind::Flat | ModelKind::PerUnit);
    match (carries_amount, row.amount_minor.is_some()) {
        (true, false) => report.violate(
            AMOUNT_PLACEMENT_INVALID,
            row.subject(),
            format!("a {wire} row keeps its money in amount_minor, and it is absent"),
        ),
        (false, true) => report.violate(
            AMOUNT_PLACEMENT_INVALID,
            row.subject(),
            format!(
                "amount_minor must be NULL on a {wire} row: its money lives in the band or \
                 package column, and two priced columns are two competing prices"
            ),
        ),
        (true, true) | (false, false) => {}
    }
}

/// A non-usage `per_unit` row states where its quantity comes from — and, when
/// it states `manual`, what that quantity is.
fn check_quantity_source(row: &PriceRow, report: &mut ValidationReport) {
    match row.quantity_source {
        None => report.violate(
            QUANTITY_SOURCE_MISSING,
            row.subject(),
            "a non-usage per_unit row must declare quantitySource \
             (subscription_seat_count | manual): nothing meters it",
        ),
        Some(QuantitySource::Manual) if row.manual_quantity.is_none() => report.violate(
            QUANTITY_SOURCE_MISSING,
            row.subject(),
            "quantitySource = manual requires manual_quantity: the fixed quantity is the \
             whole of what `manual` supplies",
        ),
        Some(_) => {}
    }
}

/// `inst-mk-forbidden` — fields on a row whose kind and charge component do not
/// admit them.
///
/// Two families. **Tier bands** are authorable only where the money lives in
/// them (`graduated` / `volume`); on `flat` and `per_unit` they are a second,
/// unreachable price. **Evaluation policy** (`tierAggregationWindow`,
/// `billingGranularity`) and **`quantitySource`** are mutually exclusive answers
/// to "where does `Q` come from": the first two are usage-row-only, and
/// `quantitySource` is legal only on a `per_unit` row that is *not* usage — a
/// `per_unit` usage row takes its quantity from the meter.
///
/// The `package` x bands case is **not** here: it is structural exclusivity
/// between two ways of pricing blocks and it belongs to `inst-pk-fields`, which
/// reports it under `PACKAGE_FIELDS_INVALID`. Reporting it twice under two codes
/// would make one authoring mistake look like two.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct KindForbiddenFields;

impl ValidationRule<PriceRow> for KindForbiddenFields {
    fn name(&self) -> &'static str {
        "inst-mk-forbidden"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        let Some(kind) = subject.model_kind else {
            return;
        };
        let wire = model_kind_wire(kind);

        if matches!(kind, ModelKind::Flat | ModelKind::PerUnit) && !subject.bands.is_empty() {
            report.violate(
                EVAL_POLICY_MISPLACED,
                subject.subject(),
                format!(
                    "tier bands are authorable only on graduated / volume rows; a {wire} row \
                     carrying {} band(s) prices from a column nothing reads",
                    subject.bands.len()
                ),
            );
        }

        if !subject.is_usage() {
            check_usage_only_policy(subject, report);
        }

        check_quantity_source_placement(subject, kind, report);
    }
}

/// `tierAggregationWindow` / `billingGranularity` are usage-row-only: a
/// non-usage row has no metered quantity for them to bound or quantize.
fn check_usage_only_policy(row: &PriceRow, report: &mut ValidationReport) {
    let mut misplaced = Vec::new();
    if row.tier_aggregation_window.is_some() {
        misplaced.push("tierAggregationWindow");
    }
    if row.billing_granularity.is_some() {
        misplaced.push("billingGranularity");
    }
    if misplaced.is_empty() {
        return;
    }
    report.violate(
        EVAL_POLICY_MISPLACED,
        row.subject(),
        format!(
            "{} are usage-row only; this row's chargeKind is {}",
            misplaced.join(" and "),
            row.charge_kind
        ),
    );
}

/// `quantitySource` (and the `manual` quantity that goes with it) belongs to
/// exactly one shape: a `per_unit` row that is not a usage row.
fn check_quantity_source_placement(row: &PriceRow, kind: ModelKind, report: &mut ValidationReport) {
    let authored_source = matches!(kind, ModelKind::PerUnit) && !row.is_usage();
    if row.quantity_source.is_some() && !authored_source {
        report.violate(
            EVAL_POLICY_MISPLACED,
            row.subject(),
            format!(
                "quantitySource is authorable only on a non-usage per_unit row; on a {} {} row \
                 the quantity comes from the meter",
                row.charge_kind,
                model_kind_wire(kind)
            ),
        );
    }
    if row.manual_quantity.is_some() && row.quantity_source != Some(QuantitySource::Manual) {
        report.violate(
            EVAL_POLICY_MISPLACED,
            row.subject(),
            "manual_quantity is authorable if and only if quantitySource = manual; \
             otherwise it is a frozen quantity nothing reads",
        );
    }
}

/// `inst-mk-chargekind` — the kind x chargeKind legality matrix (D-18).
///
/// `flat` and `per_unit` are legal on **non-usage** rows. `per_unit`,
/// `graduated`, `volume` and `package` are legal on **usage** rows — a
/// `per_unit` usage row being the plain untiered metered rate, unit price times
/// metered `Q`.
///
/// The two refusals have one reason: the tier and block machinery presupposes a
/// metered quantity stream, and no `Q` semantics exist for a non-usage row. So
/// `flat` on a usage row (a metered row that ignores its meter) and
/// `graduated` / `volume` / `package` on a `recurring` / `one_time` /
/// `one_time_setup` row (bands over a quantity nothing produces) both fail
/// publish. Tiered per-seat pricing is Future scope.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct KindChargeKindMatrix;

impl ValidationRule<PriceRow> for KindChargeKindMatrix {
    fn name(&self) -> &'static str {
        "inst-mk-chargekind"
    }

    fn evaluate(&self, subject: &PriceRow, report: &mut ValidationReport) {
        let Some(kind) = subject.model_kind else {
            return;
        };
        let legal = if subject.is_usage() {
            matches!(
                kind,
                ModelKind::PerUnit | ModelKind::Graduated | ModelKind::Volume | ModelKind::Package
            )
        } else {
            matches!(kind, ModelKind::Flat | ModelKind::PerUnit)
        };
        if legal {
            return;
        }
        report.violate(
            MODEL_KIND_CHARGEKIND_MISMATCH,
            subject.subject(),
            format!(
                "modelKind {} is not legal on a {} row: flat and per_unit are the non-usage \
                 kinds, and per_unit / graduated / volume / package are the usage kinds",
                model_kind_wire(kind),
                subject.charge_kind
            ),
        );
    }
}

#[cfg(test)]
#[path = "model_kind_tests.rs"]
mod model_kind_tests;
