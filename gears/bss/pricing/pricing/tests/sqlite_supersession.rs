//! `inst-su-commit`'s writes, across **both** planes, against a real database.
//!
//! The row half is pinned one file over (`sqlite_price_repo.rs`): the predecessor's
//! flip and the successor's publish, and the order between them. What this suite is
//! for is the half that file cannot reach — the two **window** operations landing in
//! the same transaction as the two row operations, and the ordering *between* the
//! window operations, which is a second constraint of exactly the same shape as the
//! row one and is enforced by a different mechanism.
//!
//! A unit test over a composed plan could assert the four calls happen. It could not
//! assert that the third one fails when the second has not run, which is the only
//! property that makes the composition an order rather than a list.
//!
//! Every instant here is a **fixed** date. `inst-ws-future-start` and the changeover
//! floors both compare against a clock, so a fixture computed from `now()` is one
//! that asserts something different every day it runs; the suite's dates sit in 2099
//! so that "future" stays a fact, and the one case that needs a *past* window says so
//! at its own constant.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;

use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::concurrency::RowVersion;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::domain::supersession::{ChangeoverMoment, NamedWindow, plan_supersession};
use bss_pricing::domain::window::{WindowInterval, WindowState};
use bss_pricing::infra::storage::entity::{price, price_window};
use bss_pricing::infra::storage::migrations::Migrator;
use bss_pricing::infra::storage::repo::{NewPriceDraft, PriceRepo, price_repo, window_repo};
use bss_pricing::infra::storage::{RepoError, repo_failure};
use bss_pricing::infra::supersession::{SupersessionCommit, commit_supersession};
use chrono::{DateTime, TimeZone, Utc};
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use sea_orm_migration::MigratorTrait;
use toolkit_db::migration_runner::run_migrations_for_testing;
use toolkit_db::secure::{AccessScope, SecureEntityExt, SecureUpdateExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_c0_11_a7_20);

fn tenant() -> Uuid {
    Uuid::from_u128(0x7e_31)
}

fn plan() -> PlanId {
    PlanId::new(Uuid::from_u128(0x9_2b5))
}

/// The predecessor row, its window, and the successor draft standing beside it.
const PREDECESSOR: Uuid = Uuid::from_u128(0xb_7001);
const SUCCESSOR: Uuid = Uuid::from_u128(0xb_7002);
const SUCCESSOR_WINDOW: Uuid = Uuid::from_u128(0xb_7f02);

/// The suite's fixed reading of "now" — every instant below is placed against it.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap()
}

/// The predecessor's coverage: open-ended from before `now`.
fn coverage_from() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2098, 6, 1, 0, 0, 0).unwrap()
}

/// The changeover: well clear of both of `inst-su-instant`'s floors.
fn changeover() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 4, 1, 0, 0, 0).unwrap()
}

fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: Uuid::from_u128(0xac_20),
        recorded_at: now(),
        correlation_id: TEST_CORRELATION,
    }
}

fn key() -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("USD").expect("three letters"),
        Region::new("EU").expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_6e)),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none")
}

