//! The Slice-2 **plan-shape rule codes** (`design/02-plan-definition.md` §5).
//!
//! The sibling of [`crate::domain::rules`], and the same contract: the `const`
//! codes below are the machine-readable discriminators the design set names
//! **verbatim** in §5, they are what an RFC 9457 response carries and what the
//! conformance corpus asserts against — so they are written once, here, and
//! referenced at every `report.violate` site rather than spelled at each call
//! site. A code spelled twice is a code that can be spelled two ways.
//!
//! The four validators of this slice — `CycleShapeValidator`,
//! `CompositionValidator`, the `PhaseGraph` rules and the `DescriptorSet` rules
//! — each register their own
//! [`ValidationRule`](crate::domain::validation::ValidationRule)s over a
//! [`PlanShape`](crate::domain::plan_shape::PlanShape) into the Foundation's
//! fail-closed pipeline. They append and never short-circuit: a plan with a
//! broken phase chain *and* an incomplete descriptor set reports both, because
//! an author remediates a plan in one pass.
//!
//! **No code is declared here that nothing emits.** Every `const` below has a
//! rule behind it in this group; the ones §5 names and this gear cannot enforce
//! are enumerated in the next section instead, because a `const` for an
//! unraised code reads as enforcement to everyone who greps for it.
//!
//! ## Codes referenced and never redefined
//!
//! The revision-lifecycle refusals — `LIFECYCLE_FORBIDDEN`,
//! `PLAN_RETIRED_NO_SUCCESSOR`, `OPEN_DRAFT_REVISION_EXISTS`,
//! `PLAN_ABANDONED_NO_SUCCESSOR`, `STALE_VERSION`, `DUPLICATE_SCOPE_KEY` — are
//! **Foundation-owned** (§3.3) and are referenced from this slice, never
//! redeclared (R-11).
//!
//! ## What is deliberately absent
//!
//! The precedent is [`crate::domain::rules`]'s own absence section: *a rule
//! that always passes is indistinguishable from a rule that holds*. Six of the
//! codes §5 names are **not** declared here, and five of the six share one
//! reason — they are reads against the product/SKU registry read model, and
//! this gear has no registry client at all ([`crate::domain::ports`] holds the
//! `CatalogVersion` registry and nothing else).
//!
//! | Code | Why absent |
//! |---|---|
//! | `SKU_NOT_PUBLISHED` | the parent SKU's publication state lives in the product/SKU registry read model |
//! | `PLANTIER_DIVERGENT` | needs the parent SKU's `PlanTier`, from the same registry. The `PLANTIER_MISSING` half needs nothing external and **is** implemented |
//! | `ADDON_INCOMPATIBLE` (registry half) | "add-on SKUs published + compatible with the base SKU" is a registry read. The **plan-authored** half — an edge pointing outside the plan's own add-on set, and a conflicting pair with both sides `required` — needs nothing external and **is** implemented under this code, because D-16 made those edges plan-authored |
//! | `ADDON_OVERRIDE_UNRESOLVED` | needs two things this gear does not have: the add-on SKU's plans (a registry join) and `PriceWindow` coverage for the covering-member-key half (Slice 7 storage, D-95 / D-97 / D-116) |
//! | `METER_USAGE_TYPE_UNBOUND` | the registry metering-unit declaration's `usageTypeRef` (UC3(a)) |
//! | `METER_DIMENSION_UNDECLARED` | the `UsageType`'s declared `metadata_fields` keys (UC3(c)) |
//!
//! **A seventh absence, and the only one the design set names no code for at
//! all.** `inst-cs-customfreq` states two requirements. `n > 0` and
//! `n <= cap` is implemented, under [`INVALID_CUSTOM_INTERVAL`]. Anchor
//! compatibility — `customEveryN Days(n)` MUST anchor `subscription_start` —
//! is not, because `billing_anchor_policy` is a **Slice-6** `pricing_price`
//! column (`06-consumer-contracts.md` §6) that does not exist in this schema
//! and must not be added here. Adding it would be Slice 2 minting Slice 6's
//! column, and the rule over it would then belong to whichever slice built it
//! first. Stretching [`INVALID_CUSTOM_INTERVAL`] to cover the anchor half was
//! rejected for the same reason `SETUP_ROW_INVALID` does not absorb the
//! `billingTiming` check: a code reported for something §5 does not say it
//! reports is a code no consumer can act on.
//!
//! **The eighth and ninth are signals rather than codes.**
//! `inst-cmp-tier-drift` and the drift arm of `inst-cmp-usagetype` both consume
//! a **registry change signal** this gear has no lane for — no inbound port, no
//! subscriber, no job — and both write the operator-plane flag store rather
//! than failing a publish. That store **exists**: `pricing_operator_flag` is a
//! table on both backends (D-85) carrying both flag names in its `CHECK`
//! already, and it has no repository. The absence is therefore the *signal*,
//! not the storage — worth stating, because a reader who finds the table would
//! otherwise reasonably conclude the feature is wired.
//!
//! ## Rules cross-referenced here and registered elsewhere
//!
//! Three requirements Slice 2's instructions state are **owned by other
//! slices**, and a second registration of any of them is two owners of one
//! rule, free to disagree:
//!
//! - `billingTiming` REQUIRED on every recurring row is **Slice 6's**
//!   (`inst-cs-recurring` says so in as many words: cross-referenced here,
//!   never re-registered);
//! - `billingGranularity` on all usage rows and `tierAggregationWindow` on
//!   tiered / `package` rows are **already implemented** as Slice 3's
//!   [`EVAL_POLICY_MISSING`](crate::domain::rules::EVAL_POLICY_MISSING), which
//!   `inst-cs-usage` cross-references;
//! - `taxCategory` is each row's Slice-4 `tax_category_ref` (D-110) — never a
//!   descriptor-set column and never a Slice-2 check.

