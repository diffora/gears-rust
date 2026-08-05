//! The supersession **unit** — `inst-su-compose`'s composer and `inst-su-commit`'s
//! owner, driven end to end against a real database.
//!
//! # What this suite owns that `sqlite_supersession.rs` cannot
//!
//! That file's subject is the four writes: two planes, one transaction, one order.
//! It builds its `SupersessionCommit` by hand and never asks who is allowed to build
//! one. What is here is the half above it, and every property of it is about
//! **sequence** rather than about a store:
//!
//! * the successor draft is staged **before** the unit is opened, because the door
//!   that stages it voids every `submitted` unit of the plan — so the reverse order
//!   destroys the unit being composed and answers 202 over it;
//! * the *same act arriving twice* is answered with the same unit
//!   (`inst-su-api`'s idempotency per `(planId, scope key, changeover instant)`), and
//!   that lookup has to happen **before** the staging for the same reason;
//! * every **permanent** refusal is answered before the `CatalogVersion` request
//!   (D-156), so a request no retry can ever complete strands no pending handle;
//! * the two events `inst-su-return` names are enqueued and the **third is not** —
//!   the predecessor's `PriceWindowExpired` belongs to the sweep at the changeover.
//!
//! None of those is visible from a unit test over a composed plan, and none of them
//! is visible from the store: each is a statement about what did **not** happen.
//!
//! # Every instant is a fixed date
//!
//! The changeover floors and `inst-ws-future-start` both compare against a clock, so
//! the fixtures sit inside `common`'s 2099 coverage window and the "now" every case
//! stamps with is a constant. A fixture computed from `Utc::now()` asserts something
//! different every day it runs.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;
mod rest_support;

use std::sync::Arc;

use bss_pricing::domain::approval::DecisionBy;
use bss_pricing::domain::audit::{AuditStamp, AuditSubjectKind};
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::events::CatalogEvent;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::materiality::MaterialityVerdict;
use bss_pricing::domain::money::MinorAmount;
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::scope_key::{Cohort, PlanId, PriceEligibility, ScopeKey};
use bss_pricing::domain::window::WindowState;
use bss_pricing::infra::approval::{DecideRequest, RegionGrant};
use bss_pricing::infra::storage::entity::{audit_log, outbox, price, price_window};
use bss_pricing::infra::storage::repo::approval_repo;
use bss_pricing::infra::supersession::{
    SupersessionOutcome, SupersessionRequest, SupersessionService,
};
use chrono::{DateTime, TimeZone, Utc};
use rest_support::{Harness, Publishable};
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use toolkit_db::secure::{AccessScope, SecureEntityExt};
use uuid::Uuid;

/// The submitter of every supersession here, and the independent principal who
/// approves its unit.
const SUBMITTER: Uuid = Uuid::from_u128(0x_5b_11);
const REVIEWER: Uuid = Uuid::from_u128(0x_5b_22);
const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_5b_c0);

/// The suite's fixed reading of "now": inside `common`'s coverage window
/// `[2099-08-04, 2099-09-01)` and well before the changeover.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 5, 0, 0, 0).unwrap()
}

/// The changeover: inside the predecessor's coverage and clear of **both** of
/// `inst-su-instant`'s floors.
fn changeover() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 20, 0, 0, 0).unwrap()
}

fn stamp_of(actor: Uuid) -> AuditStamp {
    AuditStamp {
        actor_principal_id: actor,
        recorded_at: now(),
        correlation_id: TEST_CORRELATION,
    }
}

/// A published plan with one published price row on the `eu` key, and the coverage
/// window that row's key holds.
///
/// Published for real through the repository doors, for `rest_support`'s stated
/// reason: a suite whose subject is the supersession unit must not depend on the
/// publish route's four preconditions.
async fn published_plan(h: &Harness) -> (PlanId, Publishable) {
    let plan_uuid = Uuid::now_v7();
    let seeded = rest_support::seed_publishable_plan(h, plan_uuid).await;
    h.publish(plan_uuid, seeded.revision).await;
    h.publish_price(plan_uuid, seeded.price_id).await;
    (PlanId::new(plan_uuid), seeded)
}

