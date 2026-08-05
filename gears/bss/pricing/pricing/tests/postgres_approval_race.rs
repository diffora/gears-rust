//! The TOCTOU guard's race, on real Postgres: a mutation of the pinned subject
//! and an approve of its record, concurrently.
//!
//! # Why this cannot be a `SQLite` suite
//!
//! `sqlite::memory:` **serializes writers**, so the interleaving under test can
//! be neither confirmed nor refuted there — not "probably fine", unanswered in
//! both directions. `tests/sqlite_approval_service.rs` proves that a mutation
//! voids a pending unit and that a decided unit refuses a second decision; what
//! it cannot show is what happens when the two statements are **in flight at the
//! same time**, which is the whole hazard `inst-ap-pin` names.
//!
//! # The choreography, and why each step is there
//!
//! A concurrency test that starts two tasks and asserts on the outcome is a coin
//! toss with a green side. Both races here are driven by observable events only,
//! in the idiom `tests/postgres_audit_chain.rs` established:
//!
//! 1. the mutating transaction runs its write — which includes the void — and
//!    then **parks**, holding the row lock on the pending record;
//! 2. the approving call starts; its read sees the still-committed `submitted`
//!    row, and its compare-and-swap blocks on that lock;
//! 3. a third connection **observes the block** in `pg_locks`, which is what
//!    proves the approve's read already happened;
//! 4. only then is the mutation released to commit, and the approve's `UPDATE`
//!    re-evaluates its `state = 'submitted'` predicate under READ COMMITTED,
//!    matches nothing, and resolves into the refusal.
//!
//! Step 3 is the load-bearing one. Without it the approve could read the *voided*
//! row and be refused by its own pendingness check before ever contending —
//! green, and about nothing.
//!
//! # What "exactly one wins" means here, precisely
//!
//! The contended object is the **pending record**, and what is at stake is which
//! of two writes to it takes effect. The two orderings are not symmetric and
//! both are asserted:
//!
//! - **The void commits first.** The approve loses and is answered
//!   `APPROVAL_NOT_PENDING` — a **409**, the outcome §5 specifies — rather than
//!   the driver error a repository that trusted its read would produce, which
//!   reaches an operator as a 500 about a race whose whole remedy is to re-read.
//! - **The approve commits first.** The void's `UPDATE` matches nothing and the
//!   mutation still commits. That is deliberate and load-bearing rather than a
//!   loss: `inst-as-immutable` makes a decided record untouchable, so a void that
//!   tried to close it would be refused by
//!   `trg_pricing_approval_append_only` — and because the void runs inside the
//!   mutation's transaction, that refusal would roll the **mutation** back. Every
//!   authoring edit to a plan that had ever been approved would fail.
//!
//! The residue of the second case is reported rather than hidden: an approval
//! that has already been granted is **not** re-invalidated by a later mutation.
//! `inst-ap-pin` scopes the void to a `submitted` record in as many words ("any
//! mutation of the subject **while `submitted`**"), so this is the design set's
//! own boundary and not a hole in the implementation of it — but it means the
//! window between `approved` and the publish commit is guarded by nothing here.
//! Closing it belongs to the publish handler, which holds the record's
//! `content_hash` and can re-verify the pin before handing
//! `PublishAuthorization::Approved` to `commit`.
//!
//! Ignored by default; they need Docker. Run with
//! `cargo test -p bss-pricing --test postgres_approval_race -- --ignored`.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod pg_support;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use bss_pricing::domain::approval::{ApprovalState, DecisionBy};
use bss_pricing::domain::audit::AuditStamp;
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::money::{CurrencyCode, MinorAmount};
use bss_pricing::domain::plan_shape::{
    BillingCycle, DescriptorSet, Frequency, PhaseKind, PlanPhase,
};
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::price_row::{ModelKind, PriceRow};
use bss_pricing::domain::scope_key::{
    ChargeKind, Cohort, PhaseId, PlanId, PriceEligibility, Region, ScopeKey,
};
use bss_pricing::infra::approval::{ApprovalService, DecideRequest, RegionGrant};
use bss_pricing::infra::storage::RepoError;
use bss_pricing::infra::storage::repo::price_repo::{self, NewPriceDraft};
use bss_pricing::infra::storage::repo::{PlanRepo, PlanShapeRepo, PriceRepo};
use chrono::{DateTime, TimeZone, Utc};
use pg_support::Pg;
use serde_json::json;
use tokio::sync::Notify;
use toolkit_db::secure::AccessScope;
use toolkit_db::{DBProvider, DbError};
use uuid::Uuid;

