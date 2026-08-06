//! `PriceOverlayValidator` — the nine `inst-plv-*` rules of
//! `design/09-price-overlays.md` §3, as one aggregate pipeline over a snapshot.
//!
//! Where [`crate::domain::overlay`] is the shapes, this module is the judgement:
//! `inst-plv-scope`, `inst-plv-lines`, `inst-plv-adjustment`,
//! `inst-plv-eligibility`, `inst-plv-precedence`, `inst-plv-dating`,
//! `inst-plv-referential` and `inst-plv-taxbasis`, with `inst-plv-disclosure`'s
//! catalog half discussed below.
//!
//! # One pipeline, not two, and D-213's shape found once
//!
//! The set runs **twice** — as a pre-check at submit and again inside the
//! publish-commit transaction (Foundation §4.2) — so every rule here is pure
//! with respect to the [`OverlayWorld`] it is handed, and none of them fetches
//! its own inputs. That is what makes one pipeline right rather than two: the
//! authoring-edge run and the publish-time run ask the *same* questions of two
//! different worlds, and a rule split across two pipelines would answer one of
//! them in only one of the runs.
//!
//! **One rule is genuinely outside it**, and it is D-213's shape one slice over:
//! [`check_tax_basis_declared`]. L5 says a basis MUST be declared and *silence
//! fails*, but `tax_basis` is `NOT NULL` in the store and [`TaxBasis`] is a
//! closed enum, so a **stored** overlay always has one and the publish-time arm
//! of that rule is unreachable. It therefore belongs to the authoring edge,
//! which is where the surface calls it from, and not to the walk below — a
//! pipeline taking `Option<TaxBasis>` would carry an arm no stored row can
//! reach. This is exactly `bundle_rules::check_basis_declared`'s argument, found
//! independently here.
//!
//! # Rules this module deliberately does **not** hold
//!
//! * **The scope pairing.** `(scope_class = 'global') = (scope_value = '')` is
//!   unrepresentable in [`ScopeSelector`], so there is no arm for it. What is
//!   left checkable — *is this value declared in its universe* — is
//!   [`check_scope`]'s, and the lookup is the repository's.
//! * **`fixed` is amount-based, a SKU line names its plan, a cohort line names
//!   its plan.** All three are unconstructible; see [`crate::domain::overlay`].
//! * **`inst-plv-disclosure`.** There is nothing here to *reject*: `disclosure`
//!   is a closed enum with a fail-closed default, and §3 step 7's rule is about
//!   **exposure** — the overlay is excluded from consumer-facing enumeration and
//!   from the base-price preview, while operator and service reads are
//!   explicitly unaffected. This gear mounts **no consumer-facing enumeration**
//!   (`GET /bss-pricing/v1/price-overlays` is the admin/Tariffs read), so the
//!   catalog's whole half of that rule is: carry the flag, default it
//!   `restricted`, publish it. The exclusion half has no subject in this crate
//!   and is owed by Presentation and the Tariffs preview (F-34). Stated here
//!   rather than left as a missing rule, because an absent arm reads as an
//!   oversight.
//! * **`inst-plv-class-tiebreak` and `inst-plv-member-preview`.** Two further
//!   `inst-plv-*` ids §3 declares that are outside this strand's census. The
//!   class order the first of them publishes **is** here —
//!   [`ScopeClass`]'s derived `Ord` — because the read model has to carry it;
//!   the cross-class equal-precedence *warning* it also asks for is not, and is
//!   owed.
//!
//! # The direction D-138's warning needs, and does not have
//!
//! [`FIXED_LINE_DISCARDS_STACK`] fires when a `fixed` line *"is not the
//! lowest-precedence layer able to match its target"*. That condition presumes
//! the stack is applied from the low end: under the opposite reading a `fixed`
//! at a higher precedence is applied **first** and discards nothing, while a
//! `fixed` at the lowest precedence discards everything above it — so the
//! warning's condition is exactly **inverted**. §F.1 records the sort direction
//! as undecided and the brief for this strand says not to encode one.
//!
//! What is built here is D-138's own words, literally: the predicate compares
//! numeric precedence and warns on a non-minimum matching `fixed` layer. That is
//! not a choice of direction — it is the condition D-138 states — but it is only
//! *correct* under one of the two readings, and a warning that fires on the
//! wrong half of the cases is worse than no warning. It is an `[H]` entry in the
//! owed register: the decision is the owner's, and this is the one place in the
//! strand where its absence is load-bearing rather than academic.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::money::CurrencyCode;
use crate::domain::overlay::{
    Adjustment, Disclosure, LineKey, OverlayInterval, OverlayLine, ScopeSelector, TargetRef,
    TargetSku, TaxBasis,
};
use crate::domain::scope_key::PlanId;
use crate::domain::validation::ValidationReport;

