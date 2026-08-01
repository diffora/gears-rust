//! Tests for the read model's subject vocabulary.

use uuid::Uuid;

use super::{GLOBAL_SCOPE, OverlayIndexShard, OverlayScopeClass, SubjectKind, SubjectRef};
use crate::domain::error::DomainError;

#[test]
fn the_subject_kind_tokens_are_the_persisted_ones() {
    assert_eq!(SubjectKind::Plan.as_str(), "plan");
    assert_eq!(SubjectKind::PriceOverlay.as_str(), "price_overlay");
    assert_eq!(SubjectKind::OverlayIndex.as_str(), "overlay_index");
    assert_eq!(SubjectKind::GroupMembership.as_str(), "group_membership");
}

#[test]
fn every_subject_kind_round_trips_through_its_token() {
    for kind in SubjectKind::ALL {
        assert_eq!(
            SubjectKind::parse(kind.as_str()).expect("its own token parses"),
            *kind
        );
    }
}

#[test]
fn an_unknown_subject_kind_fails_closed() {
    // The set is extensible, so an unknown token means the reader is older
    // than the writer. Coercing it to the nearest known kind would silently
    // project one subject's delta as another's.
    assert!(matches!(
        SubjectKind::parse("plan_overlay"),
        Err(DomainError::InvalidRequest(_))
    ));
}

#[test]
fn an_overlay_index_ref_always_carries_its_shard_key() {
    // The point of the enum shape: the unsharded tenant-wide index D-133
    // removed cannot be expressed, so nothing can reintroduce it by omission.
    let shard =
        OverlayIndexShard::scoped(OverlayScopeClass::Partner, "acme").expect("valued shard");
    let subject = SubjectRef::OverlayIndex(shard);

    assert_eq!(subject.kind(), SubjectKind::OverlayIndex);
    assert_eq!(subject.to_string(), "partner/acme");
}

#[test]
fn the_classless_shard_uses_the_global_sentinel_on_both_halves() {
    // The pair is a composite key; an empty half would make the classless
    // shard's identity depend on a null-versus-empty convention.
    let shard = OverlayIndexShard::Global;

    assert_eq!(shard.scope_class(), GLOBAL_SCOPE);
    assert_eq!(shard.scope_value(), GLOBAL_SCOPE);
}

#[test]
fn a_valued_class_needs_a_value() {
    // A partner shard with no partner matches nothing, which is a different
    // and much quieter failure than the global shard.
    assert!(matches!(
        OverlayIndexShard::scoped(OverlayScopeClass::Brand, "  "),
        Err(DomainError::InvalidRequest(_))
    ));
}

#[test]
fn the_valued_classes_are_the_five_non_global_ones() {
    // With the sentinel modelled structurally, `global` is not a class value,
    // so a shard cannot claim class `global` and carry a scope value too.
    let tokens: Vec<&str> = OverlayScopeClass::ALL
        .iter()
        .map(|class| class.as_str())
        .collect();

    assert_eq!(
        tokens,
        vec!["partner", "org_tier", "brand", "region", "customer_group"]
    );
    assert!(!tokens.contains(&GLOBAL_SCOPE));
}

#[test]
fn shards_of_different_scope_values_are_different_subjects() {
    // A commit rewrites exactly one shard; if two scope values collapsed to
    // one subject a partner commit would rewrite every partner's index.
    let acme = OverlayIndexShard::scoped(OverlayScopeClass::Partner, "acme").expect("shard");
    let other = OverlayIndexShard::scoped(OverlayScopeClass::Partner, "globex").expect("shard");

    assert_ne!(acme, other);
    assert_ne!(
        SubjectRef::OverlayIndex(acme),
        SubjectRef::OverlayIndex(other)
    );
}

#[test]
fn each_subject_ref_reports_its_own_kind() {
    let id = Uuid::from_u128(3);

    assert_eq!(SubjectRef::Plan(id).kind(), SubjectKind::Plan);
    assert_eq!(
        SubjectRef::PriceOverlay(id).kind(),
        SubjectKind::PriceOverlay
    );
    assert_eq!(
        SubjectRef::GroupMembership(id).kind(),
        SubjectKind::GroupMembership
    );
    assert_eq!(
        SubjectRef::OverlayIndex(OverlayIndexShard::Global).kind(),
        SubjectKind::OverlayIndex
    );
}

#[test]
fn a_plan_and_an_overlay_with_the_same_uuid_are_different_subjects() {
    // Subject identity is (kind, ref), never the uuid alone — the read model
    // stores one delta per subject and these two are separate publish units.
    let id = Uuid::from_u128(9);

    assert_ne!(SubjectRef::Plan(id), SubjectRef::PriceOverlay(id));
}