/// One value for a whole test binary: these suites drive a repository or a
/// service directly, where the value the HTTP edge would have established has
/// no producer. What each suite asserts *about* it is stated where it asserts
/// it.
const TEST_CORRELATION: uuid::Uuid = uuid::Uuid::from_u128(0x_c0_11_a7_10);

const TENANT: Uuid = Uuid::from_u128(0x7e_11);
const SUBMITTER: Uuid = Uuid::from_u128(0x5b_01);
const APPROVER: Uuid = Uuid::from_u128(0xab_01);

/// Generous, because a cold container under load is slow — but **finite**: a
/// racer that never resolves is a refuted claim, not a slow one.
const RACE_TIMEOUT: Duration = Duration::from_secs(30);

fn plan_id() -> PlanId {
    PlanId::new(Uuid::from_u128(0x9_1a4))
}

fn terminal_phase() -> PhaseId {
    PhaseId::new(Uuid::from_u128(0xfa_5e))
}

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 3, hour, 0, 0).unwrap()
}

fn stamp() -> AuditStamp {
    AuditStamp {
        actor_principal_id: SUBMITTER,
        recorded_at: at(11),
        correlation_id: TEST_CORRELATION,
    }
}

/// The stamp a decision is taken under: who acted, when, and the request's
/// correlation.
fn stamp_of(actor: uuid::Uuid, when: DateTime<Utc>) -> AuditStamp {
    AuditStamp {
        actor_principal_id: actor,
        recorded_at: when,
        correlation_id: TEST_CORRELATION,
    }
}

fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

fn scope_key(market: &str) -> ScopeKey {
    ScopeKey::new(
        plan_id(),
        CurrencyCode::new("EUR").expect("three letters"),
        Region::new(market).expect("a non-blank region"),
        terminal_phase(),
        PriceEligibility::AllSubscriptions,
        ChargeKind::Recurring,
        Cohort::None,
    )
    .expect("all_subscriptions pairs with cohort none")
}

fn flat_row() -> PriceContent {
    let mut row = PriceRow::new(ChargeKind::Recurring, Some(ModelKind::Flat));
    row.amount_minor = Some(MinorAmount::new(9_900).expect("a non-negative amount"));
    PriceContent {
        row,
        tax_inclusive: false,
        billing_timing: Some("advance".to_owned()),
        rounding_policy_ref: Some("half_up".to_owned()),
        grandfather_until: None,
        supersedes_price_id: None,
    }
}

fn new_price(market: &str, seed: u128) -> NewPriceDraft {
    NewPriceDraft {
        price_id: Uuid::from_u128(seed),
        scope_key: scope_key(market),
        content: flat_row(),
        created_by: SUBMITTER,
        created_at_utc: at(10),
        correlation_id: TEST_CORRELATION,
    }
}

/// One plan with a phase chain, a descriptor set and one `eu` row, committed —
/// so both racers below read one world.
async fn seed(pg: &Pg) {
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let plans = PlanRepo::new(provider.clone());
    let shapes = PlanShapeRepo::new(provider.clone());
    let prices = PriceRepo::new(provider.clone());
    let scope = scope();

    let created = plans
        .create_draft(
            &scope,
            bss_pricing::infra::storage::repo::plan_repo::NewPlanDraft {
                plan_id: plan_id(),
                tenant_id: TENANT,
                created_by: SUBMITTER,
                created_at_utc: at(10),
                sku_id: Some(Uuid::from_u128(0x5_c1)),
                plan_tier: Some("gold".to_owned()),
                billing_cycle: Some(BillingCycle::Recurring),
                frequency: Some(Frequency::Monthly),
                plan_tier_override: false,
                purchase_min_qty: None,
                purchase_max_qty: None,
                invoice_grouping_key: None,
                available_from: None,
                available_to: None,
                correlation_id: TEST_CORRELATION,
            },
        )
        .await
        .expect("create the draft");
    let after_phases = shapes
        .replace_phases(
            &scope,
            TENANT,
            plan_id(),
            created.revision,
            created.row_version,
            vec![PlanPhase {
                phase_id: terminal_phase(),
                kind: PhaseKind::Evergreen,
                ordinal: 0,
                converts_to_phase_id: None,
                phase_duration_days: None,
                display_trial_days: None,
            }],
            stamp(),
        )
        .await
        .expect("attach the phase chain");
    shapes
        .set_descriptor_set(
            &scope,
            TENANT,
            plan_id(),
            created.revision,
            after_phases.row_version,
            DescriptorSet {
                invoice_line_template: Some("{plan}".to_owned()),
                gl_code: Some("4000".to_owned()),
                itemization_rule: Some("per_charge".to_owned()),
                additional: BTreeMap::new(),
            },
            stamp(),
        )
        .await
        .expect("attach the descriptor set");
    prices
        .create_draft(&scope, TENANT, new_price("eu", 0xb_0001))
        .await
        .expect("author the price row");
}