// ---------------------------------------------------------------------------
// The codes.
// ---------------------------------------------------------------------------

/// A non-`global` scope value that its declared universe does not carry
/// (`inst-plv-scope`, D-120).
///
/// **Minted by this gear.** §2 calls it *"taxonomy failure (422)"* and §5's
/// problem-response list declares no code string for it, so a consumer needs a
/// discriminator that is not the message. A gear may mint a `DomainError`
/// variant freely — a wire code is the design set's to declare — which is why
/// this is in the owed register beside Slice 8's `BUNDLE_EXISTS_ON_PLAN`.
pub const SCOPE_VALUE_UNKNOWN: &str = "SCOPE_VALUE_UNKNOWN";

/// An overlay carrying no adjustment line at all (D-42: *"≥ 1 line per
/// overlay"*; §9: *"a zero-line overlay fails"*).
///
/// **Minted by this gear**, for [`SCOPE_VALUE_UNKNOWN`]'s reason and with one
/// more: §5 declares nine overlay codes and none of them covers an empty line
/// set. Reporting it under [`OVERLAY_LINE_TARGET_UNKNOWN`] — the nearest
/// candidate — would send the operator to check a target that does not exist.
pub const OVERLAY_HAS_NO_LINE: &str = "OVERLAY_HAS_NO_LINE";

/// A second line on one `(planId, targetSku, cohort)` key within one overlay
/// revision (§5, D-42; `cohort` since D-78).
pub const OVERLAY_LINE_DUPLICATE: &str = "OVERLAY_LINE_DUPLICATE";

/// A line naming a plan outside the overlay's `target_ref` scope, or a SKU its
/// plan does not publish (§5, D-42).
pub const OVERLAY_LINE_TARGET_UNKNOWN: &str = "OVERLAY_LINE_TARGET_UNKNOWN";

/// A line naming a `cohort` no published `existing_grandfathered` row of its
/// target plan carries (§5, D-78).
pub const OVERLAY_LINE_COHORT_UNKNOWN: &str = "OVERLAY_LINE_COHORT_UNKNOWN";

/// An amount-based line missing a value for a currency its target scope sells
/// (§5, D-08 — no implicit FX).
pub const ADJUSTMENT_CURRENCY_NOT_COVERED: &str = "ADJUSTMENT_CURRENCY_NOT_COVERED";

/// A magnitude outside D-67's range: a `discount` bp outside `(0, 10000]`, a
/// non-positive `markup` bp, or a negative amount magnitude (§5).
pub const ADJUSTMENT_MAGNITUDE_OUT_OF_RANGE: &str = "ADJUSTMENT_MAGNITUDE_OUT_OF_RANGE";

/// A scope referencing an unpublished plan or SKU (§5, `inst-plv-referential`).
pub const TARGET_UNPUBLISHED: &str = "TARGET_UNPUBLISHED";

/// A duplicate `precedence` within one scope class (§5, L2).
pub const PRECEDENCE_DUPLICATE: &str = "PRECEDENCE_DUPLICATE";

/// Overlapping effective intervals for one line key
/// `(scope_class, scope_value, planId, targetSku, cohort)` (§5, D-107).
pub const OVERLAY_INTERVAL_OVERLAP: &str = "OVERLAY_INTERVAL_OVERLAP";