fn content(amount: i64) -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(amount).expect("non-negative"));
    PriceContent {
        row,
        tax_inclusive: false,
        billing_timing: Some("advance".to_owned()),
        rounding_policy_ref: None,
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

fn draft(price_id: Uuid, amount: i64) -> NewPriceDraft {
    NewPriceDraft {
        price_id,
        scope_key: key(),
        content: content(amount),
        created_by: Uuid::from_u128(0xac_20),
        created_at_utc: now(),
        correlation_id: TEST_CORRELATION,
    }
}

async fn harness() -> (PriceRepo, DBProvider<DbError>) {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run migrator");
    let provider = DBProvider::<DbError>::new(db);
    (PriceRepo::new(provider.clone()), provider)
}

/// Move a row's `lifecycle_state` directly — `sqlite_price_repo.rs`'s helper, for
/// its reason: a suite about the commit should not have to run a publish unit to
/// reach a `published` predecessor.
async fn flip_state(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    price_id: Uuid,
    state: LifecycleState,
) {
    let conn = provider.conn().expect("conn");
    let result = price::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(price::Column::LifecycleState, Expr::value(state.as_str()))
        .filter(Condition::all().add(price::Column::PriceId.eq(price_id)))
        .exec(&conn)
        .await
        .expect("flip the lifecycle state");
    assert_eq!(result.rows_affected, 1, "the seed must have moved one row");
}

/// The world a composed supersession commits against: a published predecessor with
/// open-ended coverage, and the successor draft standing beside it on one key.
///
/// The successor is authored through **`insert_successor_draft_on`**, the D-195 door,
/// rather than fabricated — this suite's subject is the commit, and a commit staged
/// by anything but the real door would be a commit over a world the gear cannot
/// produce.
async fn composed(repo: &PriceRepo, provider: &DBProvider<DbError>, scope: &AccessScope) -> u64 {
    repo.create_draft(scope, tenant(), draft(PREDECESSOR, 1_000))
        .await
        .expect("author the predecessor");
    flip_state(provider, scope, PREDECESSOR, LifecycleState::Published).await;

    let scope_owned = scope.clone();
    let conn = provider.conn().expect("conn");
    let window = window_repo::schedule(
        &conn,
        &scope_owned,
        window_repo::NewWindow {
            window_id: predecessor_window(),
            tenant_id: tenant(),
            price_id: PREDECESSOR,
            effective_from: coverage_from(),
            effective_to: None,
            reason_code: "initialCoverage".to_owned(),
        },
        stamp(),
    )
    .await
    .expect("the predecessor's open-ended coverage");

    let scope_for_door = scope.clone();
    let (_, outcome) = provider
        .db()
        .in_transaction::<(), RepoError, _>(move |txn| {
            Box::pin(async move {
                price_repo::insert_successor_draft_on(
                    txn,
                    &scope_for_door,
                    tenant(),
                    draft(SUCCESSOR, 1_200),
                )
                .await
                .map(|_| ())
            })
        })
        .await;
    outcome.expect("stage the successor through D-195's door");

    window.mutation_seq
}

fn predecessor_window() -> Uuid {
    Uuid::from_u128(0xb_7f01)
}

/// The commit a composed plan asks for, built through `SupersessionCommit::of_plan`
/// from a real `compose_windows` output.
///
/// That constructor is the **only** way to build one, which is the point: the two
/// window instants both come out of the one changeover the plane was validated
/// against, so the coverage void the earlier public-field shape permitted cannot be
/// expressed here — not even deliberately, by a test trying to.
fn commit_built(shorten_seq: u64, successor: (Uuid, RowVersion)) -> SupersessionCommit {
    let plane = [NamedWindow {
        window_id: predecessor_window(),
        interval: WindowInterval::new(coverage_from(), None, WindowState::Scheduled),
    }];
    let composed_plan = plan_supersession(
        &content(1_000).row,
        &content(1_200).row,
        &plane,
        changeover(),
        now(),
        ChangeoverMoment::Commit,
    )
    .expect("the fixture world composes");
    SupersessionCommit::of_plan(
        &composed_plan,
        plan(),
        PREDECESSOR,
        shorten_seq,
        successor,
        SUCCESSOR_WINDOW,
        "repricing".to_owned(),
    )
}

/// The ordinary commit: the fixture's successor at the version it was authored with.
fn commit_of(shorten_seq: u64) -> SupersessionCommit {
    commit_built(shorten_seq, (SUCCESSOR, RowVersion::new(0)))
}

/// A second key of the same plan — a different eligibility class, so a legal key that
/// this supersession has nothing to do with.
fn other_key() -> ScopeKey {
    ScopeKey::new(
        plan(),
        CurrencyCode::new("USD").expect("three letters"),
        Region::new("EU").expect("a non-blank region"),
        PhaseId::new(Uuid::from_u128(0xfa_6e)),
        PriceEligibility::NewSubscriptionsOnly,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("new_subscriptions_only pairs with cohort none")
}

async fn commit(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    plan: SupersessionCommit,
) -> Result<(), RepoError> {
    let scope = scope.clone();
    let (_, outcome) = provider
        .db()
        .in_transaction::<(), RepoError, _>(move |txn| {
            Box::pin(async move {
                commit_supersession(txn, &scope, tenant(), plan, stamp())
                    .await
                    .map(|_| ())
            })
        })
        .await;
    outcome.map_err(|err| {
        err.into_domain(|infra| RepoError::Db(format!("supersession commit: {infra}")))
    })
}

async fn stored_row(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    price_id: Uuid,
) -> price::Model {
    let conn = provider.conn().expect("conn");
    price::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(price::Column::PriceId.eq(price_id)))
        .one(&conn)
        .await
        .expect("read the row")
        .expect("the row is there")
}

async fn stored_window(
    provider: &DBProvider<DbError>,
    scope: &AccessScope,
    window_id: Uuid,
) -> Option<price_window::Model> {
    let conn = provider.conn().expect("conn");
    price_window::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(Condition::all().add(price_window::Column::WindowId.eq(window_id)))
        .one(&conn)
        .await
        .expect("read the window")
}

#[tokio::test]
async fn all_four_writes_land_and_the_key_is_covered_end_to_end() {
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;

    commit(&provider, &scope, commit_of(seq))
        .await
        .expect("the unit commits");

    assert_eq!(
        stored_row(&provider, &scope, PREDECESSOR)
            .await
            .lifecycle_state,
        LifecycleState::Superseded.as_str()
    );
    assert_eq!(
        stored_row(&provider, &scope, SUCCESSOR)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str()
    );

    let shortened = stored_window(&provider, &scope, predecessor_window())
        .await
        .expect("the predecessor's window is still there");
    assert_eq!(
        shortened.effective_to,
        Some(changeover()),
        "the predecessor's coverage now ends at the changeover"
    );
    let successor = stored_window(&provider, &scope, SUCCESSOR_WINDOW)
        .await
        .expect("the successor's window was scheduled");
    assert_eq!(successor.effective_from, changeover());
    assert_eq!(
        successor.effective_to, None,
        "`inst-su-compose` schedules the successor open-ended"
    );
    assert_eq!(successor.state, WindowState::Scheduled.as_str());
    // The property the whole unit exists for: the changeover instant is covered by
    // exactly one window, and no instant of the key is left uncovered.
    assert_eq!(shortened.effective_to, Some(successor.effective_from));
}

#[tokio::test]
async fn the_successors_window_cannot_be_scheduled_before_its_predecessors_is_shortened() {
    // The window plane's own ordering constraint, and it is **not** the row plane's:
    // `window_repo::schedule` refuses an interval intersecting one already on the key,
    // so an open-ended successor scheduled while the predecessor's window is still
    // open-ended is `WINDOW_OVERLAP`. Same shape as the row ordering D-195 settled,
    // enforced by a different mechanism, which is why it needs its own case.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;

    // Scheduling first, by hand, is what the composition must never do.
    let conn = provider.conn().expect("conn");
    let refused = window_repo::schedule(
        &conn,
        &scope,
        window_repo::NewWindow {
            window_id: SUCCESSOR_WINDOW,
            tenant_id: tenant(),
            price_id: SUCCESSOR,
            effective_from: changeover(),
            effective_to: None,
            reason_code: "repricing".to_owned(),
        },
        stamp(),
    )
    .await
    .expect_err("the predecessor's open-ended window still claims that instant");
    assert!(
        matches!(refused, RepoError::WindowOverlap { .. }),
        "got: {refused:?}"
    );

    // And through the composition, which shortens first, the same schedule lands.
    commit(&provider, &scope, commit_of(seq))
        .await
        .expect("the composition orders the two window writes");
    assert!(
        stored_window(&provider, &scope, SUCCESSOR_WINDOW)
            .await
            .is_some()
    );
}

#[tokio::test]
async fn a_refused_window_write_rolls_back_the_row_writes_with_it() {
    // `inst-su-commit`: "or everything rolls back", now across both planes. The
    // window shorten is refused on a stale act sequence (D-191's precondition), and
    // what must not survive it is the predecessor's flip.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;

    let err = commit(&provider, &scope, commit_of(seq + 7))
        .await
        .expect_err("the window's own precondition still decides");
    assert!(
        matches!(err, RepoError::StaleRowVersion { .. }),
        "got: {err:?}"
    );

    assert_eq!(
        stored_row(&provider, &scope, PREDECESSOR)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str(),
        "the predecessor never left the published plane"
    );
    assert_eq!(
        stored_row(&provider, &scope, SUCCESSOR)
            .await
            .lifecycle_state,
        LifecycleState::Draft.as_str()
    );
    assert!(
        stored_window(&provider, &scope, SUCCESSOR_WINDOW)
            .await
            .is_none(),
        "and no window was scheduled"
    );
}

#[tokio::test]
async fn a_refused_row_write_rolls_back_the_window_writes_with_it() {
    // The same property from the other side: the successor is refused on a version
    // that moved, and the two window operations must not survive it. This is the
    // direction a composition that wrote the windows in a transaction of their own
    // would fail, and it is the one that would leave a key advertising coverage for a
    // row that never published.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;

    let err = commit(
        &provider,
        &scope,
        commit_built(seq, (SUCCESSOR, RowVersion::new(9))),
    )
    .await
    .expect_err("the successor's own precondition still decides");
    assert!(
        matches!(err, RepoError::StaleRowVersion { .. }),
        "got: {err:?}"
    );

    let window = stored_window(&provider, &scope, predecessor_window())
        .await
        .expect("still there");
    assert_eq!(
        window.effective_to, None,
        "the predecessor's coverage was never shortened"
    );
    assert!(
        stored_window(&provider, &scope, SUCCESSOR_WINDOW)
            .await
            .is_none()
    );
}

#[tokio::test]
async fn the_shorten_carries_the_act_sequence_it_was_composed_against() {
    // D-191's precondition on the window plane is the act counter, not a row version,
    // and the composition presents it rather than reading it again inside the commit.
    // A commit that re-read it would apply a shorten over an adjustment somebody made
    // between compose and approval — the very window the precondition exists to close.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;

    // Somebody adjusts the predecessor's window after the unit was composed.
    let conn = provider.conn().expect("conn");
    window_repo::adjust_effective_to(
        &conn,
        &scope,
        tenant(),
        predecessor_window(),
        Some(Utc.with_ymd_and_hms(2099, 8, 1, 0, 0, 0).unwrap()),
        seq,
        stamp(),
    )
    .await
    .expect("a legal future-end adjustment");

    let err = commit(&provider, &scope, commit_of(seq))
        .await
        .expect_err("the sequence the unit was composed against is gone");
    assert!(
        matches!(err, RepoError::StaleRowVersion { .. }),
        "got: {err:?}"
    );
    assert_eq!(
        stored_row(&provider, &scope, PREDECESSOR)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str()
    );
}

#[tokio::test]
async fn the_predecessors_window_is_shortened_and_never_cancelled() {
    // `inst-ws-immutable`: an active or historical window is never cancelled or
    // deleted — an active window is only shortened. A composition that cancelled the
    // predecessor's window and scheduled two new ones would produce the same coverage
    // and destroy the key's history, and D-121 keeps cancelled windows out of the read
    // model entirely, so a pinned consumer would lose the interval it resolves against.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;
    commit(&provider, &scope, commit_of(seq))
        .await
        .expect("commit");

    let window = stored_window(&provider, &scope, predecessor_window())
        .await
        .expect("still there");
    // `!= cancelled` does catch the named alternative — a cancel-and-reschedule leaves
    // this window `cancelled`, and cancellation is the only way to free the key, since
    // `OCCUPYING_STATES` includes `scheduled` and DELETE is refused. But it is the weakest
    // form of the assertion, so the state is pinned positively **and the act count with
    // it**: `mutation_seq` advances once per operator act (D-190) and is unmoved by the
    // sweeps, so `seq + 1` says exactly one act touched this window. Nothing else in the
    // suite pinned that (review, 2026-08-05).
    assert_eq!(window.state, WindowState::Scheduled.as_str());
    assert_eq!(
        u64::try_from(window.mutation_seq).expect("a stored act counter is non-negative"),
        seq + 1,
        "exactly one act: the shorten, and not a cancel plus a reschedule"
    );
    assert_eq!(
        window.effective_from,
        coverage_from(),
        "and its start is untouched"
    );
}

#[tokio::test]
async fn an_active_predecessor_window_is_shortened_the_same_way() {
    // `inst-ws-immutable`'s actual sentence is "the only permitted mutation of an
    // **active** window is moving its future `effectiveTo`", and every other case here
    // shortens a `scheduled` one — the state `window_repo::schedule` creates. In
    // production the predecessor's window has been swept to `active` long before anyone
    // reprices it, so the sentence the rule is written in was never exercised (review,
    // 2026-08-05).
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;
    activate_window(&provider, &scope, predecessor_window()).await;

    commit(&provider, &scope, commit_of(seq))
        .await
        .expect("an active window is shortened, not refused");

    let window = stored_window(&provider, &scope, predecessor_window())
        .await
        .expect("still there");
    assert_eq!(window.effective_to, Some(changeover()));
    assert_eq!(
        window.state,
        WindowState::Active.as_str(),
        "shortening does not move it off `active`; that is the expiry sweep's job"
    );
}

#[tokio::test]
async fn a_predecessor_that_has_already_been_superseded_is_refused_by_name() {
    // The replay case, through the composition rather than through the row repository.
    // Worth its own case because **windows-first changes which refusal wins**: a true
    // replay of the same unit meets the predecessor window's advanced act counter and is
    // answered `StaleRowVersion`, which sends an operator to re-read a window tag. This
    // case isolates the row-plane refusal by leaving the window untouched, so what it pins
    // is that `NotSupersedable` is reachable through this path at all and names the state
    // it found (review, 2026-08-05).
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;
    flip_state(&provider, &scope, PREDECESSOR, LifecycleState::Superseded).await;

    let err = commit(&provider, &scope, commit_of(seq))
        .await
        .expect_err("a row that has already left the published plane is not supersedable");
    let RepoError::NotSupersedable { state, .. } = err else {
        panic!("got: {err:?}");
    };
    assert_eq!(state, LifecycleState::Superseded.as_str());
}

#[tokio::test]
async fn another_tenant_cannot_commit_this_supersession() {
    // One cross-tenant case over the four-write composition, which had none. Every write
    // goes through SecureORM, so a foreign scope should see nothing to move rather than be
    // told it may not — the existence leak `RepoError::NotFound`'s doc exists to close.
    // Cheap, and in a multi-tenant gear its absence over a composition this wide was the
    // gap worth naming (review, 2026-08-05).
    let (repo, provider) = harness().await;
    let mine = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &mine).await;
    let theirs = AccessScope::for_tenant(Uuid::from_u128(0x_7e32));

    let err = commit(&provider, &theirs, commit_of(seq))
        .await
        .expect_err("another tenant's supersession is not there to commit");
    assert!(
        matches!(err, RepoError::NotFound { .. }),
        "absence rather than refusal, so the surface leaks no existence: {err:?}"
    );

    assert_eq!(
        stored_row(&provider, &mine, PREDECESSOR)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str(),
        "and nothing of mine moved"
    );
}