/// Open the pending unit whose fate the two racers decide.
async fn submit(pg: &Pg, approval_id: Uuid) {
    let approvals = ApprovalService::new(DBProvider::<DbError>::new(pg.db().await));
    approvals
        .submit(
            &scope(),
            TENANT,
            plan_id(),
            approval_id,
            json!({ "reason": "noConfiguredThreshold" }),
            stamp_of(SUBMITTER, at(12)),
        )
        .await
        .expect("open the pending unit");
}

fn approve(approval_id: Uuid) -> DecideRequest {
    DecideRequest {
        approval_id,
        decision: DecisionBy::Approve(APPROVER),
        reason: None,
        approver_regions: RegionGrant::Explicit(BTreeSet::from([
            Region::new("eu").expect("a non-blank region")
        ])),
        stamp: stamp_of(APPROVER, at(13)),
    }
}

async fn state_of(pg: &Pg, approval_id: Uuid) -> ApprovalState {
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let conn = provider.conn().expect("conn");
    bss_pricing::infra::storage::repo::approval_repo::read(&conn, &scope(), TENANT, approval_id)
        .await
        .expect("read the unit")
        .expect("the unit is there")
        .state
}

// ---------------------------------------------------------------------------
// The world in which the race means something
// ---------------------------------------------------------------------------

/// **Uncontended, the approve succeeds.**
///
/// Without this the race below would pass against a service that refuses every
/// approve, and the whole suite would be evidence about nothing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker (testcontainers)"]
async fn an_uncontended_approve_succeeds() {
    let pg = Pg::applied().await;
    seed(&pg).await;
    let id = Uuid::from_u128(0xa1);
    submit(&pg, id).await;

    let approvals = ApprovalService::new(DBProvider::<DbError>::new(pg.db().await));
    let decided = approvals
        .decide(&scope(), TENANT, approve(id))
        .await
        .expect("nothing is contending");
    assert_eq!(decided.state, ApprovalState::Approved);
}

// ---------------------------------------------------------------------------
// The race
// ---------------------------------------------------------------------------

/// The mutation commits first: the approve is `APPROVAL_NOT_PENDING`, **not** a
/// storage fault.
///
/// The two are genuinely in flight — the approve's compare-and-swap is observed
/// blocked on the row lock the mutation holds before the mutation is released —
/// so this is the interleaving `inst-ap-pin` is about and not a sequence dressed
/// up as one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Docker (testcontainers)"]
async fn a_mutation_and_an_approve_in_flight_leave_the_unit_voided_and_the_approve_refused() {
    let pg = Pg::applied().await;
    seed(&pg).await;
    let id = Uuid::from_u128(0xa2);
    submit(&pg, id).await;

    let observer = pg.raw().await;
    let written = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    // The mutation. `create_draft_on` is the runner-taking form of the price
    // create, which is what lets the transaction be held open here — the
    // method opens its own and could not be parked mid-flight. It is the same
    // code path: `record_price_mutation`, and therefore the same void.
    let mutation = {
        let db = pg.db().await;
        let (written, release) = (Arc::clone(&written), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        price_repo::create_draft_on(
                            txn,
                            &scope(),
                            TENANT,
                            new_price("us", 0xb_0002),
                        )
                        .await?;
                        // The void has run and is uncommitted; the pending
                        // record's row lock is held.
                        written.notify_one();
                        release.notified().await;
                        Ok(())
                    })
                })
                .await;
            out
        })
    };

    written.notified().await;

    let approval = {
        let db = pg.db().await;
        tokio::spawn(async move {
            ApprovalService::new(DBProvider::<DbError>::new(db))
                .decide(&scope(), TENANT, approve(id))
                .await
        })
    };

    // The approve's read has happened and its UPDATE is waiting on the lock. Only
    // now is the mutation allowed to commit.
    pg_support::wait_until_a_backend_blocks(&observer).await;
    release.notify_one();

    tokio::time::timeout(RACE_TIMEOUT, mutation)
        .await
        .expect("the mutation must finish once released")
        .expect("its task must not panic")
        .expect("the mutation itself is uncontended and must commit");

    let refusal = tokio::time::timeout(RACE_TIMEOUT, approval)
        .await
        .expect("the approve must be released by the mutation's commit")
        .expect("its task must not panic")
        .expect_err("the unit was voided under it");

    assert!(
        matches!(refusal, DomainError::ApprovalNotPending(_)),
        "the loser must be told to re-read, not that the store failed: {refusal:?}"
    );
    assert_eq!(state_of(&pg, id).await, ApprovalState::Voided);
}

