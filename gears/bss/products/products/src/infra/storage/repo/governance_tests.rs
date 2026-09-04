//! Store-level probes for the approval record's two stored values
//! (`design/05-governance.md` §5, `inst-gv-stored-snapshot`;
//! `dod-stored-snapshot`).
//!
//! # Why these live here and not in `repo_tests.rs`
//!
//! Every other repository probe in the crate lands in
//! `infra/storage/repo_tests.rs`, a single 4336-line module shared by every
//! slice. The domain layer already keeps each module's tests at its own
//! bottom — `domain/approval.rs` and `domain/materiality.rs` both do — and
//! this file introduces that convention into `repo/` for the first time, so
//! a slice's store probes sit beside the store they exercise. That is a
//! convention change rather than a local choice, and it is **registered for
//! ratification** rather than taken here alone.
//!
//! # The harness is a copy, and deliberately
//!
//! `repo_tests::harness` is private to that module and reaching it would
//! mean editing it. The eight lines below are the same shape for the same
//! reason it states: `sqlite::memory:` gives each **connection** its own
//! empty database, so a pool wider than one makes the migrations applied on
//! one connection invisible to a query on another.
//!
//! # What the flagship probe measures that the two nearest probes do not
//!
//! `dod-stored-snapshot`'s obligation is stateful: *"submit, edit the head,
//! and the **superseded record's** diff still renders the original
//! submission against the published version"*. Two probes at `HEAD` each
//! carry one half, and the split was measured by perturbation rather than
//! read off their names:
//!
//! | Perturbation | domain probe | column probe | below |
//! |---|---|---|---|
//! | `render_diff` stops reading its snapshot argument | red | **green** | red |
//! | the store stops preserving the submitted bytes | **green** | red | red |
//!
//! - `domain::approval::approval_tests::the_diff_renders_the_stored_submission_not_the_edited_head`
//!   is a pure-function probe over two literals it wrote itself. It does
//!   catch a renderer that drops its snapshot argument, so it is narrow
//!   rather than empty — but it has no head, no store and no supersession,
//!   and the "edited head" it names is a local it never passes to anything,
//!   so that one assertion of its three cannot fail.
//! - `repo_tests::approval_store_tests::a_superseded_record_keeps_the_content_it_was_submitted_with`
//!   reads the **column** back after a supersession, so it catches a store
//!   that loses the bytes — but it supersedes by a second *submission*
//!   rather than by a head edit, and it never renders a diff.
//!
//! Neither carries the join, which is where the defect the rule exists to
//! prevent actually lives: a caller handing [`render_diff`] the **live head**
//! where the stored snapshot belongs. That needs a head and a render in one
//! probe. So the case below drives the real chain — freeze a published
//! version, submit against it, edit the head, run the door's own supersede,
//! read the record back, and render from it — and it renders the live head a
//! second time to assert the two answers are distinguishable, which is what
//! stops the positive assertion being satisfiable by both.

#![allow(clippy::expect_used)]

use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, Condition, EntityTrait as _};
use sea_orm_migration::MigratorTrait as _;
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt as _, SecureUpdateExt as _};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::super::{
    HeadWrite, NewEntityVersion, NewProduct, ProductHeadSave, SavedName, VersionedEntityKind,
    find_product, insert_entity_version, insert_product, latest_entity_version, save_product_head,
    supersede_open_approval,
};
use super::{
    ApprovalPath, Consumption, DecisionOutcome, DecisionVerdict, Elevation, NewApproval,
    NewDecision, NewElevation, admit_elevated_call, consume_approval, discharge_posthoc_review,
    encode_field_set, gate_candidate_by_id, gate_candidates, open_breakglass_session,
    read_approval, record_decision, resolve_materiality_policy, submit_approval,
    write_materiality_policy,
};
use crate::domain::approval::{ApprovalState, ApproverDiff, diff_basis_for, render_diff};
use crate::domain::governance::{ApprovalId, EntityRef, GateSubject, SubjectKind};
use crate::domain::materiality::{
    DEFAULT_AFFECTED_ENTITY_TRIGGER, DEFAULT_APPROVER_COUNT, MaterialAct, MaterialityEvaluator,
    MaterialityPolicy, Resolution,
};
use crate::infra::storage::entity::{approval, approval_decision, breakglass_session};
use crate::infra::storage::migrations::Migrator;
use crate::test_support::at;

const TENANT: Uuid = Uuid::from_u128(0x9e_11);
const BRAND: Uuid = Uuid::from_u128(0x9e_b1);
const PRODUCT: Uuid = Uuid::from_u128(0x9e_f0);
const ACTOR: Uuid = Uuid::from_u128(0x9e_ac);
const AUTHOR: Uuid = Uuid::from_u128(0x9e_a0);
const APPROVER: Uuid = Uuid::from_u128(0x9e_a1);
/// The two **platform** principals an elevation's two-person path names
/// (**P-D-133** row 9). Outside the tenant on purpose: that is the whole
/// reason they are not an `ApprovalRecord`'s approvers.
const PLATFORM_A: Uuid = Uuid::from_u128(0x9e_c1);
const PLATFORM_B: Uuid = Uuid::from_u128(0x9e_c2);

/// The content frozen at published version 1 — the diff's **basis**.
const PUBLISHED: &str = r#"{"name":"Fibre 500","regionScope":"eu,apac"}"#;
/// The content the author submitted for approval.
const SUBMITTED: &str = r#"{"name":"Fibre 500 Pro","regionScope":"eu,apac"}"#;
/// The name the head is edited to **after** submission. It is written into
/// the database, not just into a local, so the assertion that it is absent
/// from the approver's diff is a claim about a value that really moved.
const EDITED_HEAD_NAME: &str = "Fibre 900 Unapproved";

/// One connection, its own in-memory database, migrations applied.
async fn harness() -> DBProvider<DbError> {
    let opts = ConnectOpts {
        max_conns: Some(1),
        min_conns: Some(1),
        ..Default::default()
    };
    let db = connect_db("sqlite::memory:", opts)
        .await
        .expect("connect in-memory sqlite");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    DBProvider::<DbError>::new(db)
}

fn subject() -> GateSubject {
    GateSubject::entity_publish(EntityRef {
        tenant_id: TENANT,
        entity_kind: bss_products_sdk::models::EntityKind::Product,
        entity_id: PRODUCT,
    })
}

/// A material act, so the descriptor's count comes from `approver_count`
/// alone and no bucket registration is in the way of what these probes
/// measure.
const MATERIAL: MaterialAct<'static> = MaterialAct::PolicyMutation;

fn submission<'a>(
    id: ApprovalId,
    subject: &'a GateSubject,
    content: &'a str,
    basis: Option<i64>,
    evaluator: MaterialityEvaluator<'a>,
) -> NewApproval<'a> {
    NewApproval {
        approval_id: id,
        subject,
        internal_revision: 1,
        content_snapshot: content,
        diff_basis: basis,
        act: &MATERIAL,
        evaluator,
        finance_material: false,
        approver_count: 2,
        submitter: AUTHOR,
        author_override_ack: None,
    }
}