/// Activate a window directly, without waiting for the sweep.
///
/// The activation job is what does this in production (`inst-ws-activate`); fabricated
/// here for the reason `flip_state` fabricates the row flips — a suite about the commit
/// should not have to run a background job to reach the state it asserts against.
///
/// **`activated_at` moves with the state, because the store will not let it not.**
/// `chk_pricing_price_window_activated_at` requires the stamp to be set exactly on an
/// `active` or `expired` window, so the first version of this seed — which wrote the
/// state token alone — was refused by the CHECK. That is the store keeping a fabrication
/// honest, and it is why this helper takes no state parameter: `active` is the one
/// transition it is for, and the stamp is part of that transition rather than a field
/// beside it.
async fn activate_window(provider: &DBProvider<DbError>, scope: &AccessScope, window_id: Uuid) {
    let conn = provider.conn().expect("conn");
    let result = price_window::Entity::update_many()
        .secure()
        .scope_with(scope)
        .col_expr(
            price_window::Column::State,
            Expr::value(WindowState::Active.as_str()),
        )
        .col_expr(
            price_window::Column::ActivatedAt,
            Expr::value(coverage_from()),
        )
        .filter(Condition::all().add(price_window::Column::WindowId.eq(window_id)))
        .exec(&conn)
        .await
        .expect("activate the window");
    assert_eq!(
        result.rows_affected, 1,
        "the seed must have moved one window"
    );
}