/// The approve commits first: the later mutation still commits, and the decided
/// unit is untouched.
///
/// The other ordering, and it is not the mirror image. `inst-as-immutable` makes
/// a decided record untouchable, so a void that reached one would be refused by
/// `trg_pricing_approval_append_only` — inside the mutation's own transaction,
/// which would roll the mutation back. Every authoring edit to a plan that had
/// ever been approved would fail. The `state = 'submitted'` predicate on the
/// void is what stops that, and this is the test that catches its removal on the
/// backend where the trigger is the PL/pgSQL one.
///
/// The residue — an already-granted approval is not re-invalidated — is the
/// design set's own boundary; see the module doc.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Docker (testcontainers)"]
async fn a_mutation_after_a_committed_approve_commits_and_leaves_the_decision_alone() {
    let pg = Pg::applied().await;
    seed(&pg).await;
    let id = Uuid::from_u128(0xa3);
    submit(&pg, id).await;

    let approvals = ApprovalService::new(DBProvider::<DbError>::new(pg.db().await));
    approvals
        .decide(&scope(), TENANT, approve(id))
        .await
        .expect("the approve commits first");

    let prices = PriceRepo::new(DBProvider::<DbError>::new(pg.db().await));
    prices
        .create_draft(&scope(), TENANT, new_price("us", 0xb_0003))
        .await
        .expect("the mutation must still be possible on an approved plan");

    assert_eq!(state_of(&pg, id).await, ApprovalState::Approved);
}

// ---------------------------------------------------------------------------
// The contention G6 introduced: two units of ONE plan on one audit segment
// ---------------------------------------------------------------------------

/// One approval unit over the seeded plan's revision 0, ready to be opened
/// through a runner the test holds.
///
/// The pin is a literal rather than a re-derivation: nothing below judges
/// content, and a digest computed here would be a second implementation of
/// `content_hash` in a test about sequence numbers.
///
/// **It holds no key, and that is what keeps the next test about the audit
/// segment.** `inst-co-single-pending`'s register enforces one holder per key
/// through `uq_pricing_approval_key_pending`, and the register insert runs
/// *before* the trail append — so two racers holding one key would contend on the
/// register and never reach the segment at all, which is a different property with
/// a different code. That property has its own race,
/// [`two_submits_holding_one_key_contend_on_the_register_and_one_is_refused`], and
/// this one keeps the segment's.
fn new_unit(approval_id: Uuid) -> bss_pricing::infra::storage::repo::approval_repo::NewApproval {
    new_unit_holding(approval_id, BTreeSet::new())
}

/// The same unit, holding `held_keys` — the register's half.
fn new_unit_holding(
    approval_id: Uuid,
    held_keys: BTreeSet<String>,
) -> bss_pricing::infra::storage::repo::approval_repo::NewApproval {
    bss_pricing::infra::storage::repo::approval_repo::NewApproval {
        approval_id,
        tenant_id: TENANT,
        subject_ref: bss_pricing::infra::storage::repo::audit_repo::plan_revision_ref(plan_id(), 0),
        subject_kind: bss_pricing::domain::audit::AuditSubjectKind::PlanRevision,
        content_hash: vec![0x11_u8; 32],
        materiality: json!({ "reason": "noConfiguredThreshold" }),
        held_keys,
    }
}

