//! `pricing_plan_phase` against a real database — the constraints, the
//! append-only triggers, and the repository that writes through them.
//!
//! Everything worth proving here is a property of the **schema** or of a
//! **statement**, not of a branch in Rust. The partial `UNIQUE` that admits one
//! terminal phase per revision, the CHECK that stops `display_trial_days`
//! drifting from its source, and the three triggers that freeze a phase row when
//! its revision publishes are all evaluated by the engine; a mock would assert
//! that the repository's own `if` fires and would keep asserting it after the
//! predicate that matters had been deleted.
//!
//! The suite carries **two** harnesses on purpose. The constraint and trigger
//! cases drive raw SQL through `common::migrated_db`, because the states they
//! need — a phase row under a published revision, a phase row under an abandoned
//! one — are states no typed path will ever produce and the guards exist
//! precisely for what reaches the table outside this gear. The last three cases
//! drive `PlanShapeRepo`, because what they prove is the repository's: which
//! refusal a caller is told, and what is left behind afterwards.
//!
//! Postgres mirrors every rule here as one PL/pgSQL trigger function and the
//! same CHECKs and indexes, so no testcontainers case is added; see the
//! migration's module doc for the transform.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::plan_shape::{PhaseKind, PlanPhase};
use bss_pricing::domain::scope_key::{PhaseId, PlanId};
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::entity::plan;
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewPlanDraft, PlanRepo, PlanShapeRepo};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, DatabaseConnection, EntityTrait};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

mod common;

use common::{exec, migrated_db, must_succeed, scalar};

const TENANT: &str = "11111111-1111-1111-1111-111111111111";
const PLAN: &str = "22222222-2222-2222-2222-222222222222";
const ACTOR: &str = "44444444-4444-4444-4444-444444444444";
const AUTHORED: &str = "2026-08-02 10:00:00 +00:00";

/// Reject, **and** for the stated reason.
///
/// The fragment is not decoration. This table carries two CHECKs, a partial
/// `UNIQUE`, a composite foreign key and three append-only triggers, and every
/// one of those refusals names `pricing_plan_phase` — as does the column list
/// `SQLite` reports for a unique violation. A test that accepted any error
/// naming the table would pass with the guard it means to prove switched off,
/// refused instead by a constraint it never intended to trip.
async fn must_be_rejected(conn: &DatabaseConnection, sql: &str, because: &str) {
    let err = exec(conn, sql)
        .await
        .expect_err(&format!("this statement must be rejected: {sql}"));
    let message = err.to_string();
    assert!(
        message.contains("pricing_plan_phase"),
        "the rejection must name the guard it came from, got: {message}"
    );
    assert!(
        message.contains(because),
        "the rejection must be the one under test (`{because}`), got: {message}"
    );
}

/// Insert one plan revision in `draft`.
async fn insert_revision(conn: &DatabaseConnection, revision: i64) {
    must_succeed(
        conn,
        &format!(
            "INSERT INTO pricing_plan (
                plan_id, revision, tenant_id, lifecycle_state, created_by, created_at_utc)
             VALUES ('{PLAN}', {revision}, '{TENANT}', 'draft', '{ACTOR}', '{AUTHORED}')"
        ),
    )
    .await;
}

/// Move a revision out of `draft`. Both flips this suite uses —
/// `draft -> published` and `draft -> abandoned` — are whitelisted by
/// `pricing_plan`'s own append-only trigger.
async fn flip(conn: &DatabaseConnection, revision: i64, state: &str) {
    must_succeed(
        conn,
        &format!(
            "UPDATE pricing_plan SET lifecycle_state = '{state}' \
             WHERE plan_id = '{PLAN}' AND revision = {revision}"
        ),
    )
    .await;
}

/// One phase row, spelled out so each case can vary exactly one thing.
struct Phase {
    id: &'static str,
    revision: i64,
    kind: &'static str,
    ordinal: i32,
    /// `None` is terminality.
    converts_to: Option<&'static str>,
    duration: Option<i32>,
    display_trial: Option<i32>,
}

const TERMINAL: Phase = Phase {
    id: "33333333-3333-3333-3333-333333333333",
    revision: 0,
    kind: "evergreen",
    ordinal: 1,
    converts_to: None,
    duration: None,
    display_trial: None,
};

fn nullable(value: Option<&str>) -> String {
    value.map_or_else(|| "NULL".to_owned(), |v| format!("'{v}'"))
}

