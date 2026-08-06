use super::*;

fn satisfied() -> PredicateAnswer {
    PredicateAnswer::Satisfied
}

fn failed() -> PredicateAnswer {
    PredicateAnswer::Failed {
        detail: "the window does not cover t".to_owned(),
    }
}

fn unevaluable() -> PredicateAnswer {
    PredicateAnswer::NotEvaluable { owed_to: "Slice 4" }
}

fn outcome(predicate: Predicate, answer: PredicateAnswer) -> PredicateOutcome {
    PredicateOutcome { predicate, answer }
}

fn component(id: u128, verdict: PlanMarketVerdict) -> ComponentSellability {
    ComponentSellability {
        component_plan_id: Uuid::from_u128(id),
        verdict,
    }
}

// ---------------------------------------------------------------------------
// Predicate (6) is excluded from a component reference.
// ---------------------------------------------------------------------------

/// D-46's flag applies to the bundle SKU, never to a component reference:
/// `sellable = false` components are exactly the composition-only SKUs bundles
/// exist to package.
#[test]
fn a_component_failing_the_registry_flag_still_passes() {
    let verdict = component_verdict(&[
        outcome(Predicate::ActiveWindowWithHorizon, satisfied()),
        outcome(Predicate::CommittedVersion, satisfied()),
        outcome(Predicate::AvailabilityDates, satisfied()),
        outcome(Predicate::PlanLifecycleState, satisfied()),
        outcome(Predicate::GaGateFlags, satisfied()),
        outcome(
            Predicate::RegistrySellable,
            PredicateAnswer::Failed {
                detail: "sellable = false".to_owned(),
            },
        ),
    ]);

    assert_eq!(verdict, PlanMarketVerdict::Sellable);
}

/// And every other failing predicate still refuses.
#[test]
fn a_component_failing_any_of_one_to_five_is_not_sellable() {
    for predicate in [
        Predicate::ActiveWindowWithHorizon,
        Predicate::CommittedVersion,
        Predicate::AvailabilityDates,
        Predicate::PlanLifecycleState,
        Predicate::GaGateFlags,
    ] {
        assert_eq!(
            component_verdict(&[outcome(predicate, failed())]),
            PlanMarketVerdict::NotSellable,
            "{predicate:?} must still refuse"
        );
    }
}

/// `Failed` dominates `NotEvaluable`: an operator told to wait for a slice that
/// cannot change the answer has been told the wrong thing.
#[test]
fn a_failure_beside_an_unevaluable_predicate_is_not_sellable() {
    let verdict = component_verdict(&[
        outcome(Predicate::ActiveWindowWithHorizon, failed()),
        outcome(Predicate::GaGateFlags, unevaluable()),
    ]);

    assert_eq!(verdict, PlanMarketVerdict::NotSellable);
}

#[test]
fn nothing_failed_and_something_unevaluable_is_not_evaluable() {
    let verdict = component_verdict(&[
        outcome(Predicate::ActiveWindowWithHorizon, satisfied()),
        outcome(Predicate::GaGateFlags, unevaluable()),
    ]);

    assert_eq!(verdict, PlanMarketVerdict::NotEvaluable);
}

// ---------------------------------------------------------------------------
// The conjunction.
// ---------------------------------------------------------------------------

#[test]
fn every_component_sellable_makes_the_bundle_sellable() {
    let verdict = bundle_verdict(
        PriceBasis::SumOfParts,
        None,
        &satisfied(),
        &[
            component(1, PlanMarketVerdict::Sellable),
            component(2, PlanMarketVerdict::Sellable),
        ],
    );

    assert_eq!(verdict, PlanMarketVerdict::Sellable);
}

/// D-94, one level up: one unsellable component makes the bundle unsellable,
/// never partially sellable.
#[test]
fn one_unsellable_component_makes_the_whole_bundle_unsellable() {
    let verdict = bundle_verdict(
        PriceBasis::SumOfParts,
        None,
        &satisfied(),
        &[
            component(1, PlanMarketVerdict::Sellable),
            component(2, PlanMarketVerdict::NotSellable),
        ],
    );

    assert_eq!(verdict, PlanMarketVerdict::NotSellable);
}

/// The bundle's own availability is in the conjunction and no component carries
/// it.
#[test]
fn the_bundles_own_availability_can_refuse_on_its_own() {
    let verdict = bundle_verdict(
        PriceBasis::SumOfParts,
        None,
        &failed(),
        &[component(1, PlanMarketVerdict::Sellable)],
    );

    assert_eq!(verdict, PlanMarketVerdict::NotSellable);
}

/// An empty conjunction is not vacuously true: a bundle binding no key on this
/// market is not sellable on it.
#[test]
fn a_bundle_referencing_no_component_is_not_sellable() {
    let verdict = bundle_verdict(PriceBasis::SumOfParts, None, &satisfied(), &[]);

    assert_eq!(verdict, PlanMarketVerdict::NotSellable);
}

/// For `own_price` the bundle's **own** rows must pass as well.
#[test]
fn an_own_price_bundles_own_rows_are_in_the_conjunction() {
    let verdict = bundle_verdict(
        PriceBasis::OwnPrice,
        Some(PlanMarketVerdict::NotSellable),
        &satisfied(),
        &[component(1, PlanMarketVerdict::Sellable)],
    );

    assert_eq!(verdict, PlanMarketVerdict::NotSellable);
}

/// For `sum_of_parts` there are no own rows (`inst-bb-rowless`), so an own
/// verdict handed in anyway is ignored rather than folded in.
#[test]
fn a_sum_of_parts_bundle_ignores_an_own_verdict() {
    let verdict = bundle_verdict(
        PriceBasis::SumOfParts,
        Some(PlanMarketVerdict::NotSellable),
        &satisfied(),
        &[component(1, PlanMarketVerdict::Sellable)],
    );

    assert_eq!(verdict, PlanMarketVerdict::Sellable);
}

#[test]
fn an_unevaluable_component_makes_the_bundle_unevaluable() {
    let verdict = bundle_verdict(
        PriceBasis::SumOfParts,
        None,
        &satisfied(),
        &[
            component(1, PlanMarketVerdict::Sellable),
            component(2, PlanMarketVerdict::NotEvaluable),
        ],
    );

    assert_eq!(verdict, PlanMarketVerdict::NotEvaluable);
}

/// And a failure beside it still dominates, at the bundle level too.
#[test]
fn a_failure_dominates_an_unevaluable_component() {
    let verdict = bundle_verdict(
        PriceBasis::SumOfParts,
        None,
        &satisfied(),
        &[
            component(1, PlanMarketVerdict::NotEvaluable),
            component(2, PlanMarketVerdict::NotSellable),
        ],
    );

    assert_eq!(verdict, PlanMarketVerdict::NotSellable);
}
