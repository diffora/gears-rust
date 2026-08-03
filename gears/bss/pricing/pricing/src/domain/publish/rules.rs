//! The **aggregate** publish rule set: one report over one publish subject.
//!
//! Until this module the rules existed as disconnected pipelines — thirteen
//! Slice-3 rules over a [`PriceRow`], twenty Slice-2 rules over a [`PlanShape`],
//! one supersession guard over a pair — each registered and none of them run by
//! anything but a test and a dev-only conformance validator.
//! [`run_publish_rules`] is what §4.2 step 2 and step 4 actually call, and it is
//! the same call in both: approval approves *content*, the commit re-validates
//! *state*, and the only way those two runs can be compared is if they are the
//! same function over the same subject.
//!
//! # Order is a reading order, never a short-circuit
//!
//! [`ValidationPipeline::run`] runs every rule and the aggregate report is the
//! product — an author remediates a plan in one pass. What the order decides is
//! how the report **reads**, and it is fixed here: the per-row rules first, then
//! the plan-shape set. That is the conformance validator's own argument
//! (`examples/regen_registry/validator.rs`): a malformed row is malformed
//! regardless of the plan it sits in, while a plan-level rule is a statement
//! *about a set of rows* and reads oddly ahead of the news that one of them is
//! not a row at all.
//!
//! Within the row half the Foundation's own rule runs after Slice 3's, because
//! Slice 3 answers "is this a row" and the rounding rule answers "does this row
//! resolve the policy every charge's last minor unit depends on" — the second
//! question presupposes the first.
//!
//! # `supersession_rules` is not run here, and that is not an oversight
//!
//! It takes a `SupersessionPair`, and this publish unit produces no successor on
//! an occupied key: `PriceRepo::create_draft` refuses a scope key already held
//! by a `draft` **or** `published` row, so no draft row can be authored onto an
//! occupied key and nothing can set `supersedes_price_id`. The two sanctioned
//! producers of that flip — the D-88 supersession unit and the D-100 cutover —
//! are Slice 7's and neither exists. The guard therefore has no pair to judge in
//! this group, and running it over a fabricated one would be a rule reporting on
//! a subject nobody authored.
//!
//! # The Foundation's base set: what is enforced structurally, and what was
//! missing
//!
//! [`crate::domain::validation`] says the Foundation "registers the money and
//! rounding rules itself rather than leaving the base set to whichever slice
//! happens to load first", and §3.3 says `ROUNDING_POLICY_UNRESOLVED` is
//! "registered into the pipeline by the Foundation itself". Of those:
//!
//! - **The money rules need no pipeline rule, because they are unrepresentable
//!   failures.** `MinorAmount::new` refuses a negative (`AMOUNT_NEGATIVE`),
//!   `CurrencyCode::new` refuses a non-ISO-4217 code (`CURRENCY_INVALID`), and
//!   `check_scale` / `check_decimal` carry `PRECISION_EXCEEDED`. A [`PriceRow`]
//!   cannot hold a violation of any of them, so a rule here would be a rule with
//!   nothing to reject — and a rule that always passes is indistinguishable from
//!   a rule that holds. The absence is a decision, recorded so it reads as one.
//! - **The rounding rule was genuinely missing.** `DomainError::RoundingPolicyUnresolved`
//!   existed and was mapped, `pricing_price.rounding_policy_ref` existed,
//!   `pricing_policy_object.default_rounding_policy_ref` existed — and **no rule
//!   read any of them**, so a row that resolved neither would have published.
//!   [`RoundingPolicyResolved`] is that rule.
//!
//! # The §14 soft caps are an advisory, and the code is now the set's
//!
//! §1.2's NFR allocation names "publish-time size validation (100 bands/row, 500
//! rows/plan soft)" and PRD §14 says the system **SHOULD** enforce them "emitting
//! a publish **warning** above the cap". An earlier version of this doc reported
//! the whole thing as a gap for one reason and one only: no document named an
//! advisory code. D-160 named it — `PLAN_SIZE_SOFT_CAP_EXCEEDED`, in
//! `01-foundation.md` §3.3 — so [`PlanSizeWithinSoftCaps`] is the rule.
//!
//! **It never blocks.** The finding rides `warnings[]`, the channel
//! `TIER_BAND_PRICE_INCREASE` already uses, and `is_publishable()` does not read
//! it. Making the caps blocking so they could reuse a violation code is D-160's
//! rejected option (b): a ratified `SHOULD` enforced as a `MUST` is a plan an
//! operator could author yesterday and cannot publish today.
//!
//! **The two *interval* caps are a different thing and stay hard.** §1.2's
//! allocation row had blurred all four under one heading; D-160 unblurs them.
//! `customEveryN` bounds fail publish under
//! [`INVALID_CUSTOM_INTERVAL`](crate::domain::plan_rules::INVALID_CUSTOM_INTERVAL)
//! and this rule must never reach them — a soft advisory over a hard cap would
//! tell an author their over-cap interval published.
//!
//! # What is deliberately absent
//!
//! **The fixture gate is not a rule and is not run here.** `FixtureGate::check`
//! returns `DomainError::FixtureMissing` and stays a separate fail-closed step
//! beside the report; [`crate::infra::publish`] is where the two meet. Two
//! reasons. It is not a statement about the plan but about what this
//! deployment's joint corpus has agreed to evaluate, so its remedy is not an
//! edit the author can make — which is the whole difference between a violation
//! and a refusal. And synthesizing a violation from it needs a code, which this
//! group will not mint on its own authority.