/// **Two submits over one plan contend on the audit segment, and exactly one
/// wins.**
///
/// The contention this suite did not cover. Its two existing races put a
/// *mutation* against a *decision* on **one** unit, where the contended object
/// is the pending record's row lock. Since the approval plane took audit tokens,
/// `submit` / `approve` / `reject` / `withdraw` each occupy a slot on the
/// **plan's** segment (D-135) — so two acts on two *different* units of the same
/// plan now contend on `(tenant_id, chain_id, seq)`, which they did not before,
/// and no row lock is involved at all.
///
/// It is also the residual race `infra::approval::submit`'s doc claims is
/// "decided by the primary key rather than left open": the single-pending check
/// runs inside the transaction, but under READ COMMITTED two submits can both
/// read before either writes, and neither the approval table's primary key
/// (two distinct `approval_id`s) nor any index on it separates them. The audit
/// chain's head does.
///
/// Driven through `approval_repo::open`, which is the runner-taking form and
/// therefore the only one that can be parked mid-transaction — the same code
/// path `ApprovalService::submit` runs, one layer down. The choreography is this
/// suite's: the first `open` writes and parks, the second is observed **blocked**
/// in `pg_locks` before the first is released, so the two really are in flight.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Docker (testcontainers)"]
async fn two_submits_over_one_plan_contend_on_the_audit_segment_and_one_is_told_to_retry() {
    let pg = Pg::applied().await;
    seed(&pg).await;

    let observer = pg.raw().await;
    let written = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let first = {
        let db = pg.db().await;
        let (written, release) = (Arc::clone(&written), Arc::clone(&release));
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        bss_pricing::infra::storage::repo::approval_repo::open(
                            txn,
                            &scope(),
                            new_unit(Uuid::from_u128(0xa5)),
                            stamp_of(SUBMITTER, at(12)),
                        )
                        .await?;
                        // The row and its `submit` record are written and
                        // uncommitted: the segment's next seq is taken.
                        written.notify_one();
                        release.notified().await;
                        Ok(())
                    })
                })
                .await;
            out
        })
    };

    written.notified().await;

    let second = {
        let db = pg.db().await;
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        bss_pricing::infra::storage::repo::approval_repo::open(
                            txn,
                            &scope(),
                            new_unit(Uuid::from_u128(0xa6)),
                            stamp_of(SUBMITTER, at(12)),
                        )
                        .await
                        .map(|_| ())
                    })
                })
                .await;
            out
        })
    };

    // The second `open` has read the segment head — the same head, because the
    // first has not committed — and its INSERT is waiting on the unique index.
    pg_support::wait_until_a_backend_blocks(&observer).await;
    release.notify_one();

    tokio::time::timeout(RACE_TIMEOUT, first)
        .await
        .expect("the first submit must finish once released")
        .expect("its task must not panic")
        .expect("the first submit is uncontended and must commit");

    let loser = tokio::time::timeout(RACE_TIMEOUT, second)
        .await
        .expect("the second submit must be released by the first's commit")
        .expect("its task must not panic")
        .expect_err("two units of one plan cannot both take the same chain position");

    // Told to retry, and told *which* aggregate to retry against (D-159). A
    // driver error here would reach an operator as a 500 about a race whose whole
    // remedy is to re-drive the call.
    match &loser {
        toolkit_db::secure::TxError::Domain(RepoError::ConcurrentMutation { aggregate }) => {
            assert!(
                aggregate.contains(&plan_id().to_string()),
                "the 409 must name the segment that contended: {aggregate}"
            );
        }
        other => panic!("the loser must be a contention, not a storage fault: {other:?}"),
    }

    // And the store holds the winner alone: the loser's approval row rolled back
    // with the record it could not append.
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let conn = provider.conn().expect("conn");
    assert!(
        bss_pricing::infra::storage::repo::approval_repo::read(
            &conn,
            &scope(),
            TENANT,
            Uuid::from_u128(0xa5)
        )
        .await
        .expect("read the winner")
        .is_some(),
        "the winner's unit must be there"
    );
    assert!(
        bss_pricing::infra::storage::repo::approval_repo::read(
            &conn,
            &scope(),
            TENANT,
            Uuid::from_u128(0xa6)
        )
        .await
        .expect("read the loser")
        .is_none(),
        "the loser's approval row must have rolled back with its record"
    );
}

// ---------------------------------------------------------------------------
// The register's constraint: `inst-co-single-pending` against two writers
// ---------------------------------------------------------------------------