fn number(value: Option<i32>) -> String {
    value.map_or_else(|| "NULL".to_owned(), |v| v.to_string())
}

fn insert_phase(phase: &Phase) -> String {
    format!(
        "INSERT INTO pricing_plan_phase (
            phase_id, plan_revision, tenant_id, plan_id, kind, ordinal,
            converts_to_phase_id, phase_duration_days, display_trial_days)
         VALUES ('{}', {}, '{TENANT}', '{PLAN}', '{}', {}, {}, {}, {})",
        phase.id,
        phase.revision,
        phase.kind,
        phase.ordinal,
        nullable(phase.converts_to),
        number(phase.duration),
        number(phase.display_trial),
    )
}

async fn count_phases(conn: &DatabaseConnection, revision: i64) -> String {
    scalar(
        conn,
        &format!(
            "SELECT CAST(count(*) AS TEXT) AS v FROM pricing_plan_phase \
             WHERE plan_id = '{PLAN}' AND plan_revision = {revision}"
        ),
    )
    .await
}

// ---------------------------------------------------------------------------
// The constraints.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn one_revision_admits_exactly_one_terminal_phase() {
    let conn = migrated_db().await;
    insert_revision(&conn, 0).await;
    must_succeed(&conn, &insert_phase(&TERMINAL)).await;

    // A second phase with no successor on the same revision. Two terminals
    // leave "the terminal phase" undefined for the scope-key default
    // (`inst-ph-default`) and for usage phase-invariance, which are both
    // written relative to *which* phase is terminal.
    must_be_rejected(
        &conn,
        &insert_phase(&Phase {
            id: "55555555-5555-5555-5555-555555555555",
            ordinal: 2,
            ..TERMINAL
        }),
        "UNIQUE constraint failed",
    )
    .await;
    assert_eq!(
        count_phases(&conn, 0).await,
        "1",
        "no second terminal landed"
    );
}

#[tokio::test]
async fn two_revisions_of_one_plan_each_carry_their_own_terminal_phase() {
    let conn = migrated_db().await;

    // Revision 0 gets its terminal phase and publishes; only then can a
    // successor revision open, because a plan holds at most one draft.
    insert_revision(&conn, 0).await;
    must_succeed(&conn, &insert_phase(&TERMINAL)).await;
    flip(&conn, 0, "published").await;

    // The copy-on-new-revision shape: the **same** phase id under a new
    // revision. The partial UNIQUE is keyed `(plan_id, plan_revision)` exactly
    // so this is legal — an index over `plan_id` alone would make D-83's copy
    // unrepresentable and stable phase ids with it.
    insert_revision(&conn, 1).await;
    must_succeed(
        &conn,
        &insert_phase(&Phase {
            revision: 1,
            ..TERMINAL
        }),
    )
    .await;

    assert_eq!(count_phases(&conn, 0).await, "1");
    assert_eq!(count_phases(&conn, 1).await, "1");
}

#[tokio::test]
async fn the_trial_projection_may_not_drift_from_its_source() {
    let conn = migrated_db().await;
    insert_revision(&conn, 0).await;

    let trial = Phase {
        id: "66666666-6666-6666-6666-666666666666",
        revision: 0,
        kind: "trial",
        ordinal: 0,
        converts_to: Some(TERMINAL.id),
        duration: Some(14),
        display_trial: Some(14),
    };

    // Subscriptions reads the published projection as its single source for
    // trial runtime, so a drift here is a trial that ends on a different day
    // than the catalog says it does.
    must_be_rejected(
        &conn,
        &insert_phase(&Phase {
            display_trial: Some(15),
            ..trial
        }),
        "chk_pricing_plan_phase_display_trial_days",
    )
    .await;

    // NULL is an untaken projection, not a drift: only `trial` phases publish
    // the alias at all.
    must_succeed(
        &conn,
        &insert_phase(&Phase {
            display_trial: None,
            ..trial
        }),
    )
    .await;
    assert_eq!(count_phases(&conn, 0).await, "1");
}