use toolkit_macros::domain_model;

use crate::domain::plan_rules::{CustomIntervalBounds, DescriptorSetComplete, plan_shape_rules};
use crate::domain::plan_shape::PlanShape;
use crate::domain::rules::price_row_rules;
use crate::domain::scope_key::PriceEligibility;
use crate::domain::validation::{ValidationPipeline, ValidationReport, ValidationRule};

/// A published row resolves neither its own `rounding_policy_ref` nor a tenant
/// default (`01-foundation.md` §3.3, PRD §17.4).
///
/// The code is §3.3's verbatim. **It has two homes in the design set and that is
/// reported rather than resolved here**: `DomainError::RoundingPolicyUnresolved`
/// is a mapped rejection of its own, and this is a `Violation` inside the
/// `ValidationFailed` envelope. The pipeline rendering is the right one — §17.4
/// calls it a validation rule and the report must enumerate every finding in one
/// pass, which a single-refusal error cannot do — but the variant is mapped and
/// may have a non-pipeline caller, so it is not deleted.
pub const ROUNDING_POLICY_UNRESOLVED: &str = "ROUNDING_POLICY_UNRESOLVED";

/// A plan or a row above one of the two **soft** size caps (`nfr-size-limits`,
/// D-160; declared in `01-foundation.md` §3.3).
///
/// **Advisory, never blocking**, and that is ratified rather than chosen: PRD §14
/// makes the caps a `SHOULD` that emits a publish warning. It rides `warnings[]`
/// and `is_publishable()` never reads it.
///
/// One code for both caps, by D-146's line: the operator's next action is the
/// same shape — shorten the ladder, or split the plan — and the report says which
/// cap and by how much. The two **interval** caps are hard and keep
/// [`INVALID_CUSTOM_INTERVAL`](crate::domain::plan_rules::INVALID_CUSTOM_INTERVAL);
/// §1.2's allocation row had blurred all four under one heading and D-160 unblurs
/// them.
pub const PLAN_SIZE_SOFT_CAP_EXCEEDED: &str = "PLAN_SIZE_SOFT_CAP_EXCEEDED";