/// Seed the head and, where `published` is set, freeze it as version 1.
async fn seed_head(runner: &impl DBRunner, scope: &AccessScope, published: Option<&str>) {
    insert_product(
        runner,
        scope,
        NewProduct {
            product_id: PRODUCT,
            tenant_id: TENANT,
            brand_id: BRAND,
            name: "Fibre 500".to_owned(),
            name_normalized: "fibre 500".to_owned(),
            product_code: Some("FIBRE-500".to_owned()),
            region_scope: "eu,apac".to_owned(),
            brand_scope: String::new(),
            created_by: "principal:author-1".to_owned(),
            created_at: at(9),
            cloned_from: None,
            cloned_from_version: None,
        },
    )
    .await
    .expect("insert the head");

    if let Some(content) = published {
        insert_entity_version(
            runner,
            scope,
            NewEntityVersion {
                tenant_id: TENANT,
                entity_kind: VersionedEntityKind::Product,
                entity_id: PRODUCT,
                published_version: 1,
                content: content.to_owned(),
                content_digest: (1..=32_u8).collect(),
                digest_version: 1,
                approval_ref: None,
                actor_ref: ACTOR,
                published_at: at(10),
            },
        )
        .await
        .expect("freeze published version 1");
    }
}

/// Edit the head after submission, then run the pair the save door runs:
/// the write, and `supersede_open_approval` against the same subject
/// (`api/rest/products.rs` drives exactly this order).
async fn edit_the_head(runner: &impl DBRunner, scope: &AccessScope, subject: &GateSubject) {
    let save = ProductHeadSave {
        name: Some(SavedName {
            value: EDITED_HEAD_NAME.to_owned(),
            normalized: "fibre 900 unapproved".to_owned(),
        }),
        ..ProductHeadSave::default()
    };
    let outcome = save_product_head(runner, scope, TENANT, PRODUCT, 1, &save, at(12))
        .await
        .expect("the head edit lands");
    assert_eq!(
        outcome,
        HeadWrite::Applied,
        "the probe's premise is a head that actually moved; an unmatched save would leave the \
         snapshot trivially equal to it and prove nothing"
    );

    supersede_open_approval(runner, scope, TENANT, subject, at(12))
        .await
        .expect("the invalidation hook's store half runs");
}

/// **The flagship probe** (`dod-stored-snapshot`, `design/05` §5): submit,
/// edit the head, and the superseded record's diff still renders the
/// **original submission** against the **published version**.
///
/// Every operand here is read back out of the database rather than being a
/// literal the test also wrote: the snapshot comes off the record, the basis
/// content comes off the frozen version row, and the head's post-edit name
/// comes off the head row. That is what makes the refutation real — a store
/// that let the snapshot follow the head, or a caller that re-derived the
/// diff from the live head, moves at least one of these three and fails.
#[tokio::test]
async fn the_superseded_records_diff_renders_the_submission_not_the_edited_head() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    let policy = MaterialityPolicy::default();
    let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));

    seed_head(&conn, &scope, Some(PUBLISHED)).await;

    let record_id = ApprovalId::new(Uuid::new_v4());
    submit_approval(
        &conn,
        &scope,
        submission(
            record_id,
            &subject,
            SUBMITTED,
            diff_basis_for(Some(1)),
            evaluator,
        ),
        at(11),
    )
    .await
    .expect("the submission is stored");

    edit_the_head(&conn, &scope, &subject).await;

    // The head really moved, so "the diff does not show it" is a claim with
    // something to be false about.
    let head = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read the head")
        .expect("the head row exists");
    assert_eq!(
        head.name, EDITED_HEAD_NAME,
        "the edit landed on the head the approver must NOT be shown"
    );

    let record = read_approval(&conn, &scope, TENANT, record_id)
        .await
        .expect("read runs")
        .expect("the record exists");
    assert_eq!(
        record.state, "superseded",
        "a frozen-content write on the subject supersedes the open record (inst-gv-supersede)"
    );

    // The basis content is read from the frozen version row, not retyped:
    // a literal here would keep the probe green against a store that lost it.
    let (basis_version, basis_content) =
        latest_entity_version(&conn, &scope, TENANT, VersionedEntityKind::Product, PRODUCT)
            .await
            .expect("read the frozen version")
            .expect("version 1 is frozen");

    let shown = render_diff(
        &record.content_snapshot,
        record.diff_basis,
        Some(&basis_content),
    );
    match &shown {
        ApproverDiff::Against {
            basis,
            submitted,
            basis_content: against,
        } => {
            assert_eq!(
                *basis, basis_version,
                "the diff renders against the pinned basis"
            );
            assert_eq!(
                submitted, SUBMITTED,
                "the approver sees what was submitted, not what the head became"
            );
            assert_eq!(
                against, PUBLISHED,
                "the basis is the published version's frozen content"
            );
            assert!(
                !submitted.contains(EDITED_HEAD_NAME),
                "the edited head reached the approver's diff: {submitted}"
            );
        }
        other => panic!("expected a diff against the published version, got {other:?}"),
    }

    // The teeth. A re-derived diff — the pricing defect — is the same call
    // with the live head in the snapshot's place. Asserting the two answers
    // differ is what proves the assertion above is not satisfied by both.
    let re_derived = render_diff(
        &format!(r#"{{"name":"{}","regionScope":"eu,apac"}}"#, head.name),
        record.diff_basis,
        Some(&basis_content),
    );
    assert_ne!(
        shown, re_derived,
        "if rendering from the stored snapshot and rendering from the live head agreed, this \
         probe could not tell them apart and would pass against the defect it exists to catch"
    );
}

/// A **first publish** submitted and superseded the same way renders a
/// whole-content addition against no basis — and still not the edited head.
///
/// The arm is separate because `diff_basis` is NULL here and the rule's own
/// wording says filling that gap by convention would most plausibly diff the
/// draft against the head, which is the re-derivation it forbids.
#[tokio::test]
async fn a_first_publish_record_renders_its_submission_against_no_basis() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    let policy = MaterialityPolicy::default();
    let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));

    // No frozen version at all: this entity has never been published.
    seed_head(&conn, &scope, None).await;

    let record_id = ApprovalId::new(Uuid::new_v4());
    submit_approval(
        &conn,
        &scope,
        submission(
            record_id,
            &subject,
            SUBMITTED,
            diff_basis_for(None),
            evaluator,
        ),
        at(11),
    )
    .await
    .expect("the first-publish submission is stored");

    edit_the_head(&conn, &scope, &subject).await;

    let record = read_approval(&conn, &scope, TENANT, record_id)
        .await
        .expect("read runs")
        .expect("the record exists");
    assert_eq!(record.diff_basis, None, "a first publish pins no basis");

    assert!(
        latest_entity_version(&conn, &scope, TENANT, VersionedEntityKind::Product, PRODUCT)
            .await
            .expect("read the frozen version")
            .is_none(),
        "the premise of the arm: there is no last published version to diff against"
    );

    assert_eq!(
        render_diff(&record.content_snapshot, record.diff_basis, None),
        ApproverDiff::WholeContentAddition {
            submitted: SUBMITTED.to_owned(),
        },
        "the whole stored submission, against no basis, and never the head that moved after it"
    );
}