/// No tax basis declared and none explicitly delegated (§5, L5).
pub const TAX_BASIS_UNDECLARED: &str = "TAX_BASIS_UNDECLARED";

/// **Warning.** A `fixed` line that is not the lowest-precedence layer able to
/// match its target: it silently voids the layers below it (§5, D-138).
///
/// Read the module doc before trusting this one — its condition inverts with
/// §F.1's undecided sort direction.
pub const FIXED_LINE_DISCARDS_STACK: &str = "FIXED_LINE_DISCARDS_STACK";

/// **Warning.** A published overlay targets a retired plan: dangling-and-flagged
/// (D-31), remediation is to end or retarget the overlay.
///
/// **Minted by this gear.** D-31 names the read-model flag and the
/// `pricing.priceoverlay.target_retired` alarm and no wire code, because the
/// design set never expected this to reach a caller — it is a state an operator
/// discovers, not a request they made. It is a warning here so an author
/// retargeting an overlay onto a retired plan is told at the moment they do it.
pub const TARGET_RETIRED: &str = "TARGET_RETIRED";

// ---------------------------------------------------------------------------
// The snapshot.
// ---------------------------------------------------------------------------

/// One published line interval of some **other** overlay, as the caller's read
/// found it — `inst-plv-dating`'s collision domain.
///
/// The interval is the **overlay's**, not the line's: overlays carry the dating
/// and lines do not (§6 puts `effective_from` / `effective_to` on the header).
/// The line key is what the collision is *per*, which is D-42's change to a rule
/// that used to be per `(scope, target)` pair.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PublishedLineInterval {
    /// The overlay holding it. Compared against the candidate's own id, because
    /// the collision domain is *other* overlays (D-107).
    pub price_overlay_id: Uuid,
    /// The holding overlay's scope — part of the collision key.
    pub scope: ScopeSelector,
    /// The holding line's key.
    pub key: LineKey,
    /// The holding overlay's own interval.
    pub interval: OverlayInterval,
}

/// Everything the rules need to know about the world, resolved by the caller.
///
/// None of it is derivable here and all of it is a read. §4.2's contract
/// requires a rule to be pure with respect to the state it is handed, because
/// the same set runs twice and the world moves between the two runs; a validator
/// that fetched its own inputs would be answering two different questions.
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OverlayWorld {
    /// Is the candidate's scope value declared in `class.taxonomy_table()`?
    ///
    /// Meaningless for [`ScopeSelector::Global`], which consults no universe —
    /// [`check_scope`] does not read it in that case.
    pub scope_value_declared: bool,
    /// Which of the candidate's `target_ref` plans are published.
    pub published_plans: BTreeSet<PlanId>,
    /// Which SKUs each published plan carries.
    pub published_skus: BTreeMap<PlanId, BTreeSet<TargetSku>>,
    /// Which of them are retired — D-31's flag, never a block.
    pub retired_plans: BTreeSet<PlanId>,
    /// The currencies each target plan sells: `ADJUSTMENT_CURRENCY_NOT_COVERED`'s
    /// domain.
    pub sold_currencies: BTreeMap<PlanId, BTreeSet<CurrencyCode>>,
    /// The cutover instants of each plan's published grandfathered generations
    /// (D-78).
    pub published_cohorts: BTreeMap<PlanId, BTreeSet<DateTime<Utc>>>,
    /// The **other** published overlay holding this `(scope_class, precedence)`,
    /// if any. `None` is the ordinary answer.
    pub precedence_holder: Option<Uuid>,
    /// Other overlays' published line intervals.
    pub interval_holders: Vec<PublishedLineInterval>,
    /// The plans for which some published overlay at a **strictly lower**
    /// precedence carries a matching line — D-138's warning domain.
    pub lower_precedence_matchers: BTreeSet<PlanId>,
}