/// A grandfathering horizon on a row whose eligibility class may not carry one
/// (S7 §5, D-147).
///
/// **The wire code already existed and the publish-side rule did not.** The
/// refusal is enforced on both authoring paths by
/// [`price_repo`](crate::infra::storage::repo::price_repo), which pre-empts the
/// column `CHECK`'s 500 and reaches the caller as this code through
/// `DomainError::GrandfatherUntilForbidden`. What was missing is the **registered
/// rule**: without it, a publish carrying such a row is refused only because
/// whichever authoring write happened to reach the store first refused, and the
/// 422 report never enumerates it beside the plan's other violations.
///
/// The two are **not** redundant. The repository guard is the guarantee — a
/// predicate on a statement, which is the only thing that holds against a caller
/// reaching the store by another path — and this rule is the report line an author
/// acts on. Deleting either leaves a hole the other does not cover.
pub const GRANDFATHER_UNTIL_FORBIDDEN: &str = "GRANDFATHER_UNTIL_FORBIDDEN";

/// The per-tenant configuration the publish rule set is run under.
///
/// Held as **fields** rather than looked up inside a rule, for the reason
/// [`crate::domain::validation`] states: the set runs twice and a rule that
/// reached for configuration itself could answer differently in the two runs for
/// no authored reason. It is also what keeps a storage read out of the domain.
///
/// There is deliberately no `Default`. [`CustomIntervalBounds`] has none either,
/// and for the sharper version of the same reason — zero caps reject every
/// custom frequency ever authored while looking exactly like a rule that is
/// switched on.
#[domain_model]
#[derive(Clone, Debug)]
pub struct PublishRuleParams {
    interval_bounds: CustomIntervalBounds,
    descriptors: DescriptorSetComplete,
    default_rounding_policy: Option<String>,
    size_caps: SoftSizeCaps,
}

impl PublishRuleParams {
    /// Bind the rule set to one tenant's configuration.
    #[must_use]
    pub const fn new(
        interval_bounds: CustomIntervalBounds,
        descriptors: DescriptorSetComplete,
        default_rounding_policy: Option<String>,
        size_caps: SoftSizeCaps,
    ) -> Self {
        Self {
            interval_bounds,
            descriptors,
            default_rounding_policy,
            size_caps,
        }
    }

    /// The tenant's default rounding policy, when it has one.
    #[must_use]
    pub fn default_rounding_policy(&self) -> Option<&str> {
        self.default_rounding_policy.as_deref()
    }
}

/// The two **soft** size caps in force for the authoring tenant (D-152, D-160).
///
/// A type rather than two loose numbers because they are read together, reported
/// under one code, and must not be confused with the two **interval** caps
/// [`CustomIntervalBounds`] carries — those are hard, and §1.2's allocation row
/// had blurred all four under one heading until D-160.
///
/// Deliberately no `Default`, for [`CustomIntervalBounds`]' reason: zero caps
/// would warn on every plan ever authored while looking exactly like a rule that
/// is switched on.
#[domain_model]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoftSizeCaps {
    max_tier_bands_per_row: u32,
    max_price_rows_per_plan: u32,
}

impl SoftSizeCaps {
    /// Bind the caps to the values in force.
    #[must_use]
    pub const fn new(max_tier_bands_per_row: u32, max_price_rows_per_plan: u32) -> Self {
        Self {
            max_tier_bands_per_row,
            max_price_rows_per_plan,
        }
    }

    /// The tier-band cap per price row.
    #[must_use]
    pub const fn max_tier_bands_per_row(self) -> u32 {
        self.max_tier_bands_per_row
    }

    /// The price-row cap per plan.
    #[must_use]
    pub const fn max_price_rows_per_plan(self) -> u32 {
        self.max_price_rows_per_plan
    }
}

/// Every rule a publish of `shape` must pass, in one report.
///
/// Runs the Slice-3 row rules over each candidate row, the Foundation's own
/// rounding rule, and the Slice-2 plan-shape set — every one of them, absorbing
/// each report into the aggregate. A non-empty `violations` blocks the publish
/// wherever the finding appeared.
///
/// The pipelines are rebuilt per call rather than held: one built per tenant
/// would enforce whichever tenant's limits it was built for, and one held across
/// time would keep rejecting plans after an operator had raised a cap.
#[must_use]
pub fn run_publish_rules(shape: &PlanShape, params: &PublishRuleParams) -> ValidationReport {
    let mut report = ValidationReport::default();

    let row_rules = price_row_rules();
    for record in &shape.rows {
        report.absorb(row_rules.run(&record.row));
    }
    report.absorb(foundation_plan_rules(params).run(shape));
    report.absorb(plan_shape_rules(params.interval_bounds, params.descriptors.clone()).run(shape));
    report
}