// ---------------------------------------------------------------------------
// The ceremony's finalization (`dod-decide`, `inst-ap-edge-reject`).
// ---------------------------------------------------------------------------

/// Submit one record against `subject` and answer its id.
async fn submit_one(
    runner: &impl DBRunner,
    scope: &AccessScope,
    subject: &GateSubject,
) -> ApprovalId {
    let policy = MaterialityPolicy::default();
    let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
    let id = ApprovalId::new(Uuid::new_v4());
    submit_approval(
        runner,
        scope,
        submission(id, subject, SUBMITTED, diff_basis_for(Some(1)), evaluator),
        at(11),
    )
    .await
    .expect("the submission is stored");
    id
}

fn decision(id: ApprovalId, verdict: DecisionVerdict, reason: Option<&str>) -> NewDecision<'_> {
    NewDecision {
        tenant_id: TENANT,
        approval_id: id,
        approver_principal: APPROVER,
        verdict,
        reason,
        override_acknowledgments: None,
    }
}

/// **A rejection finalizes the record and leaves the subject as it was**
/// (`design/05` §2 rule 4, §4 row 3).
///
/// The head is asserted unchanged as well as the record finalized, because a
/// finalizer that also touched the subject would satisfy the first half
/// alone — and there is no `published -> draft` edge in this gear for it to
/// use legally.
#[tokio::test]
async fn a_reasoned_rejection_finalizes_the_record_and_leaves_the_subject_alone() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    let before = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read the head")
        .expect("the head row exists");

    let outcome = record_decision(
        &conn,
        &scope,
        decision(id, DecisionVerdict::Rejected, Some("the tier is wrong")),
        APPROVER,
        at(12),
    )
    .await
    .expect("a reasoned rejection is admitted");
    assert_eq!(outcome, DecisionOutcome::Finalized);

    let record = read_approval(&conn, &scope, TENANT, id)
        .await
        .expect("read runs")
        .expect("the record exists");
    assert_eq!(record.state, "rejected");
    assert!(
        record.finalized_at.is_some(),
        "chk_products_approval_finalized pins the pair: a terminal state with a NULL \
         finalized_at is refused by the engine, so the flip must write both"
    );

    let after = find_product(&conn, &scope, TENANT, PRODUCT)
        .await
        .expect("read the head")
        .expect("the head row exists");
    assert_eq!(
        after.lifecycle_state, before.lifecycle_state,
        "the subject stays as it was"
    );
    assert_eq!(after.internal_revision, before.internal_revision);
    assert_eq!(after.name, before.name);
}

/// **An approval finalizes nothing.** The record stays open for the second
/// signature C1 asks for.
///
/// Without this case a finalizer that flipped on *either* verdict would pass
/// the rejection probe and close every record on its first approval.
#[tokio::test]
async fn an_approval_leaves_the_record_open() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    let outcome = record_decision(
        &conn,
        &scope,
        decision(id, DecisionVerdict::Approved, None),
        APPROVER,
        at(12),
    )
    .await
    .expect("an approval is admitted");
    assert_eq!(outcome, DecisionOutcome::Appended);
    assert_eq!(
        read_approval(&conn, &scope, TENANT, id)
            .await
            .expect("read runs")
            .expect("the record exists")
            .state,
        "pending",
        "one signature of two closes nothing"
    );
}

/// **A rejection on a `satisfied` record is refused, not silently
/// unfinalized.**
///
/// §4 row 5 admits no `satisfied -> rejected` edge, and the alternative
/// leaves a recorded rejection against a record the gate would still
/// authorize. No writer produces `satisfied` at this commit (§7 row 11), so
/// the state is written by hand here — the same shortcut `repo_tests` takes
/// for a state whose door is not this slice's, and the migration's
/// append-only guard permits it because `satisfied` is not terminal.
///
/// The paired control is that an **approval** in the same state is admitted,
/// which is what makes the refusal about the edge rather than about the
/// state.
#[tokio::test]
async fn a_rejection_of_a_satisfied_record_is_refused_and_an_approval_is_not() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    approval::Entity::update_many()
        .secure()
        .scope_with(&scope)
        .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(TENANT))
                .add(approval::Column::ApprovalId.eq(id.get())),
        )
        .exec(&conn)
        .await
        .expect("the setup write lands: satisfied is not a terminal state");

    let refused = record_decision(
        &conn,
        &scope,
        decision(id, DecisionVerdict::Rejected, Some("too late")),
        APPROVER,
        at(12),
    )
    .await
    .expect_err("no satisfied -> rejected edge exists");
    assert!(
        refused
            .to_string()
            .contains("no satisfied -> rejected edge"),
        "{refused}"
    );

    // The control: the state is not what refuses, the edge is.
    let outcome = record_decision(
        &conn,
        &scope,
        decision(id, DecisionVerdict::Approved, None),
        APPROVER,
        at(12),
    )
    .await
    .expect("an approval on a satisfied record adds a signature and moves no state");
    assert_eq!(outcome, DecisionOutcome::Appended);
    assert_eq!(
        read_approval(&conn, &scope, TENANT, id)
            .await
            .expect("read runs")
            .expect("the record exists")
            .state,
        "satisfied"
    );
}

/// **The finalize carries the open-state predicate on its own `UPDATE`.**
///
/// The racer is a frozen-content write superseding the record between the
/// caller's read and the flip. Driving it exactly: submit, supersede through
/// the door's own writer, then finalize. `record_decision`'s own guard sees
/// the superseded state first and refuses with `APPROVAL_SUPERSEDED`, so this
/// case is the *reachable* half of the same defect class — the one that
/// proves the refusal is a classified 409 and not the append-only trigger's
/// 500, which is what `supersede_open_approval`'s first build answered for a
/// legal act.
#[tokio::test]
async fn a_record_superseded_under_the_decision_refuses_rather_than_hitting_the_trigger() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    edit_the_head(&conn, &scope, &subject).await;
    assert_eq!(
        read_approval(&conn, &scope, TENANT, id)
            .await
            .expect("read runs")
            .expect("the record exists")
            .state,
        "superseded"
    );

    let refused = record_decision(
        &conn,
        &scope,
        decision(id, DecisionVerdict::Rejected, Some("stale")),
        APPROVER,
        at(13),
    )
    .await
    .expect_err("a decision is admitted only while the record is open");
    assert!(
        refused.to_string().contains("APPROVAL_SUPERSEDED"),
        "the refusal must be the declared 409 and not a trigger failure: {refused}"
    );
}

// ---------------------------------------------------------------------------
// One-shot consumption (`dod-one-shot-consumption`) and the host's operand.
// ---------------------------------------------------------------------------

/// Move `id` to `satisfied` by hand — no writer produces that state at this
/// commit (§7 row 11) and `satisfied` is not terminal, so the append-only
/// guard admits the setup write.
async fn mark_satisfied(runner: &impl DBRunner, scope: &AccessScope, id: ApprovalId) {
    approval::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(approval::Column::State, Expr::value("satisfied".to_owned()))
        .filter(
            Condition::all()
                .add(approval::Column::TenantId.eq(TENANT))
                .add(approval::Column::ApprovalId.eq(id.get())),
        )
        .exec(runner)
        .await
        .expect("satisfied is not terminal, so the setup write lands");
}