pub mod composition;
pub mod cycle_shape;
pub mod descriptor_set;
pub mod phase_graph;

use crate::domain::plan_shape::PlanShape;
use crate::domain::validation::ValidationPipeline;

pub use cycle_shape::CustomIntervalBounds;
pub use descriptor_set::DescriptorSetComplete;

// ---------------------------------------------------------------------------
// Cycle shape (design/02-plan-definition.md 5, cpt-cf-bss-pricing-algo-cycle-shape)
// ---------------------------------------------------------------------------

/// A custom interval `n` that is non-positive or over the configured cap
/// (`inst-cs-customfreq`, P1; the caps are the tenant's, from their
/// `pricing_policy_object` entry, else the ratified deployment default —
/// D-152). Over-cap is rejected at authoring rather than silently clamped.
pub const INVALID_CUSTOM_INTERVAL: &str = "INVALID_CUSTOM_INTERVAL";

/// A `hybrid` plan missing one of its two mandatory parts (`inst-cs-hybrid`).
pub const HYBRID_INCOMPLETE: &str = "HYBRID_INCOMPLETE";

/// A priced `(meter, dimensionKey)` line with no usage row in a
/// `(currency, region)` the plan sells (D-84).
///
/// The "sold but unrateable" state D-15/D-17 declare impossible by
/// construction: a hybrid selling recurring in EUR and USD with usage priced
/// only in EUR is sellable in USD, and the USD subscriber's usage events fail
/// closed. A market where usage is genuinely free is an explicit `$0` row,
/// never an absence.
pub const USAGE_MARKET_INCOMPLETE: &str = "USAGE_MARKET_INCOMPLETE";

/// A `one_time_setup` row on a plan whose cycle does not admit one, or one
/// carrying recurrence, `billingTiming` or tier fields (`inst-cs-setup`).
pub const SETUP_ROW_INVALID: &str = "SETUP_ROW_INVALID";

/// `purchase_min_qty > purchase_max_qty` (`inst-cs-onetime`).
pub const PURCHASE_QTY_RANGE_INVALID: &str = "PURCHASE_QTY_RANGE_INVALID";

