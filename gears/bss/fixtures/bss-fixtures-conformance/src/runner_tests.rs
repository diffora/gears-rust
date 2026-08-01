use super::*;
use crate::{CorpusEvaluator, EvalError, EvalInput, Evaluated, PublishValidator, ReferenceOracle};
use bss_fixtures::{Corpus, Family, ModelKind, PublishVerdict, Snapshot};

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

/// Answers whatever the corpus asked for. Stands in for a perfect validator, so
/// the runner's own bookkeeping can be tested without a gear.
struct EchoesTheCorpus;

impl PublishValidator for EchoesTheCorpus {
    fn validate(
        &self,
        _predecessor: &Snapshot,
        successor: &Snapshot,
    ) -> Result<PublishVerdict, EvalError> {
        // The corpus's only accepted publish case is the pure price change, and
        // it is the only one whose successor changes no unit field. Rather than
        // reimplement that, this stand-in answers from the one thing it can read
        // without a gear: `package` is the corpus's rejected block-size case,
        // everything else the accepted shape. It is deliberately crude -- its
        // job is to make the runner's pass/fail bookkeeping observable.
        if successor.model_kind == ModelKind::Package {
            return Ok(PublishVerdict::Rejected {
                error_code: "SUPERSESSION_UNIT_MISMATCH".to_owned(),
            });
        }
        Ok(PublishVerdict::Accepted)
    }
}

/// Declines every case it is handed.
struct CannotAssess;

impl PublishValidator for CannotAssess {
    fn validate(
        &self,
        _predecessor: &Snapshot,
        _successor: &Snapshot,
    ) -> Result<PublishVerdict, EvalError> {
        Err(EvalError::MissingField("everything"))
    }
}

#[test]
fn the_publish_suite_runs_every_publish_case_and_no_evaluation_case() {
    let corpus = corpus();
    let report = run_publish_suite(&EchoesTheCorpus, &corpus);

    let publish_assertions: usize = corpus
        .cases
        .iter()
        .filter_map(|c| match c {
            bss_fixtures::Case::Publish(p) => Some(p.assert.len()),
            bss_fixtures::Case::Evaluation(_) => None,
        })
        .sum();

    assert!(
        publish_assertions > 0,
        "the corpus must carry publish cases"
    );
    assert_eq!(report.outcomes.len(), publish_assertions);
}

/// Answers the corpus's one `declined_until` case the way the corpus states.
///
/// Stands in for the day Slice 10 lands: a subject that can represent the
/// reservation pair and applies `inst-rv-level` to it.
struct HasBuiltSliceTen;

impl PublishValidator for HasBuiltSliceTen {
    fn validate(
        &self,
        _predecessor: &Snapshot,
        successor: &Snapshot,
    ) -> Result<PublishVerdict, EvalError> {
        if successor.reservation_flavor == Some(bss_fixtures::ReservationFlavor::Consumption) {
            return Ok(PublishVerdict::Rejected {
                error_code: "LEVEL_RESERVATION_CONSUMPTION_FORBIDDEN".to_owned(),
            });
        }
        Ok(PublishVerdict::Accepted)
    }
}

/// Only the publish cases the corpus does **not** declare undecidable.
fn answerable_publish_cases(corpus: &Corpus) -> usize {
    corpus
        .cases
        .iter()
        .filter_map(|c| match c {
            bss_fixtures::Case::Publish(p) if p.declined_until.is_none() => Some(p.assert.len()),
            bss_fixtures::Case::Publish(_) | bss_fixtures::Case::Evaluation(_) => None,
        })
        .sum()
}

#[test]
fn a_declined_case_never_earns_a_kind() {
    // Failing rather than guessing is the sanctioned answer, and it is still not
    // evidence that the rule holds -- so it earns nothing.
    let report = run_publish_suite(&CannotAssess, &corpus());

    // Every case the corpus expects an answer to is a failure. The ones it
    // declares undecidable are not: refusing those is agreement.
    assert_eq!(
        answerable_publish_cases(&corpus()),
        report.failures().count()
    );
    assert!(report.earned_kinds().is_empty());
}

#[test]
fn a_case_the_corpus_declines_is_recorded_rather_than_counted() {
    // The `trailing-tier` reading at case granularity: an unbuilt slice reads as
    // declined -- recorded, never green, and never mistaken for a fault of the
    // subject that honestly could not represent the row.
    let report = run_publish_suite(&CannotAssess, &corpus());

    let declined: Vec<&str> = report.declined().map(|o| o.case_id.as_str()).collect();

    assert_eq!(declined, vec!["consumption-on-level-rejected"]);
    assert!(report.failures().all(|o| o.declined_until.is_none()));
}

#[test]
fn a_declined_case_answered_with_the_wrong_verdict_is_still_a_failure() {
    // `declined_until` suspends nothing about the verdict. `EchoesTheCorpus`
    // answers `accepted` where the corpus states a rejection, and that is a
    // disagreement whatever the corpus said about buildability -- otherwise the
    // marker would be a way to make a case unfalsifiable.
    let report = run_publish_suite(&EchoesTheCorpus, &corpus());

    assert!(
        report
            .failures()
            .any(|o| o.case_id == "consumption-on-level-rejected"),
        "a declined case answered wrongly must stay red"
    );
}

#[test]
fn answering_a_declined_case_correctly_marks_the_declaration_stale() {
    // The marker is self-retiring. A subject that reproduces the case has built
    // the slice, so "nothing can answer this yet" has stopped being true and the
    // line owes the registry its evidence again.
    let report = run_publish_suite(&HasBuiltSliceTen, &corpus());

    let stale: Vec<&str> = report
        .stale_declines()
        .map(|o| o.case_id.as_str())
        .collect();

    assert_eq!(stale, vec!["consumption-on-level-rejected"]);
    assert_eq!(report.declined().count(), 0);
}

#[test]
fn a_kind_with_no_publish_case_earns_nothing() {
    // The same clause `is_green_for` carries, one axis over: absent coverage must
    // never read as success. The committed corpus no longer has such a kind --
    // `check_publish_case_coverage` refuses one -- so the property is shown over
    // a corpus cut down to a single `graduated` case.
    let full = corpus();
    let one_case = Corpus {
        cases: full
            .cases
            .iter()
            .filter(|c| c.id() == "price-change-accepted")
            .cloned()
            .collect(),
        families: Vec::new(),
    };
    let report = run_publish_suite(&EchoesTheCorpus, &one_case);
    let earned = report.earned_kinds();

    assert_eq!(earned, vec![ModelKind::Graduated]);
    assert!(!earned.contains(&ModelKind::Flat));
    assert!(!earned.contains(&ModelKind::PerUnit));
}

#[test]
fn an_empty_corpus_earns_nothing() {
    let empty = Corpus {
        cases: Vec::new(),
        families: Vec::new(),
    };
    let report = run_publish_suite(&EchoesTheCorpus, &empty);

    assert!(report.outcomes.is_empty());
    assert!(report.earned_kinds().is_empty());
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