/// Keeps `repo_failure` referenced from this suite the way the sibling suites do —
/// the conversion is what a service layer above this composition will apply, and a
/// suite that never names it would not notice the day it stops compiling.
#[test]
fn the_repository_refusals_convert_for_a_service_caller() {
    let converted = repo_failure(&RepoError::StaleRowVersion {
        subject: "price".to_owned(),
        id: SUCCESSOR.to_string(),
        current: 1,
        submitted: 0,
    });
    assert!(matches!(
        converted,
        bss_pricing::domain::error::DomainError::StaleVersion(_)
    ));
}

// ---------------------------------------------------------------------------
// The cross-field invariants the composed plan proves, and the commit must not
// be able to drop (found by review, 2026-08-05).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_successor_on_another_key_cannot_be_paired_with_this_predecessor() {
    // The invariant whose violation is worst in kind: flip the predecessor, freeing
    // its key on the published plane, and publish a successor standing on a
    // *different* key of the same plan. The predecessor's key is then left with **no
    // current row at all** — the state `commit_supersession_rows`' own doc calls
    // worse than either move not having happened — and nothing refused it.
    //
    // The two ids arrive as independent arguments, so only a check inside the commit
    // can tell them apart: `publish_rows` filters by plan, not by key, and
    // `supersede_row` never looks at the successor.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;

    // A draft of the same plan, on a different eligibility class — a legal row that
    // this supersession has nothing to do with.
    let stranger = Uuid::from_u128(0xb_7003);
    let mut elsewhere = draft(stranger, 900);
    elsewhere.scope_key = other_key();
    repo.create_draft(&scope, tenant(), elsewhere)
        .await
        .expect("author an unrelated draft of the same plan");

    let err = commit(
        &provider,
        &scope,
        commit_built(seq, (stranger, RowVersion::new(0))),
    )
    .await
    .expect_err("a successor on another key is not this predecessor's successor");
    assert!(
        matches!(err, RepoError::NotSupersedable { .. }),
        "got: {err:?}"
    );

    assert_eq!(
        stored_row(&provider, &scope, PREDECESSOR)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str(),
        "the predecessor's key never lost its current row"
    );
    assert_eq!(
        stored_row(&provider, &scope, stranger)
            .await
            .lifecycle_state,
        LifecycleState::Draft.as_str()
    );
}

