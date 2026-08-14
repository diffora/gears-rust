//! The retirement orchestrator (`inst-rt-cancel`, `inst-rt-event`,
//! `inst-re-governed`, D-109, D-128, D-182).
//!
//! `sqlite_cutover_unit`'s shape, because the two acts are the same shape of
//! thing: an always-material unit that a single principal may compose and may
//! not commit. What differs is what the act *is* — a cutover stages two rows and
//! moves three windows, a retirement stages nothing and, in this system, moves
//! nothing at all.
//!
//! That last clause is the reason this file exists rather than a route test
//! standing in for it. D-182 makes the D-79 presence lane absent, D-131's
//! fail-closed clause then reads every key as occupied, and retirement keeps
//! every scheduled window. The distance between "keeps every window because the
//! rule says so" and "cancels nothing because nobody wired the loop" is not
//! visible from a green suite — it is visible from a preview that says *why*
//! each window was kept, and from a cancellation loop that runs over an empty
//! list rather than not existing.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use std::sync::Arc;

use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::retirement::{KeptReason, WindowDisposition, WindowVerdict};
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::retirement::{RetirementOutcome, RetirementService};
use chrono::{DateTime, TimeZone, Utc};
use rest_support::Harness;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::SecureEntityExt;
use uuid::Uuid;

const SUBMITTER: Uuid = Uuid::from_u128(0x_5c_12);
const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_5c_c1);

/// Inside `common`'s coverage window `[2099-08-04, 2099-09-01)`.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 5, 0, 0, 0).unwrap()
}

fn stamp_of(actor: Uuid) -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        actor_principal_id: actor,
        recorded_at: now(),
        correlation_id: TEST_CORRELATION,
    }
}

/// The plan's current lifecycle token, read straight off the row.
///
/// Local rather than added to `rest_support`: that module is a shared append
/// point between two concurrent strands, and a helper only this file needs is a
/// merge conflict nobody is buying anything with.
async fn plan_state(h: &Harness, plan_id: PlanId) -> String {
    let conn = h.db.conn().expect("conn");
    let rows = bss_pricing::infra::storage::entity::plan::Entity::find()
        .secure()
        .scope_with(&h.scope())
        .filter(
            Condition::all()
                .add(bss_pricing::infra::storage::entity::plan::Column::TenantId.eq(h.tenant))
                .add(bss_pricing::infra::storage::entity::plan::Column::PlanId.eq(plan_id.get())),
        )
        .all(&conn)
        .await
        .expect("read the plan rows");
    let mut states: Vec<String> = rows.into_iter().map(|row| row.lifecycle_state).collect();
    states.sort();
    states.join(",")
}

/// The pending unit over this plan's retirement, if any.
async fn open_retirement_unit(
    h: &Harness,
    plan_id: PlanId,
) -> Option<bss_pricing::infra::storage::repo::approval_repo::ApprovalRecord> {
    let conn = h.db.conn().expect("conn");
    bss_pricing::infra::storage::repo::approval_repo::find_pending_for_subject(
        &conn,
        &h.scope(),
        h.tenant,
        &bss_pricing::infra::approval::retirement_unit_ref(plan_id, 0),
    )
    .await
    .expect("read the approval plane")
}

fn service(h: &Harness) -> RetirementService {
    RetirementService::new(h.db.clone(), Arc::clone(&h.registry) as Arc<_>)
}

/// A published plan with one published price row and its coverage window.
async fn published_plan(h: &Harness) -> PlanId {
    let plan_uuid = Uuid::now_v7();
    let seeded = rest_support::seed_publishable_plan(h, plan_uuid).await;
    h.publish(plan_uuid, seeded.revision).await;
    h.publish_price(plan_uuid, seeded.price_id).await;
    PlanId::new(plan_uuid)
}

async fn retire(h: &Harness, plan_id: PlanId) -> Result<RetirementOutcome, DomainError> {
    service(h)
        .retire(
            &rest_support::security_context(SUBMITTER, h.tenant),
            &h.scope(),
            h.tenant,
            plan_id,
            bss_pricing::api::rest::windows::verdict_json,
            stamp_of(SUBMITTER),
        )
        .await
}

fn pending(outcome: &RetirementOutcome) -> &bss_pricing::infra::retirement::RetirementPending {
    match outcome {
        RetirementOutcome::SubmittedForApproval(pending) => pending,
        RetirementOutcome::Retired(_) => {
            panic!("a retirement must never commit on one principal (D-109)")
        }
    }
}

#[tokio::test]
async fn a_retirement_never_commits_on_one_principal() {
    // D-109: retirement is a **registered** always-material trigger, so no
    // threshold policy can make this arm commit. It is the act D-62 made
    // two-person for a single window, applied to every key of a plan at once,
    // and it is irreversible.
    let h = Harness::new().await;
    let plan_id = published_plan(&h).await;

    let outcome = retire(&h, plan_id).await.expect("the retirement composes");
    let pending = pending(&outcome);

    assert_eq!(pending.preview.plan_id, plan_id);
    assert_eq!(pending.preview.revision, 0);

    // And the plan is still published: the composed arm writes no flip.
    let state = plan_state(&h, plan_id).await;
    assert_eq!(
        state, "published",
        "the unit is open, the plan has not moved"
    );
}

