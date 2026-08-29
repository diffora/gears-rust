use uuid::Uuid;

use super::{CreateEntityCandidate, NameShapeRule};
use crate::domain::validation::{Phase, ValidationPipeline, ValidationRule};

fn candidate(name: &str) -> CreateEntityCandidate {
    CreateEntityCandidate {
        tenant_id: Uuid::nil(),
        brand_id: Uuid::nil(),
        name: name.to_owned(),
        code: None,
    }
}

#[test]
fn a_real_name_is_admitted() {
    let pipeline = ValidationPipeline::new().with_rule(Box::new(NameShapeRule));
    assert!(pipeline.run(&candidate("Acme Widget")).is_none());
}

#[test]
fn a_whitespace_only_name_is_refused_in_the_shape_phase() {
    let pipeline = ValidationPipeline::new().with_rule(Box::new(NameShapeRule));
    let (phase, report) = pipeline
        .run(&candidate("   \t "))
        .expect("a whitespace-only name must be refused");
    assert_eq!(phase, Phase::Shape);
    assert_eq!(report.audit_code(), Some("VALIDATION"));
    assert_eq!(report.violations().len(), 1);
    assert_eq!(report.violations()[0].subject, "name");
}

#[test]
fn the_rule_reports_the_instruction_id_it_answers_to() {
    // Attribution in a rejection rides the error code; the name is
    // observability only, and this is the assertion that keeps it honest.
    assert_eq!(NameShapeRule.name(), "inst-fd-name-unique");
    assert_eq!(NameShapeRule.phase(), Phase::Shape);
}

#[test]
fn the_candidate_normalizes_its_own_name_once() {
    // Two rules that each normalized independently could disagree; the
    // candidate is the single site, and this is what pins that.
    // LATIN SMALL LETTER SHARP S, written escaped: this file is not the
    // normalization suite and carries no blanket allowance for the class.
    assert_eq!(candidate("  Stra\u{00DF}e  ").name_normalized(), "strasse");
}
