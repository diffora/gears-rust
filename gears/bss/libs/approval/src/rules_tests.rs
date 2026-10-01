#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::{
    ApproveEligibility, ApproveRefusal, ApproveStep, approve_eligibility, counted_approvals,
    evaluate_approve,
};
use crate::model::{ApprovalError, Decision, ItemRef, Policy, Unit, UnitState, Verdict};
use time::macros::datetime;
use uuid::Uuid;

fn unit(quorum: u32, submitted_by: Uuid) -> Unit {
    Unit {
        id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4(),
        kind: "prices".into(),
        ref_type: "book".into(),
        ref_id: Uuid::new_v4(),
        state: UnitState::Pending,
        common_effective_date: None,
        quorum_required: quorum,
        generation: 1,
        submitted_by,
        submitted_at: datetime!(2026-09-24 10:00 UTC),
        submit_note: None,
        decided_at: None,
        decided_note: None,
        snapshot: serde_json::json!({}),
        snapshot_hash: "h".into(),
        version: 1,
    }
}
fn vote(unit: &Unit, actor: Uuid, generation: i32) -> Decision {
    Decision {
        unit_id: unit.id,
        actor,
        generation,
        verdict: Verdict::Approve,
        note: None,
        at: datetime!(2026-09-24 11:00 UTC),
        stale: generation < unit.generation,
    }
}

/// One stored item per author, as a unit's items name them (only `created_by` is judged).
fn authored(authors: &[Uuid]) -> Vec<ItemRef> {
    authors
        .iter()
        .map(|a| ItemRef {
            item_type: "price".into(),
            item_id: Uuid::new_v4(),
            created_by: *a,
            before: None,
            after: serde_json::json!({}),
        })
        .collect()
}

