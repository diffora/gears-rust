//! The grandfathering cutover **unit** — `inst-gc-compose`'s composer, its approval
//! unit and `inst-gc-commit`'s owner, driven through the service.
//!
//! `sqlite_supersession_unit.rs`'s sibling, and deliberately its shape: the two acts
//! share eleven steps and differ in three, so a reader comparing the files should
//! find the differences and nothing else. The three: **two keys** pended instead of
//! one, a **second staged row** (the grandfathered copy), and a materiality verdict
//! that is **fixed** rather than asked — `inst-mat-registered` registers a
//! grandfathering cutover, so the evaluator is not consulted at all.
//!
//! # Every instant is a fixed date
//!
//! The instant floors compare against a clock, so the fixtures sit inside `common`'s
//! 2099 coverage window and the "now" every case stamps with is a constant.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;
mod rest_support;

use std::sync::Arc;

use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::money::MinorAmount;
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::scope_key::{PlanId, PriceEligibility, ScopeKey};
use bss_pricing::infra::cutover::{CutoverOutcome, CutoverRequest, CutoverService};
use bss_pricing::infra::storage::entity::price;
use bss_pricing::infra::storage::repo::approval_repo;
use chrono::{DateTime, TimeZone, Utc};
use rest_support::{Harness, Publishable};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, SecureEntityExt};
use uuid::Uuid;

const SUBMITTER: Uuid = Uuid::from_u128(0x_5c_11);
const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_5c_c0);

/// Inside `common`'s coverage window `[2099-08-04, 2099-09-01)` and well before the
/// cutover.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 5, 0, 0, 0).unwrap()
}

/// The cutover instant: inside the predecessor's coverage and clear of both floors.
fn cutover_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 20, 0, 0, 0).unwrap()
}

fn stamp_of(actor: Uuid) -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        actor_principal_id: actor,
        recorded_at: now(),
        correlation_id: TEST_CORRELATION,
    }
}

/// A published plan with one published price row on the `eu` key.
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

fn successor_content(amount: i64) -> PriceContent {
    let mut content = rest_support::publishable_row();
    content.row.amount_minor = Some(MinorAmount::new(amount).expect("non-negative"));
    content
}

fn request_of(key: &ScopeKey, amount: i64) -> CutoverRequest {
    CutoverRequest {
        predecessor_key: key.clone(),
        cutover_at: cutover_at(),
        successor: successor_content(amount),
        successor_price_id: Uuid::now_v7(),
        successor_window_id: Uuid::now_v7(),
        copy_price_id: Uuid::now_v7(),
        copy_window_id: Uuid::now_v7(),
        reason_code: "grandfatheringCutover".to_owned(),
    }
}

fn service(h: &Harness) -> CutoverService {
    CutoverService::new(h.db.clone(), Arc::clone(&h.registry) as Arc<_>)
}

async fn cut_over(
    h: &Harness,
    request: CutoverRequest,
    actor: Uuid,
) -> Result<CutoverOutcome, DomainError> {
    service(h)
        .cut_over(
            &rest_support::security_context(actor, h.tenant),
            &h.scope(),
            h.tenant,
            request,
            bss_pricing::api::rest::windows::verdict_json,
            stamp_of(actor),
        )
        .await
}

fn pending(outcome: &CutoverOutcome) -> &bss_pricing::infra::cutover::CutoverPending {
    match outcome {
        CutoverOutcome::SubmittedForApproval(pending) => pending,
        CutoverOutcome::Committed(_) => panic!("this act must have opened a unit"),
    }
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

// ---------------------------------------------------------------------------
// The compose arm: two rows staged, one unit opened, nothing else moved.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_cutover_stages_both_rows_and_opens_one_unit() {
    // `inst-gc-compose` clause (a) with the cutover's own second arrival: the
    // successor draft on the predecessor's **own** key, and the grandfathered copy
    // on the generation this act mints. A supersession stages one row; this stages
    // two, and both are the reviewer's subject — there is nothing to review until
    // they exist.
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let request = request_of(&key, 12_000);
    let staged_successor = request.successor_price_id;
    let staged_copy = request.copy_price_id;
    let outcome = cut_over(&h, request, SUBMITTER)
        .await
        .expect("a cutover opens its unit");
    let pending = pending(&outcome);

    assert_eq!(pending.successor_price_id, staged_successor);
    assert_eq!(pending.copy_price_id, staged_copy);
    assert_eq!(
        state_of(&h, staged_successor).await,
        LifecycleState::Draft.as_str()
    );
    assert_eq!(
        state_of(&h, staged_copy).await,
        LifecycleState::Draft.as_str()
    );
    assert_eq!(
        state_of(&h, seeded.price_id).await,
        LifecycleState::Published.as_str(),
        "the predecessor is still the key's current row"
    );
}

