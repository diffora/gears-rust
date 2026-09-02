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

use chrono::{TimeZone as _, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait as _, Condition, EntityTrait as _};
use sea_orm_migration::MigratorTrait as _;
use toolkit_db::secure::{AccessScope, DBRunner, SecureUpdateExt as _};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::super::{
    HeadWrite, NewEntityVersion, NewProduct, ProductHeadSave, SavedName, VersionedEntityKind,
    find_product, insert_entity_version, insert_product, latest_entity_version, save_product_head,
    supersede_open_approval,
};
use super::{
    Consumption, DecisionOutcome, DecisionVerdict, NewApproval, NewDecision, consume_approval,
    gate_candidates, read_approval, record_decision, submit_approval,
};
use crate::domain::approval::{ApprovalState, ApproverDiff, diff_basis_for, render_diff};
use crate::domain::governance::{ApprovalId, EntityRef, GateSubject};
use crate::domain::materiality::{
    MaterialAct, MaterialityEvaluator, MaterialityPolicy, Resolution,
};
use crate::infra::storage::entity::approval;
use crate::infra::storage::migrations::Migrator;

const TENANT: Uuid = Uuid::from_u128(0x9e_11);
const BRAND: Uuid = Uuid::from_u128(0x9e_b1);
const PRODUCT: Uuid = Uuid::from_u128(0x9e_f0);
const ACTOR: Uuid = Uuid::from_u128(0x9e_ac);
const AUTHOR: Uuid = Uuid::from_u128(0x9e_a0);
const APPROVER: Uuid = Uuid::from_u128(0x9e_a1);

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

fn at(hour: u32) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 2, hour, 0, 0)
        .single()
        .expect("a real instant")
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
    let claims = vec!["catalog-admin".to_owned()];
    let evaluator =
        MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));

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
    let claims = vec!["catalog-admin".to_owned()];
    let evaluator =
        MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));

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
    let claims = vec!["catalog-admin".to_owned()];
    let evaluator =
        MaterialityEvaluator::new(Resolution::Resolved(&policy), Resolution::Resolved(&claims));
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