/// **Two acts off one satisfied record, and exactly one spends it** — the
/// probe `dod-one-shot-consumption` names, at the store.
///
/// The one-shot is the `UPDATE`'s own `state = 'satisfied'` predicate, so the
/// second call matches zero rows whatever order the two ran in. The
/// door-level half of the `DoD` — *"in the same transaction as the authorized
/// act"* — is not measurable here and is not claimed: this function opens no
/// transaction.
#[tokio::test]
async fn one_satisfied_record_is_spent_exactly_once() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;
    mark_satisfied(&conn, &scope, id).await;

    assert_eq!(
        consume_approval(&conn, &scope, TENANT, id, at(13))
            .await
            .expect("the first act spends it"),
        Consumption::Spent
    );
    assert_eq!(
        consume_approval(&conn, &scope, TENANT, id, at(14))
            .await
            .expect("the second act reads an answer, not a driver failure"),
        Consumption::AlreadySpentOrClosed,
        "a second publish off one approval must fail, and it must fail as a classified answer \
         rather than on the append-only trigger"
    );

    let record = read_approval(&conn, &scope, TENANT, id)
        .await
        .expect("read runs")
        .expect("the record exists");
    assert_eq!(record.state, "consumed");
    assert!(
        record.finalized_at.is_some(),
        "chk_products_approval_finalized pins the pair on both dialects"
    );
}

/// A record that never reached `satisfied` is not spendable, and the refusal
/// is the same classified answer rather than an error.
///
/// Without this case a `consume_approval` whose predicate had drifted to "any
/// state" would pass the probe above — its first call would spend a `pending`
/// record and its second would still find nothing.
#[tokio::test]
async fn a_pending_record_is_not_spendable() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    assert_eq!(
        consume_approval(&conn, &scope, TENANT, id, at(13))
            .await
            .expect("a verdict"),
        Consumption::AlreadySpentOrClosed
    );
    assert_eq!(
        read_approval(&conn, &scope, TENANT, id)
            .await
            .expect("read runs")
            .expect("the record exists")
            .state,
        "pending",
        "and nothing was written"
    );
}

/// **`gate_candidates` carries every state, which is what makes
/// `PreAuthorized` answerable at all.**
///
/// A reader scoped to `satisfied` would leave that mode with no operand,
/// which is how it came to have no call path. Two records on one subject —
/// one consumed, one open — and both are returned.
#[tokio::test]
async fn the_gates_operand_carries_consumed_records_too() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;

    let spent = submit_one(&conn, &scope, &subject).await;
    mark_satisfied(&conn, &scope, spent).await;
    consume_approval(&conn, &scope, TENANT, spent, at(13))
        .await
        .expect("spend it");
    // The partial UNIQUE admits a second open record now that the first is
    // terminal, which is the history `PreAuthorized` reads back.
    let open = submit_one(&conn, &scope, &subject).await;

    let candidates = gate_candidates(&conn, &scope, &subject)
        .await
        .expect("read the candidates");
    assert_eq!(candidates.len(), 2, "both records, not just the open one");
    let states: Vec<ApprovalState> = candidates.iter().map(|c| c.state).collect();
    assert!(states.contains(&ApprovalState::Consumed), "{states:?}");
    assert!(states.contains(&ApprovalState::Pending), "{states:?}");
    assert!(
        candidates.iter().any(|c| c.approval_id == spent),
        "the consumed record is reachable by id, which is what PreAuthorized matches on"
    );
    assert!(candidates.iter().any(|c| c.approval_id == open));
    assert!(
        candidates.iter().all(|c| !c.override_acknowledged),
        "no acknowledgment was stored on either record"
    );
}

/// An acknowledgment stored on a decision row reaches the host's operand.
///
/// The flag is read off **both** homes — the author's column at effective
/// quorum zero and any approver's decision row above it — so a reader that
/// checked only the record column would answer `false` for every `N >= 1`
/// override, which is the majority of them.
#[tokio::test]
async fn a_decision_rows_acknowledgment_reaches_the_operand() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    record_decision(
        &conn,
        &scope,
        NewDecision {
            tenant_id: TENANT,
            approval_id: id,
            approver_principal: APPROVER,
            verdict: DecisionVerdict::Approved,
            reason: None,
            override_acknowledgments: Some("uncomposed-bundle"),
        },
        APPROVER,
        at(12),
    )
    .await
    .expect("the approval with an acknowledgment lands");

    let candidates = gate_candidates(&conn, &scope, &subject)
        .await
        .expect("read the candidates");
    assert!(
        candidates
            .iter()
            .any(|c| c.approval_id == id && c.override_acknowledged),
        "the acknowledgment on the decision row must reach the verdict's operand"
    );
}

// ---------------------------------------------------------------------------
// Break-glass (`dod-breakglass-open`, `dod-breakglass-expiry`).
// ---------------------------------------------------------------------------

const SESSION: Uuid = Uuid::from_u128(0x9e_5e);
const OPERATOR: Uuid = Uuid::from_u128(0x9e_09);
const TARGET: Uuid = Uuid::from_u128(0x9e_ff);
const SECOND_PRINCIPAL: Uuid = Uuid::from_u128(0x9e_0a);

/// The session is scoped by **`target_tenant`**, not by an owning tenant: the
/// acting principal is outside the tenant entirely and the thing the session
/// reaches is one tenant's data.
fn elevation_scope() -> AccessScope {
    AccessScope::for_tenant(TARGET)
}

fn elevation(path: ApprovalPath) -> NewElevation {
    NewElevation {
        session_id: SESSION,
        principal: OPERATOR,
        target_tenant: TARGET,
        valid_from: at(10),
        valid_until: at(14),
        path,
        opened_at: at(10),
    }
}

/// **A session opens with its reason, window, target and one path**, and the
/// two paths are exclusive by construction.
///
/// `ApprovalPath` makes the wrong shape unrepresentable rather than guarded:
/// there is no value of it that sets both columns or neither, which is what
/// `chk_products_breakglass_path` enforces at the engine on both dialects.
/// Both arms are opened so the assertion covers the enum and not one variant.
#[tokio::test]
async fn a_session_opens_on_either_path_and_never_on_both() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = elevation_scope();

    open_breakglass_session(
        &conn,
        &scope,
        elevation(ApprovalPath::PostHoc),
        "cross-tenant incident 4711",
    )
    .await
    .expect("the post-hoc session opens");

    let row = breakglass_session::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(Condition::all().add(breakglass_session::Column::SessionId.eq(SESSION)))
        .one(&conn)
        .await
        .expect("read runs")
        .expect("the session exists");
    assert_eq!(row.reason, "cross-tenant incident 4711");
    assert_eq!(row.target_tenant, TARGET);
    assert_eq!(row.valid_from, at(10));
    assert_eq!(row.valid_until, at(14));
    assert_eq!(row.posthoc_state.as_deref(), Some("pending"));
    assert_eq!(row.two_person_approval_ref, None, "the paths are exclusive");
    assert!(!row.expired_emitted, "the CAS stamp starts unflipped");

    // The other arm, on its own session id.
    let reference = Uuid::from_u128(0xa9_77);
    let mut two_person = elevation(ApprovalPath::TwoPerson {
        reference,
        approver_a: PLATFORM_A,
        approver_b: PLATFORM_B,
    });
    two_person.session_id = Uuid::from_u128(0x9e_5f);
    open_breakglass_session(&conn, &scope, two_person, "planned drill")
        .await
        .expect("the two-person session opens");
    let row = breakglass_session::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(
            Condition::all()
                .add(breakglass_session::Column::SessionId.eq(Uuid::from_u128(0x9e_5f))),
        )
        .one(&conn)
        .await
        .expect("read runs")
        .expect("the session exists");
    assert_eq!(row.two_person_approval_ref, Some(reference));
    assert_eq!(row.posthoc_state, None, "the paths are exclusive");
}