#[test]
fn quorum_one_applies_on_the_first_independent_approve() {
    let author = Uuid::new_v4();
    let u = unit(1, author);
    assert_eq!(
        evaluate_approve(&u, &[], Uuid::new_v4(), &authored(&[author])).unwrap(),
        ApproveStep::Apply
    );
}
#[test]
fn quorum_two_needs_a_second_distinct_approver() {
    let author = Uuid::new_v4();
    let u = unit(2, author);
    let first = Uuid::new_v4();
    assert_eq!(
        evaluate_approve(&u, &[], first, &authored(&[author])).unwrap(),
        ApproveStep::NeedMore { have: 1, need: 2 }
    );
    assert_eq!(
        evaluate_approve(
            &u,
            &[vote(&u, first, 1)],
            Uuid::new_v4(),
            &authored(&[author])
        )
        .unwrap(),
        ApproveStep::Apply
    );
}
#[test]
fn the_submitter_and_every_item_author_are_excluded() {
    let author = Uuid::new_v4();
    let submitter = Uuid::new_v4();
    let u = unit(1, submitter);
    assert!(matches!(
        evaluate_approve(&u, &[], submitter, &authored(&[author])),
        Err(ApprovalError::SodViolation)
    ));
    assert!(matches!(
        evaluate_approve(&u, &[], author, &authored(&[author])),
        Err(ApprovalError::SodViolation)
    ));
}
#[test]
fn a_second_vote_by_the_same_actor_in_the_same_generation_is_refused() {
    let u = unit(2, Uuid::new_v4());
    let a = Uuid::new_v4();
    assert!(matches!(
        evaluate_approve(&u, &[vote(&u, a, 1)], a, &authored(&[])),
        Err(ApprovalError::DuplicateVote)
    ));
}
#[test]
fn a_vote_from_an_earlier_generation_neither_counts_nor_blocks_the_actor() {
    let mut u = unit(2, Uuid::new_v4());
    u.generation = 2;
    let a = Uuid::new_v4();
    let old = vote(&u, a, 1);
    assert!(old.stale);
    assert_eq!(
        evaluate_approve(&u, &[old], a, &authored(&[])).unwrap(),
        ApproveStep::NeedMore { have: 1, need: 2 }
    );
}
#[test]
fn a_decided_unit_takes_no_vote() {
    let mut u = unit(1, Uuid::new_v4());
    u.state = UnitState::Approved;
    assert!(matches!(
        evaluate_approve(&u, &[], Uuid::new_v4(), &authored(&[])),
        Err(ApprovalError::AlreadyDecided)
    ));
}
/// W2: the counts the readers show are the approve votes the quorum counts: of the unit's
/// current generation and not stale. A reject, a stale vote and a vote of an earlier generation
/// do not count.
#[test]
fn the_predicate_counts_the_current_generations_live_approves() {
    let mut refreshed = unit(3, Uuid::new_v4());
    refreshed.generation = 2;
    let (live, earlier, refuser, marked_voter) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let mut rejected = vote(&refreshed, refuser, 2);
    rejected.verdict = Verdict::Reject;
    let mut marked = vote(&refreshed, marked_voter, 2);
    marked.stale = true;
    let decisions = [
        vote(&refreshed, live, 2),
        vote(&refreshed, earlier, 1),
        rejected,
        marked,
    ];
    let judged = approve_eligibility(&refreshed, [], &decisions, Uuid::new_v4());
    assert_eq!(judged.approvals, 1, "{judged:?}");
    assert!(judged.refusal.is_none(), "{judged:?}");
    // The count alone needs no item and no actor (the phase 9 review's R4).
    assert_eq!(counted_approvals(&refreshed, &decisions), 1);
}
/// W2: the predicate is the engine's rule. On every fixture (quorum 0, 1 and 2, a stale vote, a
/// vote of an earlier generation, a decided unit) and for every actor (the submitter, an item
/// author, a voter of the current generation, a voter of an earlier one, a fresh reviewer), its
/// refusal is `evaluate_approve`'s error, and where the engine counts a vote, `have` is the
/// predicate's approvals plus that vote.
#[test]
fn the_predicate_answers_what_the_engine_answers() {
    let (submitter, author, reviewer, earlier_reviewer, fresh) = (
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
        Uuid::new_v4(),
    );
    let items = authored(&[author]);
    let mut fixtures = Vec::new();
    for quorum in [0, 1, 2] {
        let u = unit(quorum, submitter);
        fixtures.push((format!("quorum {quorum}, no vote"), u.clone(), Vec::new()));
        let cast = vec![vote(&u, reviewer, 1)];
        fixtures.push((format!("quorum {quorum}, one vote"), u, cast));
    }
    let mut refreshed = unit(2, submitter);
    refreshed.generation = 2;
    let cast = vec![
        vote(&refreshed, earlier_reviewer, 1),
        vote(&refreshed, reviewer, 2),
    ];
    fixtures.push((
        "a stale vote beside a live one".into(),
        refreshed.clone(),
        cast,
    ));
    let mut marked = vote(&refreshed, earlier_reviewer, 2);
    marked.stale = true;
    fixtures.push(("a vote marked stale".into(), refreshed, vec![marked]));
    let mut decided = unit(1, submitter);
    decided.state = UnitState::Approved;
    fixtures.push(("a decided unit".into(), decided, Vec::new()));
    for (name, u, decisions) in &fixtures {
        for actor in [submitter, author, reviewer, earlier_reviewer, fresh] {
            let judged =
                approve_eligibility(u, items.iter().map(|i| i.created_by), decisions, actor);
            assert_eq!(judged.approvals, counted_approvals(u, decisions), "{name}");
            match evaluate_approve(u, decisions, actor, &items) {
                Err(error) => assert_eq!(
                    judged.refusal.map(|r| ApprovalError::from(r).code()),
                    Some(error.code()),
                    "{name}, {actor}"
                ),
                Ok(step) => {
                    assert!(judged.refusal.is_none(), "{name}, {actor}: {judged:?}");
                    let have = judged.approvals + 1;
                    let expected = if have >= u.quorum_required {
                        ApproveStep::Apply
                    } else {
                        ApproveStep::NeedMore {
                            have,
                            need: u.quorum_required,
                        }
                    };
                    assert_eq!(step, expected, "{name}, {actor}");
                }
            }
        }
    }
}
/// The phase 9 review's R3: the predicate's refusal is one of three, each the engine's own error,
/// with its code and its text unchanged.
#[test]
fn each_refusal_is_the_engines_error_byte_for_byte() {
    for (refusal, code, text) in [
        (
            ApproveRefusal::AlreadyDecided,
            "UNIT_ALREADY_DECIDED",
            "the unit is already decided",
        ),
        (
            ApproveRefusal::SodViolation,
            "SOD_VIOLATION",
            "the actor authored or submitted this unit",
        ),
        (
            ApproveRefusal::DuplicateVote,
            "DUPLICATE_VOTE",
            "the actor already voted in this generation",
        ),
    ] {
        let error = ApprovalError::from(refusal);
        assert_eq!((error.code(), error.to_string().as_str()), (code, text));
    }
    let author = Uuid::new_v4();
    let u = unit(1, Uuid::new_v4());
    // The authors alone judge the separation of duties, from any iterator of them.
    assert_eq!(
        approve_eligibility(&u, vec![Uuid::new_v4(), author], &[], author),
        ApproveEligibility {
            approvals: 0,
            refusal: Some(ApproveRefusal::SodViolation),
        }
    );
    assert_eq!(
        approve_eligibility(&u, std::iter::empty(), &[], author).refusal,
        None
    );
}
#[test]
fn the_policy_overrides_per_kind_and_falls_back_to_star() {
    let p = Policy {
        default_quorum: 1,
        overrides: [("promotion".to_owned(), 0u32)].into_iter().collect(),
    };
    assert_eq!(p.quorum_for("promotion"), 0);
    assert_eq!(p.quorum_for("prices"), 1);
}
/// Every variant's `code()`, which the gears map to their wire codes. The expectation is an
/// exhaustive match, so a new variant does not compile until its code is pinned here.
#[test]
fn every_error_has_a_stable_code() {
    fn expected(error: &ApprovalError) -> &'static str {
        match error {
            ApprovalError::SodViolation => "SOD_VIOLATION",
            ApprovalError::AlreadyDecided => "UNIT_ALREADY_DECIDED",
            ApprovalError::DuplicateVote => "DUPLICATE_VOTE",
            ApprovalError::Contended => "UNIT_CONTENDED",
            ApprovalError::Locked { .. } => "ROW_LOCKED_PENDING",
            ApprovalError::NotSubmitter => "NOT_SUBMITTER",
            ApprovalError::NoteRequired => "NOTE_REQUIRED",
            ApprovalError::NoteTooLong => "NOTE_TOO_LONG",
            ApprovalError::UnitNotFound { .. } => "UNIT_NOT_FOUND",
            ApprovalError::Empty | ApprovalError::InvalidSubmit { .. } => "VALIDATION",
            ApprovalError::ApplyRefused { .. } => "APPLY_REFUSED",
            ApprovalError::GenerationMismatch { .. } => "GENERATION_MISMATCH",
            ApprovalError::Db(_) => "DB",
            ApprovalError::Store(_) => "STORE",
        }
    }
    let every = [
        ApprovalError::SodViolation,
        ApprovalError::AlreadyDecided,
        ApprovalError::DuplicateVote,
        ApprovalError::Contended,
        ApprovalError::Locked {
            item_type: "sku".into(),
            item_id: Uuid::nil(),
        },
        ApprovalError::NotSubmitter,
        ApprovalError::NoteRequired,
        ApprovalError::NoteTooLong,
        ApprovalError::UnitNotFound {
            unit_id: Uuid::nil(),
        },
        ApprovalError::Empty,
        ApprovalError::InvalidSubmit {
            code: "USAGE_NEEDS_METER",
            field: "unit".into(),
            detail: String::new(),
        },
        ApprovalError::ApplyRefused {
            code: "SKU_NAME_TAKEN",
            detail: String::new(),
        },
        ApprovalError::GenerationMismatch {
            seen: 1,
            current: 2,
        },
        ApprovalError::Db(sea_orm::DbErr::Custom("driver".into())),
        ApprovalError::Store("store".into()),
    ];
    let pinned: Vec<&str> = every.iter().map(expected).collect();
    assert_eq!(
        pinned,
        [
            "SOD_VIOLATION",
            "UNIT_ALREADY_DECIDED",
            "DUPLICATE_VOTE",
            "UNIT_CONTENDED",
            "ROW_LOCKED_PENDING",
            "NOT_SUBMITTER",
            "NOTE_REQUIRED",
            "NOTE_TOO_LONG",
            "UNIT_NOT_FOUND",
            "VALIDATION",
            "VALIDATION",
            "APPLY_REFUSED",
            "GENERATION_MISMATCH",
            "DB",
            "STORE",
        ],
        "one of each variant, in declaration order"
    );
    for error in &every {
        assert_eq!(error.code(), expected(error), "{error:?}");
    }
    let db_error = sea_orm::DbErr::Custom("retry classification".into());
    let error = ApprovalError::from(db_error.clone());
    assert_eq!(error.db_err(), Some(&db_error));
    assert!(ApprovalError::Contended.db_err().is_none());
}