/// A **newly set or changed** `availableFrom` in the past, on any billing cycle
/// (`inst-cs-availability`).
///
/// The rule is cycle-independent (hoisted from the one-time step, 2026-07-28)
/// and binds only changed values (2026-07-31): a revision re-publishing an
/// unchanged date that has legitimately passed is not backdating, and blocking
/// it would make every later re-publish of a once-future-dated plan impossible
/// until the operator erased the date. The Slice-5 historical-import path is
/// the only sanctioned backdating.
pub const AVAILABLE_FROM_IN_PAST: &str = "AVAILABLE_FROM_IN_PAST";

// ---------------------------------------------------------------------------
// Composition (cpt-cf-bss-pricing-algo-composition)
// ---------------------------------------------------------------------------

/// No `PlanTier` declared at publish (`inst-cmp-plantier`). Optional at draft,
/// required at publish.
pub const PLANTIER_MISSING: &str = "PLANTIER_MISSING";

/// Two priced lines for one `(meter, dimensionKey)` within one scope-key slice
/// (`inst-cmp-injective`, D-103).
///
/// Injectivity is **per line per slice**, not per plan: `currency`, `region`,
/// `priceOverlay`, `phase`, `priceEligibility` and `cohort` legitimately
/// multiply rows, and a plan MAY price several `meteringUnit`s. Only a
/// duplicate line *within* one slice is the ambiguity that fails publish.
pub const METER_AMBIGUOUS: &str = "METER_AMBIGUOUS";

/// A dependency cycle over the plan-authored `depends_on` edges
/// (`inst-cmp-addons`, D-16).
pub const ADDON_CYCLE: &str = "ADDON_CYCLE";

/// A plan-authored add-on edge pointing outside the plan's own add-on set, or a
/// conflicting pair with both sides `required` (`inst-cmp-addons`, D-16).
///
/// The registry half of this code — add-on SKUs published and compatible with
/// the base SKU — is absent; see the module doc.
pub const ADDON_INCOMPATIBLE: &str = "ADDON_INCOMPATIBLE";

// ---------------------------------------------------------------------------
// Phase schedule (cpt-cf-bss-pricing-algo-phases)
// ---------------------------------------------------------------------------

/// A `convertsToPhaseId` chain that dangles or cycles, or a phase set without
/// **exactly one** terminal phase (`inst-ph-graph`).
pub const PHASE_GRAPH_INVALID: &str = "PHASE_GRAPH_INVALID";

/// A chain that skips the ordinal order, branches, or leaves a phase
/// unreachable from the entry phase (`inst-ph-graph`, 2026-07-31 review fix).
///
/// Acyclicity plus a single terminal alone admit dead phases that still demand
/// coverage rows and leave "first" undefined.
pub const PHASE_CHAIN_NONLINEAR: &str = "PHASE_CHAIN_NONLINEAR";

/// A terminal phase whose `kind` is not `evergreen` (C-4, 2026-08-01).
///
/// The constraint was carried only by a parenthetical while terminality is
/// structural and the column admits all three kinds, so nothing rejected a
/// `trial`-terminal chain — which leaves "the first non-trial phase" undefined
/// for setup timing and D-39 and collides with the
/// `display_trial_days = phase_duration_days` CHECK. "Intro pricing forever" is
/// an `evergreen` terminal phase at the intro price, not an `intro` terminal.
pub const TERMINAL_PHASE_KIND_INVALID: &str = "TERMINAL_PHASE_KIND_INVALID";

/// A non-terminal phase without `phaseDurationDays`, or a terminal phase with
/// one (`inst-ph-duration`).
pub const PHASE_DURATION_INVALID: &str = "PHASE_DURATION_INVALID";