/// An empty reason is refused by the engine, on both dialects.
///
/// The probe exists because this module deliberately does **not** restate the
/// `CHECK` in Rust: a second guard could drift from the schema, so the claim
/// under test is that the schema is the guard.
#[tokio::test]
async fn a_session_with_no_reason_is_refused_by_the_engine() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = elevation_scope();
    let err = open_breakglass_session(&conn, &scope, elevation(ApprovalPath::PostHoc), "")
        .await
        .expect_err("a mandatory reason is mandatory");
    assert!(
        err.to_string().contains("chk_products_breakglass_reason"),
        "{err}"
    );
}

/// **Ten calls after expiry emit exactly one event** — P-D-68 arm 2's whole
/// point, and the answer to item 19's *"a session called ten times after
/// expiry emits ten"*.
///
/// **What ten calls prove, and what they do not.** They prove the stamp is
/// consulted at all: without the `expired_emitted = false` predicate every
/// call flips and emits, and the count reads ten. They do **not** distinguish
/// a CAS from a guarded read-then-write — an earlier revision of this comment
/// claimed they did. These are sequential `await`s on a single-connection
/// harness, so nothing races, and a read-then-write survives ten calls exactly
/// as it survives two. The concurrent property is the `UPDATE`'s own predicate
/// and is unmeasurable here; ten is item 19's own number, kept because it is
/// the shape the question was asked in.
#[tokio::test]
async fn ten_calls_past_the_window_emit_exactly_one_expiry() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = elevation_scope();
    open_breakglass_session(&conn, &scope, elevation(ApprovalPath::PostHoc), "incident")
        .await
        .expect("the session opens");

    let mut emissions = 0_u32;
    for _ in 0..10 {
        match admit_elevated_call(&conn, &scope, SESSION, at(15))
            .await
            .expect("the judgement runs")
        {
            Elevation::Expired { emit_expired } => emissions += u32::from(emit_expired),
            other => panic!("past the window every call is refused, got {other:?}"),
        }
    }
    assert_eq!(
        emissions, 1,
        "the winner emits and every replay emits nothing (P-D-68 arm 2)"
    );
    assert!(
        breakglass_session::Entity::find()
            .secure()
            .scope_with(&scope)
            .filter(Condition::all().add(breakglass_session::Column::SessionId.eq(SESSION)))
            .one(&conn)
            .await
            .expect("read runs")
            .expect("the session exists")
            .expired_emitted,
        "the stamp is flipped and stays flipped"
    );
}

/// **A read inside the window is admitted** — the positive control on
/// `BREAKGLASS_EXPIRED`, without which an inverted comparison passes every
/// other criterion.
///
/// The boundaries are swept rather than sampled: `valid_from` itself is
/// inside, `valid_until` itself is **outside** (the interval is half-open),
/// and an instant before the window is its own answer rather than an expiry.
#[tokio::test]
async fn the_window_is_half_open_and_both_boundaries_are_probed() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = elevation_scope();
    open_breakglass_session(&conn, &scope, elevation(ApprovalPath::PostHoc), "incident")
        .await
        .expect("the session opens");

    assert_eq!(
        admit_elevated_call(&conn, &scope, SESSION, at(9))
            .await
            .expect("judged"),
        Elevation::NotYetValid,
        "before valid_from is not an expiry, and folding it into one would emit \
         BreakGlassExpired for a session that has not begun"
    );
    assert_eq!(
        admit_elevated_call(&conn, &scope, SESSION, at(10))
            .await
            .expect("judged"),
        Elevation::Admitted,
        "valid_from itself is inside: the interval is [from, until)"
    );
    assert_eq!(
        admit_elevated_call(&conn, &scope, SESSION, at(13))
            .await
            .expect("judged"),
        Elevation::Admitted,
        "the positive control: a read inside the window is admitted"
    );
    assert!(
        matches!(
            admit_elevated_call(&conn, &scope, SESSION, at(14))
                .await
                .expect("judged"),
            Elevation::Expired { .. }
        ),
        "valid_until itself is outside: the interval is half-open, and a closed one would \
         admit one call too many"
    );
}

/// A caller naming a session outside its scope gets the same answer as one
/// naming a session that does not exist.
#[tokio::test]
async fn a_session_of_another_tenant_is_not_visible() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    open_breakglass_session(
        &conn,
        &elevation_scope(),
        elevation(ApprovalPath::PostHoc),
        "incident",
    )
    .await
    .expect("the session opens");

    let err = admit_elevated_call(&conn, &AccessScope::for_tenant(TENANT), SESSION, at(13))
        .await
        .expect_err("another tenant's scope sees no session");
    assert!(err.to_string().contains("no elevation session"), "{err}");

    // The in-scope control, in this test rather than a sibling: without it a
    // read broken for **every** scope passes here unchanged, and the probe
    // would be measuring "the read fails" rather than "the read is scoped".
    assert_eq!(
        admit_elevated_call(&conn, &elevation_scope(), SESSION, at(13))
            .await
            .expect("the session's own scope sees it"),
        Elevation::Admitted
    );
}