#[tokio::test]
async fn a_successor_whose_link_names_someone_else_is_refused() {
    // D-127 requires the successor to carry `supersedes_price_id`, and the door
    // stamps it. It is **not** the only writer of that column — the authoring routes
    // plumb a caller-supplied value straight through — so the commit checks the link
    // rather than trusting that it was stamped. Key equality is the load-bearing
    // half; this is the second belt, and it is the one that catches a draft authored
    // on a free key that later becomes occupied.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;

    let conn = provider.conn().expect("conn");
    price::Entity::update_many()
        .secure()
        .scope_with(&scope)
        .col_expr(
            price::Column::SupersedesPriceId,
            Expr::value(Uuid::from_u128(0xdead_beef)),
        )
        .filter(Condition::all().add(price::Column::PriceId.eq(SUCCESSOR)))
        .exec(&conn)
        .await
        .expect("point the link at a stranger");

    let err = commit(&provider, &scope, commit_of(seq))
        .await
        .expect_err("the successor does not claim to supersede this predecessor");
    assert!(
        matches!(err, RepoError::NotSupersedable { .. }),
        "got: {err:?}"
    );
    assert_eq!(
        stored_row(&provider, &scope, PREDECESSOR)
            .await
            .lifecycle_state,
        LifecycleState::Published.as_str()
    );
}