#[tokio::test]
async fn the_kind_column_holds_only_the_three_tokens_the_domain_renders() {
    let conn = migrated_db().await;
    insert_revision(&conn, 0).await;

    // The repository reads this column back through `PhaseKind::ALL` and
    // answers `CorruptRow` for anything else, so a near-miss token stored here
    // would make the whole revision unreadable through every typed path. The
    // CHECK is what stops it being stored.
    must_be_rejected(
        &conn,
        &insert_phase(&Phase {
            kind: "promotional",
            ..TERMINAL
        }),
        "chk_pricing_plan_phase_kind",
    )
    .await;
    assert_eq!(count_phases(&conn, 0).await, "0");
}

#[tokio::test]
async fn a_phase_row_belongs_to_a_revision_that_exists() {
    let conn = migrated_db().await;

    // No revision at all: the row is refused. The **append-only trigger**
    // answers rather than the foreign key, and that is not an accident — its
    // predicate ("a draft revision with this key exists") strictly implies the
    // key's ("a revision with this key exists"), so on the insert path the
    // trigger can never be reached second.
    must_be_rejected(
        &conn,
        &insert_phase(&TERMINAL),
        "under a non-draft plan revision is not permitted",
    )
    .await;

    // The foreign key is declared anyway, over **both** columns, and it says
    // something the trigger does not: that a phase row is filed under one
    // revision of one plan. The trigger is a statement about that revision's
    // *state*; keeping only one of the two would leave the schema asserting
    // something narrower than it means.
    let declared = scalar(
        &conn,
        "SELECT group_concat(\"from\" || '->' || \"to\", ',') AS v
           FROM pragma_foreign_key_list('pricing_plan_phase')
          WHERE \"table\" = 'pricing_plan'",
    )
    .await;
    assert!(
        declared.contains("plan_id->plan_id") && declared.contains("plan_revision->revision"),
        "the composite FK must cover both key columns, got: {declared}"
    );
}

// ---------------------------------------------------------------------------
// The append-only triggers.
// ---------------------------------------------------------------------------

/// Revision 0 published with a phase, revision 2 abandoned with a phase,
/// revision 1 left open as the plan's one draft — in that order, because a plan
/// admits one draft at a time and each phase has to be written while its own
/// revision is still one.
async fn seeded_planes(conn: &DatabaseConnection) {
    const FROZEN: &str = "33333333-3333-3333-3333-333333333333";
    const TOMBSTONED: &str = "77777777-7777-7777-7777-777777777777";
    const OPEN: &str = "88888888-8888-8888-8888-888888888888";

    insert_revision(conn, 0).await;
    must_succeed(
        conn,
        &insert_phase(&Phase {
            id: FROZEN,
            ..TERMINAL
        }),
    )
    .await;
    flip(conn, 0, "published").await;

    insert_revision(conn, 2).await;
    must_succeed(
        conn,
        &insert_phase(&Phase {
            id: TOMBSTONED,
            revision: 2,
            ..TERMINAL
        }),
    )
    .await;
    flip(conn, 2, "abandoned").await;

    insert_revision(conn, 1).await;
    must_succeed(
        conn,
        &insert_phase(&Phase {
            id: OPEN,
            revision: 1,
            ..TERMINAL
        }),
    )
    .await;
}

const FROZEN_PHASE: &str = "33333333-3333-3333-3333-333333333333";
const TOMBSTONED_PHASE: &str = "77777777-7777-7777-7777-777777777777";
const OPEN_PHASE: &str = "88888888-8888-8888-8888-888888888888";

#[tokio::test]
async fn a_frozen_revision_takes_no_new_phase_and_a_draft_one_does() {
    let conn = migrated_db().await;
    seeded_planes(&conn).await;

    // INSERT is guarded and not only UPDATE and DELETE, because it is the one
    // verb that *adds* a phase to a frozen revision — the projector's warm
    // re-drive reads truth rows, so an appended phase would re-materialize a
    // frozen CatalogVersion at a shape nobody published.
    let appended = Phase {
        id: "99999999-9999-9999-9999-999999999999",
        ordinal: 5,
        converts_to: Some(FROZEN_PHASE),
        ..TERMINAL
    };
    must_be_rejected(
        &conn,
        &insert_phase(&Phase {
            revision: 0,
            ..appended
        }),
        "INSERT of a phase under a non-draft plan revision",
    )
    .await;
    // `abandoned` is not `draft`, which is exactly what makes the drop-then-flip
    // ordering in `PlanRepo::abandon_draft` mandatory rather than stylistic.
    must_be_rejected(
        &conn,
        &insert_phase(&Phase {
            revision: 2,
            ..appended
        }),
        "INSERT of a phase under a non-draft plan revision",
    )
    .await;
    must_succeed(
        &conn,
        &insert_phase(&Phase {
            revision: 1,
            ..appended
        }),
    )
    .await;

    assert_eq!(count_phases(&conn, 0).await, "1");
    assert_eq!(count_phases(&conn, 2).await, "1");
    assert_eq!(count_phases(&conn, 1).await, "2");
}

