//! `domain::materiality` — every clause of `dod-materiality-evaluator`
//! probed on the case whose absence would ship the defect the `DoD` names.

use bss_products_sdk::models::{EntityKind, LifecycleState};

use super::{
    APPROVER_COUNT_FLOOR, BucketBearing, DEFAULT_AFFECTED_ENTITY_TRIGGER, DEFAULT_APPROVER_COUNT,
    EnumeratedOp, MaterialAct, MaterialLiveOp, Materiality, MaterialityEvaluator, MaterialityInput,
    MaterialityPolicy, MaterialityRefusal, Resolution, bucket_bearing,
};
use crate::domain::bucket::FieldBucket;

/// A resolved default policy plus a resolved claim set — the shape every
/// positive control needs.
fn resolved<'a>(policy: &'a MaterialityPolicy, claims: &'a [String]) -> MaterialityEvaluator<'a> {
    MaterialityEvaluator::new(Resolution::Resolved(policy), Resolution::Resolved(claims))
}

/// **A bucket-iii touch is material.** The `DoD`'s first positive clause.
#[test]
fn a_bucket_iii_touch_is_material() {
    let policy = MaterialityPolicy::default();
    let claims = vec!["catalog-admin".to_owned()];
    let ev = resolved(&policy, &claims);
    let verdict = ev
        .verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Product,
            touched: &["name"],
        })
        .expect("a resolved policy and claim set");
    assert_eq!(verdict, Materiality::Material);
}

/// **A structural-only touch moves no verdict**, so the positive control
/// above cannot be passing because every touch answers material.
#[test]
fn a_touch_outside_bucket_iii_is_not_material() {
    let policy = MaterialityPolicy::default();
    let claims = vec!["catalog-admin".to_owned()];
    let ev = resolved(&policy, &claims);
    let verdict = ev
        .verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Product,
            touched: &["product_code"],
        })
        .expect("a resolved policy and claim set");
    assert_eq!(verdict, Materiality::NonMaterial);
}

/// **A bucket-iv-only re-publish is non-material** — asserted over the tag,
/// because bucket iv carries no registered column today and a
/// column-driven probe could not reach the clause at all.
#[test]
fn the_bucket_rule_is_total_and_bucket_iv_is_immaterial() {
    assert_eq!(
        bucket_bearing(FieldBucket::MaterialMutable),
        BucketBearing::Material
    );
    assert_eq!(
        bucket_bearing(FieldBucket::Descriptive),
        BucketBearing::Immaterial,
        "a bucket-iv-only re-publish is non-material"
    );
    assert_eq!(
        bucket_bearing(FieldBucket::Structural),
        BucketBearing::Immaterial
    );
    assert_eq!(
        bucket_bearing(FieldBucket::Correctable),
        BucketBearing::NotAnOrdinaryTouch
    );
}

/// **The policy object's own mutation is material regardless of direction.**
/// Both directions are asserted because C4's wording is what a loosening
/// edit would quietly drop: a policy edit that *reduces* `N` is exactly the
/// one an attacker wants judged non-material.
#[test]
fn a_policy_mutation_is_material_in_either_direction() {
    let claims = vec!["config-admin".to_owned()];
    for approver_count in [0_u32, 1, 2, 7] {
        let policy =
            MaterialityPolicy::new(Vec::new(), DEFAULT_AFFECTED_ENTITY_TRIGGER, approver_count);
        let ev = resolved(&policy, &claims);
        let verdict = ev
            .verdict(&MaterialAct::PolicyMutation)
            .expect("a resolved policy");
        assert_eq!(
            verdict,
            Materiality::Material,
            "a policy mutation at N = {approver_count} must be material"
        );
    }
}

/// **An unresolvable policy refuses the act** rather than falling back — the
/// clause whose absence "would publish a finance-material change on one
/// signature".
#[test]
fn an_unresolvable_policy_refuses_rather_than_defaulting() {
    let claims = vec!["catalog-admin".to_owned()];
    let ev = MaterialityEvaluator::new(Resolution::Unresolvable, Resolution::Resolved(&claims));
    let err = ev
        .verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Product,
            touched: &["name"],
        })
        .expect_err("an absent policy is a refusal, never the default");
    match err {
        MaterialityRefusal::Unresolved(u) => assert_eq!(u.input, MaterialityInput::Policy),
        other => panic!("expected the policy input named, got {other:?}"),
    }
}

