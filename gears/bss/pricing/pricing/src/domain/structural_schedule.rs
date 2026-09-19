//! Simultaneous structural cutovers across a logical line's markets.
//!
//! A charge line's **structure** — model kind, tier geometry, package size, the
//! usage policy, the descriptor — is shared by every market of the line, and a
//! market's money is bound to one immutable
//! [`ChargeLineVersion`](crate::domain::charge_line::ChargeLineVersion) through
//! its price row's `line_version_id`. Money therefore schedules per market
//! (`draft_window`'s W7), while the **structure** those monies are priced
//! against may not: at any instant inside the coverage horizon every market of
//! one line must be bound to the *same* version.
//!
//! Without that rule the split is unsound rather than merely untidy. Two markets
//! of one line whose ladders change on different days are two different products
//! sold under one line id: a tier index means one thing in EUR/eu and another in
//! USD/us for the days between the boundaries, and every downstream consumer
//! that resolves "the line's structure" — the projection, the snapshot, Rating's
//! step 2 — has to pick one and be wrong in the other market.
//!
//! # What this module is not
//!
//! It is **not** a scheduling subsystem. Nothing here decides when a structure
//! changes: the schedule is read off the window plane the author already built,
//! one [`StructureBinding`] per composed window, and this module only judges the
//! boundaries that plane implies. A cutover is authored the way every other
//! change of coverage is — split the market's window and bind the new interval
//! to a new monetary version of the new structure — and this rule is what
//! refuses doing it in one market and forgetting the other.

use std::collections::{BTreeMap, BTreeSet};

use time::OffsetDateTime;
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::error::DomainError;
use crate::domain::instant::format_rfc3339;
use crate::domain::scope_key::{ChargeLineScopeKey, MarketPriceScopeKey};
use crate::domain::validation::{ValidationReport, ValidationRule};
use crate::domain::window::WINDOW_OVERLAP;

/// A required market holds **no** structure at an instant inside the horizon
/// (422; the report names the market and the boundary).
///
/// Distinct from `WINDOW_COVERAGE_MISSING`, which says a key has no live window
/// at all. This one fires where coverage exists but does not reach: the market
/// has windows, and the instant one of its siblings changes structure falls
/// between them.
pub const STRUCTURE_MARKET_BINDING_MISSING: &str = "STRUCTURE_MARKET_BINDING_MISSING";

/// Two markets of one logical line are bound to different structure versions at
/// one instant (422; the report names the line, the boundary and each market's
/// version).
pub const STRUCTURE_CUTOVER_MISMATCH: &str = "STRUCTURE_CUTOVER_MISMATCH";

/// Which shared structure one market is priced against over one interval.
///
/// Derived from a composed window: `market` is the window's conflict scope and
/// `line_version_id` is the immutable structure its price row names. Intervals
/// are half-open `[effective_from, effective_to)`, `None` being open-ended, as
/// everywhere else on the window plane.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StructureBinding {
    /// The full market scope the window competes on.
    pub market: MarketPriceScopeKey,
    /// The immutable charge-line version the money is bound to.
    pub line_version_id: Uuid,
    /// Inclusive start, UTC.
    pub effective_from: OffsetDateTime,
    /// Exclusive end, UTC; `None` is open-ended.
    pub effective_to: Option<OffsetDateTime>,
}

impl StructureBinding {
    /// Is this binding active at `at` — `effective_from <= at < effective_to`?
    #[must_use]
    pub fn covers(&self, at: OffsetDateTime) -> bool {
        self.effective_from <= at && self.effective_to.is_none_or(|end| at < end)
    }
}