/// A phase with no covering recurring row for a sold `(currency, region)`, on a
/// plan whose cycle carries a recurring part (`inst-ph-coverage`, D-15).
///
/// A phase conversion must never resolve to nothing, and the row-based Slice-7
/// coverage check cannot see a phase that has no rows at all.
pub const PHASE_UNCOVERED: &str = "PHASE_UNCOVERED";

/// A phase-scoped usage row whose `(meter, dimensionKey)` line has no
/// phase-invariant terminal-phase row (`inst-ph-usage-invariant`, D-117).
///
/// Without the base row the D-89 unit guard has no comparison target, D-84's
/// per-market completeness exempts the line entirely as an additive override,
/// and after the phase converts the line resolves to **nothing** — "sold but
/// unrateable" through the override door.
pub const PHASE_OVERRIDE_ORPHANED: &str = "PHASE_OVERRIDE_ORPHANED";

/// A phase-scoped usage override that changes a unit- or counter-determining
/// field of the terminal-phase row it overrides (`inst-ph-override-units`,
/// D-89, extended by D-122).
///
/// The tier counter `Q` is keyed `(subscription, meter, dimensionKey, window)`
/// and is **phase-blind**, so conversion never resets it: a `per_hour` trial row
/// converting into a `per_day` evergreen row mid-window applies an
/// hours-denominated `Q` to day-denominated bands. Free-trial pricing stays
/// fully expressible — a `$0` rate at the same denomination.
pub const PHASE_OVERRIDE_UNIT_MISMATCH: &str = "PHASE_OVERRIDE_UNIT_MISMATCH";

/// A revision re-terminalizing an existing phase or introducing a different
/// terminal phase (`inst-ph-terminal-stable` (a), D-64).
///
/// The scope-key phase default and usage phase-invariance are both defined
/// relative to *which* phase is terminal, so re-terminalizing moves them
/// silently: a usage-only plan published on an implicit terminal `T0`, revised
/// to add a trial plus a new evergreen phase, leaves its metered row no longer
/// phase-invariant and a subscription in the new phase resolving no usage row.
pub const TERMINAL_PHASE_CHANGED: &str = "TERMINAL_PHASE_CHANGED";

/// A revision dropping a phase still referenced by a current published price
/// row (`inst-ph-terminal-stable` (b), D-64).
pub const PHASE_IN_USE: &str = "PHASE_IN_USE";

// ---------------------------------------------------------------------------
// Descriptors (cpt-cf-bss-pricing-algo-descriptors)
// ---------------------------------------------------------------------------

/// A missing element of the descriptor required-set (`inst-ds-required`, D-48
/// v1 as recomposed by D-110).
///
/// The set is the **three** descriptor-set fields plus whatever the tenant's
/// config-extensible required-set adds (P5, carried per tenant in
/// `pricing_policy_object` — D-152). `billingTiming` and `taxCategory`
/// are row-borne and are **not** checked here: the first is Slice 6's
/// registered rule and the second is each row's `tax_category_ref` (Slice 4),
/// and a second registration of either would be two owners of one rule, free to
/// disagree.
pub const DESCRIPTOR_INCOMPLETE: &str = "DESCRIPTOR_INCOMPLETE";

// ---------------------------------------------------------------------------
// Registration
// ---------------------------------------------------------------------------

