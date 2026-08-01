//! Tests for the validation pipeline framework.

use super::{ValidationPipeline, ValidationReport, ValidationRule};

struct AlwaysViolates(&'static str);
struct AlwaysWarns(&'static str);
struct Silent(&'static str);

impl ValidationRule<u32> for AlwaysViolates {
    fn name(&self) -> &'static str {
        self.0
    }

    fn evaluate(&self, subject: &u32, report: &mut ValidationReport) {
        report.violate(self.0, subject.to_string(), "blocked");
    }
}

impl ValidationRule<u32> for AlwaysWarns {
    fn name(&self) -> &'static str {
        self.0
    }

    fn evaluate(&self, subject: &u32, report: &mut ValidationReport) {
        report.warn(self.0, subject.to_string(), "advisory");
    }
}

impl ValidationRule<u32> for Silent {
    fn name(&self) -> &'static str {
        self.0
    }

    fn evaluate(&self, _subject: &u32, _report: &mut ValidationReport) {}
}

#[test]
fn an_empty_report_is_publishable() {
    assert!(ValidationReport::default().is_publishable());
}

#[test]
fn a_warning_alone_never_blocks() {
    // If an advisory could block, the distinction between the two lists would
    // be decorative and a fail-closed rule could hide behind a soft word.
    let pipeline = ValidationPipeline::new().with_rule(Box::new(AlwaysWarns("W1")));

    let report = pipeline.run(&7);

    assert!(report.is_publishable());
    assert_eq!(report.warnings.len(), 1);
    assert!(report.violations.is_empty());
}

#[test]
fn one_violation_blocks_the_publish() {
    let pipeline = ValidationPipeline::new().with_rule(Box::new(AlwaysViolates("V1")));

    assert!(!pipeline.run(&7).is_publishable());
}

#[test]
fn every_rule_runs_even_after_a_failure() {
    // The aggregate report is the point: an author remediates a plan in one
    // pass, so a run must not stop at the first blocking rule.
    let pipeline = ValidationPipeline::new()
        .with_rule(Box::new(AlwaysViolates("V1")))
        .with_rule(Box::new(AlwaysViolates("V2")))
        .with_rule(Box::new(AlwaysWarns("W1")));

    let report = pipeline.run(&7);

    assert_eq!(report.violations.len(), 2);
    assert_eq!(report.warnings.len(), 1);
}

#[test]
fn findings_appear_in_registration_order() {
    let pipeline = ValidationPipeline::new()
        .with_rule(Box::new(AlwaysViolates("first")))
        .with_rule(Box::new(Silent("quiet")))
        .with_rule(Box::new(AlwaysViolates("second")));

    let report = pipeline.run(&1);

    assert_eq!(report.violations[0].code, "first");
    assert_eq!(report.violations[1].code, "second");
    assert_eq!(pipeline.rule_names(), vec!["first", "quiet", "second"]);
}

#[test]
fn a_run_carries_nothing_into_the_next_one() {
    // The same pipeline runs as a submit-time pre-check and again inside the
    // publish commit. If a run kept state, the commit-time verdict would depend
    // on how many times the plan had been submitted.
    let pipeline = ValidationPipeline::new().with_rule(Box::new(AlwaysViolates("V1")));

    let first = pipeline.run(&7);
    let second = pipeline.run(&7);

    assert_eq!(first, second);
    assert_eq!(second.violations.len(), 1);
}

#[test]
fn a_pipeline_with_no_rules_publishes_everything() {
    // Recorded deliberately: this is why the Foundation registers the money and
    // rounding rules itself instead of relying on a slice to have loaded.
    let pipeline: ValidationPipeline<u32> = ValidationPipeline::new();

    assert!(pipeline.run(&7).is_publishable());
    assert!(pipeline.rule_names().is_empty());
}

#[test]
fn absorb_concatenates_both_lists() {
    let mut left = ValidationReport::default();
    left.violate("A", "row-1", "x");
    let mut right = ValidationReport::default();
    right.violate("B", "row-2", "y");
    right.warn("C", "row-2", "z");

    left.absorb(right);

    assert_eq!(left.violations.len(), 2);
    assert_eq!(left.warnings.len(), 1);
    assert_eq!(left.violations[1].code, "B");
}

#[test]
fn the_report_renders_its_blocking_count() {
    let mut report = ValidationReport::default();
    report.violate("A", "row-1", "x");
    report.violate("B", "row-2", "y");
    report.warn("C", "row-3", "z");

    assert_eq!(report.to_string(), "2");
}