/// **The post-hoc obligation is discharged once**, by the second platform
/// principal's late decision (P-D-68 arm 3) — and a two-person session has
/// nothing to discharge.
#[tokio::test]
async fn the_posthoc_obligation_discharges_once_and_only_where_it_exists() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = elevation_scope();
    open_breakglass_session(&conn, &scope, elevation(ApprovalPath::PostHoc), "incident")
        .await
        .expect("the session opens");

    assert!(
        discharge_posthoc_review(
            &conn,
            &scope,
            SESSION,
            SECOND_PRINCIPAL,
            SECOND_PRINCIPAL,
            at(16)
        )
        .await
        .expect("the discharge runs"),
        "the second platform principal's late decision closes the obligation"
    );
    assert!(
        !discharge_posthoc_review(
            &conn,
            &scope,
            SESSION,
            SECOND_PRINCIPAL,
            SECOND_PRINCIPAL,
            at(17)
        )
        .await
        .expect("the second discharge runs"),
        "two reviewers racing produce one discharge, and the loser is told so"
    );
    let row = breakglass_session::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(Condition::all().add(breakglass_session::Column::SessionId.eq(SESSION)))
        .one(&conn)
        .await
        .expect("read runs")
        .expect("the session exists");
    assert_eq!(row.posthoc_state.as_deref(), Some("reviewed"));
    assert_eq!(row.reviewed_by, Some(SECOND_PRINCIPAL));
    assert_eq!(row.reviewed_at, Some(at(16)));

    // A two-person session has no obligation, so there is nothing to close.
    let mut two_person = elevation(ApprovalPath::TwoPerson {
        reference: Uuid::from_u128(0xa9_78),
        approver_a: PLATFORM_A,
        approver_b: PLATFORM_B,
    });
    two_person.session_id = Uuid::from_u128(0x9e_60);
    open_breakglass_session(&conn, &scope, two_person, "drill")
        .await
        .expect("opens");
    assert!(
        !discharge_posthoc_review(
            &conn,
            &scope,
            Uuid::from_u128(0x9e_60),
            SECOND_PRINCIPAL,
            SECOND_PRINCIPAL,
            at(16)
        )
        .await
        .expect("the discharge runs"),
        "a session approved before it opened has no post-hoc obligation to discharge"
    );
}

// ---------------------------------------------------------------------------
// Regressions for the 2026-09-02 four-lens review.
// ---------------------------------------------------------------------------

/// **The opener cannot be their own post-hoc reviewer** — the regression for
/// the review's HIGH on `inst-bg-open`'s floor.
///
/// The floor is *"two **distinct** platform principals"*, and on the post-hoc
/// arm both are columns of one row, so the comparison presupposes nothing
/// about §7 row 9. While it was absent, the operator who opened a session
/// could discharge their own obligation and the row stood — permanently,
/// the table being append-only evidence — as a two-person ceremony performed
/// by one human.
///
/// The paired control is that a genuinely second principal discharges it, so
/// the refusal is about the identity and not about the call.
#[tokio::test]
async fn the_session_opener_cannot_discharge_their_own_obligation() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = elevation_scope();
    open_breakglass_session(&conn, &scope, elevation(ApprovalPath::PostHoc), "incident")
        .await
        .expect("the session opens");

    let refused = discharge_posthoc_review(&conn, &scope, SESSION, OPERATOR, OPERATOR, at(16))
        .await
        .expect_err("one principal may not be both halves of a two-person floor");
    assert!(
        refused.to_string().contains("cannot be its own"),
        "{refused}"
    );

    // Still pending, and the second principal closes it.
    assert!(
        discharge_posthoc_review(
            &conn,
            &scope,
            SESSION,
            SECOND_PRINCIPAL,
            SECOND_PRINCIPAL,
            at(16)
        )
        .await
        .expect("the discharge runs"),
        "the refusal above left the obligation open for the principal who may close it"
    );
}

/// A review may not be **attributed** to someone else, which is the same
/// guard `record_decision` applies to a verdict and for the same reason: the
/// row is append-only, so a misattribution is permanent.
#[tokio::test]
async fn a_review_cannot_be_attributed_to_another_principal() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = elevation_scope();
    open_breakglass_session(&conn, &scope, elevation(ApprovalPath::PostHoc), "incident")
        .await
        .expect("the session opens");

    let refused =
        discharge_posthoc_review(&conn, &scope, SESSION, SECOND_PRINCIPAL, OPERATOR, at(16))
            .await
            .expect_err("the acting principal must be the one the row will name");
    assert!(
        refused.to_string().contains("may not record a review"),
        "{refused}"
    );
}

/// An absent or out-of-scope session is **refused**, not collapsed into the
/// `Ok(false)` that means "already discharged".
///
/// A reviewer who typo'd the id, or whose scope names the wrong tenant, would
/// otherwise be told there was nothing to discharge while the real
/// obligation stayed `pending` and nothing said so.
#[tokio::test]
async fn discharging_an_unknown_session_is_refused_rather_than_answered_false() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = elevation_scope();
    open_breakglass_session(&conn, &scope, elevation(ApprovalPath::PostHoc), "incident")
        .await
        .expect("the session opens");

    let refused = discharge_posthoc_review(
        &conn,
        &scope,
        Uuid::from_u128(0x9e_ff_ff),
        SECOND_PRINCIPAL,
        SECOND_PRINCIPAL,
        at(16),
    )
    .await
    .expect_err("no such session");
    assert!(
        refused.to_string().contains("no elevation session"),
        "{refused}"
    );
}

/// **The finalize's own open-state predicate**, probed directly.
///
/// Nothing measured it: `record_decision`'s read-time guard refuses a closed
/// record first, so no path through the public surface reaches
/// `finalize_rejected` with a record that is not `pending`, and deleting
/// either the predicate or its zero-rows branch left the whole suite green.
/// The function is private to this module's parent, so the probe calls it
/// directly rather than inventing an interleaving the harness cannot produce.
///
/// The refusal is asserted to be the declared **409** rather than a storage
/// failure — the same fact the read-time guard reports, one statement later.
#[tokio::test]
async fn the_finalize_refuses_a_record_that_left_pending_under_it() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    // The racer, standing in for a frozen-content write that landed between
    // the decision's read and its flip.
    supersede_open_approval(&conn, &scope, TENANT, &subject, at(12))
        .await
        .expect("the supersede lands");

    let refused = super::finalize_rejected(&conn, &scope, TENANT, id, at(13))
        .await
        .expect_err("the record is no longer pending");
    assert!(
        refused.to_string().contains("APPROVAL_SUPERSEDED"),
        "the loser of the race is told the record closed, not that the database broke: \
         {refused}"
    );

    // The control: against a pending record the same call succeeds.
    let open = submit_one(&conn, &scope, &subject).await;
    super::finalize_rejected(&conn, &scope, TENANT, open, at(14))
        .await
        .expect("a pending record finalizes");
    assert_eq!(
        read_approval(&conn, &scope, TENANT, open)
            .await
            .expect("read runs")
            .expect("the record exists")
            .state,
        "rejected"
    );
}

/// **The author's own acknowledgment home reaches the gate's operand.**
///
/// Only the decision-row home was probed, so deleting the
/// `author_override_ack` half left the suite green — and that half is the one
/// covering effective quorum zero, where P-D-68 arm 1 puts the author's
/// acknowledgment.
#[tokio::test]
async fn the_authors_acknowledgment_reaches_the_operand() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;

    let policy = MaterialityPolicy::default();
    let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
    let id = ApprovalId::new(Uuid::new_v4());
    let mut new = submission(id, &subject, SUBMITTED, diff_basis_for(Some(1)), evaluator);
    // Effective quorum zero is the only count that admits the author's own
    // acknowledgment (P-D-68 arm 1).
    new.approver_count = 0;
    new.author_override_ack = Some("uncomposed-bundle");
    submit_approval(&conn, &scope, new, at(11))
        .await
        .expect("the submission is stored");

    assert!(
        gate_candidates(&conn, &scope, &subject)
            .await
            .expect("read the candidates")
            .iter()
            .any(|c| c.approval_id == id && c.override_acknowledged),
        "the author's column is the other home of the acknowledgment and must reach the verdict"
    );
}