/// Every Slice-2 plan-shape rule, in report order.
///
/// **Ordered by theme** — cycle shape, then composition, then the phase
/// schedule, then the descriptors — so a report reads the way a plan is
/// authored: what commercial shape it is, what it is made of, how it runs over
/// time, and how it is billed. That is [`crate::domain::rules::price_row_rules`]'s
/// arrangement applied to a subject one level up. Order is cosmetic to the
/// verdict, because the pipeline never short-circuits and a single blocking
/// violation blocks the publish wherever it appears — but the report is the
/// product, and an author reads it top-down.
///
/// One ordering preference was considered and not adopted: running D-84's
/// per-market completeness **after** the phase rules, so an author meets a
/// broken phase graph before a completeness finding computed over it. The theme
/// order is kept instead, because grouping by algorithm is the property a reader
/// can predict from the design set, and a single exception to it is a rule
/// nobody can locate.
///
/// # Two rules are configured **per tenant**, and that is why this takes arguments
///
/// [`CustomIntervalBounds`] carries the `customEveryN` caps and
/// [`DescriptorSetComplete`] carries P5's required-field list. Both hold their
/// configuration as **fields**, because [`crate::domain::validation`] requires a
/// rule to be pure with respect to the state it is handed; a rule that read
/// configuration inside `evaluate` could answer differently in the two runs
/// below for no authored reason. Neither can be defaulted into existence here:
/// `CustomIntervalBounds` deliberately has no `Default`, since zero caps reject
/// every custom frequency ever authored while looking exactly like a rule that
/// is switched on.
///
/// Both values are **the authoring tenant's** (D-152) — their
/// `pricing_policy_object` entry, falling back per column to the ratified
/// deployment default — so the caller resolves them for the tenant whose plan is
/// being judged and hands them in. That resolution is a storage read against
/// `crate::config`'s defaults, and neither is anything the domain may import,
/// which is the whole of why this function has parameters instead of a body that
/// looks them up:
///
/// ```ignore
/// let policy = policies.authoring_policy(scope, tenant_id).await?;
/// plan_shape_rules(policy.interval_bounds(), policy.descriptor_rule())
/// ```
///
/// The pipeline is therefore built per authoring run rather than once at init.
/// A pipeline held across tenants would enforce whichever tenant's limits it was
/// built for, and one held across time would keep rejecting plans after an
/// operator had raised the cap.
///
/// # What this pipeline is **not**: it is not run here
///
/// G4 registers; the publish unit executes. `01-foundation.md` §4.2 runs this
/// set **twice** on the way to a publish — as the step-2 pre-check when the
/// author submits, and again inside the commit transaction, because approval
/// approves *content* while the commit re-validates *state* and the world moved
/// between the two. No rule may therefore carry anything between runs, and none
/// of the twenty below holds mutable state at all.
#[must_use]
pub fn plan_shape_rules(
    interval_bounds: CustomIntervalBounds,
    descriptors: DescriptorSetComplete,
) -> ValidationPipeline<PlanShape> {
    ValidationPipeline::new()
        // Cycle shape: what commercial shape the plan is.
        .with_rule(Box::new(interval_bounds))
        .with_rule(Box::new(cycle_shape::HybridCompleteness))
        .with_rule(Box::new(cycle_shape::UsageMarketCompleteness))
        .with_rule(Box::new(cycle_shape::SetupRowShape))
        .with_rule(Box::new(cycle_shape::PurchaseQtyRange))
        .with_rule(Box::new(cycle_shape::AvailableFromNotBackdated))
        // Composition: what the plan is made of.
        .with_rule(Box::new(composition::PlanTierDeclared))
        .with_rule(Box::new(composition::MeterInjectivity))
        .with_rule(Box::new(composition::AddonEdgeMembership))
        .with_rule(Box::new(composition::AddonDependencyAcyclic))
        .with_rule(Box::new(composition::AddonConflictBothRequired))
        // Phase schedule: how the plan runs over time.
        .with_rule(Box::new(phase_graph::PhaseGraphIntegrity))
        .with_rule(Box::new(phase_graph::PhaseChainLinear))
        .with_rule(Box::new(phase_graph::TerminalPhaseKind))
        .with_rule(Box::new(phase_graph::PhaseDuration))
        .with_rule(Box::new(phase_graph::PhaseCoverage))
        .with_rule(Box::new(phase_graph::PhaseOverrideBase))
        .with_rule(Box::new(phase_graph::PhaseOverrideUnits))
        .with_rule(Box::new(phase_graph::TerminalPhaseStable))
        // Descriptors: how the plan is billed.
        .with_rule(Box::new(descriptors))
}

#[cfg(test)]
#[path = "plan_rules_tests.rs"]
mod plan_rules_tests;