#[tokio::test]
async fn a_frozen_revisions_phase_may_not_be_edited_and_a_drafts_may() {
    let conn = migrated_db().await;
    seeded_planes(&conn).await;

    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_plan_phase SET ordinal = 9 WHERE phase_id = '{FROZEN_PHASE}'"),
        "UPDATE of a phase under a non-draft plan revision",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_plan_phase SET ordinal = 9 WHERE phase_id = '{TOMBSTONED_PHASE}'"),
        "UPDATE of a phase under a non-draft plan revision",
    )
    .await;
    must_succeed(
        &conn,
        &format!("UPDATE pricing_plan_phase SET ordinal = 9 WHERE phase_id = '{OPEN_PHASE}'"),
    )
    .await;

    let moved = scalar(
        &conn,
        &format!(
            "SELECT CAST(ordinal AS TEXT) AS v FROM pricing_plan_phase \
             WHERE phase_id = '{FROZEN_PHASE}'"
        ),
    )
    .await;
    assert_eq!(moved, "1", "no rejected UPDATE may have landed");
}

#[tokio::test]
async fn a_phase_row_may_not_be_re_pointed_at_a_frozen_revision() {
    let conn = migrated_db().await;
    seeded_planes(&conn).await;

    // Checking the NEW parent on UPDATE is not belt-and-braces: re-pointing a
    // child row's `plan_revision` is how one would otherwise append a phase to
    // a frozen revision without ever issuing an INSERT.
    must_be_rejected(
        &conn,
        &format!("UPDATE pricing_plan_phase SET plan_revision = 0 WHERE phase_id = '{OPEN_PHASE}'"),
        "UPDATE of a phase under a non-draft plan revision",
    )
    .await;
    // And the OLD parent, for the mirror-image reason: the row is being mutated
    // out from under a revision whose freeze governs it.
    must_be_rejected(
        &conn,
        &format!(
            "UPDATE pricing_plan_phase SET plan_revision = 1 WHERE phase_id = '{FROZEN_PHASE}'"
        ),
        "UPDATE of a phase under a non-draft plan revision",
    )
    .await;

    assert_eq!(count_phases(&conn, 0).await, "1");
    assert_eq!(count_phases(&conn, 1).await, "1");
}

#[tokio::test]
async fn a_frozen_revisions_phase_may_not_be_deleted_and_a_drafts_may() {
    let conn = migrated_db().await;
    seeded_planes(&conn).await;

    must_be_rejected(
        &conn,
        &format!("DELETE FROM pricing_plan_phase WHERE phase_id = '{FROZEN_PHASE}'"),
        "DELETE of a phase under a non-draft plan revision",
    )
    .await;
    must_be_rejected(
        &conn,
        &format!("DELETE FROM pricing_plan_phase WHERE phase_id = '{TOMBSTONED_PHASE}'"),
        "DELETE of a phase under a non-draft plan revision",
    )
    .await;
    must_succeed(
        &conn,
        &format!("DELETE FROM pricing_plan_phase WHERE phase_id = '{OPEN_PHASE}'"),
    )
    .await;

    assert_eq!(count_phases(&conn, 0).await, "1");
    assert_eq!(count_phases(&conn, 2).await, "1");
    assert_eq!(count_phases(&conn, 1).await, "0");
}

// ---------------------------------------------------------------------------
// The repository.
// ---------------------------------------------------------------------------

/// The two repositories, plus the provider the seeding helper needs to put a
/// revision into a state `plan_repo::publish_revision` now reaches - fabricated
/// here rather than published, so this suite tests the child table and not the
/// publish unit.
async fn repo_harness() -> (PlanRepo, PlanShapeRepo, DBProvider<DbError>) {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    (
        PlanRepo::new(provider.clone()),
        PlanShapeRepo::new(provider.clone()),
        provider,
    )
}

