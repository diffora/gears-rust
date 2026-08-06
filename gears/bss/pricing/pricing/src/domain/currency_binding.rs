//! `CurrencyBindingChecker` — `design/04-currency-tax.md` §3's
//! single-currency-per-invoice binding, case (i) (`inst-cb-addon`).
//!
//! # The type D-211 says does not exist
//!
//! D-211 (2026-08-06) records that *"`CurrencyBindingChecker` is Slice 4's and
//! **does not exist yet**, so S8's coverage walk owns **both** axes until it
//! lands, at which point the currency arm delegates and the region arm stays
//! S8's"*. This is the type landing. The delegation itself is **not** performed
//! here: `domain::bundle_rules` and `infra::bundle` belong to another strand, so
//! [`uncovered_pairs`] is exposed as the shared predicate S8's walk can call, and
//! the swap is reported as owed rather than made.
//!
//! Cases (ii) and (iii) — `sum_of_parts` and `own_price` bundle coverage — are
//! **already enforced**, by `bundle_rules::check_coverage`, over both axes. So
//! this module deliberately builds only case (i): building a second bundle
//! coverage walk here would be the duplication D-211's ordering note exists to
//! prevent, and it would run against a composition this plane does not hold.
//!
//! # Case (i) is per `(currency, region)` **pair**, not per currency (D-95)
//!
//! The currency-only reading left the region axis unchecked: *"a required add-on
//! covering EUR only in `US` while the base sells EUR in `EU` passed publish and
//! died at order assembly"*. So the domain here is the set of pairs the base
//! plan sells, and coverage is asked of the pair.
//!
//! # And over the `depends_on` **closure**, not the flat required set (C-3)
//!
//! §3 step 1's 2026-08-01 fix: a required add-on may declare `depends_on` on an
//! *optional* one, which is then transitively mandatory at order time — while
//! under the flat reading its coverage was never checked. That is the same D-95
//! asymmetry arriving through the dependency door and ending at the same
//! order-assembly failure on a plan that published clean.
//!
//! An **optional** add-on that nothing required depends on is deliberately out of
//! scope: its gap does not block the base plan's publish and is enforced at
//! attachment instead (2026-07-28 review fix, confirmed 2026-07-31). That is not
//! leniency — it is what keeps a tenant able to publish a plan with a promotional
//! add-on that only covers two of its twenty markets.
//!
//! # What this rule cannot see, and says so
//!
//! `inst-cb-boundary`: currency **selection** at activation is Subscriptions',
//! and `invoiceGroupingKey` is a layout hint that MUST NOT override the
//! invariant. The second half is enforceable here only in the negative — this
//! walk never reads the key — and `plan_shape` carries it as "shape-checked only".
//! `inst-cb-region-binding` is joint with Subscriptions and binds at activation,
//! which is not a publish-time property of a plan at all.

use std::collections::{BTreeMap, BTreeSet};

use uuid::Uuid;

use toolkit_macros::domain_model;

use crate::domain::money::CurrencyCode;
use crate::domain::plan_shape::PlanShape;
use crate::domain::scope_key::{PriceEligibility, Region};
use crate::domain::validation::{ValidationReport, ValidationRule};

/// A component that does not cover a market the thing selling it sells
/// (§5, 422 — the refusal names the component **and** the market).
///
/// The same code `bundle_rules` raises for cases (ii)/(iii), and deliberately so:
/// §5 declares one code for all three enumerated configurations, because the
/// operator's remedy is the same in each — author the missing row.
pub const CURRENCY_NOT_COVERED: &str = "CURRENCY_NOT_COVERED";

/// One `(currency, region)` market.
pub type Market = (CurrencyCode, Region);

/// What a plan's add-on composition looks like to this rule, as the caller
/// resolved it.
///
/// Handed in for `RoundingPolicyResolved`'s reason: the coverage of *another*
/// plan's rows is a storage read, the rule set runs twice on one publish, and a
/// rule that fetched its own inputs could answer differently in the two runs.
#[domain_model]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddonCoverage {
    /// Per add-on SKU: the markets it publishes a covering row on.
    ///
    /// An add-on absent from this map covers **nothing**, which is the
    /// fail-closed reading and the right one: an add-on the caller could not
    /// resolve is not an add-on whose coverage may be assumed.
    by_addon: BTreeMap<Uuid, BTreeSet<Market>>,
}

impl AddonCoverage {
    /// Build from the resolved per-add-on market sets.
    #[must_use]
    pub fn new(by_addon: BTreeMap<Uuid, BTreeSet<Market>>) -> Self {
        Self { by_addon }
    }

