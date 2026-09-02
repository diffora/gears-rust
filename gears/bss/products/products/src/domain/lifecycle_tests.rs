//! The two fillings of `RegisteredValidators` for publish-ordering, and the
//! residue that they are not interchangeable.

use bss_products_sdk::models::LifecycleState;

use super::{
    LifecycleRefusal, ParentPublishedRequired, PublishOrderingSubject, parent_must_be_published,
};
use crate::domain::validation::{Phase, ValidationPipeline, ValidationRule};

#[test]
fn parent_published_required_is_a_registered_validators_rule() {
    assert_eq!(
        ParentPublishedRequired.phase(),
        Phase::RegisteredValidators,
        "the host fills the phase slot, not a parallel phase"
    );
    assert_eq!(ParentPublishedRequired.name(), "inst-pc-ordering");
}

#[test]
fn a_published_parent_admits_and_a_draft_parent_refuses_the_code() {
    let pipeline = ValidationPipeline::new().with_rule(Box::new(ParentPublishedRequired));
    assert!(
        pipeline
            .run(&PublishOrderingSubject {
                parent: LifecycleState::Published
            })
            .is_none()
    );
    let (phase, report) = pipeline
        .run(&PublishOrderingSubject {
            parent: LifecycleState::Draft,
        })
        .expect("draft parent refuses");
    assert_eq!(phase, Phase::RegisteredValidators);
    assert_eq!(
        report.audit_code(),
        Some(LifecycleRefusal::PARENT_NOT_PUBLISHED)
    );
}

/// The continuation refuses on the first finding and does not collect.
#[test]
fn the_continuation_refuses_draft_and_deprecated_and_passes_published() {
    parent_must_be_published(LifecycleState::Published).expect("published parent");
    for live_not_published in [LifecycleState::Draft, LifecycleState::Deprecated] {
        let err = parent_must_be_published(live_not_published).expect_err("not published");
        assert_eq!(err.code, LifecycleRefusal::PARENT_NOT_PUBLISHED);
        assert!(
            !err.detail.contains("child"),
            "P-D-96: the new code must not name children either"
        );
    }
}

/// Terminal parents are foundation's `PARENT_TERMINAL`, not this code — and
/// both fillings must agree, or the registration line would fork them.
#[test]
fn a_terminal_parent_is_not_this_rules_operand_in_either_filling() {
    let pipeline = ValidationPipeline::new().with_rule(Box::new(ParentPublishedRequired));
    for terminal in [LifecycleState::Retired, LifecycleState::Discarded] {
        parent_must_be_published(terminal)
            .expect("terminal is PARENT_TERMINAL on the identity check, not here");
        assert!(
            pipeline
                .run(&PublishOrderingSubject { parent: terminal })
                .is_none(),
            "the registered filling must carve out {terminal:?} the same way"
        );
    }
}

/// Every admitted state produces the same verdict from both fillings.
#[test]
fn both_fillings_agree_on_every_lifecycle_state() {
    let pipeline = ValidationPipeline::new().with_rule(Box::new(ParentPublishedRequired));
    for state in [
        LifecycleState::Draft,
        LifecycleState::Published,
        LifecycleState::Deprecated,
        LifecycleState::Retired,
        LifecycleState::Discarded,
    ] {
        let continuation = parent_must_be_published(state);
        let registered = pipeline.run(&PublishOrderingSubject { parent: state });
        match continuation {
            Ok(()) => assert!(
                registered.is_none(),
                "{state:?}: continuation admits, registered must too"
            ),
            Err(refusal) => {
                let (_, report) = registered.expect("registered must refuse too");
                assert_eq!(report.audit_code(), Some(refusal.code));
            }
        }
    }
}
