//! The plan revision chain against a real database.
//!
//! Everything worth proving here is a property of a **statement**, not of a
//! branch in Rust. The compare-and-swap is a conjunction the database evaluates
//! under the row lock, the row-version bump rides inside it, and the partial
//! `UNIQUE` indexes decide what "the current revision" and "the open draft"
//! even mean. A mock would assert that the repository's own `if` fires and
//! would keep asserting it after the predicate that matters had been deleted.
//!
//! The suite also pins the three-way refusal after a failed swap. A repository
//! that answered "conflict" to all three would send an operator into a retry
//! loop for the one case — a frozen revision — where no retry can ever succeed.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::plan::PlanShapePatch;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::entity::plan;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewPlanDraft, PlanRepo};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, ConnectionTrait, Database, EntityTrait, Statement};
use sea_orm_migration::{MigrationTrait, MigratorTrait, SchemaManager};
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

/// The repository, plus the provider the seeding helper needs to put a row into
/// a state only the publish unit (G5) will be able to reach.
async fn harness() -> (PlanRepo, DBProvider<DbError>) {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    (PlanRepo::new(provider.clone()), provider)
}

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 2, hour, 0, 0).unwrap()
}

fn new_draft(plan_id: PlanId, tenant_id: Uuid) -> NewPlanDraft {
    NewPlanDraft {
        plan_id,
        tenant_id,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: at(10),
        sku_id: Some(Uuid::from_u128(0x5_c1)),
        plan_tier: Some("gold".to_owned()),
        billing_cycle: Some("monthly".to_owned()),
        available_from: Some(at(11)),
        available_to: Some(at(23)),
    }
}

/// Move a revision's `lifecycle_state` directly.
///
/// The publish unit that owns this flip lands in G5, and the append-only
/// trigger permits it: it fires only when the row is already past `draft`, and
/// `published -> retired` is one of the two flips it whitelists.
async fn flip_state(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    plan_id: PlanId,
    revision: i64,
    state: LifecycleState,
) {
    let conn = provider.conn().expect("conn");
    let result = plan::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(plan::Column::LifecycleState, Expr::value(state.as_str()))
        .filter(
            Condition::all()
                .add(plan::Column::PlanId.eq(plan_id.get()))
                .add(plan::Column::Revision.eq(revision)),
        )
        .exec(&conn)
        .await
        .expect("flip the lifecycle state");
    assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
}

