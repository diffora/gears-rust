//! `inst-bc-sellability` — a bundle's sellability is the **conjunction over its
//! components** (`design/08-bundles.md` §3, D-94, D-46).
//!
//! A bundle is sellable at `t` iff **every** referenced component key passes the
//! Slice-7 gate's predicates (1)–(5) at `t`, plus the bundle's own
//! `availableFrom`/`availableTo`. One unsellable component makes the bundle
//! unsellable, never partially sellable — which is D-94 applied one level up.
//!
//! # Predicate (6) is excluded, and excluding it is the point
//!
//! The registry `sellable` flag (D-46) applies to the **bundle SKU itself**, not
//! to component references: `sellable = false` components are exactly the
//! composition-only SKUs bundles exist to package, so folding (6) into the
//! component conjunction would make every such bundle permanently unsellable —
//! the flag would refuse the one use it was introduced to enable.
//!
//! The exclusion lives in [`component_verdict`] rather than in the callers,
//! because a caller that simply "did not pass predicate 6" and a caller that
//! forgot to look it up are indistinguishable from here. Passing the whole
//! outcome list and filtering it in one place makes the exemption a fact of this
//! module that a probe can reach.
//!
//! # `sum_of_parts` and `own_price` differ in exactly one input
//!
//! For `sum_of_parts` there are **no own rows** (`inst-bb-rowless`), so the
//! components are the only key inputs. For `own_price` the bundle's own rows must
//! pass **and** the component keys too — the matching-currency component set is
//! part of the offer (`inst-bb-own`, Slice 4 case iii). One parameter, and the
//! basis decides whether it is present.
//!
//! # A bundle referencing nothing is `NotSellable`, not vacuously sellable
//!
//! An empty conjunction is `true` in logic and wrong here, for
//! [`PlanMarketVerdict::NotSellable`]'s own stated reason: *"or the plan binds no
//! key on this market at all"*. A `sum_of_parts` bundle with no components sums
//! nothing and has nothing to sell; publish refuses it anyway
//! ([`crate::domain::bundle_rules`]), and the gate must not disagree with the
//! validator about a state the store can still hold.
//!
//! # The frozen component set is narrowed the same way coverage is
//!
//! The component keys the gate ranges over span `priceEligibility =
//! all_subscriptions` (`cohort = none`) keys **only** — grandfathered generations
//! are never gate inputs. That narrowing is the caller's, exactly as
//! [`crate::domain::bundle_rules`]'s coverage set is, and for the same reason.

use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::bundle::PriceBasis;
use crate::domain::sellability::{PlanMarketVerdict, Predicate, PredicateAnswer, PredicateOutcome};

/// One component's contribution to the conjunction.
#[domain_model]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ComponentSellability {
    /// The component's plan.
    pub component_plan_id: Uuid,
    /// Its verdict over predicates (1)–(5) — see [`component_verdict`].
    pub verdict: PlanMarketVerdict,
}

/// A component's verdict over the predicates a **component reference** answers
/// to: (1)–(5), never (6).
///
/// Takes the whole outcome list and filters it here; see the module doc for why
/// that is not the caller's job.
#[must_use]
pub fn component_verdict(outcomes: &[PredicateOutcome]) -> PlanMarketVerdict {
    fold(
        outcomes
            .iter()
            .filter(|outcome| outcome.predicate != Predicate::RegistrySellable)
            .map(|outcome| &outcome.answer),
    )
}

/// Fold a set of predicate answers into one verdict.
///
/// `Failed` dominates `NotEvaluable` dominates `Satisfied`. The ordering is
/// forced rather than chosen: `NotEvaluable` means *"the gate is not yet a gate"*
/// and a consumer must not read it as sellable, while a `Failed` beside it is a
/// fact that will not improve by waiting — reporting `NotEvaluable` there would
/// tell an operator to wait for a slice that will not change the answer.
fn fold<'a>(answers: impl Iterator<Item = &'a PredicateAnswer>) -> PlanMarketVerdict {
    let mut unevaluable = false;
    for answer in answers {
        match answer {
            PredicateAnswer::Failed { .. } => return PlanMarketVerdict::NotSellable,
            PredicateAnswer::NotEvaluable { .. } => unevaluable = true,
            PredicateAnswer::Satisfied => {}
        }
    }
    if unevaluable {
        PlanMarketVerdict::NotEvaluable
    } else {
        PlanMarketVerdict::Sellable
    }
}

/// The bundle's verdict on one market at one instant.
///
/// `own` is the bundle's **own** key verdict and is `Some` for `own_price` and
/// `None` for `sum_of_parts`; a `sum_of_parts` bundle carrying one anyway is a
/// caller error the type cannot refuse, so the basis is passed alongside and the
/// mismatch is resolved in the direction the design set states — `sum_of_parts`
/// has no own rows, so an own verdict is ignored rather than folded in.
///
/// `bundle_available` is the bundle's own `availableFrom`/`availableTo` answer,
/// which §3 adds to the conjunction explicitly and which no component can carry.
#[must_use]
pub fn bundle_verdict(
    basis: PriceBasis,
    own: Option<PlanMarketVerdict>,
    bundle_available: &PredicateAnswer,
    components: &[ComponentSellability],
) -> PlanMarketVerdict {
    // An empty conjunction is not vacuously true here; see the module doc.
    if components.is_empty() {
        return PlanMarketVerdict::NotSellable;
    }

    let mut verdicts: Vec<PlanMarketVerdict> = components.iter().map(|c| c.verdict).collect();
    if let (PriceBasis::OwnPrice, Some(own)) = (basis, own) {
        verdicts.push(own);
    }

    let mut unevaluable = matches!(bundle_available, PredicateAnswer::NotEvaluable { .. });
    if matches!(bundle_available, PredicateAnswer::Failed { .. }) {
        return PlanMarketVerdict::NotSellable;
    }

    for verdict in verdicts {
        match verdict {
            PlanMarketVerdict::NotSellable => return PlanMarketVerdict::NotSellable,
            PlanMarketVerdict::NotEvaluable => unevaluable = true,
            PlanMarketVerdict::Sellable => {}
        }
    }
    if unevaluable {
        PlanMarketVerdict::NotEvaluable
    } else {
        PlanMarketVerdict::Sellable
    }
}

#[cfg(test)]
#[path = "bundle_sellability_tests.rs"]
mod bundle_sellability_tests;