    /// The empty coverage — every add-on covers nothing, so every **required**
    /// one blocks.
    ///
    /// A `const fn` and not `Default::default`, because `PublishRuleParams::new`
    /// is `const` and must seed the fail-closed state without a caller.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            by_addon: BTreeMap::new(),
        }
    }

    /// The markets one add-on covers. Absent reads as none.
    #[must_use]
    pub fn markets_of(&self, addon: Uuid) -> BTreeSet<Market> {
        self.by_addon.get(&addon).cloned().unwrap_or_default()
    }
}

/// The markets `sold` that `covered` does not reach — the shared predicate.
///
/// **Exposed so Slice 8's coverage walk can delegate its currency arm to it**
/// when D-211's ordering note is acted on. It answers over pairs, which is a
/// superset of what that arm needs: S8 keeps the region axis, and a pair-wise
/// answer restricted to one region is exactly a currency answer.
#[must_use]
pub fn uncovered_pairs(sold: &BTreeSet<Market>, covered: &BTreeSet<Market>) -> Vec<Market> {
    sold.difference(covered).cloned().collect()
}

/// The markets a plan's **candidate rows** sell on.
///
/// `all_subscriptions` and `new_subscriptions_only` only — grandfathered
/// generations are never coverage candidates (ADR-0002, and the narrowing
/// `inst-bc-coverage` states for the bundle plane). A market a plan reaches only
/// through a frozen generation is not a market it is selling.
#[must_use]
pub fn sold_markets(shape: &PlanShape) -> BTreeSet<Market> {
    shape
        .rows
        .iter()
        .filter(|record| {
            record.scope_key.price_eligibility() != PriceEligibility::ExistingGrandfathered
        })
        .map(|record| {
            (
                record.scope_key.currency().clone(),
                record.scope_key.region().clone(),
            )
        })
        .collect()
}

/// `inst-cb-addon` case (i): every **transitively required** add-on covers every
/// market the base plan sells.
#[domain_model]
#[derive(Clone, Debug, Default)]
pub struct RequiredAddonsCoverMarkets {
    /// The resolved coverage of each add-on this plan composes.
    pub coverage: AddonCoverage,
}

impl ValidationRule<PlanShape> for RequiredAddonsCoverMarkets {
    fn name(&self) -> &'static str {
        "inst-cb-addon"
    }

    fn evaluate(&self, subject: &PlanShape, report: &mut ValidationReport) {
        let sold = sold_markets(subject);
        if sold.is_empty() {
            // A plan with no candidate row sells nothing, so nothing has to be
            // covered. Reported as no finding rather than as "everything is
            // uncovered", which is what an empty-domain rule answers if it walks
            // the components instead of the markets.
            return;
        }

        for addon in mandatory_closure(subject) {
            let covered = self.coverage.markets_of(addon);
            let missing = uncovered_pairs(&sold, &covered);
            if missing.is_empty() {
                continue;
            }
            // §5 requires the refusal to name the component **and** the market.
            // Both, and every missing market rather than the first: an operator
            // authoring rows for a plan at C1's 20-currency floor otherwise
            // discovers the gaps one publish at a time.
            report.violate(
                CURRENCY_NOT_COVERED,
                addon.to_string(),
                format!(
                    "required add-on {addon} publishes no covering row on {}, which this plan \
                     sells. A subscription bound to that market could not resolve all of its \
                     lines, and an invoice may not mix currencies (D-95: the check is per \
                     (currency, region) pair, not per currency)",
                    missing
                        .iter()
                        .map(|(currency, region)| format!("{}/{region}", currency.as_str()))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            );
        }
    }
}

/// Every add-on that is mandatory at order time: the `depends_on` closure of the
/// declared-required set (C-3).
///
/// **The closure, not the flat set.** A required add-on may declare `depends_on`
/// on an optional one, which is then transitively mandatory — and under the flat
/// reading its coverage was never checked. Walked iteratively with a seen-set, so
/// a dependency cycle terminates: cycles are `ADDON_CYCLE`'s to refuse and this
/// rule must not hang on a plan another rule is about to reject.
#[must_use]
fn mandatory_closure(shape: &PlanShape) -> BTreeSet<Uuid> {
    let by_sku: BTreeMap<Uuid, &Vec<Uuid>> = shape
        .addon_rules
        .iter()
        .map(|rule| (rule.addon_sku_id, &rule.depends_on))
        .collect();

    let mut mandatory: BTreeSet<Uuid> = BTreeSet::new();
    let mut frontier: Vec<Uuid> = shape
        .addon_rules
        .iter()
        .filter(|rule| rule.required)
        .map(|rule| rule.addon_sku_id)
        .collect();

    while let Some(addon) = frontier.pop() {
        if !mandatory.insert(addon) {
            continue;
        }
        if let Some(dependencies) = by_sku.get(&addon) {
            frontier.extend(dependencies.iter().copied());
        }
    }
    mandatory
}

#[cfg(test)]
#[path = "currency_binding_tests.rs"]
mod currency_binding_tests;