#[tokio::test]
async fn a_retry_finds_the_open_unit_rather_than_opening_a_second() {
    // `retirement_unit_ref` is deterministic precisely so this holds. A second
    // unit over one retirement would put two decisions in a reviewer's queue for
    // one act, and deciding either would leave the other permanently
    // undecidable.
    let h = Harness::new().await;
    let plan_id = published_plan(&h).await;

    let first = retire(&h, plan_id).await.expect("the first");
    let second = retire(&h, plan_id).await.expect("the retry");

    assert_eq!(
        pending(&first).approval.approval_id,
        pending(&second).approval.approval_id,
        "the retry must resolve the unit the first attempt opened"
    );
}

#[tokio::test]
async fn every_scheduled_window_is_kept_and_the_preview_says_why() {
    // D-182 through D-131's fail-closed clause. The seeded plan carries one
    // scheduled coverage window; with no presence lane its key reads occupied,
    // so the window is kept - and the preview labels it `PresenceUnresolved`
    // rather than `InFlightSubscribers`, because "kept because nobody could be
    // asked" is not the same fact for the operator confirming.
    let h = Harness::new().await;
    let plan_id = published_plan(&h).await;

    let preview = service(&h)
        .preview(&h.scope(), h.tenant, plan_id)
        .await
        .expect("the dry-run");

    assert!(
        preview.presence_unresolved,
        "the D-79 lane has no client in this system"
    );
    assert!(
        !preview.windows.is_empty(),
        "the seed must carry a scheduled window for this case to mean anything"
    );
    assert!(
        preview
            .windows
            .iter()
            .all(|w| w.disposition == WindowDisposition::Kept(KeptReason::PresenceUnresolved)),
        "{:?}",
        preview.windows
    );
    assert_eq!(
        preview.windows.iter().filter(|w| w.is_cancelled()).count(),
        0
    );
}

#[tokio::test]
async fn the_dry_run_writes_nothing() {
    // `inst-rt-api`'s first clause, and D-61's reviewability invariant depends on
    // it: the approver reads this screen **before** deciding, so a preview that
    // opened a unit would make reading it an act.
    let h = Harness::new().await;
    let plan_id = published_plan(&h).await;

    service(&h)
        .preview(&h.scope(), h.tenant, plan_id)
        .await
        .expect("the dry-run");

    assert_eq!(plan_state(&h, plan_id).await, "published");
    assert!(
        open_retirement_unit(&h, plan_id).await.is_none(),
        "a dry-run opens no unit"
    );
}

#[tokio::test]
async fn a_plan_that_is_not_published_is_refused_with_its_own_state() {
    // Asked of the state machine, so the refusal names the state rather than
    // saying "something is in the way". An operator told their plan is already
    // retired stops; one told "forbidden" goes looking for a permission.
    let h = Harness::new().await;
    let plan_id = published_plan(&h).await;
    h.retire(plan_id.get(), 0).await;

    let refusal = service(&h)
        .preview(&h.scope(), h.tenant, plan_id)
        .await
        .expect_err("a retired plan does not retire again");
    let DomainError::LifecycleForbidden(detail) = refusal else {
        panic!("expected LifecycleForbidden, got {refusal:?}");
    };
    assert!(detail.contains("retired"), "{detail}");
}

// ---------------------------------------------------------------------------
// `inst-re-cancelflow`'s second clause — the event, which is half of "invoked"
// (this file's own module doc: "cancels nothing because nobody wired the loop").
// ---------------------------------------------------------------------------

/// Every event name enqueued for this tenant after `floor`, in `seq` order.
///
/// By **sequence and not by name**, `sqlite_cutover_unit`'s rule: a name filter
/// has to know what the fixture produces and silently drops the act's own event
/// the day the act starts producing one.
async fn events_since(h: &Harness, floor: i64) -> Vec<String> {
    use bss_pricing::infra::storage::entity::outbox;
    use sea_orm::Order;
    let conn = h.db.conn().expect("conn");
    outbox::Entity::find()
        .secure()
        .scope_with(&toolkit_db::secure::AccessScope::allow_all())
        .filter(
            Condition::all()
                .add(outbox::Column::TenantId.eq(h.tenant))
                .add(outbox::Column::Seq.gt(floor)),
        )
        .order_by(outbox::Column::Seq, Order::Asc)
        .all(&conn)
        .await
        .expect("read the outbox")
        .into_iter()
        .map(|row| row.event_name)
        .collect()
}

