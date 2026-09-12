//! The live-intent guard, including the child-revival case that checking
//! only the subject missed.

use uuid::Uuid;

use super::{BlockingIntent, children_the_reversal_touches, refuse_if_live_retire_intents};
use crate::domain::deprecation::Provenance;
use crate::domain::lifecycle::LifecycleRefusal;

const SUBJECT: Uuid = Uuid::from_u128(0x10);
const CASCADED: Uuid = Uuid::from_u128(0x20);
const DIRECT: Uuid = Uuid::from_u128(0x30);

#[test]
fn a_clean_subject_and_clean_revived_children_admit() {
    refuse_if_live_retire_intents(SUBJECT, &[CASCADED], &[]).expect("no live intents");
}

#[test]
fn the_subjects_own_live_intent_refuses() {
    let err = refuse_if_live_retire_intents(
        SUBJECT,
        &[CASCADED],
        &[BlockingIntent { entity_id: SUBJECT }],
    )
    .expect_err("subject still retiring");
    assert_eq!(err.code, LifecycleRefusal::RETIREMENT_PENDING);
    assert!(err.detail.contains(&SUBJECT.to_string()));
}

/// The case the subject's-only check missed: a cascaded child still holds
/// a retire intent after the parent cancelled its own.
#[test]
fn a_revived_childs_live_intent_refuses_and_is_named() {
    let err = refuse_if_live_retire_intents(
        SUBJECT,
        &[CASCADED],
        &[BlockingIntent {
            entity_id: CASCADED,
        }],
    )
    .expect_err("revived child still retiring");
    assert_eq!(err.code, LifecycleRefusal::RETIREMENT_PENDING);
    assert!(err.detail.contains(&CASCADED.to_string()));
}

#[test]
fn a_direct_childs_intent_is_outside_the_revival_set() {
    refuse_if_live_retire_intents(
        SUBJECT,
        &[CASCADED],
        &[BlockingIntent { entity_id: DIRECT }],
    )
    .expect("a direct child is not revived, so its intent is not this refusal");
}

#[test]
fn the_reversal_touches_cascaded_children_only() {
    let touched = children_the_reversal_touches(&[
        (CASCADED, Some(Provenance::Cascaded)),
        (DIRECT, Some(Provenance::Direct)),
        (Uuid::from_u128(0x40), None),
    ]);
    assert_eq!(touched, vec![CASCADED]);
}
