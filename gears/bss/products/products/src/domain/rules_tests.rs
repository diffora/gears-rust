use uuid::Uuid;

use super::{CreateEntityCandidate, NameShapeRule};
use crate::domain::error::DomainError;
use crate::domain::validation::{Phase, ValidationPipeline, ValidationReport, ValidationRule};

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
    assert_eq!(report.audit_code(), Some(NameShapeRule::CODE));
    assert_eq!(report.violations().len(), 1);
    assert_eq!(report.violations()[0].subject, "name");
}

#[test]
fn the_rules_code_constant_cannot_drift_from_domain_errors_own() {
    // `NameShapeRule::CODE` cannot be derived from `DomainError::code()` in a
    // `const` initializer — the value owns a `String` and a value with a
    // destructor cannot be dropped there (`E0493`) — so the constant is a
    // literal and **this test is the whole anti-drift guarantee**. Rename the
    // wire code on either side and exactly one of these two assertions fails.
    assert_eq!(NameShapeRule::CODE, "VALIDATION");
    assert_eq!(
        NameShapeRule::CODE,
        DomainError::Validation(ValidationReport::new()).code()
    );
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

// ---------------------------------------------------------------------------
// P-D-37's `state`-phase precedence (`design/01-foundation.md` §3.3):
// `ENTITY_TERMINAL` → `PARENT_TERMINAL` → `ILLEGAL_TRANSITION` →
// `ILLEGAL_FIELD_MUTATION`, and no other phase may carry one of the four.
//
// **What is not yet expressible here, and why.** The precedence is produced
// by the *order* the `state` phase's own rules are registered in —
// `ValidationReport::audit_code()` answers with whichever violation a
// phase's rules pushed first, so the ordering above holds once four
// `state`-phase `ValidationRule`s exist, registered in that order. None do
// yet: this file's `NameShapeRule` is `domain/rules.rs`'s only rule, and it
// runs in `Phase::Shape`. Writing four throwaway `state`-phase
// `ValidationRule` impls here, just to make the ordering assertable, would
// fabricate the very rules this test exists to find missing — so this file
// does not do that. What follows is the strongest pair this crate supports
// today instead:
//
// 1. Exclusivity, over every rule the crate actually registers: the one rule
//    that exists neither raises one of the four codes nor runs in
//    `Phase::State`.
// 2. The mechanism the precedence depends on, exercised directly against
//    `ValidationReport` with the four real codes pushed in P-D-37's order:
//    `audit_code()` answers with the first violation collected, which is the
//    property that makes "register the four `state`-phase rules in this
//    order" sufficient to produce the precedence once they land. This is not
//    a claim that the precedence holds today (it cannot, with no
//    `state`-phase rules to hold it) — it pins the one piece of machinery
//    the precedence will run on.

#[test]
fn the_crates_one_registered_rule_is_exclusive_of_the_state_phase_precedence_codes() {
    let precedence_codes = [
        "ENTITY_TERMINAL",
        "PARENT_TERMINAL",
        "ILLEGAL_TRANSITION",
        "ILLEGAL_FIELD_MUTATION",
    ];
    assert_ne!(NameShapeRule.phase(), Phase::State);
    assert!(!precedence_codes.contains(&NameShapeRule::CODE));
}

#[test]
fn phase_state_sits_between_shape_and_identity_in_execution_order() {
    // `design/01-foundation.md` §3.1's seven-phase order, and the derived
    // `Ord` the pipeline runs on (`Phase::ordered`); a `state` phase inserted
    // anywhere else would still compile but would run its checks against a
    // payload the earlier phases had not yet admitted (P-D-24), or after
    // identity had already spent a write.
    let ordered = Phase::ordered();
    let shape_at = ordered
        .iter()
        .position(|phase| *phase == Phase::Shape)
        .expect("Shape is one of the seven ordered phases");
    let state_at = ordered
        .iter()
        .position(|phase| *phase == Phase::State)
        .expect("State is one of the seven ordered phases");
    let identity_at = ordered
        .iter()
        .position(|phase| *phase == Phase::Identity)
        .expect("Identity is one of the seven ordered phases");
    assert!(shape_at < state_at, "Shape must run before State");
    assert!(state_at < identity_at, "State must run before Identity");
}

#[test]
fn audit_code_answers_the_first_violation_collected_the_mechanism_the_precedence_needs() {
    // The four codes P-D-37 orders, pushed in that order. This does not
    // prove the `state` phase produces this order today — it has no rules
    // yet, see this file's header comment above — it pins the report
    // machinery those future rules would rely on: whichever runs first wins
    // the audit row.
    let mut report = ValidationReport::new();
    report.violate("ENTITY_TERMINAL", "head", "the row is retired");
    report.violate("PARENT_TERMINAL", "parent_id", "the parent is retired");
    report.violate("ILLEGAL_TRANSITION", "state", "no edge admits this move");
    report.violate(
        "ILLEGAL_FIELD_MUTATION",
        "sku_code",
        "bucket-i after first publish",
    );
    assert_eq!(report.audit_code(), Some("ENTITY_TERMINAL"));
    assert_eq!(report.violations().len(), 4);
}