/// Every required market of one logical line carries one and the same structure
/// version at every boundary inside `[from, to)`.
///
/// Call it **once per logical line and selection class** — which is one call per
/// [`ChargeLineScopeKey`], since eligibility and cohort are axes of that key, so
/// a grandfathered generation's frozen selection is judged on its own and never
/// against its successor's.
///
/// `to = None` is an open horizon, and the sweep still judges the tail: the last
/// boundary any binding names is inside it, and the arm selected there runs to
/// the end of time.
///
/// # Errors
/// [`DomainError::InvalidRequest`] when a binding names a market outside
/// `required` or carries an empty interval — a caller assembling a set it did
/// not declare, not an authoring fault. Otherwise
/// [`DomainError::ValidationFailed`] carrying
/// [`STRUCTURE_MARKET_BINDING_MISSING`], [`STRUCTURE_CUTOVER_MISMATCH`] and
/// [`WINDOW_OVERLAP`] violations.
pub fn validate_structure_schedule(
    required: &BTreeSet<MarketPriceScopeKey>,
    bindings: &[StructureBinding],
    from: OffsetDateTime,
    to: Option<OffsetDateTime>,
) -> Result<(), DomainError> {
    if required.is_empty() {
        return Ok(());
    }
    for binding in bindings {
        if !required.contains(&binding.market) {
            return Err(DomainError::InvalidRequest(format!(
                "structure binding on market {} is outside the required set of this logical line",
                binding.market
            )));
        }
        if binding
            .effective_to
            .is_some_and(|end| end <= binding.effective_from)
        {
            return Err(DomainError::InvalidRequest(format!(
                "the structure binding interval [{}, {}) on market {} is empty",
                format_rfc3339(binding.effective_from),
                binding
                    .effective_to
                    .map_or_else(|| "open-ended".to_owned(), format_rfc3339),
                binding.market
            )));
        }
    }

    let mut report = ValidationReport::default();
    let mut previous: Option<BTreeMap<&MarketPriceScopeKey, Uuid>> = None;
    for boundary in boundaries(bindings, from, to) {
        let selection = select_at(required, bindings, boundary, &mut report);
        // **An instant at which no market of the line is bound is not this
        // rule's instant.** A plan whose coverage opens tomorrow is bound to
        // nothing today, and a line that has not launched yet is a question for
        // `inst-wc-required`, which counts a *scheduled* window as coverage. A
        // sweep that refused here would refuse every plan published ahead of its
        // start date.
        if selection.is_empty() {
            previous = None;
            continue;
        }
        refuse_mismatch(&selection, boundary, &mut report);
        if let Some(before) = &previous {
            refuse_dropped_market(required, before, &selection, boundary, &mut report);
        }
        previous = Some(selection);
    }
    if report.is_publishable() {
        return Ok(());
    }
    Err(DomainError::ValidationFailed(report))
}

/// The instants the schedule can change at, clipped to `[from, to)`.
///
/// `from` is one of them so a horizon that opens with coverage already live has
/// a first state to compare the next boundary against.
fn boundaries(
    bindings: &[StructureBinding],
    from: OffsetDateTime,
    to: Option<OffsetDateTime>,
) -> BTreeSet<OffsetDateTime> {
    let mut boundaries = BTreeSet::from([from]);
    for binding in bindings {
        if binding.effective_from >= from && to.is_none_or(|end| binding.effective_from < end) {
            boundaries.insert(binding.effective_from);
        }
        if let Some(end) = binding.effective_to
            && end >= from
            && to.is_none_or(|limit| end < limit)
        {
            boundaries.insert(end);
        }
    }
    boundaries
}

/// Which version each required market is bound to at `at`, reporting a market
/// bound to two at once as the existing overlap refusal.
fn select_at<'a>(
    required: &'a BTreeSet<MarketPriceScopeKey>,
    bindings: &[StructureBinding],
    at: OffsetDateTime,
    report: &mut ValidationReport,
) -> BTreeMap<&'a MarketPriceScopeKey, Uuid> {
    let mut selected = BTreeMap::new();
    for market in required {
        let active: Vec<&StructureBinding> = bindings
            .iter()
            .filter(|binding| &binding.market == market && binding.covers(at))
            .collect();
        match active.as_slice() {
            [] => {}
            [one] => {
                selected.insert(market, one.line_version_id);
            }
            [first, second, ..] => report.violate(
                WINDOW_OVERLAP,
                market.to_string(),
                format!(
                    "market {market} is bound to charge-line versions {} and {} at the same \
                     instant {}",
                    first.line_version_id,
                    second.line_version_id,
                    format_rfc3339(at)
                ),
            ),
        }
    }
    selected
}

/// Two markets of one line, covered at one instant, priced against two
/// structures.
fn refuse_mismatch(
    selection: &BTreeMap<&MarketPriceScopeKey, Uuid>,
    at: OffsetDateTime,
    report: &mut ValidationReport,
) {
    let versions: BTreeSet<Uuid> = selection.values().copied().collect();
    if versions.len() < 2 {
        return;
    }
    let Some(line) = selection.keys().next().map(|market| market.line()) else {
        return;
    };
    report.violate(
        STRUCTURE_CUTOVER_MISMATCH,
        line.to_string(),
        format!(
            "at {} the markets of logical line {line} are bound to different charge-line \
             versions ({}); a shared structure becomes effective in every market at the same \
             instant",
            format_rfc3339(at),
            render_selection(selection)
        ),
    );
}