async fn outbox_floor(h: &Harness) -> i64 {
    use bss_pricing::infra::storage::entity::outbox;
    use sea_orm::Order;
    let conn = h.db.conn().expect("conn");
    outbox::Entity::find()
        .secure()
        .scope_with(&toolkit_db::secure::AccessScope::allow_all())
        .filter(Condition::all().add(outbox::Column::TenantId.eq(h.tenant)))
        .order_by(outbox::Column::Seq, Order::Desc)
        .one(&conn)
        .await
        .expect("read the outbox")
        .map_or(0, |row| row.seq)
}

/// The window's stored state token.
async fn window_state(h: &Harness, window_id: Uuid) -> String {
    use bss_pricing::infra::storage::entity::price_window;
    let conn = h.db.conn().expect("conn");
    price_window::Entity::find()
        .secure()
        .scope_with(&h.scope())
        .filter(
            Condition::all()
                .add(price_window::Column::TenantId.eq(h.tenant))
                .add(price_window::Column::WindowId.eq(window_id)),
        )
        .one(&conn)
        .await
        .expect("read the window")
        .expect("the seeded window is there")
        .state
}

/// Run the cancellation flow over `verdicts` in one transaction.
async fn cancel(h: &Harness, verdicts: Vec<WindowVerdict>) -> Vec<Uuid> {
    let scope = h.scope();
    let tenant = h.tenant;
    let (_, outcome) =
        h.db.db()
            .in_transaction::<Vec<Uuid>, DomainError, _>(move |txn| {
                Box::pin(async move {
                    bss_pricing::infra::retirement::cancel_windows_in(
                        txn,
                        &scope,
                        tenant,
                        &verdicts,
                        "retirement/fixture",
                        now(),
                        stamp_of(SUBMITTER),
                    )
                    .await
                })
            })
            .await;
    outcome.expect("the cancellation flow")
}

/// **A retirement-cancelled window emits `PriceWindowCancelled`.**
///
/// `inst-re-cancelflow`: Slice 7's cancellation flow is *invoked* — *"each
/// cancellation emits `PriceWindowCancelled` and drives its cache-eviction
/// path"* — and *"marking-invalid without the event is forbidden (consumers
/// would keep warm caches)"*. Until this case, `retire_in` moved the row through
/// `window_repo::transition` and enqueued nothing, under a comment asserting the
/// opposite. Every other window-mutating service in the crate pairs the flip
/// with its own enqueue; retirement was the one that did not.
///
/// **It is driven through the flow rather than through `retire_in`**, and that
/// is the point rather than a shortcut: D-182 keeps the presence map fail-closed,
/// so `retire_in` condemns nothing and the loop never runs — which is exactly
/// why the bug was green. A rule that fires only once a lane nobody has built
/// starts answering has to be armed directly, or the day the D-79 lane lands it
/// fires wrong.
#[tokio::test]
async fn a_retirement_cancelled_window_emits_its_event() {
    let h = Harness::new().await;
    let plan_uuid = Uuid::now_v7();
    let seeded = rest_support::seed_publishable_plan(&h, plan_uuid).await;
    h.publish(plan_uuid, seeded.revision).await;
    h.publish_price(plan_uuid, seeded.price_id).await;
    let window_id = common::coverage_window_id(seeded.price_id);

    let floor = outbox_floor(&h).await;
    let cancelled = cancel(
        &h,
        vec![WindowVerdict {
            window_id,
            price_id: seeded.price_id,
            disposition: WindowDisposition::Cancelled,
        }],
    )
    .await;

    assert_eq!(cancelled, vec![window_id], "the flow answers what it moved");
    assert_eq!(
        window_state(&h, window_id).await,
        "cancelled",
        "the row moved, which was never the missing half"
    );
    assert_eq!(
        events_since(&h, floor).await,
        vec!["PriceWindowCancelled".to_owned()],
        "the cancellation announces itself, or every consumer keeps a warm cache \
         over a window that no longer exists"
    );
}

/// **The control: a kept window is neither moved nor announced.**
///
/// Without it the case above passes against a flow that cancelled and announced
/// every window it was handed, which is the D-51 hazard inverted — a key with
/// in-flight subscribers losing its continuing coverage.
#[tokio::test]
async fn a_kept_window_is_left_alone_and_announces_nothing() {
    let h = Harness::new().await;
    let plan_uuid = Uuid::now_v7();
    let seeded = rest_support::seed_publishable_plan(&h, plan_uuid).await;
    h.publish(plan_uuid, seeded.revision).await;
    h.publish_price(plan_uuid, seeded.price_id).await;
    let window_id = common::coverage_window_id(seeded.price_id);

    let floor = outbox_floor(&h).await;
    let cancelled = cancel(
        &h,
        vec![WindowVerdict {
            window_id,
            price_id: seeded.price_id,
            disposition: WindowDisposition::Kept(KeptReason::InFlightSubscribers),
        }],
    )
    .await;

    assert!(cancelled.is_empty(), "a kept window is not a cancellation");
    assert_eq!(window_state(&h, window_id).await, "scheduled");
    assert!(
        events_since(&h, floor).await.is_empty(),
        "and nothing is announced about it"
    );
}