/// Publish a revision straight at the column, which is the same flip
/// `plan_repo::publish_revision` performs; `pricing_plan`'s append-only trigger
/// whitelists it. Fabricated rather than published so this suite stays about
/// the child table.
async fn publish(provider: &DBProvider<DbError>, scope: &AccessScope, plan_id: PlanId) {
    let conn = provider.conn().expect("conn");
    let result = plan::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            plan::Column::LifecycleState,
            Expr::value(LifecycleState::Published.as_str()),
        )
        .filter(
            Condition::all()
                .add(plan::Column::PlanId.eq(plan_id.get()))
                .add(plan::Column::Revision.eq(0_i64)),
        )
        .exec(&conn)
        .await
        .expect("publish the revision");
    assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
}

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 2, hour, 0, 0).unwrap()
}

fn draft_of(plan_id: PlanId, tenant_id: Uuid) -> NewPlanDraft {
    NewPlanDraft {
        plan_id,
        tenant_id,
        created_by: Uuid::from_u128(0xac_10),
        created_at_utc: at(10),
        sku_id: None,
        plan_tier: None,
        billing_cycle: None,
        frequency: None,
        plan_tier_override: false,
        purchase_min_qty: None,
        purchase_max_qty: None,
        invoice_grouping_key: None,
        available_from: None,
        available_to: None,
    }
}

/// A two-phase chain, terminal last.
fn chain() -> Vec<PlanPhase> {
    vec![
        PlanPhase {
            phase_id: PhaseId::new(Uuid::from_u128(0xf1a)),
            kind: PhaseKind::Trial,
            ordinal: 0,
            converts_to_phase_id: Some(PhaseId::new(Uuid::from_u128(0xf1b))),
            phase_duration_days: Some(14),
            display_trial_days: Some(14),
        },
        PlanPhase {
            phase_id: PhaseId::new(Uuid::from_u128(0xf1b)),
            kind: PhaseKind::Evergreen,
            ordinal: 1,
            converts_to_phase_id: None,
            phase_duration_days: None,
            display_trial_days: None,
        },
    ]
}

#[tokio::test]
async fn a_shared_ordinal_still_reads_back_in_one_fixed_order() {
    let (repo, shapes, _provider) = repo_harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    // Two phases on ordinal 0. Nothing on the table forbids it — a duplicate
    // ordinal is an authoring fault `PHASE_CHAIN_NONLINEAR` reports, and the
    // pipeline needs the row that produced the finding to be the same row on
    // every run. `ordinal` alone leaves these two unordered, so the `phase_id`
    // tie-break is what makes the read total.
    let terminal = PhaseId::new(Uuid::from_u128(0xf1c));
    let later = PlanPhase {
        phase_id: PhaseId::new(Uuid::from_u128(0xf1b)),
        kind: PhaseKind::Intro,
        ordinal: 0,
        converts_to_phase_id: Some(terminal),
        phase_duration_days: Some(7),
        display_trial_days: None,
    };
    let earlier = PlanPhase {
        phase_id: PhaseId::new(Uuid::from_u128(0xf1a)),
        ..later
    };
    let last = PlanPhase {
        phase_id: terminal,
        kind: PhaseKind::Evergreen,
        ordinal: 1,
        converts_to_phase_id: None,
        phase_duration_days: None,
        display_trial_days: None,
    };

    repo.create_draft(&scope, draft_of(plan_id, tenant))
        .await
        .expect("create");
    // Authored in the opposite order to the one the read must produce.
    shapes
        .replace_phases(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            vec![last, later, earlier],
            stamp(),
        )
        .await
        .expect("author the chain");

    assert_eq!(
        shapes
            .list_phases(&scope, tenant, plan_id, 0)
            .await
            .expect("read"),
        vec![earlier, later, last],
        "the chain must come back in ordinal then phase-id order"
    );
}

