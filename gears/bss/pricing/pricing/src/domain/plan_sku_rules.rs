//! The plan's **own** SKU, judged independently of the rows it carries.
//!
//! [`crate::domain::row_sku_rules`] judges the SKU a *charge line* prices. This
//! module judges the SKU the *plan itself is sold as* — a different binding,
//! answered at a different moment, and until now answered by nothing.
//!
//! # Why the row rules could not carry this
//!
//! `inst-pr-sku-role` exempts the plan's own SKU by identity: a row may
//! name it whatever it is. That exemption is correct and it is also a blind
//! spot, because the exempting rule never looks at the plan SKU itself. Two
//! shapes reach publish through it untouched: a plan with **no rows at all**,
//! which the row rules never visit, and a plan whose every row prices some
//! component, where the plan SKU is never a subject. Both would publish a plan
//! sold as a component or as a bundle SKU with nothing composed.
//!
//! # The role is read alone
//!
//! `sellable` is deliberately not consulted here. An offer closed for new sales
//! is a legitimate plan binding — its prices stay valid, its existing
//! subscriptions keep billing, and an author must be able to maintain it.
//! Whether the plan may be **sold** is the registry's current permission, asked
//! by the sellability surface against the whole frozen sale set. Folding the
//! flag into this rule would refuse the authoring of exactly the catalogue an
//! operator closes sales on in order to rework.

#[cfg(test)]
#[path = "plan_sku_rules_tests.rs"]
mod plan_sku_rules_tests;

use crate::domain::registry_view::SkuIndex;
use crate::domain::rules::{PLAN_SKU_DEPRECATED, PLAN_SKU_TYPE_INVALID, SKU_NOT_PUBLISHED};
use crate::domain::scope_key::SkuId;
use crate::domain::validation::ValidationReport;

/// The role an ordinary plan's own SKU must carry.
pub const ROLE_OFFER: &str = "offer";

/// The role a plan that carries a bundle composition must carry.
pub const ROLE_BUNDLE: &str = "bundle";

/// The role a SKU must carry to be priced by a charge line that is not its
/// plan's own. Declared here beside its two siblings so the closed set reads in
/// one place; the rule that spends it is
/// [`RowSkuRole`](crate::domain::row_sku_rules::RowSkuRole).
pub const ROLE_COMPONENT: &str = "component";

/// The registry `status` a plan's own SKU must be in.
const PUBLISHED: &str = "published";

/// Judge one plan's own SKU binding.
///
/// `expected_role` is the context's, not the plan's: [`ROLE_OFFER`] for an
/// ordinary plan and [`ROLE_BUNDLE`] where a composition is attached. The caller
/// decides which, from the composition it can see — never from a field the
/// author wrote.
///
/// `introducing` is true when this evaluation **binds** the SKU: a create, a
/// change of binding, a clone, or a first publish of a draft that carries one.
/// It is false for a plan already published against this SKU, which keeps its
/// binding even if the registry has since deprecated it.
///
/// One fault, one violation. A SKU that cannot be read is reported as absent and
/// nothing else is said about it — a role refusal about a SKU the registry does
/// not have would send the author to correct a value that is not the problem.
#[must_use]
pub fn validate_plan_sku(
    sku_id: SkuId,
    index: &SkuIndex,
    expected_role: &str,
    introducing: bool,
) -> ValidationReport {
    let mut report = ValidationReport::default();
    let Some(sku) = index.get(sku_id) else {
        report.violate_at_write(
            SKU_NOT_PUBLISHED,
            "sku_id",
            format!("SKU {sku_id} is not in the registry read model"),
        );
        return report;
    };
    if sku.status != PUBLISHED {
        report.violate_at_write(
            SKU_NOT_PUBLISHED,
            "sku_id",
            format!("SKU {sku_id} is {}", sku.status),
        );
        return report;
    }
    if sku.sku_type != expected_role {
        report.violate_at_write(
            PLAN_SKU_TYPE_INVALID,
            "sku_id",
            format!(
                "expected {expected_role} SKU, received {}",
                // An unknown token fails here like any other mismatch: a
                // non-Products catalog provider may return one, and this rule
                // admits exactly the role the context asks for.
                sku.sku_type
            ),
        );
    }
    if introducing && sku.deprecated {
        report.violate_at_write(
            PLAN_SKU_DEPRECATED,
            "sku_id",
            format!("SKU {sku_id} is deprecated and may not be newly bound"),
        );
    }
    report
}

/// Judge a plan's own SKU at **creation**, before any composition exists.
///
/// A bundle is staged through ordinary plan creation: the draft is made first
/// and the composition attached afterwards, so at this moment the caller cannot
/// know which of the two a plan is going to be. Both roles are therefore
/// admitted here and the context decides later — at the attach, which requires
/// [`ROLE_BUNDLE`], and at an ordinary publish, which requires [`ROLE_OFFER`].
/// A bundle-bound draft may sit without a composition; it simply cannot publish
/// as an ordinary plan.
///
/// `component` is refused outright: no sequence of later acts makes a
/// constituent into something a plan is sold as, so admitting it here would only
/// defer the refusal to a point where the author has built more on top of it.
#[must_use]
pub fn validate_plan_sku_at_create(
    sku_id: SkuId,
    index: &SkuIndex,
    introducing: bool,
) -> ValidationReport {
    let offer = validate_plan_sku(sku_id, index, ROLE_OFFER, introducing);
    if offer.violations.is_empty() {
        return offer;
    }
    let bundle = validate_plan_sku(sku_id, index, ROLE_BUNDLE, introducing);
    if bundle.violations.is_empty() {
        return bundle;
    }
    // Neither role fits. Report the offer reading: it is the ordinary case, and
    // its message names the role the author most likely meant to bind. Both
    // readings agree on every violation that is not the role itself, so nothing
    // is lost by choosing one.
    offer
}