#[tokio::test]
async fn a_created_draft_reads_back_whole() {
    let (repo, _provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    let created = repo
        .create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create the first revision");

    // Revision 0, draft, version 0: the three facts every later assertion in
    // this file is measured against.
    assert_eq!(created.revision, 0);
    assert_eq!(created.lifecycle_state, LifecycleState::Draft);
    assert_eq!(created.row_version, RowVersion::new(0));

    let read = repo
        .find_revision(&scope, tenant, plan_id, 0)
        .await
        .expect("read")
        .expect("the revision just created is there");

    // Field for field: a mapping that dropped a column would still round-trip
    // the identity, and the drop would only surface at publish.
    assert_eq!(read, created);
    assert_eq!(read.sku_id, Some(Uuid::from_u128(0x5_c1)));
    assert_eq!(read.plan_tier.as_deref(), Some("gold"));
    assert_eq!(read.billing_cycle.as_deref(), Some("monthly"));
    assert_eq!(read.available_from, Some(at(11)));
    assert_eq!(read.available_to, Some(at(23)));
    assert_eq!(read.created_by, Uuid::from_u128(0xac_10));
    assert_eq!(read.created_at_utc, at(10));
}

#[tokio::test]
async fn a_plan_whose_only_revision_is_a_draft_has_no_current_one() {
    let (repo, _provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");

    // "Current" is not "the greatest revision number". An unpublished plan has
    // nothing a consumer may resolve, and answering with the draft would hand
    // the projector authoring state (§4.2: consumers never read draft state).
    assert_eq!(
        repo.find_current(&scope, tenant, plan_id)
            .await
            .expect("read current"),
        None
    );
    assert!(
        repo.find_open_draft(&scope, tenant, plan_id)
            .await
            .expect("read open draft")
            .is_some(),
        "the draft itself is still there"
    );
}

#[tokio::test]
async fn an_edit_advances_the_tag_and_the_previous_tag_stops_working() {
    let (repo, _provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");

    let edited = repo
        .update_draft(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            PlanShapePatch {
                plan_tier: Some("platinum".to_owned()),
                ..PlanShapePatch::default()
            },
        )
        .await
        .expect("the first edit holds the current version");
    assert_eq!(edited.plan_tier.as_deref(), Some("platinum"));
    assert_eq!(edited.row_version, RowVersion::new(1));

    // The second writer is the bulk import that read before the interactive
    // edit landed. It is refused, and the refusal names both versions so an
    // operator can tell a caller that never refreshed from a real collision.
    let err = repo
        .update_draft(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            PlanShapePatch {
                plan_tier: Some("silver".to_owned()),
                ..PlanShapePatch::default()
            },
        )
        .await
        .expect_err("a submit against a superseded tag must be refused");
    assert_eq!(
        err,
        RepoError::StaleRowVersion {
            subject: "plan revision".to_owned(),
            id: format!("{plan_id}/0"),
            current: 1,
            submitted: 0,
        }
    );

    // And nothing of the refused edit landed.
    let read = repo
        .find_revision(&scope, tenant, plan_id, 0)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read.plan_tier.as_deref(), Some("platinum"));
    assert_eq!(read.row_version, RowVersion::new(1));
}

#[tokio::test]
async fn an_empty_patch_is_a_request_and_still_moves_the_tag() {
    let (repo, _provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    let created = repo
        .create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");

    // It asserts the caller's tag and advances it. Treating it as a no-op would
    // make a lost edit and a deliberately empty one indistinguishable to every
    // observer downstream.
    let touched = repo
        .update_draft(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            PlanShapePatch::default(),
        )
        .await
        .expect("an empty patch is a valid request");

    assert_eq!(touched.row_version, RowVersion::new(1));
    assert_eq!(touched.plan_tier, created.plan_tier);
    assert_eq!(touched.sku_id, created.sku_id);
    assert_eq!(touched.available_to, created.available_to);
}

#[tokio::test]
async fn a_published_revision_refuses_the_edit_by_name_not_by_trigger() {
    let (repo, provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");
    flip_state(&provider, &scope, plan_id, 0, LifecycleState::Published).await;

    // The submitted version is the row's real one, so only the draft-only
    // conjunct can have failed. The typed refusal is the whole point: the
    // table trigger would answer with a database error carrying no state, and
    // the caller would be told the store is broken.
    let err = repo
        .update_draft(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            PlanShapePatch {
                plan_tier: Some("silver".to_owned()),
                ..PlanShapePatch::default()
            },
        )
        .await
        .expect_err("a published revision is frozen in content");
    assert_eq!(
        err,
        RepoError::NotDraft {
            subject: "plan revision".to_owned(),
            id: format!("{plan_id}/0"),
            state: "published".to_owned(),
        }
    );

    // Deleting it is refused the same way, and again before the trigger.
    let err = repo
        .delete_draft(&scope, tenant, plan_id, 0, RowVersion::new(0))
        .await
        .expect_err("only a never-published draft is deletable");
    assert!(matches!(err, RepoError::NotDraft { .. }));
}

#[tokio::test]
async fn frozen_beats_stale_when_a_write_is_both() {
    let (repo, provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");
    flip_state(&provider, &scope, plan_id, 0, LifecycleState::Published).await;

    // The real shape of the case: an author holding a tag from before the
    // publish. **Two** conjuncts of the swap failed at once - the version and
    // the draft-only predicate - so the refusal is a precedence decision, not a
    // lookup, and it is the only test that observes which one wins.
    //
    // It has to be `NotDraft`. `StaleRowVersion` reads as "refresh and retry",
    // and no refresh will ever make a published revision editable: the caller
    // would refetch, read version 0, submit again, and be told it is stale
    // again - a loop the surface itself created. The remedy is a different
    // operation (open a new revision), and only the frozen answer says so.
    let err = repo
        .update_draft(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(9),
            PlanShapePatch {
                plan_tier: Some("silver".to_owned()),
                ..PlanShapePatch::default()
            },
        )
        .await
        .expect_err("a published revision is frozen whatever tag is submitted");
    assert_eq!(
        err,
        RepoError::NotDraft {
            subject: "plan revision".to_owned(),
            id: format!("{plan_id}/0"),
            state: "published".to_owned(),
        },
        "a write that is both frozen and stale must be refused as frozen"
    );

    // `delete_draft` shares the arms, so it inherits the precedence and has to
    // be held to it too.
    let err = repo
        .delete_draft(&scope, tenant, plan_id, 0, RowVersion::new(9))
        .await
        .expect_err("a published revision is undeletable whatever tag is submitted");
    assert_eq!(
        err,
        RepoError::NotDraft {
            subject: "plan revision".to_owned(),
            id: format!("{plan_id}/0"),
            state: "published".to_owned(),
        },
        "a delete that is both frozen and stale must be refused as frozen"
    );
}

#[tokio::test]
async fn every_patched_column_reaches_the_row_it_names() {
    let (repo, _provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");

    // Every field-to-column pair in one edit, each moved to a value the seed
    // does not hold. The pairs are hand-wired, and two of them encode through a
    // type the column spells differently on each backend (`uuid` and
    // `timestamptz` on Postgres, `text` on SQLite) - a path the insert does not
    // share. A transposed pair, or a value that silently fails to encode, is
    // invisible to a suite that only ever patches one column.
    let sku_id = Uuid::from_u128(0x5_c2);
    let updated = repo
        .update_draft(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            PlanShapePatch {
                sku_id: Some(sku_id),
                plan_tier: Some("platinum".to_owned()),
                billing_cycle: Some("annual".to_owned()),
                available_from: Some(at(14)),
                available_to: Some(at(20)),
            },
        )
        .await
        .expect("a five-column patch is one edit");

    // Asymmetric on purpose: `available_from` and `available_to` are both moved,
    // to distinct instants, and neither may end up holding the other's.
    assert_eq!(updated.sku_id, Some(sku_id));
    assert_eq!(updated.plan_tier.as_deref(), Some("platinum"));
    assert_eq!(updated.billing_cycle.as_deref(), Some("annual"));
    assert_eq!(updated.available_from, Some(at(14)));
    assert_eq!(updated.available_to, Some(at(20)));
    assert_eq!(updated.row_version, RowVersion::new(1));

    // And it is the stored row that changed, not just the value handed back.
    let read = repo
        .find_revision(&scope, tenant, plan_id, 0)
        .await
        .expect("read")
        .expect("present");
    assert_eq!(read, updated);
}

#[tokio::test]
async fn an_absent_revision_is_not_found_rather_than_stale() {
    let (repo, _provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");

    let err = repo
        .update_draft(
            &scope,
            tenant,
            plan_id,
            7,
            RowVersion::new(0),
            PlanShapePatch::default(),
        )
        .await
        .expect_err("revision 7 was never opened");
    assert_eq!(
        err,
        RepoError::NotFound {
            subject: "plan revision".to_owned(),
            id: format!("{plan_id}/7"),
        }
    );
}

#[tokio::test]
async fn a_draft_is_deletable_only_under_its_own_version() {
    let (repo, _provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");

    // Abandoning a draft is a write like any other: a caller working from a
    // read it did not refresh would otherwise discard an edit it never saw.
    let err = repo
        .delete_draft(&scope, tenant, plan_id, 0, RowVersion::new(4))
        .await
        .expect_err("a stale tag must not delete");
    assert_eq!(
        err,
        RepoError::StaleRowVersion {
            subject: "plan revision".to_owned(),
            id: format!("{plan_id}/0"),
            current: 0,
            submitted: 4,
        }
    );

    repo.delete_draft(&scope, tenant, plan_id, 0, RowVersion::new(0))
        .await
        .expect("the current tag deletes");
    assert_eq!(
        repo.find_revision(&scope, tenant, plan_id, 0)
            .await
            .expect("read"),
        None
    );
}

#[tokio::test]
async fn a_new_revision_copies_the_current_shape_forward() {
    let (repo, provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    let published = repo
        .create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");
    flip_state(&provider, &scope, plan_id, 0, LifecycleState::Published).await;

    let opened = repo
        .open_revision(&scope, tenant, plan_id, Uuid::from_u128(0xac_20), at(12))
        .await
        .expect("a published plan may open its next revision");

    // The successor is a fresh draft on the same plan: identity unchanged, so
    // the scope-key axis and the price attachment never have to move (§4.3).
    assert_eq!(opened.plan_id, plan_id);
    assert_eq!(opened.revision, 1);
    assert_eq!(opened.lifecycle_state, LifecycleState::Draft);
    assert_eq!(opened.row_version, RowVersion::new(0));
    assert_eq!(opened.created_by, Uuid::from_u128(0xac_20));
    assert_eq!(opened.created_at_utc, at(12));

    // The shape is carried over, so a revision opened to change one field does
    // not silently blank the rest.
    assert_eq!(opened.sku_id, published.sku_id);
    assert_eq!(opened.plan_tier, published.plan_tier);
    assert_eq!(opened.billing_cycle, published.billing_cycle);
    assert_eq!(opened.available_from, published.available_from);
    assert_eq!(opened.available_to, published.available_to);

    // Both links of the chain are readable, and each is what it should be.
    assert_eq!(
        repo.find_current(&scope, tenant, plan_id)
            .await
            .expect("read current")
            .map(|row| row.revision),
        Some(0)
    );
    assert_eq!(
        repo.find_open_draft(&scope, tenant, plan_id)
            .await
            .expect("read open draft")
            .map(|row| row.revision),
        Some(1)
    );
}

#[tokio::test]
async fn a_plan_gets_one_editable_shape_and_no_more() {
    let (repo, provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");
    flip_state(&provider, &scope, plan_id, 0, LifecycleState::Published).await;
    repo.open_revision(&scope, tenant, plan_id, Uuid::from_u128(0xac_20), at(12))
        .await
        .expect("the first successor opens");

    // Two concurrently editable shapes on one plan is the state
    // `uq_pricing_plan_open_draft` exists to forbid. The refusal names the
    // revision holding the slot so the caller edits it instead of guessing
    // which of its own requests won.
    let err = repo
        .open_revision(&scope, tenant, plan_id, Uuid::from_u128(0xac_20), at(13))
        .await
        .expect_err("a second open draft must be refused");
    assert_eq!(
        err,
        RepoError::OpenDraftExists {
            plan_id: plan_id.to_string(),
            revision: 1,
        }
    );
}

#[tokio::test]
async fn a_retired_plan_can_never_open_another_revision() {
    let (repo, provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");
    flip_state(&provider, &scope, plan_id, 0, LifecycleState::Published).await;
    flip_state(&provider, &scope, plan_id, 0, LifecycleState::Retired).await;

    // Retirement is terminal (D-128): there is no edge out of `retired`, so the
    // predecessor could never flip `superseded` when the successor published.
    // The revision would be unpublishable from the moment it was opened, and
    // refusing at publish time would waste an author's whole editing session.
    //
    // It is **not** `NotDraft`, which answered this case first and whose
    // sentence — "only draft content is mutable" — is about editing a
    // revision's content. Nobody asked to edit anything here, and the remedy
    // that sentence implies is to open the next revision: exactly the call
    // being refused, so a caller following the diagnosis would loop.
    let err = repo
        .open_revision(&scope, tenant, plan_id, Uuid::from_u128(0xac_20), at(12))
        .await
        .expect_err("a retired plan takes no further revision");
    assert_eq!(
        err,
        RepoError::NoSuccessorRevision {
            plan_id: plan_id.to_string(),
            state: "retired".to_owned(),
        }
    );
}

#[tokio::test]
async fn a_plan_with_nothing_published_has_no_revision_to_open_from() {
    let (repo, _provider) = harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, new_draft(plan_id, tenant))
        .await
        .expect("create");

    // The plan exists, but the source a new revision copies from does not. The
    // refusal names the missing referent rather than the plan, because "plan
    // not found" would be false and would send the caller looking for the
    // wrong thing.
    let err = repo
        .open_revision(&scope, tenant, plan_id, Uuid::from_u128(0xac_20), at(12))
        .await
        .expect_err("there is no current revision to succeed");
    assert_eq!(
        err,
        RepoError::NotFound {
            subject: "current plan revision".to_owned(),
            id: plan_id.to_string(),
        }
    );
}

#[tokio::test]
async fn another_tenants_plan_is_invisible_and_unwritable() {
    let (repo, _provider) = harness().await;
    let mine = Uuid::from_u128(0x7e_11);
    let theirs = Uuid::from_u128(0x7e_22);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&AccessScope::for_tenant(theirs), new_draft(plan_id, theirs))
        .await
        .expect("the other tenant creates its plan");

    // SQL-level BOLA, the same shape `PinFrontierRepo::read` documents: my
    // scope resolves their row to nothing, whichever tenant id I name. The
    // catalog is commercially sensitive, so the reads fail to `None` and the
    // writes fail to "not found" — never to "forbidden", which would confirm
    // the row exists.
    let scope = AccessScope::for_tenant(mine);
    assert_eq!(
        repo.find_revision(&scope, theirs, plan_id, 0)
            .await
            .expect("read"),
        None
    );
    assert_eq!(
        repo.find_current(&scope, theirs, plan_id)
            .await
            .expect("read current"),
        None
    );
    assert_eq!(
        repo.find_open_draft(&scope, theirs, plan_id)
            .await
            .expect("read open draft"),
        None
    );

    let err = repo
        .update_draft(
            &scope,
            theirs,
            plan_id,
            0,
            RowVersion::new(0),
            PlanShapePatch {
                plan_tier: Some("free".to_owned()),
                ..PlanShapePatch::default()
            },
        )
        .await
        .expect_err("a foreign draft is not writable");
    assert!(matches!(err, RepoError::NotFound { .. }));

    let err = repo
        .delete_draft(&scope, theirs, plan_id, 0, RowVersion::new(0))
        .await
        .expect_err("a foreign draft is not deletable");
    assert!(matches!(err, RepoError::NotFound { .. }));

    let err = repo
        .open_revision(&scope, theirs, plan_id, Uuid::from_u128(0xac_20), at(12))
        .await
        .expect_err("a foreign plan takes no revision");
    assert!(matches!(err, RepoError::NotFound { .. }));

    // And the row is untouched for its owner.
    let read = repo
        .find_revision(&AccessScope::for_tenant(theirs), theirs, plan_id, 0)
        .await
        .expect("read")
        .expect("their revision is still there");
    assert_eq!(read.plan_tier.as_deref(), Some("gold"));
    assert_eq!(read.row_version, RowVersion::new(0));
}

#[tokio::test]
async fn a_plan_may_not_be_created_into_another_tenant() {
    let (repo, _provider) = harness().await;
    let mine = Uuid::from_u128(0x7e_11);
    let theirs = Uuid::from_u128(0x7e_22);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    // The insert side of the same BOLA the reads, updates and deletes above
    // close, and the only one that no `WHERE` clause can: a scoped read filters
    // rows that exist, while a scoped insert has to refuse a row before it
    // does. `scope_with_model` is what refuses it — the `ActiveModel`'s own
    // `tenant_id` is checked against the caller's scope — and without that
    // check a caller could plant a plan inside another tenant's catalog and
    // then be unable to see, edit or delete what it had written.
    let err = repo
        .create_draft(&AccessScope::for_tenant(mine), new_draft(plan_id, theirs))
        .await
        .expect_err("a plan may not be created into a tenant the caller is not scoped to");
    let RepoError::Db(detail) = err else {
        panic!("a refused insert scope is a storage failure, not a typed refusal");
    };
    assert!(detail.contains("pricing_plan scope"), "got: {detail}");

    // And nothing landed under either tenant — least of all the victim's.
    assert_eq!(
        repo.find_revision(&AccessScope::for_tenant(theirs), theirs, plan_id, 0)
            .await
            .expect("read"),
        None
    );
    assert_eq!(
        repo.find_revision(&AccessScope::for_tenant(mine), mine, plan_id, 0)
            .await
            .expect("read"),
        None
    );
}

/// D-83's copy-on-new-revision requires the child shape tables to travel with
/// the revision. They do not exist yet — they are Slice-2 storage and land in
/// **G4** — so `PlanRepo::open_revision` copies the plan's own columns and
/// nothing else.
///
/// This is the anchor for that gap, not an endorsement of it. It interrogates
/// the **schema** rather than migration names on purpose: a G4 migration named
/// for its slice rather than its tables (`m..._create_slice2_plan_shape`) would
/// create all three and leave a name-matching anchor green, which is exactly the
/// silence the gap must not be allowed. The moment any of the three tables
/// exists this fails, and the fix is to make `open_revision` copy it forward
/// with stable `phase_id`s — never to relax the assertion.
#[tokio::test]
async fn no_child_shape_table_exists_yet_for_a_new_revision_to_copy_d83() {
    const CHILD_TABLES: [&str; 3] = [
        "pricing_plan_phase",
        "pricing_plan_addon_rule",
        "pricing_plan_descriptor_set",
    ];

    // A raw `SeaORM` connection, as `sqlite_migrations.rs` uses for the same
    // question: `sqlite_master` has no entity, and `toolkit-db` deliberately
    // hands out no raw executor.
    let conn = Database::connect("sqlite::memory:")
        .await
        .expect("connect in-memory sqlite");
    let manager = SchemaManager::new(&conn);
    let mut chain: Vec<Box<dyn MigrationTrait>> = Migrator::migrations();
    chain.sort_by(|a, b| a.name().cmp(b.name()));
    for migration in &chain {
        migration
            .up(&manager)
            .await
            .unwrap_or_else(|e| panic!("up {} must succeed: {e}", migration.name()));
    }

    for table in CHILD_TABLES {
        let sql = format!(
            "SELECT count(*) AS c FROM sqlite_master WHERE type = 'table' AND name = '{table}'"
        );
        let row = conn
            .query_one(Statement::from_string(
                sea_orm::DatabaseBackend::Sqlite,
                sql,
            ))
            .await
            .expect("query sqlite_master")
            .expect("count query returns a row");
        let found: i32 = row.try_get("", "c").expect("read count");

        assert_eq!(
            found, 0,
            "{table} now exists, so D-83's copy-on-new-revision is no longer \
             discharged by copying columns: PlanRepo::open_revision must copy \
             it forward with stable phase ids"
        );
    }
}