#[tokio::test]
async fn the_two_window_instants_cannot_disagree_because_one_value_produces_both() {
    // The invariant whose violation **commits silently**, which is why it is closed
    // structurally rather than refused. A shorten to T1 and a schedule from T2 > T1
    // leaves `[T1, T2)` uncovered, and nothing in the transaction notices: window
    // overlap is an intersection test, and a gap is not an intersection.
    //
    // So `SupersessionCommit` is built from the `ComposedWindows` that
    // `compose_windows` proved adjacent, and both instants come from the one
    // changeover. There is no way to spell the disagreement, which is what this case
    // asserts — it is a statement about the type, driven through the store to show
    // the type is what the commit actually uses.
    let (repo, provider) = harness().await;
    let scope = AccessScope::for_tenant(tenant());
    let seq = composed(&repo, &provider, &scope).await;

    let plan = commit_of(seq);
    assert_eq!(
        plan.shorten().effective_to,
        plan.successor_from(),
        "one changeover, two writes, no second value to disagree with"
    );
    commit(&provider, &scope, plan).await.expect("commit");

    let shortened = stored_window(&provider, &scope, predecessor_window())
        .await
        .expect("there");
    let scheduled = stored_window(&provider, &scope, SUCCESSOR_WINDOW)
        .await
        .expect("there");
    assert_eq!(shortened.effective_to, Some(scheduled.effective_from));
}