fn key_of(plan_id: PlanId, seeded: &Publishable) -> ScopeKey {
    rest_support::publishable_scope_key(plan_id, seeded.phase, "eu")
}

/// The successor's content: the seeded row repriced, and nothing structural moved.
fn successor_content(amount: i64) -> PriceContent {
    let mut content = rest_support::publishable_row();
    content.row.amount_minor = Some(MinorAmount::new(amount).expect("non-negative"));
    content
}

fn request_of(key: &ScopeKey, amount: i64) -> SupersessionRequest {
    SupersessionRequest {
        key: key.clone(),
        changeover: changeover(),
        successor: successor_content(amount),
        successor_price_id: Uuid::now_v7(),
        successor_window_id: Uuid::now_v7(),
        reason_code: "repricing".to_owned(),
    }
}

fn service(h: &Harness) -> SupersessionService {
    SupersessionService::new(h.db.clone(), Arc::clone(&h.registry) as Arc<_>)
}

async fn supersede(
    h: &Harness,
    request: SupersessionRequest,
    actor: Uuid,
) -> Result<SupersessionOutcome, DomainError> {
    service(h)
        .supersede(
            &rest_support::security_context(actor, h.tenant),
            &h.scope(),
            h.tenant,
            request,
            bss_pricing::api::rest::windows::verdict_json,
            stamp_of(actor),
        )
        .await
}

/// Approve the unit the first call opened, with an independent principal.
async fn approve(h: &Harness, approval_id: Uuid) {
    h.governance
        .approvals
        .decide(
            &h.scope(),
            h.tenant,
            DecideRequest {
                approval_id,
                decision: DecisionBy::Approve(REVIEWER),
                reason: None,
                approver_regions: RegionGrant::Untransported,
                stamp: stamp_of(REVIEWER),
            },
        )
        .await
        .expect("the reviewer approves the unit");
}

// ---------------------------------------------------------------------------
// Readbacks. Every one reads with `allow_all`, so what is asserted is what
// landed rather than what the caller was allowed to see.
// ---------------------------------------------------------------------------

async fn rows_on_key(h: &Harness, key: &ScopeKey) -> Vec<price::Model> {
    let conn = h.db.conn().expect("conn");
    price::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(price::Column::TenantId.eq(h.tenant)))
        .order_by(price::Column::PriceId, Order::Asc)
        .all(&conn)
        .await
        .expect("read the price rows")
        .into_iter()
        .filter(|row| {
            row.currency == key.currency().as_str() && row.region == key.region().as_str()
        })
        .collect()
}

async fn state_of(h: &Harness, price_id: Uuid) -> String {
    let conn = h.db.conn().expect("conn");
    price::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(price::Column::PriceId.eq(price_id)))
        .one(&conn)
        .await
        .expect("read the row")
        .expect("the row is there")
        .lifecycle_state
}

async fn window_of(h: &Harness, window_id: Uuid) -> Option<price_window::Model> {
    let conn = h.db.conn().expect("conn");
    price_window::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(price_window::Column::WindowId.eq(window_id)))
        .one(&conn)
        .await
        .expect("read the window")
}

/// Every event name the tenant's outbox holds, in `seq` order.
async fn event_names(h: &Harness) -> Vec<String> {
    let conn = h.db.conn().expect("conn");
    outbox::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(outbox::Column::TenantId.eq(h.tenant)))
        .order_by(outbox::Column::Seq, Order::Asc)
        .all(&conn)
        .await
        .expect("read the outbox")
        .into_iter()
        .map(|row| row.event_name)
        .collect()
}

async fn event_payload(h: &Harness, event: CatalogEvent) -> serde_json::Value {
    let conn = h.db.conn().expect("conn");
    outbox::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(
            Condition::all()
                .add(outbox::Column::TenantId.eq(h.tenant))
                .add(outbox::Column::EventName.eq(event.as_str())),
        )
        .one(&conn)
        .await
        .expect("read the outbox")
        .unwrap_or_else(|| panic!("no {} event was enqueued", event.as_str()))
        .payload
}

