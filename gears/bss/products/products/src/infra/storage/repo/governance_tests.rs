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
use sea_orm_migration::MigratorTrait as _;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use super::super::{
    HeadWrite, NewEntityVersion, NewProduct, ProductHeadSave, SavedName, VersionedEntityKind,
    find_product, insert_entity_version, insert_product, latest_entity_version, save_product_head,
    supersede_open_approval,
};
use super::{NewApproval, read_approval, submit_approval};
use crate::domain::approval::{ApproverDiff, diff_basis_for, render_diff};
use crate::domain::governance::{ApprovalId, EntityRef, GateSubject};
use crate::domain::materiality::{
    MaterialAct, MaterialityEvaluator, MaterialityPolicy, Resolution,
};
use crate::infra::storage::migrations::Migrator;

const TENANT: Uuid = Uuid::from_u128(0x9e_11);
const BRAND: Uuid = Uuid::from_u128(0x9e_b1);
const PRODUCT: Uuid = Uuid::from_u128(0x9e_f0);
const ACTOR: Uuid = Uuid::from_u128(0x9e_ac);
const AUTHOR: Uuid = Uuid::from_u128(0x9e_a0);

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