/// **Two submits holding one key contend on the register, and the loser is
/// refused** — the race the in-transaction check cannot see, and the whole reason
/// the register is a table with an index rather than a comparison.
///
/// # Why this cannot be a `SQLite` suite, and why the check alone is not the rule
///
/// `ApprovalService::submit` reads the register inside the transaction that writes
/// it, and `infra::approval`'s doc used to argue the residual race away: *"the
/// residual race — both reading before either writes, under an isolation level that
/// permits it — is decided by the primary key rather than left open"*. **That
/// premise is false.** `pricing_approval`'s primary key is `approval_id`, which the
/// caller mints — a fresh `Uuid` per request — so two concurrent submits over one
/// plan carry two different primary keys and collide on nothing. Under
/// `READ COMMITTED` both read a free key, both insert, and without
/// `uq_pricing_approval_key_pending` both commit: one canonical scope key held by
/// two approvable always-material units, with the final state decided by whichever
/// commits last.
///
/// `sqlite::memory:` serializes writers, so it can neither confirm nor refute that.
/// This is where the claim is settled.
///
/// # What separates this from the segment race above
///
/// [`two_submits_over_one_plan_contend_on_the_audit_segment_and_one_is_told_to_retry`]
/// uses units holding **no** key, so its loser blocks on `pricing_audit_log`'s
/// unique index and is answered `ConcurrentMutation` — retry, and the retry
/// succeeds. This one's loser is answered **`PENDING_CHANGE_UNIT_EXISTS`**, which
/// is not retriable and must not be: the key is held, and the remedy is to decide
/// or withdraw the unit holding it. Two races, two codes, and the difference is the
/// point — a register that answered `ConcurrentMutation` would send an operator
/// into a retry loop against an invariant.
///
/// The choreography is the file's own: the first `open` parks with its register row
/// written and uncommitted, so the index slot is taken; the second is observed
/// blocked on it before the first is released.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires Docker (testcontainers)"]
async fn two_submits_holding_one_key_contend_on_the_register_and_one_is_refused() {
    let pg = Pg::applied().await;
    seed(&pg).await;
    let key = scope_key("eu").to_string();

    let observer = pg.raw().await;
    let written = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());

    let first = {
        let db = pg.db().await;
        let (written, release) = (Arc::clone(&written), Arc::clone(&release));
        let key = key.clone();
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        bss_pricing::infra::storage::repo::approval_repo::open(
                            txn,
                            &scope(),
                            new_unit_holding(Uuid::from_u128(0xa7), BTreeSet::from([key])),
                            stamp_of(SUBMITTER, at(12)),
                        )
                        .await?;
                        // The register row is written and uncommitted: the partial
                        // index's slot for this key is taken.
                        written.notify_one();
                        release.notified().await;
                        Ok(())
                    })
                })
                .await;
            out
        })
    };

    written.notified().await;

    let second = {
        let db = pg.db().await;
        let key = key.clone();
        // A **different** approval id, which is the whole hazard: the primary key
        // the old argument relied on cannot separate two submits, only two attempts
        // at one submit.
        tokio::spawn(async move {
            let (_db, out) = db
                .in_transaction::<(), RepoError, _>(move |txn| {
                    Box::pin(async move {
                        bss_pricing::infra::storage::repo::approval_repo::open(
                            txn,
                            &scope(),
                            new_unit_holding(Uuid::from_u128(0xa8), BTreeSet::from([key])),
                            stamp_of(SUBMITTER, at(12)),
                        )
                        .await
                        .map(|_| ())
                    })
                })
                .await;
            out
        })
    };

    pg_support::wait_until_a_backend_blocks(&observer).await;
    release.notify_one();

    tokio::time::timeout(RACE_TIMEOUT, first)
        .await
        .expect("the first submit must finish once released")
        .expect("its task must not panic")
        .expect("the first submit is uncontended and must commit");

    let loser = tokio::time::timeout(RACE_TIMEOUT, second)
        .await
        .expect("the second submit must be released by the first's commit")
        .expect("its task must not panic")
        .expect_err("one key cannot be held by two submitted units");

    match &loser {
        toolkit_db::secure::TxError::Domain(RepoError::PendingKeyHeld { key: held }) => {
            assert_eq!(
                *held, key,
                "the refusal must name the key the register refused a second hold on"
            );
        }
        other => panic!("the loser must be the register's refusal, got: {other:?}"),
    }

    // The store holds the winner alone, and the register holds the key once.
    let provider = DBProvider::<DbError>::new(pg.db().await);
    let conn = provider.conn().expect("conn");
    assert_eq!(
        bss_pricing::infra::storage::repo::approval_repo::held_keys_still_pending(
            &conn,
            &scope(),
            TENANT,
            Uuid::from_u128(0xa7)
        )
        .await
        .expect("read the register"),
        vec![key],
        "the winner holds the key"
    );
    assert!(
        bss_pricing::infra::storage::repo::approval_repo::read(
            &conn,
            &scope(),
            TENANT,
            Uuid::from_u128(0xa8)
        )
        .await
        .expect("read the loser")
        .is_none(),
        "and the loser's whole transaction rolled back - unit, register row and trail"
    );
}