/// **An unresolvable claim set refuses too**, and on an act whose shape
/// alone would have answered material — so the refusal cannot be the shape's.
#[test]
fn an_unresolvable_claim_set_refuses_even_a_material_shape() {
    let policy = MaterialityPolicy::default();
    let ev = MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Unresolvable);
    let err = ev
        .verdict(&MaterialAct::PolicyMutation)
        .expect_err("an absent claim set is a refusal");
    match err {
        MaterialityRefusal::Unresolved(u) => assert_eq!(u.input, MaterialityInput::ClaimSet),
        other => panic!("expected the claim set named, got {other:?}"),
    }
}

/// **An untagged column refuses** — the bucket registry's own fail-closed
/// arm, reached through the evaluator rather than asserted at `classify`.
#[test]
fn an_untagged_column_refuses_through_the_registry() {
    let policy = MaterialityPolicy::default();
    let claims = vec!["catalog-admin".to_owned()];
    let ev = resolved(&policy, &claims);
    let err = ev
        .verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Product,
            touched: &["no_such_column"],
        })
        .expect_err("a column with no bucket tag is refused, never defaulted");
    assert!(
        matches!(err, MaterialityRefusal::Registry(_)),
        "got {err:?}"
    );
}

/// **A bucket-ii column never arrives as an ordinary touch** (L-1), so the
/// evaluator refuses instead of judging it. `metering_unit` is a real
/// bucket-ii member since 03's meter pair landed, which is what makes this
/// probe reachable at all.
#[test]
fn a_bucket_ii_touch_is_refused_not_judged() {
    let policy = MaterialityPolicy::default();
    let claims = vec!["catalog-admin".to_owned()];
    let ev = resolved(&policy, &claims);
    let err = ev
        .verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Sku,
            touched: &["metering_unit"],
        })
        .expect_err("bucket ii reaches publish through the save or correction door, never a touch");
    match err {
        MaterialityRefusal::CorrectableTouch(column) => assert_eq!(column, "metering_unit"),
        other => panic!("expected the column named, got {other:?}"),
    }
}

/// **The PRD enumeration's exact three targets are material and
/// `draft → discarded` is not** (M-1) — the arm a "every transition is
/// material" simplification would break.
#[test]
fn the_enumerated_transitions_are_the_prd_three() {
    let policy = MaterialityPolicy::default();
    let claims = vec!["catalog-admin".to_owned()];
    for to in [
        LifecycleState::Published,
        LifecycleState::Deprecated,
        LifecycleState::Retired,
    ] {
        let ev = resolved(&policy, &claims);
        let verdict = ev
            .verdict(&MaterialAct::Enumerated(EnumeratedOp::LifecycleTransition(
                to,
            )))
            .expect("a resolved policy");
        assert_eq!(verdict, Materiality::Material, "transition to {to:?}");
    }

    // **The two outside the enumeration are refused, not judged.**
    // `NonMaterial` feeds `required = min(N, 1)` — one approver at the
    // default — so answering it for `draft -> discarded` would mint a
    // ceremony for the one transition M-1 leaves ungated.
    for to in [LifecycleState::Draft, LifecycleState::Discarded] {
        let ev = resolved(&policy, &claims);
        let err = ev
            .verdict(&MaterialAct::Enumerated(EnumeratedOp::LifecycleTransition(
                to,
            )))
            .expect_err("outside the FR's enumeration means no verdict, not a small one");
        match err {
            MaterialityRefusal::OutsideTheEnumeration(named) => assert_eq!(named, to),
            other => panic!("expected the target named, got {other:?}"),
        }
    }
}