/// **An empty acknowledgment is not an acknowledgment.**
///
/// Both columns are request-borne free text with no `<> ''` CHECK, while four
/// neighbouring columns on the same two tables carry one. A reader testing
/// NULL-ness alone let `Some("")` set the gate's override operand, which the
/// publish door writes straight into `composition_pending` — clearing a flag
/// nobody acknowledged.
#[tokio::test]
async fn an_empty_acknowledgment_does_not_set_the_operand() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    record_decision(
        &conn,
        &scope,
        NewDecision {
            tenant_id: TENANT,
            approval_id: id,
            approver_principal: APPROVER,
            verdict: DecisionVerdict::Approved,
            reason: None,
            override_acknowledgments: Some("   "),
        },
        APPROVER,
        at(12),
    )
    .await
    .expect("the row lands: no CHECK forbids the value, which is the point");

    assert!(
        gate_candidates(&conn, &scope, &subject)
            .await
            .expect("read the candidates")
            .iter()
            .all(|c| !c.override_acknowledged),
        "an acknowledgment column holding only whitespace was not an acknowledgment"
    );
}

/// **The stored `subject_kind` token round-trips, and an unknown one is
/// refused** — the falsifiable half of building a candidate's subject from
/// its own row.
///
/// # What cannot be probed here, said plainly
///
/// The host's `candidate.subject == subject` guard was a tautology while this
/// module stamped the queried subject onto every row. Building it from the
/// row instead makes the guard live — but **not against this producer**:
/// `gate_candidates` filters on `(tenant_id, subject_kind, subject_ref)` in
/// SQL, so every row it returns matches the query by construction, and an
/// assertion that a candidate carries the queried subject is satisfied
/// identically either way. A probe written that way was tried and measured
/// **blind**: reverting to `subject.clone()` left it green.
///
/// The guard's value is against a producer that loads differently — a batch
/// over several subjects, or a load by `approval_id` for the `PreAuthorized`
/// path — and no test can reach that today. What *is* falsifiable is the
/// parser the row-built subject needs, and a corrupt token cannot be inserted
/// to exercise its refusal (`chk_products_approval_subject_kind` forbids it on
/// both engines), so the parser is probed directly.
#[test]
fn the_stored_subject_kind_round_trips_and_an_unknown_token_is_refused() {
    for kind in [
        SubjectKind::EntityPublish,
        SubjectKind::GovernedLiveOp,
        SubjectKind::SystemSignal,
        SubjectKind::SkuCorrection,
        SubjectKind::BulkBatch,
    ] {
        assert_eq!(
            super::subject_kind_from_stored(kind.as_str()),
            Some(kind),
            "the seam declares no parser for its own stored token, so this roster is a copy \
             and has to be held to the original"
        );
    }
    assert_eq!(
        super::subject_kind_from_stored("entity_discard"),
        None,
        "a token outside chk_products_approval_subject_kind's roster is a row this gear wrote \
         wrong, and gate_candidates answers CorruptRow for it"
    );
}

/// **The by-id reader answers a record the by-subject reader cannot find** —
/// P-D-105's operand for a cascade leg.
///
/// The record is submitted against one subject; the query that a leg would
/// make names a different one. `gate_candidates` on the leg's subject finds
/// nothing, which is exactly why the mechanical stage needs a lookup by the
/// row's pin — and the candidate it answers carries the record's **own**
/// subject, which the host then does not compare.
#[tokio::test]
async fn the_by_id_reader_finds_what_the_by_subject_reader_cannot() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    // A leg's own subject: same tenant, different entity.
    let leg = GateSubject::entity_publish(EntityRef {
        tenant_id: TENANT,
        entity_kind: bss_products_sdk::models::EntityKind::Sku,
        entity_id: Uuid::from_u128(0x9e_c1),
    });
    assert!(
        gate_candidates(&conn, &scope, &leg)
            .await
            .expect("read the candidates")
            .is_empty(),
        "the by-subject reader cannot serve a leg whose record names the parent"
    );

    let found = gate_candidate_by_id(&conn, &scope, TENANT, id)
        .await
        .expect("read by id")
        .expect("the record exists");
    assert_eq!(found.approval_id, id);
    assert_eq!(
        found.subject.reference, subject.reference,
        "the candidate carries the record's own subject, not the leg's"
    );
    assert_eq!(found.state, ApprovalState::Pending);
}

/// An id no record carries, and an id outside the caller's scope, both answer
/// `None` rather than an error — the host refuses either way, and a row
/// pinning an invisible approval is not one this stage may act on.
#[tokio::test]
async fn the_by_id_reader_answers_none_for_an_unknown_or_foreign_record() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let subject = subject();
    seed_head(&conn, &scope, Some(PUBLISHED)).await;
    let id = submit_one(&conn, &scope, &subject).await;

    assert!(
        gate_candidate_by_id(
            &conn,
            &scope,
            TENANT,
            ApprovalId::new(Uuid::from_u128(0xdead))
        )
        .await
        .expect("the read runs")
        .is_none()
    );
    assert!(
        gate_candidate_by_id(&conn, &scope, Uuid::from_u128(0x9e_22), id)
            .await
            .expect("the read runs")
            .is_none(),
        "another tenant's scope sees no record"
    );
}

// ---------------------------------------------------------------------------
// The materiality policy's store (**P-D-112**).
// ---------------------------------------------------------------------------

/// **A tenant with no row resolves to the default, and does not refuse.**
///
/// This is P-D-112 arm 2, and the decision calls it *"the one a builder will
/// get wrong"*. Every tenant is this tenant at launch, so the arm that must
/// hold is the one with no setup at all: `Resolved(default)`, `N = 2`, trigger
/// 10, no extra fields. An implementation that mapped the read's `None` onto
/// `Unresolvable` would refuse every act in every unconfigured tenant, against
/// C4's *"enforceable at launch"*.
#[tokio::test]
async fn a_tenant_with_no_policy_row_resolves_to_the_default() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let resolved = resolve_materiality_policy(&conn, &scope, TENANT)
        .await
        .expect("the read runs");
    match resolved {
        Resolution::Resolved(policy) => {
            assert_eq!(
                policy.approver_count(),
                DEFAULT_APPROVER_COUNT,
                "P-D-11: absent implies the default, interim 2"
            );
            assert_eq!(
                policy.affected_entity_trigger(),
                DEFAULT_AFFECTED_ENTITY_TRIGGER
            );
            assert!(policy.field_set().is_empty());
            assert_eq!(
                policy,
                MaterialityPolicy::default(),
                "the whole value, not just N: a partial default is a third policy nobody chose"
            );
        }
        Resolution::Unresolvable => panic!(
            "an absent row is a resolved default (P-D-112 arm 2); refusing here refuses every \
             act in every tenant that has never configured anything"
        ),
    }
}