/// A market whose coverage **ends** exactly where a sibling moves to a new
/// structure.
///
/// Reported only at a boundary where some market's version actually changed, and
/// only for a market that was bound at the previous boundary. Both conditions
/// keep this rule off questions that belong to the coverage set: a market that
/// simply launches later than its sibling changes nobody's structure, and a
/// market with no window at all is `inst-wc-required`'s finding, not this one's.
/// Two rules answering one question is two answers free to disagree.
fn refuse_dropped_market(
    required: &BTreeSet<MarketPriceScopeKey>,
    before: &BTreeMap<&MarketPriceScopeKey, Uuid>,
    selection: &BTreeMap<&MarketPriceScopeKey, Uuid>,
    at: OffsetDateTime,
    report: &mut ValidationReport,
) {
    let cutover = selection
        .iter()
        .any(|(market, version)| before.get(market).is_some_and(|had| had != version));
    if !cutover {
        return;
    }
    for market in required {
        if selection.contains_key(market) || !before.contains_key(market) {
            continue;
        }
        report.violate(
            STRUCTURE_MARKET_BINDING_MISSING,
            market.to_string(),
            format!(
                "market {market} is bound to no charge-line version at {}, the instant a sibling \
                 market of the same logical line moves to a new one. Split this market's window \
                 at the same boundary and bind the new interval to the new version",
                format_rfc3339(at)
            ),
        );
    }
}

fn render_selection(selected: &BTreeMap<&MarketPriceScopeKey, Uuid>) -> String {
    selected
        .iter()
        .map(|(market, version)| {
            let axes = market.parts();
            format!("{}/{} => {version}", axes.currency, axes.region)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Group a plan's bindings by logical line, in canonical order.
#[must_use]
pub fn by_line(
    bindings: &[StructureBinding],
) -> BTreeMap<ChargeLineScopeKey, Vec<StructureBinding>> {
    let mut grouped: BTreeMap<ChargeLineScopeKey, Vec<StructureBinding>> = BTreeMap::new();
    for binding in bindings {
        grouped
            .entry(binding.market.line().clone())
            .or_default()
            .push(binding.clone());
    }
    grouped
}

/// A plan's shared structures become effective in every market of a line at the
/// same instant (`inst-sc-simultaneous`).
///
/// Ranges over the horizon `[evaluated_at, open)`: history is frozen and a
/// boundary already past cannot be re-authored, so judging it would refuse a
/// publish for a cutover somebody completed correctly at the time.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct StructureCutoverSimultaneous;

impl ValidationRule<crate::domain::plan_shape::PlanShape> for StructureCutoverSimultaneous {
    /// `inst-sc-simultaneous` — **and this rule holds `inst-sc-scope` and
    /// `inst-sc-derived` too**, so `rule_names()` is not 1:1 with the three
    /// instruction ids §3 declares for this algorithm.
    ///
    /// The fold is deliberate, for `coverage::KeyCoverageRequired`'s reason: all
    /// three are properties of one sweep over one binding set. `inst-sc-scope`
    /// is which instants the sweep asks about and `inst-sc-derived` is where the
    /// bindings come from — neither is a second question, and registering either
    /// separately would be two rules asking one. It is recorded here rather than
    /// left to be noticed because a census written from the design document's
    /// ids would look for three names, find one, and be "fixed" by weakening it
    /// against a pipeline that is correct.
    fn name(&self) -> &'static str {
        "inst-sc-simultaneous"
    }

    fn evaluate(
        &self,
        subject: &crate::domain::plan_shape::PlanShape,
        report: &mut ValidationReport,
    ) {
        let billable = crate::domain::coverage::billable_keys(subject);
        for (line, bindings) in by_line(&subject.structure_bindings) {
            let required: BTreeSet<MarketPriceScopeKey> = billable
                .iter()
                .filter(|key| key.line() == &line)
                .cloned()
                .collect();
            // A line whose bindings all sit on markets no billable row holds is
            // not this rule's business: the coverage set is what says which
            // markets a publish owes, and a binding outside it belongs to a row
            // that is not being sold.
            let scoped: Vec<StructureBinding> = bindings
                .into_iter()
                .filter(|binding| required.contains(&binding.market))
                .collect();
            match validate_structure_schedule(&required, &scoped, subject.evaluated_at, None) {
                Ok(()) => {}
                Err(DomainError::ValidationFailed(found)) => report.absorb(found),
                // Unreachable by construction: `scoped` is filtered to
                // `required` and the plane cannot carry an empty interval —
                // `compose_windows` refused one before this list was built. It
                // is reported rather than dropped so a future caller that breaks
                // either premise learns about it from a refused publish instead
                // of a silently skipped rule.
                Err(other) => report.violate(
                    STRUCTURE_MARKET_BINDING_MISSING,
                    line.to_string(),
                    format!("the structure schedule of logical line {line} is unreadable: {other}"),
                ),
            }
        }
    }
}

#[cfg(test)]
#[path = "structural_schedule_tests.rs"]
mod structural_schedule_tests;