/// Every audit entry of `subject_kind = price_unit`, in `seq` order.
async fn price_unit_audit(h: &Harness) -> Vec<audit_log::Model> {
    let conn = h.db.conn().expect("conn");
    audit_log::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(
            Condition::all()
                .add(audit_log::Column::TenantId.eq(h.tenant))
                .add(audit_log::Column::SubjectKind.eq(AuditSubjectKind::PriceUnit.as_str())),
        )
        .order_by(audit_log::Column::Seq, Order::Asc)
        .all(&conn)
        .await
        .expect("read the audit log")
}

fn pending(
    outcome: &SupersessionOutcome,
) -> &bss_pricing::infra::supersession::SupersessionPending {
    match outcome {
        SupersessionOutcome::SubmittedForApproval(pending) => pending,
        SupersessionOutcome::Committed(_) => {
            panic!("the act committed where a second principal was required")
        }
    }
}

fn receipt(
    outcome: &SupersessionOutcome,
) -> &bss_pricing::infra::supersession::SupersessionReceipt {
    match outcome {
        SupersessionOutcome::Committed(receipt) => receipt,
        SupersessionOutcome::SubmittedForApproval(_) => {
            panic!("the act opened a unit where it was entitled to commit")
        }
    }
}

// ---------------------------------------------------------------------------
// The controlled arm: what it writes, and everything it must not.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_material_supersession_stages_its_successor_and_opens_a_unit() {
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let request = request_of(&key, 12_000);
    let staged_id = request.successor_price_id;
    let outcome = supersede(&h, request, SUBMITTER)
        .await
        .expect("a material supersession opens its unit");
    let pending = pending(&outcome);

    // The draft `inst-su-compose` clause (a) authors, on the predecessor's own key,
    // carrying the link D-127 requires and the door stamps.
    assert_eq!(pending.successor_price_id, staged_id);
    let rows = rows_on_key(&h, &key).await;
    assert_eq!(rows.len(), 2, "the predecessor and its staged successor");
    let staged = rows
        .iter()
        .find(|row| row.price_id == staged_id)
        .expect("the successor is on the key");
    assert_eq!(staged.lifecycle_state, LifecycleState::Draft.as_str());
    assert_eq!(staged.supersedes_price_id, Some(seeded.price_id));

    // The unit, under the token S5 §6 declares and the subject that names the act.
    assert_eq!(
        pending.approval.subject_kind,
        AuditSubjectKind::PriceUnit,
        "S5 section 6 declares no `supersession` member and this gear mints none"
    );
    assert!(
        pending.approval.subject_ref.contains("/supersession/"),
        "the subject names the act: {}",
        pending.approval.subject_ref
    );
    assert!(
        pending
            .approval
            .subject_ref
            .contains(&changeover().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        "and the changeover, to the millisecond: {}",
        pending.approval.subject_ref
    );
    assert_eq!(
        approval_repo::held_keys_of(
            &h.db.conn().expect("conn"),
            &h.scope(),
            h.tenant,
            pending.approval.approval_id
        )
        .await
        .expect("read the register"),
        vec![key.to_string()],
        "one key, the one being repriced"
    );

    // And **nothing else moved**: the four things a committed unit touches.
    assert_eq!(
        state_of(&h, seeded.price_id).await,
        LifecycleState::Published.as_str(),
        "the predecessor is still the key's current row"
    );
    let coverage = window_of(&h, common::coverage_window_id(seeded.price_id))
        .await
        .expect("the predecessor's window is there");
    assert_eq!(
        coverage.effective_to,
        Some(common::coverage_to()),
        "its end has not moved to the changeover"
    );
    assert!(
        rest_support::pending_version_refs(&h).await.is_empty(),
        "no CatalogVersion was requested for an act that did not commit"
    );
    assert!(
        event_names(&h).await.is_empty(),
        "and nothing was announced"
    );
    assert!(
        price_unit_audit(&h)
            .await
            .iter()
            .all(|entry| entry.action != "publish"),
        "the unit's own record is written by the commit, not by the compose"
    );
}