/// The other two enumerated ops are material outright.
#[test]
fn category_and_attribute_ops_are_material() {
    let policy = MaterialityPolicy::default();
    let claims = vec!["catalog-admin".to_owned()];
    for op in [
        EnumeratedOp::CategoryOp,
        EnumeratedOp::AttributeDefinitionChange,
    ] {
        let ev = resolved(&policy, &claims);
        let verdict = ev
            .verdict(&MaterialAct::Enumerated(op))
            .expect("a resolved policy");
        assert_eq!(verdict, Materiality::Material, "{op:?}");
    }
}

/// **The affected-entity trigger is a threshold, and it is `>=`.** Both
/// sides of the boundary are asserted, since an off-by-one here lets a batch
/// of exactly the trigger size close on one signature.
#[test]
fn the_batch_trigger_is_inclusive() {
    let policy = MaterialityPolicy::default();
    let claims = vec!["catalog-admin".to_owned()];
    let trigger = policy.affected_entity_trigger();
    for (affected, expected) in [
        (trigger - 1, Materiality::NonMaterial),
        (trigger, Materiality::Material),
        (trigger + 1, Materiality::Material),
    ] {
        let ev = resolved(&policy, &claims);
        let verdict = ev
            .verdict(&MaterialAct::BatchAct { affected })
            .expect("a resolved policy");
        assert_eq!(
            verdict, expected,
            "{affected} affected against a trigger of {trigger}"
        );
    }
}

/// A tenant-configured trigger moves the boundary — so the probe above is
/// reading the policy rather than a constant.
#[test]
fn the_trigger_comes_from_the_policy_not_a_constant() {
    let policy = MaterialityPolicy::new(Vec::new(), 3, DEFAULT_APPROVER_COUNT);
    let claims = vec!["catalog-admin".to_owned()];
    let ev = resolved(&policy, &claims);
    let verdict = ev
        .verdict(&MaterialAct::BatchAct { affected: 3 })
        .expect("a resolved policy");
    assert_eq!(verdict, Materiality::Material);
    let ev = resolved(&policy, &claims);
    let below = ev
        .verdict(&MaterialAct::BatchAct { affected: 2 })
        .expect("a resolved policy");
    assert_eq!(below, Materiality::NonMaterial);
}

/// **Every registered `GovernedLiveOp` kind is material**, and the roster is
/// exactly six — one per owning slice. A seventh cannot arrive without this
/// assertion moving.
#[test]
fn every_registered_live_op_kind_is_material_and_there_are_six() {
    let policy = MaterialityPolicy::default();
    let claims = vec!["catalog-admin".to_owned()];
    // Every variant is in the roster, matched exhaustively — the assertion
    // a `len()` cannot make, since the array's type gives its length at
    // compile time. A seventh variant forces an arm here.
    for kind in MaterialLiveOp::ALL {
        match kind {
            MaterialLiveOp::TaxonomyOp
            | MaterialLiveOp::RecognizedSetOp
            | MaterialLiveOp::ScheduledTransitionCancel
            | MaterialLiveOp::FreezeParticipantOp
            | MaterialLiveOp::ReferenceProducerOp
            | MaterialLiveOp::PiiAllowListOp => {}
        }
    }
    let mut slices: Vec<&str> = MaterialLiveOp::ALL
        .iter()
        .map(|k| k.owning_slice())
        .collect();
    slices.sort_unstable();
    assert_eq!(slices, ["02", "03", "04", "06", "07", "10"]);
    for kind in MaterialLiveOp::ALL {
        let ev = resolved(&policy, &claims);
        let verdict = ev
            .verdict(&MaterialAct::LiveOp(kind))
            .expect("a resolved policy");
        assert_eq!(
            verdict,
            Materiality::Material,
            "{kind:?}, registered by slice {}",
            kind.owning_slice()
        );
    }
}

/// **The policy's own field set widens the registry**, never narrows it: a
/// tenant may add a column, and a column the registry calls bucket iii stays
/// material whatever the policy says.
#[test]
fn the_policy_field_set_widens_the_registry() {
    let policy = MaterialityPolicy::new(
        vec!["product_code".to_owned()],
        DEFAULT_AFFECTED_ENTITY_TRIGGER,
        DEFAULT_APPROVER_COUNT,
    );
    let claims = vec!["catalog-admin".to_owned()];
    let ev = resolved(&policy, &claims);
    let verdict = ev
        .verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Product,
            touched: &["product_code"],
        })
        .expect("a resolved policy");
    assert_eq!(
        verdict,
        Materiality::Material,
        "a structural column the policy names is material"
    );
}

