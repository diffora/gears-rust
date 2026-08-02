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
//! # What is deliberately absent
//!
//! **The §14 soft caps.** §1.2's NFR allocation names "publish-time size
//! validation (100 bands/row, 500 rows/plan soft)", `AuthoringPolicy` already
//! exposes both numbers per tenant, and nothing reads them. PRD §14 says the
//! system **SHOULD** enforce them "emitting a publish **warning** above the cap"
//! — an advisory, not a violation — and **no document names an advisory code for
//! it**. `TIER_BAND_PRICE_INCREASE` shows what such a code looks like when the
//! set provides one. This group mints none: a `warn` under an invented code puts
//! a discriminator on the wire that no consumer can act on and no document
//! defines. Implemented as nothing, and **reported as a gap**.
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
}

impl PublishRuleParams {
    /// Bind the rule set to one tenant's configuration.
    #[must_use]
    pub const fn new(
        interval_bounds: CustomIntervalBounds,
        descriptors: DescriptorSetComplete,
        default_rounding_policy: Option<String>,
    ) -> Self {
        Self {
            interval_bounds,
            descriptors,
            default_rounding_policy,
        }
    }

    /// The tenant's default rounding policy, when it has one.
    #[must_use]
    pub fn default_rounding_policy(&self) -> Option<&str> {
        self.default_rounding_policy.as_deref()
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
    ValidationPipeline::new().with_rule(Box::new(RoundingPolicyResolved {
        tenant_default: params.default_rounding_policy.clone(),
    }))
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