#[tokio::test]
async fn the_same_act_arriving_twice_is_answered_with_the_same_unit() {
    // `inst-su-api`: idempotent per `(planId, scope key, changeover instant)`. The
    // second call must find the unit rather than refuse it — and the lookup has to
    // run **before** the staging, because the door voids every `submitted` unit of
    // the plan. A staging that ran first would destroy the unit this answer names.
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let first = supersede(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the first call opens the unit");
    // A second request of the same act, with its own freshly minted ids — which is
    // what a retrying client actually sends.
    let second = supersede(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the retry is answered, not refused");

    assert_eq!(
        pending(&second).approval.approval_id,
        pending(&first).approval.approval_id,
        "one act, one unit"
    );
    assert_eq!(
        pending(&second).successor_price_id,
        pending(&first).successor_price_id,
        "and the staged row's id, never the id the retry minted"
    );
    assert_eq!(
        rest_support::approval_rows(&h)
            .await
            .iter()
            .filter(|record| record.subject_ref.contains("/supersession/"))
            .count(),
        1,
        "no second unit was opened"
    );
    assert_eq!(
        approval_repo::find_pending_for_subject(
            &h.db.conn().expect("conn"),
            &h.scope(),
            h.tenant,
            &pending(&first).approval.subject_ref
        )
        .await
        .expect("read the store")
        .map(|record| record.approval_id),
        Some(pending(&first).approval.approval_id),
        "and the unit the first call opened is still submitted rather than voided"
    );
    assert_eq!(
        rows_on_key(&h, &key).await.len(),
        2,
        "and the successor was staged once"
    );
}

#[tokio::test]
async fn a_request_that_disagrees_with_the_staged_successor_is_refused() {
    // The hazard the reuse of a staged draft creates: the content pin covers the
    // plan shape, which carries the **staged** row — so an approved unit matches
    // whatever the retry's body says, and a commit that trusted the body would
    // publish content nobody reviewed. The refusal is the store's own
    // `DUPLICATE_SCOPE_KEY`, with a sentence the store cannot write.
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    supersede(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the unit opens");
    let refused = supersede(&h, request_of(&key, 13_000), SUBMITTER)
        .await
        .expect_err("a different price is a different act on a key already staged");
    let DomainError::DuplicateScopeKey(message) = refused else {
        panic!("got: {refused:?}");
    };
    assert!(
        message.contains("different content"),
        "the sentence has to say what the store's cannot: {message}"
    );
    assert_eq!(
        rows_on_key(&h, &key).await.len(),
        2,
        "and no second draft was authored"
    );
}

// ---------------------------------------------------------------------------
// The commit arm.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_approved_unit_commits_all_four_writes_and_announces_two_events() {
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let opened = supersede(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the unit opens");
    let unit = pending(&opened).approval.approval_id;
    let successor_id = pending(&opened).successor_price_id;
    approve(&h, unit).await;

    let committed = supersede(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the call after an independent approve commits");
    let receipt = receipt(&committed);
    assert_eq!(receipt.successor_price_id, successor_id);
    assert_eq!(receipt.predecessor_price_id, seeded.price_id);
    assert_eq!(receipt.changeover, changeover());

    // The row plane.
    assert_eq!(
        state_of(&h, seeded.price_id).await,
        LifecycleState::Superseded.as_str()
    );
    assert_eq!(
        state_of(&h, successor_id).await,
        LifecycleState::Published.as_str()
    );
    // The window plane, and the property the whole unit exists for: one instant,
    // two writes, no gap.
    let shortened = window_of(&h, receipt.shortened_window_id)
        .await
        .expect("the predecessor's window");
    let scheduled = window_of(&h, receipt.successor_window_id)
        .await
        .expect("the successor's window");
    assert_eq!(shortened.effective_to, Some(changeover()));
    assert_eq!(scheduled.effective_from, changeover());
    assert_eq!(scheduled.effective_to, None, "open-ended by decision");
    assert_eq!(scheduled.state, WindowState::Scheduled.as_str());
    assert_eq!(shortened.effective_to, Some(scheduled.effective_from));

    // The pending ref, carrying the plan subject the projector re-freezes.
    let refs = rest_support::pending_version_refs(&h).await;
    let mine = refs
        .iter()
        .find(|row| row.pending_ref == receipt.pending_version_ref)
        .expect("the handle this commit requested was recorded");
    assert_eq!(mine.subject_ref, plan_id.to_string());
    assert_eq!(
        mine.subject_kind,
        bss_pricing::domain::read_model::SubjectRef::Plan(plan_id.get())
            .kind()
            .as_str(),
        "the plan subject D-99/D-128 project, not a subject of this act's own"
    );
    assert_eq!(
        u64::try_from(mine.subject_revision.expect("a plan subject carries one"))
            .expect("non-negative"),
        receipt.revision
    );

    // `inst-su-return`'s two events, and **only** those two.
    assert_eq!(
        event_names(&h).await,
        vec![
            CatalogEvent::PriceUpdated.as_str().to_owned(),
            CatalogEvent::PriceWindowScheduled.as_str().to_owned(),
        ],
        "the predecessor's PriceWindowExpired is the sweep's at the changeover, not this \
         transaction's"
    );
    let updated = event_payload(&h, CatalogEvent::PriceUpdated).await;
    assert_eq!(updated["priceId"], serde_json::json!(successor_id));
    assert_eq!(
        updated["supersedesPriceId"],
        serde_json::json!(seeded.price_id)
    );
    assert_eq!(updated["scopeKey"], serde_json::json!(key.to_string()));
    assert_eq!(
        updated["pendingVersionRef"],
        serde_json::json!(receipt.pending_version_ref)
    );

    // The record of who did it, at the act's level, naming the unit that authorized
    // it. One entry for the act rather than one per row moved.
    let trail: Vec<(String, Option<Uuid>)> = price_unit_audit(&h)
        .await
        .into_iter()
        .filter(|entry| entry.subject_ref == pending(&opened).approval.subject_ref)
        .map(|entry| (entry.action, entry.approval_ref))
        .collect();
    // The whole trail of one act under **one** subject string: the unit opening, the
    // independent approve, and the commit. That the three share a subject is what makes
    // them joinable at all — the approval record stands under the same string, so
    // `approval_ref` on the last entry closes the loop rather than pointing sideways.
    assert_eq!(
        trail,
        vec![
            ("submit".to_owned(), Some(unit)),
            ("approve".to_owned(), Some(unit)),
            ("publish".to_owned(), Some(unit)),
        ],
        "one act, one subject, three records"
    );
}

#[tokio::test]
async fn an_immaterial_supersession_commits_on_one_principal() {
    // `inst-su-commit`: "materiality = the standard per-currency price-delta
    // evaluation". A configured bar the delta does not reach makes the act
    // auto-publishable, and the whole unit runs on one call. Without this case every
    // other case here would pass against a service that refuses everything.
    let h = Harness::new().await;
    rest_support::approve_threshold_policy(&h, &[("EUR", 1_000_000)]).await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let committed = supersede(&h, request_of(&key, 10_000), SUBMITTER)
        .await
        .expect("a sub-threshold reprice commits directly");
    let receipt = receipt(&committed);
    assert_eq!(
        state_of(&h, seeded.price_id).await,
        LifecycleState::Superseded.as_str()
    );
    assert_eq!(
        state_of(&h, receipt.successor_price_id).await,
        LifecycleState::Published.as_str()
    );
    // No unit at all, and the audit record says so rather than naming one.
    assert!(
        rest_support::approval_rows(&h)
            .await
            .iter()
            .all(|record| !record.subject_ref.contains("/supersession/")),
        "an immaterial act opens no unit"
    );
    let entry = price_unit_audit(&h)
        .await
        .into_iter()
        .find(|entry| entry.subject_ref.contains("/supersession/"))
        .expect("the act is still recorded");
    assert_eq!(
        entry.approval_ref, None,
        "and the record does not name an approval that does not exist"
    );
}

// ---------------------------------------------------------------------------
// The refusals, and the ordering that keeps each of them ahead of the registry.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_commit_floor_is_answered_before_any_catalog_version_is_requested() {
    // `inst-su-instant`'s commit bound is **permanent** for this request: the remedy
    // is a recomposition against a fresh instant, which is a different act with a
    // different subject and therefore a different registry handle. A refusal after
    // the request would leave the first handle pending forever (D-156, and the class
    // `PublishService::commit` hoists `refuse_unpublishable_predecessor` for).
    let h = Harness::new().await;
    rest_support::approve_threshold_policy(&h, &[("EUR", 1_000_000)]).await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    // Inside `MAX_BATCHING_DELAY` of the stamped instant, and strictly future — so
    // the submit floor passes and only the commit floor can refuse it.
    let mut request = request_of(&key, 10_000);
    request.changeover = now() + chrono::Duration::minutes(2);
    let refused = supersede(&h, request, SUBMITTER)
        .await
        .expect_err("two minutes is inside the batching delay");
    assert!(
        matches!(refused, DomainError::SupersessionInstantPassed(_)),
        "got: {refused:?}"
    );
    assert!(
        rest_support::pending_version_refs(&h).await.is_empty(),
        "and no handle was requested for an act that can never complete"
    );
    assert_eq!(
        rows_on_key(&h, &key).await.len(),
        1,
        "and nothing was staged"
    );
    assert_eq!(
        state_of(&h, seeded.price_id).await,
        LifecycleState::Published.as_str()
    );
}

#[tokio::test]
async fn an_existing_grandfathered_generation_cannot_be_superseded() {
    // Foundation §4.3, and the refusal is the door's own — asked here one step
    // earlier for the reason above, through the one function both callers share.
    let h = Harness::new().await;
    rest_support::approve_threshold_policy(&h, &[("EUR", 1_000_000)]).await;
    let (plan_id, seeded) = published_plan(&h).await;
    let base = key_of(plan_id, &seeded);
    let retained = ScopeKey::new(
        plan_id,
        base.currency().clone(),
        base.region().clone(),
        seeded.phase,
        PriceEligibility::ExistingGrandfathered,
        base.charge_kind(),
        Cohort::Generation(Utc.with_ymd_and_hms(2099, 1, 1, 0, 0, 0).unwrap()),
    )
    .expect("existing_grandfathered carries a generation");

    let refused = supersede(&h, request_of(&retained, 10_000), SUBMITTER)
        .await
        .expect_err("a retained generation is immutable in price");
    // The class refusal is step **0** of the orchestrator and precedes even the read,
    // which is what this case pins: the key carries no published row at all, so a
    // `NotFound` here would mean the class question was asked too late to be the
    // permanent refusal D-156 needs ahead of the registry.
    let DomainError::LifecycleForbidden(message) = refused else {
        panic!("got: {refused:?}");
    };
    assert!(message.contains("existing_grandfathered"), "got: {message}");
    assert!(rest_support::pending_version_refs(&h).await.is_empty());
}

#[tokio::test]
async fn a_key_with_no_current_row_is_absent_rather_than_refused() {
    // The row-plane half of `inst-su-compose`'s presupposition of current coverage.
    // `NotFound` on the key rather than a conflict, because the remedy the design
    // names is a plain publish plus a window schedule — a different act.
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let unpriced = rest_support::publishable_scope_key(plan_id, seeded.phase, "us");

    let refused = supersede(&h, request_of(&unpriced, 10_000), SUBMITTER)
        .await
        .expect_err("there is nothing on that key to supersede");
    assert!(
        matches!(refused, DomainError::NotFound { .. }),
        "got: {refused:?}"
    );
}

#[tokio::test]
async fn another_tenants_key_is_not_there_to_supersede() {
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let refused = service(&h)
        .supersede(
            &rest_support::security_context(SUBMITTER, h.other),
            &h.other_scope(),
            h.other,
            request_of(&key, 12_000),
            bss_pricing::api::rest::windows::verdict_json,
            stamp_of(SUBMITTER),
        )
        .await
        .expect_err("another tenant's plan is not visible");
    assert!(
        matches!(refused, DomainError::NotFound { .. }),
        "got: {refused:?}"
    );
    assert_eq!(
        state_of(&h, seeded.price_id).await,
        LifecycleState::Published.as_str(),
        "and nothing of mine moved"
    );
}

#[tokio::test]
async fn a_dormant_key_fails_compose_rather_than_committing_half_a_unit() {
    // `inst-su-compose`: a key with no scheduled or active window covering the
    // changeover presupposes nothing to hand over from. A changeover past the
    // seeded window's end is exactly that.
    let h = Harness::new().await;
    rest_support::approve_threshold_policy(&h, &[("EUR", 1_000_000)]).await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let mut request = request_of(&key, 10_000);
    request.changeover = common::coverage_to() + chrono::Duration::days(30);
    let refused = supersede(&h, request, SUBMITTER)
        .await
        .expect_err("coverage has already ended at that instant");
    let DomainError::LifecycleForbidden(message) = refused else {
        panic!("got: {refused:?}");
    };
    assert!(message.contains("dormant"), "got: {message}");
    assert_eq!(
        rows_on_key(&h, &key).await.len(),
        1,
        "nothing was staged for an act that cannot be composed"
    );
}

/// The verdict a pending unit reports is the evaluator's, carried rather than
/// re-minted — `infra::window::pending_approval`'s lesson one plane over.
#[tokio::test]
async fn the_pending_answer_carries_the_evaluators_own_reason() {
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let outcome = supersede(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the unit opens");
    assert!(matches!(
        pending(&outcome).verdict,
        MaterialityVerdict::Material { .. }
    ));
    assert_eq!(
        pending(&outcome).shortened_window_id,
        common::coverage_window_id(seeded.price_id),
        "and it names the window the composition would shorten"
    );
}

#[tokio::test]
async fn the_units_pin_covers_the_successor_it_stages() {
    // **The order's real ground, and it is not the one first written down.** The
    // successor draft is one of the plan's `draft` rows, so it is inside the shape
    // `submit_supersession_on` digests. A unit opened *before* the staging pins a world
    // without the row under review, and then no reviewer can ever approve it: the
    // re-derivation at decision time carries the draft the pin does not.
    //
    // A probe measured that, and it also measured that the reason recorded first — "the
    // door voids every submitted unit of the plan" — is false for this unit:
    // `void_pending_for_plan` filters `subject_kind = 'plan_revision'` and a supersession
    // unit is a `price_unit`. So this case, not that argument, is what holds the order.
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let outcome = supersede(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the unit opens");
    let detail = h
        .governance
        .approvals
        .find(
            &h.scope(),
            h.tenant,
            pending(&outcome).approval.approval_id,
            now(),
        )
        .await
        .expect("read the unit back")
        .expect("it is there");
    assert!(
        detail.content_matches_pin,
        "a unit whose pin does not cover its own successor can be opened and never approved"
    );
}

#[tokio::test]
async fn the_stored_verdict_names_the_row_that_will_publish() {
    // The one field of the evaluator's input that has to be the **staged** row's and not
    // the id the request minted: `MaterialityVerdict::tripped_row` carries a `price_id`,
    // the verdict is stored on the unit as jsonb, and it is read years later. A retry
    // mints a fresh id, so a verdict built from the request would name a row that never
    // existed.
    //
    // This case exists because a removal proof came back **green**: dropping the
    // staged-id preference changed nothing, since every other case here reaches a verdict
    // with no tripped row at all (`noConfiguredThreshold`, `autoPublishable`). A bar the
    // delta *reaches* is what makes the field observable.
    let h = Harness::new().await;
    rest_support::approve_threshold_policy(&h, &[("EUR", 100)]).await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let first = supersede(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("9900 -> 12000 reaches a bar of 100");
    let staged_id = pending(&first).successor_price_id;
    let MaterialityVerdict::Material {
        tripped: Some(tripped),
        ..
    } = &pending(&first).verdict
    else {
        panic!("got: {:?}", pending(&first).verdict);
    };
    assert_eq!(tripped.price_id, staged_id);

    // And the retry, whose request carries a **different** minted id, still names the
    // row that is going to publish.
    let second = supersede(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the retry is answered");
    let MaterialityVerdict::Material {
        tripped: Some(tripped),
        ..
    } = &pending(&second).verdict
    else {
        panic!("got: {:?}", pending(&second).verdict);
    };
    assert_eq!(
        tripped.price_id, staged_id,
        "never the id this request minted and threw away"
    );
}