/// The Foundation's own rules over a publish subject.
///
/// One rule today. It is a pipeline rather than a bare call so that the next
/// Foundation-owned rule registers beside it instead of being appended to a
/// slice's set — which is exactly the "whichever slice happens to load first"
/// outcome §4.2 keeps the base set out of.
#[must_use]
fn foundation_plan_rules(params: &PublishRuleParams) -> ValidationPipeline<PlanShape> {
    ValidationPipeline::new()
        .with_rule(Box::new(RoundingPolicyResolved {
            tenant_default: params.default_rounding_policy.clone(),
        }))
        .with_rule(Box::new(PlanSizeWithinSoftCaps {
            caps: params.size_caps,
        }))
        .with_rule(Box::new(GrandfatherHorizonOnItsClass))
}

/// Every published row must resolve a rounding policy — its own, or the
/// tenant's.
///
/// Rounding decides the last minor unit of **every** charge computed from the
/// row, so an unresolved policy is not a missing nicety: it is a plan whose
/// prices are exact to within a unit nobody agreed on, and downstream would have
/// to pick a mode to compute anything at all. The design set's answer is to fail
/// publish rather than to pick one (PRD §17.4's no-implicit-rounding rule), and
/// the resolved id then freezes into the read model and the snapshot.
///
/// The tenant default is a **field**, resolved by the caller from
/// `pricing_policy_object` — see [`PublishRuleParams`] for why it is not read
/// here.
///
/// Reported **once per row**, naming the row, because that is the edit: the
/// author sets `rounding_policy_ref` on the rows that lack one, or an operator
/// configures the tenant default and every one of them resolves at once.
#[domain_model]
#[derive(Clone, Debug)]
struct RoundingPolicyResolved {
    /// The tenant's `default_rounding_policy_ref`, when they have configured
    /// one. `None` is the fail-closed reading: every row then carries its own or
    /// the plan does not publish.
    tenant_default: Option<String>,
}