#[tokio::test]
async fn another_tenants_phase_chain_is_invisible() {
    let (repo, shapes, _provider) = repo_harness().await;
    let owner = Uuid::from_u128(0x7e_11);
    let owner_scope = AccessScope::for_tenant(owner);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&owner_scope, draft_of(plan_id, owner))
        .await
        .expect("create");
    shapes
        .replace_phases(
            &owner_scope,
            owner,
            plan_id,
            0,
            RowVersion::new(0),
            chain(),
            stamp(),
        )
        .await
        .expect("author the chain");

    // SQL-level BOLA: the catalog is commercially sensitive, so a foreign scope
    // sees absence rather than a refusal that would confirm the plan exists.
    let intruder = Uuid::from_u128(0x7e_22);
    let intruder_scope = AccessScope::for_tenant(intruder);
    assert!(
        shapes
            .list_phases(&intruder_scope, intruder, plan_id, 0)
            .await
            .expect("read")
            .is_empty(),
        "a foreign tenant must see no phases"
    );
    // Not even by naming the owner's tenant id: the scope decides, not the
    // argument.
    assert!(
        shapes
            .list_phases(&intruder_scope, owner, plan_id, 0)
            .await
            .expect("read")
            .is_empty(),
        "naming the owning tenant must not widen a foreign scope"
    );
    assert_eq!(
        shapes
            .list_phases(&owner_scope, owner, plan_id, 0)
            .await
            .expect("read"),
        chain(),
        "the owner still sees the whole chain"
    );
}

#[tokio::test]
async fn a_stale_version_replaces_nothing_and_leaves_the_chain_standing() {
    let (repo, shapes, _provider) = repo_harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, draft_of(plan_id, tenant))
        .await
        .expect("create");
    let authored = shapes
        .replace_phases(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            chain(),
            stamp(),
        )
        .await
        .expect("author the chain");

    // The child set has no entity tag of its own: the plan revision's covers
    // it, so writing phases advanced the revision's version. Without that bump
    // two authors editing different phases of one draft would both satisfy
    // `If-Match` and silently interleave.
    assert_eq!(authored.row_version, RowVersion::new(1));

    let replacement = vec![PlanPhase {
        phase_id: PhaseId::new(Uuid::from_u128(0xf1c)),
        kind: PhaseKind::Evergreen,
        ordinal: 0,
        converts_to_phase_id: None,
        phase_duration_days: None,
        display_trial_days: None,
    }];
    let err = shapes
        .replace_phases(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            replacement,
            stamp(),
        )
        .await
        .expect_err("a shape edit under a superseded tag must be refused");
    assert!(
        matches!(
            err,
            RepoError::StaleRowVersion {
                current: 1,
                submitted: 0,
                ..
            }
        ),
        "got: {err:?}"
    );

    // The delete runs before the swap answers, so the rollback is what keeps
    // this true — not an ordering that avoided touching the table.
    assert_eq!(
        shapes
            .list_phases(&scope, tenant, plan_id, 0)
            .await
            .expect("read"),
        chain(),
        "a refused replacement must leave the previous chain intact"
    );
}

#[tokio::test]
async fn a_published_revisions_chain_is_refused_by_name_not_by_trigger() {
    let (repo, shapes, provider) = repo_harness().await;
    let tenant = Uuid::from_u128(0x7e_11);
    let scope = AccessScope::for_tenant(tenant);
    let plan_id = PlanId::new(Uuid::from_u128(0x9_1a4));

    repo.create_draft(&scope, draft_of(plan_id, tenant))
        .await
        .expect("create");
    let authored = shapes
        .replace_phases(
            &scope,
            tenant,
            plan_id,
            0,
            RowVersion::new(0),
            chain(),
            stamp(),
        )
        .await
        .expect("author the chain");
    publish(&provider, &scope, plan_id).await;

    // The table's own trigger would answer this with a raw driver error
    // carrying no state and no subject, which reaches a caller as an internal
    // fault. The read that precedes the delete is what turns it into a refusal
    // a surface can render — and the remedy it implies, open a new revision, is
    // a real one.
    let err = shapes
        .replace_phases(
            &scope,
            tenant,
            plan_id,
            0,
            authored.row_version,
            Vec::new(),
            stamp(),
        )
        .await
        .expect_err("a frozen revision's shape is not editable");
    assert!(
        matches!(err, RepoError::NotDraft { ref state, .. } if state == "published"),
        "got: {err:?}"
    );
    assert_eq!(
        shapes
            .list_phases(&scope, tenant, plan_id, 0)
            .await
            .expect("read"),
        chain(),
        "the frozen chain must be untouched"
    );
}

/// The actor and instant every mutating repository call now records (D-135 - the
/// audit row commits inside the mutation's own transaction).
fn stamp() -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        actor_principal_id: uuid::Uuid::from_u128(0xac_10),
        recorded_at: chrono::Utc::now(),
        correlation_id: None,
    }
}
