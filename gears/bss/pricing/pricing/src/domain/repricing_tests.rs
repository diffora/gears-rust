//! The selector's two asymmetries — the ones a reader would get backwards.

use super::{RUN_SELECTOR_EMPTY, RunSelector};
use crate::domain::money::CurrencyCode;
use crate::domain::scope_key::{Cohort, PriceEligibility};

#[test]
fn a_selector_naming_nothing_is_unconstrained_and_one_naming_any_axis_is_not() {
    assert!(RunSelector::default().is_unconstrained());

    let one_axis = RunSelector {
        currency: Some(CurrencyCode::new("EUR").expect("three letters")),
        ..RunSelector::default()
    };
    assert!(
        !one_axis.is_unconstrained(),
        "section 2's `a currency segment` is one axis, and naming it constrains the run"
    );
}

#[test]
fn the_cohort_axis_tells_unconstrained_from_the_classless_value() {
    // The distinction the wire surface cannot express and this type must: the
    // outer `None` is *the run does not constrain the axis*, the inner one is
    // *rows that retain nobody*. A reader who conflated them would build a
    // selector that quietly narrowed every run to the non-grandfathered rows and
    // would never see it, because that is also what the default exclusion does.
    let unconstrained = RunSelector::default();
    let classless = RunSelector {
        cohort: Some(Cohort::None),
        ..RunSelector::default()
    };

    assert!(unconstrained.is_unconstrained());
    assert!(!classless.is_unconstrained());
    assert_ne!(unconstrained, classless);
}

#[test]
fn only_an_explicit_grandfathered_eligibility_admits_that_class() {
    // `inst-mp-grandfathered` clause 1. The absent axis is the one place an
    // unconstrained axis narrows the run instead of widening it, so all three
    // named values are asserted rather than the one: a `matches!` written against
    // the wrong variant would still pass a test that only checked the default.
    assert!(!RunSelector::default().admits_grandfathered());

    for eligibility in [
        PriceEligibility::AllSubscriptions,
        PriceEligibility::NewSubscriptionsOnly,
    ] {
        let selector = RunSelector {
            price_eligibility: Some(eligibility),
            ..RunSelector::default()
        };
        assert!(
            !selector.admits_grandfathered(),
            "{eligibility:?} is not the retained class"
        );
    }

    let explicit = RunSelector {
        price_eligibility: Some(PriceEligibility::ExistingGrandfathered),
        ..RunSelector::default()
    };
    assert!(
        explicit.admits_grandfathered(),
        "naming the class is how an operator asks for it, and clause 2 then owes them a per-row \
         refusal rather than a set that silently shrank"
    );
}

#[test]
fn the_wire_code_is_the_one_the_design_set_declares() {
    // Spelled out rather than compared to itself: this is the one place the token
    // is written, and section 5 declares it verbatim.
    assert_eq!(RUN_SELECTOR_EMPTY, "RUN_SELECTOR_EMPTY");
}
