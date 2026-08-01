//! Runs the whole corpus against a conformance subject.
//!
//! There is no per-case filter on purpose. The only way to decline work is to
//! omit a family from `supported_families`, which is recorded and is never
//! green — so a subject cannot quietly skip the family it finds inconvenient.

use crate::traits::{CorpusEvaluator, EvalError, EvalInput, Evaluated};
use bss_fixtures::{Case, Corpus, Expect, Family};

#[derive(Debug)]
pub struct Outcome {
    pub case_id: String,
    pub family: Family,
    pub index: usize,
    pub expected: Expect,
    pub actual: Result<Evaluated, EvalError>,
}

impl Outcome {
    #[must_use]
    pub fn passed(&self) -> bool {
        match (&self.actual, self.expected) {
            (Ok(Evaluated::Charge(got)), Expect::Charge(want)) => *got == want.charge_minor,
            (
                Ok(Evaluated::Units {
                    charged, in_basis, ..
                }),
                Expect::Units(want),
            ) => *charged == want.units_charged && *in_basis == want.units_in_basis,
            (Ok(Evaluated::Fold { q }), Expect::Fold(want)) => *q == want.folded_q,
            // A subject answering the wrong shape has misunderstood the case,
            // not merely got the number wrong.
            _ => false,
        }
    }
}

#[derive(Debug)]
pub struct Report {
    pub outcomes: Vec<Outcome>,
    pub declined: Vec<Family>,
}

impl Report {
    pub fn failures(&self) -> impl Iterator<Item = &Outcome> {
        self.outcomes.iter().filter(|o| !o.passed())
    }

    /// Green means: claimed, non-empty, and every assertion reproduced.
    ///
    /// An empty family is **not** green. Without that clause absent coverage
    /// would report as success, which is the exact failure the corpus exists to
    /// prevent.
    #[must_use]
    pub fn is_green_for(&self, family: Family) -> bool {
        if self.declined.contains(&family) {
            return false;
        }
        let mut seen = false;
        for o in self.outcomes.iter().filter(|o| o.family == family) {
            seen = true;
            if !o.passed() {
                return false;
            }
        }
        seen
    }
}

pub fn run_evaluation_suite<E: CorpusEvaluator>(evaluator: &E, corpus: &Corpus) -> Report {
    let claimed = evaluator.supported_families();
    let mut outcomes = Vec::new();

    for case in &corpus.cases {
        if !claimed.contains(&case.family()) {
            continue;
        }
        // Publish cases are answered by a `PublishValidator`, which only the
        // pricing gear implements; they are not this suite's business.
        let Case::Evaluation(case) = case else {
            continue;
        };
        for (index, a) in case.assert.iter().enumerate() {
            let input = EvalInput {
                snapshot: &case.snapshot,
                runtime: &case.runtime,
                given: &a.given,
            };
            outcomes.push(Outcome {
                case_id: case.id.clone(),
                family: case.family,
                index,
                expected: a.expect,
                actual: evaluator.evaluate(&input),
            });
        }
    }

    let declined = Family::ALL
        .iter()
        .copied()
        .filter(|f| !claimed.contains(f))
        .collect();

    Report { outcomes, declined }
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod runner_tests;