/// One overlay revision as an author submits it, plus the world it is judged in.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayCandidate {
    /// The overlay's identity.
    pub price_overlay_id: Uuid,
    /// The revision being validated.
    pub revision: u64,
    /// Scope class and value.
    pub scope: ScopeSelector,
    /// L2's explicit precedence.
    pub precedence: i32,
    /// The overlay's own interval.
    pub interval: OverlayInterval,
    /// The declared basis. Already past [`check_tax_basis_declared`] by the time
    /// a candidate exists — see the module doc.
    pub tax_basis: TaxBasis,
    /// L6's exposure flag. Carried, defaulted and published; see the module doc
    /// for why no rule below reads it.
    pub disclosure: Disclosure,
    /// The plans the lines may target.
    pub target_ref: TargetRef,
    /// The adjustment lines, whole (D-42).
    pub lines: Vec<OverlayLine>,
    /// The resolved world.
    pub world: OverlayWorld,
}

// ---------------------------------------------------------------------------
// The authoring-edge entry point.
// ---------------------------------------------------------------------------

/// `inst-plv-taxbasis` / L5: the basis MUST be declared, and *silence fails*.
///
/// A **declaration** and not merely a value: [`TaxBasis::DelegatedTariffs`] is
/// an explicit delegation to Tariffs and passes, which is the distinction L5
/// draws — what fails is saying nothing.
///
/// Its own entry point rather than an arm of [`validate`], because `tax_basis`
/// is `NOT NULL` in the store and [`TaxBasis`] is closed, so a stored overlay
/// always has one and the publish-time arm would be unreachable. See the module
/// doc; this is D-213's shape.
///
/// # Errors
/// [`TAX_BASIS_UNDECLARED`] when the request carried none.
pub const fn check_tax_basis_declared(basis: Option<TaxBasis>) -> Result<TaxBasis, &'static str> {
    match basis {
        Some(basis) => Ok(basis),
        None => Err(TAX_BASIS_UNDECLARED),
    }
}

// ---------------------------------------------------------------------------
// The pipeline.
// ---------------------------------------------------------------------------

/// Run every overlay rule, collecting the whole report.
///
/// Aggregate rather than fail-fast, per §4.2: one pass tells the author
/// everything that is wrong, which makes an overlay remediable in one edit
/// instead of in as many edits as it has faults.
#[must_use]
pub fn validate(candidate: &OverlayCandidate) -> ValidationReport {
    let mut report = ValidationReport::default();
    check_scope(candidate, &mut report);
    check_referential(candidate, &mut report);
    check_lines(candidate, &mut report);
    check_eligibility(candidate, &mut report);
    check_adjustment(candidate, &mut report);
    check_precedence(candidate, &mut report);
    check_dating(candidate, &mut report);
    report
}

/// `inst-plv-scope`: every non-`global` value validates against its universe.
///
/// The class/value **pairing** is not checked because it is unrepresentable; the
/// universe lookup is the caller's, and this reports its answer naming the table
/// consulted so an operator knows where to declare the value.
fn check_scope(candidate: &OverlayCandidate, report: &mut ValidationReport) {
    let Some(value) = candidate.scope.value() else {
        // The classless scope has no value and therefore no universe.
        return;
    };
    if candidate.world.scope_value_declared {
        return;
    }
    let class = candidate.scope.class();
    let universe = class
        .taxonomy_table()
        .unwrap_or("a taxonomy this class does not declare");
    report.violate(
        SCOPE_VALUE_UNKNOWN,
        value.as_str(),
        format!(
            "scope value `{value}` is not declared in {universe}; a {class}-scoped overlay \
             selects who receives an adjustment, so its value must exist in the tenant's \
             declared universe (D-120)"
        ),
    );
}

