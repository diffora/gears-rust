use super::*;
use crate::{CorpusEvaluator, EvalError, EvalInput, Evaluated, ReferenceOracle};
use bss_fixtures::{Corpus, Family};

/// Claims everything but always answers 1 — stands in for a wrong evaluator.
struct AlwaysOne;

impl CorpusEvaluator for AlwaysOne {
    fn evaluate(&self, _input: &EvalInput<'_>) -> Result<Evaluated, EvalError> {
        Ok(Evaluated::Charge(1))
    }
    fn supported_families(&self) -> Vec<Family> {
        vec![
            Family::TierBoundary,
            Family::Package,
            Family::PerUnit,
            Family::Flat,
            Family::Proration,
            Family::SupersessionContinuity,
            Family::LevelAggregation,
            Family::Reserved,
        ]
    }
}

/// Claims nothing at all.
struct ClaimsNothing;

impl CorpusEvaluator for ClaimsNothing {
    fn evaluate(&self, _input: &EvalInput<'_>) -> Result<Evaluated, EvalError> {
        Ok(Evaluated::Charge(0))
    }
    fn supported_families(&self) -> Vec<Family> {
        Vec::new()
    }
}

fn corpus() -> Corpus {
    Corpus::load(&Corpus::corpus_root()).expect("corpus loads")
}

#[test]
fn the_oracle_is_green_on_every_family_it_claims() {
    let report = run_evaluation_suite(&ReferenceOracle, &corpus());

    assert_eq!(
        report.failures().count(),
        0,
        "oracle must reproduce the corpus"
    );
    assert!(report.is_green_for(Family::TierBoundary));
    assert!(report.is_green_for(Family::Package));
    assert!(report.is_green_for(Family::PerUnit));
}

#[test]
fn a_wrong_evaluator_is_not_green() {
    let report = run_evaluation_suite(&AlwaysOne, &corpus());

    assert!(report.failures().count() > 0);
    assert!(!report.is_green_for(Family::TierBoundary));
}

#[test]
fn declining_a_family_is_recorded_and_is_not_green() {
    let report = run_evaluation_suite(&ClaimsNothing, &corpus());

    // Silence is not available: declining shows up as a declined family, and a
    // declined family is never green.
    assert!(report.declined.contains(&Family::TierBoundary));
    assert!(!report.is_green_for(Family::TierBoundary));
}

#[test]
fn a_family_with_no_assertions_is_not_green() {
    // Guards the hole where an empty family would vacuously pass.
    let empty = Corpus {
        cases: Vec::new(),
        families: Vec::new(),
    };
    let report = run_evaluation_suite(&ReferenceOracle, &empty);

    assert!(!report.is_green_for(Family::TierBoundary));
}

#[test]
fn the_unbuilt_families_are_declined_not_green() {
    // `trailing-tier` is the one family left unbuilt, and deliberately: SEAMS
    // M12 is open, so rating has no counterpart for `tierQualificationWindow`
    // at all. A fixture would pin one side of a contract the other has not
    // accepted. It must read as declined, never as green.
    let report = run_evaluation_suite(&ReferenceOracle, &corpus());

    let f = Family::TrailingTier;
    assert!(report.declined.contains(&f), "{f:?} must be declined");
    assert!(!report.is_green_for(f), "{f:?} must not be green");
}