impl ValidationRule<PlanShape> for RoundingPolicyResolved {
    fn name(&self) -> &'static str {
        "foundation.rounding_policy_resolved"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        if self.tenant_default.is_some() {
            return;
        }
        for record in &subject.rows {
            if record.rounding_policy_ref.is_none() {
                report.violate(
                    ROUNDING_POLICY_UNRESOLVED,
                    record.price_id.to_string(),
                    "this row carries no roundingPolicyRef and the tenant has no default \
                     rounding policy; rounding decides the last minor unit of every charge, \
                     so publish stops rather than choosing one",
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "rules_tests.rs"]
mod rules_tests;

/// The §14 soft size caps, reported and never enforced (`nfr-size-limits`,
/// D-160).
///
/// Two caps, one code, `warnings[]` only:
///
/// - **price rows per plan** — a plan above the cap is one whose publish, read
///   model delta and snapshot are all proportional to a number nobody bounded;
/// - **tier bands per row** — a ladder above the cap is one every rating call
///   walks.
///
/// **Why this is an advisory and not a violation.** PRD §14 ratifies both as a
/// `SHOULD`. Making them blocking would let a raised-then-lowered cap turn a
/// published plan into one its own tenant cannot re-publish, and it would reject
/// a legitimately large catalogue with no remedy but to split it — D-160's
/// rejected option (b), taken there rather than here.
///
/// **The caps are the authoring tenant's** (D-152), resolved by the caller from
/// `pricing_policy_object` and held as a **field**: a rule that read
/// configuration inside `evaluate` could answer differently in §4.2's two runs
/// for no authored reason. A tenant with no policy row takes the ratified
/// deployment default, which is what makes the number in force readable at all.
///
/// The report names **which** cap, **its limit** and **the count**, because those
/// three are the whole of the operator's next action: shorten this ladder by six
/// bands, or split this plan.
#[domain_model]
#[derive(Clone, Copy, Debug)]
pub struct PlanSizeWithinSoftCaps {
    /// The caps in force for the authoring tenant.
    pub caps: SoftSizeCaps,
}

impl ValidationRule<PlanShape> for PlanSizeWithinSoftCaps {
    fn name(&self) -> &'static str {
        "nfr-size-limits"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        let rows = u32::try_from(subject.rows.len()).unwrap_or(u32::MAX);
        let row_cap = self.caps.max_price_rows_per_plan();
        if rows > row_cap {
            report.warn(
                PLAN_SIZE_SOFT_CAP_EXCEEDED,
                subject.subject(),
                format!(
                    "the plan carries {rows} price rows, above the soft cap of {row_cap} rows per \
                     plan: the publish, the read-model delta and the snapshot are all \
                     proportional to this number. Advisory - the publish proceeds"
                ),
            );
        }

        let band_cap = self.caps.max_tier_bands_per_row();
        for record in &subject.rows {
            let bands = u32::try_from(record.row.bands.len()).unwrap_or(u32::MAX);
            if bands <= band_cap {
                continue;
            }
            report.warn(
                PLAN_SIZE_SOFT_CAP_EXCEEDED,
                format!("{}|{}", subject.subject(), record.price_id),
                format!(
                    "row {} carries {bands} tier bands, above the soft cap of {band_cap} bands \
                     per row: every rating call walks the ladder. Advisory - the publish proceeds",
                    record.price_id
                ),
            );
        }
    }
}

/// A grandfathering horizon belongs only to an `existing_grandfathered` row
/// (S7 §5, D-147).
///
/// **Why the subject is the plan and not the row.** `grandfather_until` lives on
/// [`PriceRecord`](crate::domain::price_record::PriceRecord), not on
/// [`PriceRow`](crate::domain::price_row::PriceRow), so
/// [`price_row_rules`](crate::domain::rules::price_row_rules) cannot see it —
/// and widening `PriceRow` to reach it would move a field across the D-162
/// roster boundary and imply a generation bump for something that is not
/// evaluation policy at all. The eligibility class it pairs with is on the
/// **scope key**, one level up again. So the rule's subject is the plan shape,
/// which already carries both.
///
/// **It is registered in the aggregate's own pipeline rather than in Slice 2's.**
/// The rule is S7's, and `plan_rules::plan_shape_rules` is the Slice-2 set by its
/// own doc — a rule filed there would make Slice 2 the owner of a Slice-7
/// sentence. This is the same reason [`RoundingPolicyResolved`] sits here.
///
/// **What it does not duplicate.** The repository refuses the same pairing on
/// both authoring paths and pre-empts the column `CHECK`'s 500 there. That guard
/// is the guarantee and this is the report: an author who submits a plan with
/// three faults gets three lines, one of them this one, instead of discovering it
/// one write at a time.
#[domain_model]
#[derive(Clone, Copy, Debug, Default)]
pub struct GrandfatherHorizonOnItsClass;

impl ValidationRule<PlanShape> for GrandfatherHorizonOnItsClass {
    fn name(&self) -> &'static str {
        "inst-el-fields"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        for record in &subject.rows {
            let Some(horizon) = record.grandfather_until else {
                continue;
            };
            let class = record.scope_key.price_eligibility();
            if class == PriceEligibility::ExistingGrandfathered {
                continue;
            }
            report.violate(
                GRANDFATHER_UNTIL_FORBIDDEN,
                format!("{}|{}", subject.subject(), record.price_id),
                format!(
                    "row {} is filed under priceEligibility {class} and carries a grandfathering \
                     horizon of {horizon}: the horizon says when a retained cohort stops being \
                     retained, and a row that retains nobody has no cohort for it to end",
                    record.price_id
                ),
            );
        }
    }
}