/// `inst-plv-referential`: the overlay's targets must be published, and a
/// retired one dangles rather than blocking (D-31).
fn check_referential(candidate: &OverlayCandidate, report: &mut ValidationReport) {
    for plan in &candidate.target_ref.plans {
        if !candidate.world.published_plans.contains(plan) {
            report.violate(
                TARGET_UNPUBLISHED,
                plan.to_string(),
                format!(
                    "overlay target plan {plan} has not published; an overlay on an unpublished \
                     plan is rejected and never exposed in the read model"
                ),
            );
        }
        if candidate.world.retired_plans.contains(plan) {
            // D-31: dangling-and-flagged. In-flight subscribers legitimately
            // keep resolving a retired plan's rows, so the overlay stays
            // evaluable for them and a retire-time block is exactly the rule
            // D-31 refused.
            report.warn(
                TARGET_RETIRED,
                plan.to_string(),
                format!(
                    "overlay target plan {plan} is retired; the overlay stays evaluable for \
                     in-flight subscribers and is flagged — remediation is to end or retarget it \
                     (D-31)"
                ),
            );
        }
    }
}

/// `inst-plv-lines` (D-42): at least one line, no duplicate key, every non-default
/// line inside the overlay's target scope.
fn check_lines(candidate: &OverlayCandidate, report: &mut ValidationReport) {
    if candidate.lines.is_empty() {
        report.violate(
            OVERLAY_HAS_NO_LINE,
            candidate.price_overlay_id.to_string(),
            "an overlay carries at least one adjustment line: a container of nothing adjusts \
             nothing (D-42)",
        );
        return;
    }

    let mut seen: BTreeSet<&LineKey> = BTreeSet::new();
    for line in &candidate.lines {
        if !seen.insert(&line.key) {
            report.violate(
                OVERLAY_LINE_DUPLICATE,
                render_key(&line.key),
                format!(
                    "line key {} appears twice in this revision; one default line, one line per \
                     plan, one per (plan, sku), and one of each per targeted generation (D-42, \
                     D-78)",
                    render_key(&line.key)
                ),
            );
        }

        let Some(plan) = line.key.plan_id() else {
            // The list-default line targets every plan of `target_ref` and has
            // nothing of its own to place.
            continue;
        };
        if !candidate.target_ref.covers(plan) {
            report.violate(
                OVERLAY_LINE_TARGET_UNKNOWN,
                render_key(&line.key),
                format!(
                    "line targets plan {plan}, which is outside this overlay's target_ref scope"
                ),
            );
            continue;
        }
        if let Some(sku) = line.key.target_sku()
            && !candidate
                .world
                .published_skus
                .get(&plan)
                .is_some_and(|published| published.contains(sku))
        {
            report.violate(
                OVERLAY_LINE_TARGET_UNKNOWN,
                render_key(&line.key),
                format!("line targets SKU `{sku}`, which plan {plan} does not publish"),
            );
        }
    }
}

/// `inst-plv-eligibility` (D-78): a `cohort` must name a generation the line's
/// target plan actually published.
///
/// The `plan_id` requirement is not checked — a cohort key without one is
/// unconstructible ([`LineKey::for_cohort`]), which is §6's
/// `CHECK (cohort IS NULL OR plan_id IS NOT NULL)` with no `CHECK`.
fn check_eligibility(candidate: &OverlayCandidate, report: &mut ValidationReport) {
    for line in &candidate.lines {
        let (Some(cohort), Some(plan)) = (line.key.cohort(), line.key.plan_id()) else {
            continue;
        };
        let known = candidate
            .world
            .published_cohorts
            .get(&plan)
            .is_some_and(|generations| generations.contains(&cohort));
        if !known {
            report.violate(
                OVERLAY_LINE_COHORT_UNKNOWN,
                render_key(&line.key),
                format!(
                    "line names cohort {} on plan {plan}, which carries no published \
                     existing_grandfathered row at that cutover instant; adjusting a \
                     grandfathered generation takes an explicit line naming its own instant \
                     (D-78)",
                    cohort.to_rfc3339()
                ),
            );
        }
    }
}

/// `inst-plv-adjustment`: D-67's ranges, D-08's per-currency coverage and
/// D-138's replacement warning.
fn check_adjustment(candidate: &OverlayCandidate, report: &mut ValidationReport) {
    for line in &candidate.lines {
        check_magnitude_range(line, report);
        check_currency_coverage(candidate, line, report);
        check_replacement(candidate, line, report);
    }
}