#[tokio::test]
async fn the_unit_holds_the_market_key_and_the_generation() {
    // `inst-co-single-pending` through the orchestrator rather than through
    // `ApprovalService` directly, which is where `sqlite_approval_service` pins it:
    // this case is that the **act** reaches it, with the keys the act really
    // touches.
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let outcome = cut_over(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the unit opens");

    let held = approval_repo::held_keys_of(
        &h.db.conn().expect("conn"),
        &h.scope(),
        h.tenant,
        pending(&outcome).approval.approval_id,
    )
    .await
    .expect("read the register");

    assert_eq!(held.len(), 2, "the market key and the generation: {held:?}");
    assert!(held.iter().any(|held| held == &key.to_string()));
    assert!(
        held.iter()
            .any(|held| held.contains(PriceEligibility::ExistingGrandfathered.as_str())),
        "and the generation this act mints: {held:?}"
    );
}

#[tokio::test]
async fn a_cutover_is_always_material_and_the_evaluator_is_not_asked() {
    // The third difference from a supersession. `inst-mat-registered` registers a
    // grandfathering cutover, so materiality is a **fixed verdict** rather than a
    // threshold question — there is no configured-threshold world in which this act
    // auto-publishes on one principal, and the stored verdict says which trigger
    // made it material rather than naming a delta nobody computed.
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let outcome = cut_over(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the unit opens");

    let materiality = pending(&outcome).approval.materiality.to_string();
    assert!(
        materiality.contains("alwaysMaterialTrigger"),
        "the verdict names the registered trigger: {materiality}"
    );
    // **It does NOT say which trigger, and that is a finding rather than this
    // case's business.** `MaterialityReason::AlwaysMaterialTrigger` is a *unit*
    // variant, so the stored verdict reads `alwaysMaterialTrigger` for a window
    // cancellation, a shortening, a bundle composition, a rev-share re-split and
    // this act alike — while D-104's own entry requires the record to name which
    // act it was, *"an operator reading the approval record should not have to
    // infer that from a trigger called composition"*. Recorded here and carried to
    // the register; repairing it moves a shared enum and every stored verdict, so
    // it is not this clause's to do.
    assert!(
        !materiality.contains("grandfatheringCutover"),
        "if this starts passing, the enum gained its trigger and the register entry is paid: \
         {materiality}"
    );
}

#[tokio::test]
async fn a_dormant_key_is_refused_before_anything_is_staged() {
    // `inst-co-shorten` presupposes coverage to shorten, and the refusal has to
    // arrive before the two drafts exist — a half-composed act leaves two rows on
    // the plan that no unit is about and that `inst-ps-nodelete` will not let
    // anybody remove.
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);
    // Past the fixture's coverage window entirely.
    let mut request = request_of(&key, 12_000);
    request.cutover_at = Utc.with_ymd_and_hms(2099, 12, 1, 0, 0, 0).unwrap();
    let staged_successor = request.successor_price_id;

    let err = cut_over(&h, request, SUBMITTER)
        .await
        .expect_err("a dormant key cannot be cut over");

    assert!(matches!(err, DomainError::CutoverGap(_)), "got: {err:?}");
    let conn = h.db.conn().expect("conn");
    let staged = price::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(price::Column::PriceId.eq(staged_successor)))
        .one(&conn)
        .await
        .expect("read");
    assert!(
        staged.is_none(),
        "nothing is staged for an act that cannot compose"
    );
}

#[tokio::test]
async fn the_same_act_arriving_twice_is_answered_with_the_same_unit() {
    // `inst-gc-api`'s idempotency, through the subject D-28 names. The second call
    // must **find** the unit rather than refuse it, and it must not stage a second
    // pair of rows — the staged drafts are the reviewer's subject, and two of them
    // on one key is what `insert_successor_draft_on` refuses one layer down.
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let first = cut_over(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the first call opens the unit");
    let second = cut_over(&h, request_of(&key, 12_000), SUBMITTER)
        .await
        .expect("the retry finds it");

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
        pending(&second).copy_price_id,
        pending(&first).copy_price_id,
        "the copy likewise: both drafts are the act's, not the call's"
    );
}