/// The evaluator accepts what the absent row resolves to.
///
/// The store's answer is only useful if it is the operand the evaluator takes,
/// so this drives the whole first link: no row, resolve, evaluate, and a
/// verdict rather than a refusal. Without it the two halves could each be
/// right and not meet.
#[tokio::test]
async fn the_evaluator_runs_on_the_default_an_absent_row_resolves_to() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let Resolution::Resolved(policy) = resolve_materiality_policy(&conn, &scope, TENANT)
        .await
        .expect("the read runs")
    else {
        panic!("an absent row resolves");
    };
    let evaluator = MaterialityEvaluator::new(Resolution::Resolved(&policy));
    assert!(
        evaluator.verdict(&MaterialAct::PolicyMutation).is_ok(),
        "the evaluator refuses an unresolved policy by design, so an unconfigured tenant \
         reaching a verdict at all is the whole of what P-D-112's first link buys"
    );
}

/// A written policy round-trips, and replaces rather than accumulating.
///
/// `N = 0` is the value written, because it is the one P-D-11 made reachable
/// and the one a `CHECK (approver_count >= 1)` would have silently refused —
/// so the probe is armed where the schema could be wrong rather than where it
/// is comfortable.
#[tokio::test]
async fn a_written_policy_round_trips_and_the_second_write_replaces_the_first() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let first = MaterialityPolicy::new(vec!["tax_category".to_owned()], 25, 0);
    write_materiality_policy(&conn, &scope, TENANT, &first, ACTOR, at(11))
        .await
        .expect("the first governed mutation creates the row");
    let Resolution::Resolved(read_back) = resolve_materiality_policy(&conn, &scope, TENANT)
        .await
        .expect("the read runs")
    else {
        panic!("a written policy resolves");
    };
    assert_eq!(read_back, first, "every field, not just N");
    assert_eq!(
        read_back.approver_count(),
        0,
        "P-D-11 made zero reachable and the CHECK floors at zero, not at one"
    );

    let second = MaterialityPolicy::new(Vec::new(), 10, 3);
    write_materiality_policy(&conn, &scope, TENANT, &second, ACTOR, at(12))
        .await
        .expect("the second replaces");
    let Resolution::Resolved(read_back) = resolve_materiality_policy(&conn, &scope, TENANT)
        .await
        .expect("the read runs")
    else {
        panic!("resolves");
    };
    assert_eq!(
        read_back, second,
        "one row per tenant, replaced and not appended"
    );
}

/// One tenant's policy is not another's, and the absent neighbour still gets
/// the default rather than the configured tenant's value.
#[tokio::test]
async fn a_policy_is_scoped_to_its_tenant() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    write_materiality_policy(
        &conn,
        &scope,
        TENANT,
        &MaterialityPolicy::new(Vec::new(), 10, 5),
        ACTOR,
        at(11),
    )
    .await
    .expect("written");

    let other = Uuid::from_u128(0x9e_77);
    let Resolution::Resolved(neighbour) =
        resolve_materiality_policy(&conn, &AccessScope::for_tenant(other), other)
            .await
            .expect("the read runs")
    else {
        panic!("an absent row resolves");
    };
    assert_eq!(
        neighbour.approver_count(),
        DEFAULT_APPROVER_COUNT,
        "the neighbour has no row and gets the default, not the configured tenant's N"
    );
}

/// The stored `field_set` has one producer, and it survives a round trip with
/// its members intact.
#[tokio::test]
async fn the_field_set_round_trips_through_its_canonical_rendering() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);

    let fields = vec!["gl_code".to_owned(), "tax_category".to_owned()];
    write_materiality_policy(
        &conn,
        &scope,
        TENANT,
        &MaterialityPolicy::new(fields.clone(), 10, 2),
        ACTOR,
        at(11),
    )
    .await
    .expect("written");
    let Resolution::Resolved(policy) = resolve_materiality_policy(&conn, &scope, TENANT)
        .await
        .expect("the read runs")
    else {
        panic!("resolves");
    };
    for field in &fields {
        assert!(
            policy.names_field(field),
            "{field} did not survive the round trip"
        );
    }
    assert_eq!(
        encode_field_set(&fields),
        r#"["gl_code","tax_category"]"#,
        "the stored bytes are the canonical rendering, and both engines hold these"
    );
}

/// **A record at `required = 0` is born `satisfied`, in the submit
/// transaction, and writes no decision rows** (**P-D-119** row 31, P-D-11).
///
/// Three assertions, each of which a different defect moves: the answer the
/// door reads, the column a later gate reads, and the decision table §4's
/// human arm would have had to fill. A probe asserting only the first would
/// pass against a store that answered `Satisfied` and wrote `pending`, which
/// is precisely the split that would leave the record unusable by the gate.
#[tokio::test]
async fn a_record_at_zero_is_born_satisfied_and_writes_no_decision_rows() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    seed_head(&conn, &scope, None).await;

    let policy = MaterialityPolicy::default();
    let subject = subject();
    let id = ApprovalId::new(Uuid::new_v4());
    let answered = submit_approval(
        &conn,
        &scope,
        NewApproval {
            approver_count: 0,
            ..submission(
                id,
                &subject,
                SUBMITTED,
                None,
                MaterialityEvaluator::new(Resolution::Resolved(&policy)),
            )
        },
        at(11),
    )
    .await
    .expect("the submission lands");

    assert_eq!(
        answered.descriptor.required(),
        0,
        "the probe's premise is an effective quorum of zero"
    );
    assert_eq!(
        answered.state,
        ApprovalState::Satisfied,
        "P-D-119 row 31: required = 0 is met by construction at submission"
    );

    let stored = read_approval(&conn, &scope, TENANT, id)
        .await
        .expect("the read runs")
        .expect("the record exists");
    assert_eq!(
        stored.state, "satisfied",
        "the column a later gate reads carries the same fact the answer did"
    );

    let decisions = approval_decision::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(Condition::all().add(approval_decision::Column::TenantId.eq(TENANT)))
        .all(&conn)
        .await
        .expect("the decision read runs");
    assert!(
        decisions.is_empty(),
        "zero principals cast zero verdicts: SS4's 'met by distinct principals' arm cannot fire \
         on none, which is why the count decides the born state"
    );
}

/// **A record above zero is born `pending`** — without this the probe above
/// would pass against a store that wrote `satisfied` unconditionally, which
/// would authorize every governed act in the gear on submission alone.
#[tokio::test]
async fn a_record_above_zero_is_born_pending() {
    let provider = harness().await;
    let conn = provider.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    seed_head(&conn, &scope, None).await;

    let policy = MaterialityPolicy::default();
    let subject = subject();
    let id = ApprovalId::new(Uuid::new_v4());
    let answered = submit_approval(
        &conn,
        &scope,
        submission(
            id,
            &subject,
            SUBMITTED,
            None,
            MaterialityEvaluator::new(Resolution::Resolved(&policy)),
        ),
        at(11),
    )
    .await
    .expect("the submission lands");

    assert_eq!(answered.descriptor.required(), 2, "the default N");
    assert_eq!(answered.state, ApprovalState::Pending);
    let stored = read_approval(&conn, &scope, TENANT, id)
        .await
        .expect("the read runs")
        .expect("the record exists");
    assert_eq!(stored.state, "pending");
}