/// D-67: `0 < v <= 10000` on a discount, `v > 0` on a markup, `>= 0` on money.
///
/// A pipeline rule and not a constructor refusal, because §5 requires the
/// refusal to **name the line** and a constructor cannot appear in an aggregate
/// report. The store's two `CHECK`s are the backstop; the two guards are in
/// series, which is why `overlay_rules_tests` drives this one directly.
fn check_magnitude_range(line: &OverlayLine, report: &mut ValidationReport) {
    if let Some(bp) = line.adjustment.percent_bp() {
        let ceiling = matches!(line.adjustment, Adjustment::Discount(_));
        if bp <= 0 {
            report.violate(
                ADJUSTMENT_MAGNITUDE_OUT_OF_RANGE,
                render_key(&line.key),
                format!(
                    "line {} carries a {} magnitude of {bp} bp; a magnitude must be strictly \
                     positive — a line that adjusts nothing is a line that should not have been \
                     authored (D-67)",
                    render_key(&line.key),
                    line.adjustment.kind()
                ),
            );
        } else if ceiling && bp > 10_000 {
            report.violate(
                ADJUSTMENT_MAGNITUDE_OUT_OF_RANGE,
                render_key(&line.key),
                format!(
                    "line {} carries a discount of {bp} bp; a discount above 100% is not \
                     authorable, and {bp} is the shape the \"150% of list\" data-entry inversion \
                     takes (D-67)",
                    render_key(&line.key)
                ),
            );
        }
    }

    let Some(amounts) = line.adjustment.amounts() else {
        return;
    };
    for (currency, value) in amounts.iter() {
        if value < 0 {
            report.violate(
                ADJUSTMENT_MAGNITUDE_OUT_OF_RANGE,
                render_key(&line.key),
                format!(
                    "line {} carries {value} minor units in {}; an amount magnitude is >= 0 at \
                     the currency's ISO 4217 minor unit (D-67)",
                    render_key(&line.key),
                    currency.as_str()
                ),
            );
        }
    }
}

/// D-08: an amount-based line covers **every** currency its target scope sells.
///
/// A **percent** line is currency-neutral and needs no values at all, which is
/// the whole reason the magnitude's type is declared rather than inferred from
/// the presence of amount rows.
fn check_currency_coverage(
    candidate: &OverlayCandidate,
    line: &OverlayLine,
    report: &mut ValidationReport,
) {
    let Some(amounts) = line.adjustment.amounts() else {
        return;
    };
    for currency in coverage_domain(candidate, &line.key) {
        if amounts.get(&currency).is_none() {
            report.violate(
                ADJUSTMENT_CURRENCY_NOT_COVERED,
                render_key(&line.key),
                format!(
                    "line {} carries no value for {}, which its target scope sells; the catalog \
                     performs no FX, so an absolute magnitude exists only per currency (D-08)",
                    render_key(&line.key),
                    currency.as_str()
                ),
            );
        }
    }
}

/// The currencies one line must cover.
///
/// A per-plan or per-`(plan, sku)` line covers its own plan's markets; the
/// **list-default** line applies to every target of the overlay, so its domain
/// is the union over `target_ref`.
fn coverage_domain(candidate: &OverlayCandidate, key: &LineKey) -> BTreeSet<CurrencyCode> {
    let plans: Vec<PlanId> = key
        .plan_id()
        .map_or_else(|| candidate.target_ref.plans.clone(), |plan| vec![plan]);
    plans
        .into_iter()
        .filter_map(|plan| candidate.world.sold_currencies.get(&plan))
        .flat_map(|set| set.iter().cloned())
        .collect()
}

