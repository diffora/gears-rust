use super::{Phase, ValidationPipeline, ValidationReport, ValidationRule};

/// A rule that always fails, in a phase chosen by the test.
struct AlwaysFails {
    id: &'static str,
    phase: Phase,
    code: &'static str,
}

impl ValidationRule<()> for AlwaysFails {
    fn name(&self) -> &'static str {
        self.id
    }
    fn phase(&self) -> Phase {
        self.phase
    }
    fn evaluate(&self, _subject: &(), report: &mut ValidationReport) {
        report.violate(self.code, "subject", "always fails");
    }
}

#[test]
fn an_empty_pipeline_admits_everything() {
    let pipeline: ValidationPipeline<()> = ValidationPipeline::new();
    assert!(pipeline.run(&()).is_none());
    assert!(pipeline.rule_names().is_empty());
}

#[test]
fn the_run_stops_at_the_first_failing_phase() {
    // Registered in reverse phase order on purpose: the run must be ordered by
    // phase, not by registration.
    let pipeline = ValidationPipeline::new()
        .with_rule(Box::new(AlwaysFails {
            id: "late",
            phase: Phase::Identity,
            code: "DUPLICATE_NAME",
        }))
        .with_rule(Box::new(AlwaysFails {
            id: "early",
            phase: Phase::Shape,
            code: "VALIDATION",
        }));
    let (phase, report) = pipeline.run(&()).expect("both rules fail");
    assert_eq!(phase, Phase::Shape, "the earlier phase must win");
    assert_eq!(
        report.violations().len(),
        1,
        "a report is one phase's findings, never a mixture"
    );
    assert_eq!(report.audit_code(), Some("VALIDATION"));
}

#[test]
fn violations_within_one_phase_are_collected_together() {
    let pipeline = ValidationPipeline::new()
        .with_rule(Box::new(AlwaysFails {
            id: "first",
            phase: Phase::Shape,
            code: "VALIDATION",
        }))
        .with_rule(Box::new(AlwaysFails {
            id: "second",
            phase: Phase::Shape,
            code: "VALIDATION",
        }));
    let (_, report) = pipeline.run(&()).expect("both rules fail");
    assert_eq!(report.violations().len(), 2);
    // The caller sees both; the audit row records one.
    assert_eq!(report.audit_code(), Some("VALIDATION"));
}

#[test]
fn rule_names_are_reported_in_registration_order() {
    let pipeline = ValidationPipeline::new()
        .with_rule(Box::new(AlwaysFails {
            id: "one",
            phase: Phase::Shape,
            code: "VALIDATION",
        }))
        .with_rule(Box::new(AlwaysFails {
            id: "two",
            phase: Phase::Shape,
            code: "VALIDATION",
        }));
    assert_eq!(pipeline.rule_names(), vec!["one", "two"]);
}

#[test]
fn the_phase_roster_is_the_documented_execution_order() {
    assert_eq!(
        Phase::ordered(),
        [
            Phase::Idempotency,
            Phase::Precondition,
            Phase::Shape,
            Phase::State,
            Phase::Identity,
            Phase::RegisteredValidators,
            Phase::GovernanceGate,
        ]
    );
    // Declaration order is execution order, and the derived Ord is what the run
    // relies on. This is the assertion that catches a phase inserted wrongly.
    let mut sorted = Phase::ordered();
    sorted.sort_unstable();
    assert_eq!(sorted, Phase::ordered());
}

#[test]
fn an_empty_report_audits_nothing() {
    let report = ValidationReport::new();
    assert!(report.is_empty());
    assert_eq!(report.audit_code(), None);
}