/// **The policy's field set may raise a verdict; it may never switch a
/// refusal off.** Both fail-closed arms re-asserted with the policy naming
/// the very column under test — the interaction a `continue` on
/// `names_field` silently removed, making L-1's correction-door-only
/// guarantee tenant-configurable.
#[test]
fn the_policy_field_set_cannot_disable_a_refusal() {
    let claims = vec!["catalog-admin".to_owned()];

    let policy = MaterialityPolicy::new(
        vec!["metering_unit".to_owned()],
        DEFAULT_AFFECTED_ENTITY_TRIGGER,
        DEFAULT_APPROVER_COUNT,
    );
    let ev = resolved(&policy, &claims);
    let err = ev
        .verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Sku,
            touched: &["metering_unit"],
        })
        .expect_err("a tenant's field set cannot make bucket ii an ordinary touch");
    assert!(
        matches!(err, MaterialityRefusal::CorrectableTouch(_)),
        "{err:?}"
    );

    let policy = MaterialityPolicy::new(
        vec!["no_such_column".to_owned()],
        DEFAULT_AFFECTED_ENTITY_TRIGGER,
        DEFAULT_APPROVER_COUNT,
    );
    let ev = resolved(&policy, &claims);
    let err = ev
        .verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Product,
            touched: &["no_such_column"],
        })
        .expect_err("a tenant's field set cannot register a column");
    assert!(matches!(err, MaterialityRefusal::Registry(_)), "{err:?}");
}

/// `N`'s default is two and its floor is zero, reachable only by explicit
/// configuration — nothing clamps a configured zero upward.
#[test]
fn n_defaults_to_two_and_zero_is_reachable() {
    assert_eq!(DEFAULT_APPROVER_COUNT, 2);
    assert_eq!(MaterialityPolicy::default().approver_count(), 2);
    assert_eq!(APPROVER_COUNT_FLOOR, 0);
    let floor = MaterialityPolicy::new(
        Vec::new(),
        DEFAULT_AFFECTED_ENTITY_TRIGGER,
        APPROVER_COUNT_FLOOR,
    );
    assert_eq!(
        floor.approver_count(),
        APPROVER_COUNT_FLOOR,
        "the floor is reachable and nothing clamps it upward"
    );
}

/// **The policy's field set is a union with the registry, including the
/// bucket-less columns.** `names_field`'s contract is that a column in
/// *either* is material; scoping the promotion to the `Immaterial` arm
/// dropped it for `CreateOnly` and the two outside-the-scheme classes, so a
/// tenant naming `deprecation_provenance` got `min(N, 1)` — one approver —
/// where it asked for `N`. Bucket ii is still refused first, which is the
/// half the ordering exists for.
#[test]
fn the_policy_field_set_promotes_a_bucket_less_column_too() {
    let claims = vec!["catalog-admin".to_owned()];
    // `deprecation_provenance` is `Outside(Mechanical)` — it carries no
    // bucket at all, so nothing in the registry can make it material.
    let plain = MaterialityPolicy::default();
    let ev = resolved(&plain, &claims);
    assert_eq!(
        ev.verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Product,
            touched: &["deprecation_provenance"],
        })
        .expect("a resolved policy"),
        Materiality::NonMaterial,
        "the registry alone makes it immaterial"
    );

    let named = MaterialityPolicy::new(
        vec!["deprecation_provenance".to_owned()],
        DEFAULT_AFFECTED_ENTITY_TRIGGER,
        DEFAULT_APPROVER_COUNT,
    );
    let ev = resolved(&named, &claims);
    assert_eq!(
        ev.verdict(&MaterialAct::EntityPublish {
            kind: EntityKind::Product,
            touched: &["deprecation_provenance"],
        })
        .expect("a resolved policy"),
        Materiality::Material,
        "and the tenant's own set promotes it - the union, not a subset"
    );
}