/// D-138: a `fixed` line that is not the lowest matching layer voids the layers
/// below it, and authoring warns.
///
/// **A warning, never a refusal** — voiding a lower layer is a legitimate act
/// and the operator may well intend it; what they must not do is intend it by
/// accident. Read the module doc on the sort direction before trusting the
/// condition.
fn check_replacement(
    candidate: &OverlayCandidate,
    line: &OverlayLine,
    report: &mut ValidationReport,
) {
    if !line.adjustment.replaces() {
        return;
    }
    let targets: Vec<PlanId> = line
        .key
        .plan_id()
        .map_or_else(|| candidate.target_ref.plans.clone(), |plan| vec![plan]);
    let discarded: Vec<String> = targets
        .into_iter()
        .filter(|plan| candidate.world.lower_precedence_matchers.contains(plan))
        .map(|plan| plan.to_string())
        .collect();
    if discarded.is_empty() {
        return;
    }
    report.warn(
        FIXED_LINE_DISCARDS_STACK,
        render_key(&line.key),
        format!(
            "line {} is a fixed replacement at precedence {} and is not the lowest-precedence \
             layer able to match {}; a replacement discards every layer beneath it (D-138)",
            render_key(&line.key),
            candidate.precedence,
            discarded.join(", ")
        ),
    );
}

/// `inst-plv-precedence` (L2): unique within one scope class, among published
/// revisions.
///
/// The refusal names the overlay holding the slot, because the operator's next
/// action is a real one — go and look at that overlay — and it is unreachable
/// from a refusal that does not say which one is in the way.
fn check_precedence(candidate: &OverlayCandidate, report: &mut ValidationReport) {
    let Some(holder) = candidate.world.precedence_holder else {
        return;
    };
    report.violate(
        PRECEDENCE_DUPLICATE,
        candidate.precedence.to_string(),
        format!(
            "precedence {} is already held by published overlay {holder} in scope class {}; \
             precedence is unique within a class, and a tie across classes is broken by the \
             published class order instead",
            candidate.precedence,
            candidate.scope.class()
        ),
    );
}

/// `inst-plv-dating` (D-107): overlapping intervals collide **per line key**, over
/// published revisions of **other** overlays only.
///
/// The two exclusions are the rule rather than optimisations. Restricting the
/// domain to published revisions is what lets a draft exist at all; restricting
/// it to *other* overlays is D-107 — since D-92 gave overlays draft-revision
/// rows, an unscoped check matched the overlay's own published revision and
/// rejected every edit of a live overlay.
fn check_dating(candidate: &OverlayCandidate, report: &mut ValidationReport) {
    for line in &candidate.lines {
        for holder in &candidate.world.interval_holders {
            if holder.price_overlay_id == candidate.price_overlay_id {
                // D-107: never against another revision of the same overlay.
                continue;
            }
            if holder.scope != candidate.scope || holder.key != line.key {
                continue;
            }
            if !holder.interval.intersects(&candidate.interval) {
                continue;
            }
            report.violate(
                OVERLAY_INTERVAL_OVERLAP,
                render_key(&line.key),
                format!(
                    "line {} on scope {}/{} overlaps the interval published overlay {} holds on \
                     the same line key",
                    render_key(&line.key),
                    candidate.scope.class(),
                    candidate.scope.stored_value(),
                    holder.price_overlay_id
                ),
            );
        }
    }
}

/// A line key as an operator reads it.
///
/// The rendering carries all three components even when two are absent, because
/// `OVERLAY_LINE_DUPLICATE` on `default` and on `plan=… sku=…` are different
/// facts and a refusal naming only what is present cannot say which.
fn render_key(key: &LineKey) -> String {
    let plan = key
        .plan_id()
        .map_or_else(|| "default".to_owned(), |plan| plan.to_string());
    let sku = key
        .target_sku()
        .map_or_else(|| "-".to_owned(), TargetSku::to_string);
    let cohort = key
        .cohort()
        .map_or_else(|| "-".to_owned(), |at| at.to_rfc3339());
    format!("plan={plan} sku={sku} cohort={cohort}")
}

#[cfg(test)]
#[path = "overlay_rules_tests.rs"]
mod overlay_rules_tests;
