//! D-05's retirement unwind, held open by probes rather than by prose.
//!
//! `inst-co-retirement-unwind` (Slice 7 step 7) and `inst-rt-cancel` (Slice 11
//! step 2) require that a plan retirement **unwind a live cutover unit in the
//! same transaction**: the predecessor window's `effectiveTo` restored to its
//! recorded pre-cutover value, the scheduled copy and successor windows
//! cancelled, the unit closed as `unwound` — a merely `submitted` unit voided per
//! the Slice 5 pin semantics — and the whole act registered always-material.
//!
//! None of the first four is built. That much was already on record in one place
//! (Slice 11's AC list, *"Slice 7's `inst-co-retirement-unwind` has no
//! implementation"*) and absent from the one list whose job is to name what
//! `retire_in` does not do. What was on record **nowhere** is the part this file
//! exists for: two of D-05's clauses are not merely unbuilt, they are not
//! **constructible** in this crate as it stands, and each is blocked on a
//! different missing thing.
//!
//! * The predecessor's pre-cutover `effectiveTo` is **overwritten in place** by
//!   the act that would have to be undone, and no row, no event and no audit
//!   entry preserves it. `a_cutover_records_the_effective_to_it_overwrote_nowhere`
//!   is that fact, measured over the three stores the act writes.
//! * `unwound` is not a state this crate's approval machine has, and it is not a
//!   state the **design set** declares either: `05-governance.md` §7 lists
//!   `submitted | approved | rejected | voided` and the `state` column's enum
//!   repeats exactly those four. Minting a fifth here is what D-204 clause (2)
//!   refuses — *"a gear may mint an internal variant freely; a wire code is the
//!   set's to declare"* — which
//!   `domain::retirement::strand_free_disposition` already cites from the other
//!   side. `the_approval_machine_has_no_unwound_state` holds that.
//!
//! And one clause **is** paid, under another name:
//! `a_retirement_over_a_live_cutover_is_material_under_the_trigger_it_already_declares`
//! shows that D-05's always-material half needs no
//! `Trigger::RetirementUnwindingACutover` at all, because D-109 made retirement
//! unconditionally material and `MaterialityVerdict` carries no trigger identity
//! for the two to differ in.
//!
//! # Why the whole unwind and not the two clauses that *are* constructible
//!
//! Finding the live unit and cancelling the two scheduled windows can both be
//! written today. Writing only those two produces exactly the state D-05 rejected
//! option (a) to avoid: a predecessor still shortened to the cutover instant with
//! the schedules that were to take over from it gone — the trailing void no gap
//! check can see, arriving as the *result* of the fix. A half unwind is worse
//! than none, so this file records the whole of it as absent and
//! `a_retirement_over_a_live_cutover_unwinds_nothing` is the case that flips when
//! it lands.
//!
//! # Every instant is a fixed date
//!
//! `sqlite_cutover_unit`'s rule and its fixtures: the world sits inside `common`'s
//! coverage window `[2099-08-04, 2099-09-01)`, the cutover is at 2099-08-20, and
//! "now" is a constant. The predecessor's coverage window therefore ends at a
//! **finite** instant, which is the whole point of the first case: after the
//! shorten there is no way to tell that window from one that had been open-ended,
//! and `compose_cutover_windows` accepts both (`WindowInterval::covers` is
//! satisfied by an absent end and by any end strictly after the cutover alike).

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;
mod rest_support;

use std::collections::BTreeSet;
use std::sync::Arc;

use bss_pricing::domain::approval::ApprovalState;
use bss_pricing::domain::error::DomainError;
use bss_pricing::domain::materiality::triggers::Trigger;
use bss_pricing::domain::materiality::{self, ChangeSet, MaterialityReason, MaterialityVerdict};
use bss_pricing::domain::money::MinorAmount;
use bss_pricing::domain::price_record::PriceContent;
use bss_pricing::domain::scope_key::{PlanId, ScopeKey};
use bss_pricing::infra::cutover::{
    CutoverOutcome, CutoverReceipt, CutoverRequest, CutoverService, cutover_unit_ref,
};
use bss_pricing::infra::retirement::{RetirementOutcome, RetirementService};
use bss_pricing::infra::storage::entity::{audit_log, outbox, price_window};
use bss_pricing::infra::storage::repo::approval_repo;
use chrono::{DateTime, TimeZone, Utc};
use rest_support::{Harness, Publishable};
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, SecureEntityExt};
use uuid::Uuid;

const SUBMITTER: Uuid = Uuid::from_u128(0x_5c_13);
const REVIEWER: Uuid = Uuid::from_u128(0x_5c_23);
const TEST_CORRELATION: Uuid = Uuid::from_u128(0x_5c_c2);

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 5, 0, 0, 0).unwrap()
}

/// Inside the predecessor's coverage and clear of both changeover floors.
fn cutover_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 8, 20, 0, 0, 0).unwrap()
}

/// The instant the fixture's coverage window ends at, and the one the cutover
/// overwrites — `common::COVERAGE_TO_UTC` at midnight.
///
/// Named here rather than reached through `common` so the assertion below says
/// what it is looking for; `common`'s own doc makes the same point in reverse
/// about not re-spelling the constant at a call site, and this **is** the call
/// site that has to name the value as a value.
fn pre_cutover_effective_to() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2099, 9, 1, 0, 0, 0).unwrap()
}

fn stamp_of(actor: Uuid) -> bss_pricing::domain::audit::AuditStamp {
    bss_pricing::domain::audit::AuditStamp {
        actor_principal_id: actor,
        recorded_at: now(),
        correlation_id: TEST_CORRELATION,
    }
}

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

async fn cut_over(
    h: &Harness,
    request: CutoverRequest,
    actor: Uuid,
) -> Result<CutoverOutcome, DomainError> {
    CutoverService::new(h.db.clone(), Arc::clone(&h.registry) as Arc<_>)
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

async fn retire(h: &Harness, plan_id: PlanId) -> Result<RetirementOutcome, DomainError> {
    RetirementService::new(h.db.clone(), Arc::clone(&h.registry) as Arc<_>)
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

async fn approve(h: &Harness, approval_id: Uuid) {
    use bss_pricing::domain::approval::{DecisionBy, WithdrawAuthority};
    use bss_pricing::infra::approval::{DecideRequest, RegionGrant};
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
                withdraw_authority: WithdrawAuthority::OwnUnitsOnly,
            },
        )
        .await
        .expect("the reviewer approves the unit");
}

/// Submit, approve, re-post: one committed cutover on the seeded key.
async fn committed_cutover(h: &Harness, key: &ScopeKey) -> CutoverReceipt {
    let opened = cut_over(h, request_of(key, 12_000), SUBMITTER)
        .await
        .expect("the unit opens");
    let approval_id = match &opened {
        CutoverOutcome::SubmittedForApproval(pending) => pending.approval.approval_id,
        CutoverOutcome::Committed(_) => panic!("the first call must open a unit"),
    };
    approve(h, approval_id).await;
    match cut_over(h, request_of(key, 12_000), SUBMITTER)
        .await
        .expect("the approved cutover commits")
    {
        CutoverOutcome::Committed(receipt) => *receipt,
        CutoverOutcome::SubmittedForApproval(_) => panic!("the second call must commit"),
    }
}

/// Submit, approve, re-post: one committed retirement of the plan.
async fn committed_retirement(h: &Harness, plan_id: PlanId) {
    let opened = retire(h, plan_id).await.expect("the retirement unit opens");
    let approval_id = match &opened {
        RetirementOutcome::SubmittedForApproval(pending) => pending.approval.approval_id,
        RetirementOutcome::Retired(_) => {
            panic!("D-109 makes retirement always material; the first call must open a unit")
        }
    };
    approve(h, approval_id).await;
    match retire(h, plan_id)
        .await
        .expect("the approved retirement commits")
    {
        RetirementOutcome::Retired(_) => (),
        RetirementOutcome::SubmittedForApproval(_) => panic!("the second call must commit"),
    }
}

/// Every JSON artefact the tenant carries, keyed by the row's own identity — the
/// outbox payloads by `outbox_id`, the audit records by `(chain_id, seq)`.
///
/// **Identities and not a high-water mark**, because `seq` is not a global
/// sequence in either table: `audit_repo::page`'s doc states it counts *within a
/// segment*, so `seq > floor` over a tenant would silently drop the rows of a
/// chain that happens to be behind. Diffing the identity sets is exact and says
/// nothing about ordering.
///
/// Rendered as text rather than walked field by field, because the claim is about
/// the **value** and not about any field name: an operand that survived under a
/// name nobody guessed would still be an operand, and this search would find it.
async fn json_artefacts(h: &Harness) -> Vec<(String, String)> {
    let conn = h.db.conn().expect("conn");
    let mut rendered = Vec::new();
    for row in outbox::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(outbox::Column::TenantId.eq(h.tenant)))
        .all(&conn)
        .await
        .expect("read the outbox")
    {
        rendered.push((
            format!("outbox:{}", row.outbox_id),
            format!("outbox {}: {}", row.event_name, row.payload),
        ));
    }
    for row in audit_log::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(audit_log::Column::TenantId.eq(h.tenant)))
        .all(&conn)
        .await
        .expect("read the audit log")
    {
        rendered.push((
            format!("audit:{}:{}", row.chain_id, row.seq),
            format!(
                "audit {} {}: before={} after={}",
                row.action,
                row.subject_ref,
                row.before_state.unwrap_or(serde_json::Value::Null),
                row.after_state.unwrap_or(serde_json::Value::Null)
            ),
        ));
    }
    rendered
}

/// The artefacts present after the act and absent before it.
fn written_by_the_act(before: &[(String, String)], after: Vec<(String, String)>) -> Vec<String> {
    let seen: BTreeSet<&str> = before.iter().map(|(id, _)| id.as_str()).collect();
    after
        .into_iter()
        .filter(|(id, _)| !seen.contains(id.as_str()))
        .map(|(_, rendered)| rendered)
        .collect()
}

/// Every window row of the tenant, rendered.
async fn window_rows(h: &Harness) -> Vec<price_window::Model> {
    let conn = h.db.conn().expect("conn");
    price_window::Entity::find()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .filter(Condition::all().add(price_window::Column::TenantId.eq(h.tenant)))
        .all(&conn)
        .await
        .expect("read the window plane")
}

// ---------------------------------------------------------------------------
// (1) The operand D-05 calls "its recorded pre-cutover value".
// ---------------------------------------------------------------------------

/// D-05's first clause names a value the act that destroys it never records.
///
/// `commit_cutover` is the only site that shortens a window on this path, and it
/// calls `window_repo::adjust_effective_to`, which is an `UPDATE` of
/// `pricing_price_window.effective_to`. The table has no before-image column;
/// `cutover_in` enqueues three events and none of them is about the shorten;
/// `record_cutover`'s `cutover_state` names three price ids and their lifecycle
/// states and no window and no instant at all. So the previous end is gone the
/// moment the cutover commits.
///
/// **It is not reconstructible from the shape either.** `compose_cutover_windows`
/// picks the window `WindowInterval::covers(cutover)` answers `true` for, which
/// is satisfied by an absent end *and* by any end strictly after the cutover; both
/// are legal predecessors and after the shorten they are the same row. An unwind
/// that guessed `None` would hand the predecessor open-ended coverage it never
/// had — inventing coverage on a plan being retired, which is the one direction
/// nothing here may move in.
///
/// The positive control is in the same assertion set: the **cutover** instant is
/// found in these artefacts, so the search is one that finds instants that are
/// recorded.
#[tokio::test]
async fn a_cutover_records_the_effective_to_it_overwrote_nowhere() {
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    // The fixture's coverage window is finite, and this is the value under test.
    let before = window_rows(&h).await;
    assert!(
        before
            .iter()
            .any(|row| row.effective_to == Some(pre_cutover_effective_to())),
        "the fixture's predecessor window must end at a finite instant, or this case is \
         asserting about a value that was never there: {before:?}"
    );

    let before_the_act = json_artefacts(&h).await;
    let receipt = committed_cutover(&h, &key).await;

    // Positive control on the write itself: the shorten happened.
    let after = window_rows(&h).await;
    let shortened = after
        .iter()
        .find(|row| row.window_id == receipt.shortened_window_id)
        .expect("the predecessor's window is still there");
    assert_eq!(
        shortened.effective_to,
        Some(cutover_at()),
        "the act moved the predecessor's end to the cutover"
    );
    assert!(
        !after
            .iter()
            .any(|row| row.effective_to == Some(pre_cutover_effective_to())),
        "and no window row carries the old end any more: {after:?}"
    );

    let artefacts = written_by_the_act(&before_the_act, json_artefacts(&h).await);
    assert!(
        !artefacts.is_empty(),
        "the act must have written something, or the search below proves nothing"
    );

    // Positive control on the search: the instant the act *does* record is found
    // by exactly this method.
    let cutover_token = cutover_at().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    assert!(
        artefacts
            .iter()
            .any(|rendered| rendered.contains(&cutover_token)
                || rendered.contains("2099-08-20T00:00:00")),
        "the search finds the instant the act records: {artefacts:?}"
    );

    let lost = "2099-09-01T00:00:00";
    let carrying: Vec<&String> = artefacts
        .iter()
        .filter(|rendered| rendered.contains(lost))
        .collect();
    assert!(
        carrying.is_empty(),
        "D-05 restores the predecessor's `effectiveTo` to *its recorded pre-cutover value*, and \
         this is the act that would have to have recorded it. If something here now carries \
         {lost}, the operand exists and this case is the one to delete - until then the first \
         clause of the unwind has nothing to read: {carrying:?}"
    );
}

// ---------------------------------------------------------------------------
// (2) The state D-05 closes the unit into.
// ---------------------------------------------------------------------------

/// D-05's fourth clause names an approval state neither this crate nor the design
/// set has.
///
/// `05-governance.md` §7 states the machine's states as
/// *"submitted, approved, rejected, voided"* and its `state` column's type as
/// `submitted | approved | rejected | voided`; `ApprovalState::as_str`'s doc
/// records that those same four literals are exactly what
/// `chk_pricing_approval_state` admits. `unwound` occurs in the design set only
/// inside D-05's own sentence and Slice 7's step 7 — in prose, never in a state
/// list or a column type.
///
/// So the unwind's closing move cannot be built here without minting a fifth
/// persisted **and wire** token, which is what D-204 clause (2) refuses: *"a gear
/// may mint an internal variant freely; a wire code is the set's to declare."*
/// `domain::retirement::strand_free_disposition` cites the same clause for the
/// same reason one surface over. This is a report, not a repair.
///
/// Armed forwards: add `Unwound` to `ApprovalState::ALL` and the count reddens;
/// give it the token and `from_token` reddens too. The positive control is
/// `voided`, the state D-05's *other* arm — a merely `submitted` unit — is
/// supposed to reach, which does exist and is reachable.
#[test]
fn the_approval_machine_has_no_unwound_state() {
    let tokens: Vec<&str> = ApprovalState::ALL.iter().map(|s| s.as_str()).collect();
    assert_eq!(
        tokens,
        vec!["submitted", "approved", "rejected", "voided"],
        "the four `chk_pricing_approval_state` admits, and the four `05-governance.md` section 7 \
         declares. A fifth here is a wire token the design set has not declared (D-204 (2))"
    );
    assert_eq!(
        ApprovalState::from_token("unwound"),
        None,
        "D-05 closes the unwound cutover unit as `unwound`; nothing in this machine can hold it"
    );
    assert_eq!(
        ApprovalState::from_token("voided"),
        Some(ApprovalState::Voided),
        "positive control: the state D-05's `submitted` arm reaches does exist"
    );
}

// ---------------------------------------------------------------------------
// (3) The act, as the absence it is.
// ---------------------------------------------------------------------------

/// Retiring a plan that carries an approved-not-yet-effective cutover moves
/// nothing the unwind is about.
///
/// Every assertion here is a clause of `inst-co-retirement-unwind` read back off
/// the store after the retirement has committed in its own transaction. They are
/// in one case rather than four because the unwind is one act: a wave that landed
/// the cancellations without the restore would leave the predecessor ending at the
/// cutover with nothing taking over from it — the trailing void D-05 rejected
/// option (a) to avoid — so no subset of these four is a state anything should be
/// green in.
///
/// The retirement itself is the positive control: it commits, on an approved unit,
/// and the plan is `retired` at the end. What did not happen is the unwind, not
/// the retirement.
#[tokio::test]
async fn a_retirement_over_a_live_cutover_unwinds_nothing() {
    let h = Harness::new().await;
    let (plan_id, seeded) = published_plan(&h).await;
    let key = key_of(plan_id, &seeded);

    let receipt = committed_cutover(&h, &key).await;
    let unit_ref = cutover_unit_ref(plan_id, std::slice::from_ref(&key), cutover_at());

    committed_retirement(&h, plan_id).await;

    // (a) The unit. D-05 closes a live one as `unwound`; an approved one is
    //     terminal here and nothing looks at it.
    let conn = h.db.conn().expect("conn");
    let units = approval_repo::list_page(&conn, &h.scope(), h.tenant, &[], None, 100)
        .await
        .expect("read the approval plane");
    let cutover_unit = units
        .iter()
        .find(|record| record.subject_ref == unit_ref)
        .unwrap_or_else(|| panic!("the cutover's unit is on record: {unit_ref}"));
    assert_eq!(
        cutover_unit.state,
        ApprovalState::Approved,
        "the retirement left the cutover's unit exactly as it found it"
    );

    // (b) and (c) The two scheduled windows the unwind cancels, and the
    //     predecessor's end it restores.
    //
    // **The claim is "nothing was cancelled", not a count.** A first draft of this
    // block asserted two scheduled rows and reddened at three: the predecessor's
    // own window is `scheduled` here too, because the fixture schedules it and no
    // activation sweep runs in this suite. That is a fact about the fixture and
    // not about the act — D-05's scenario has the shortened predecessor *active*
    // — and a count is the wrong operand for a claim about cancellation either
    // way.
    let rows = window_rows(&h).await;
    assert!(
        !rows.iter().any(|row| row.state == "cancelled"),
        "the unwind cancels the cutover's copy and successor windows and the retirement cancelled \
         nothing at all: {rows:?}"
    );
    let born_at_the_cutover: Vec<&price_window::Model> = rows
        .iter()
        .filter(|row| row.effective_from == cutover_at())
        .collect();
    assert_eq!(
        born_at_the_cutover.len(),
        2,
        "the successor's and the copy's windows: {rows:?}"
    );
    assert!(
        born_at_the_cutover
            .iter()
            .all(|row| row.state == "scheduled" && row.effective_to.is_none()),
        "both still scheduled and open-ended, exactly as the composition wrote them: \
         {born_at_the_cutover:?}"
    );

    let shortened = rows
        .iter()
        .find(|row| row.window_id == receipt.shortened_window_id)
        .expect("the predecessor's window");
    assert_eq!(
        shortened.effective_to,
        Some(cutover_at()),
        "the predecessor still ends at the cutover. This is the clause that matters: with the two \
         schedules cancelled and this end left where the cutover put it, every in-flight \
         subscriber on the key is uncovered from the cutover instant onward, and no gap check \
         sees a trailing void. It is inert today only because the cancellations did not happen \
         either"
    );

    // The positive control: the retirement did commit.
    let plan_rows = bss_pricing::infra::storage::entity::plan::Entity::find()
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
    assert!(
        plan_rows.iter().any(|row| row.lifecycle_state == "retired"),
        "the act under test happened: {plan_rows:?}"
    );
}

// ---------------------------------------------------------------------------
// (4) What lands with the D-79 lane, on the day it lands.
// ---------------------------------------------------------------------------

/// Ordinary D-51 retirement opens D-05's trailing void by itself, the moment the
/// presence lane resolves — and yesterday's coverage guard does not close it,
/// because it protects a different key.
///
/// D-182 keeps `PresenceMap::fail_closed()` the built system's only case, so
/// today the condemned set is empty and the case above measures a retirement that
/// cancels nothing. This one asks the domain the question the lane will make
/// answerable, over the window plane a committed cutover leaves behind:
///
/// * the predecessor, on the `all_subscriptions` key, ending **at** the cutover;
/// * the successor, on that same key, `[cutover, …)` — a candidate, not yet active;
/// * the grandfathered copy, on the generation's key, `[cutover, …)` — also a
///   candidate.
///
/// With nobody in flight, `dispose_windows` condemns both candidates and
/// `strand_free_disposition` re-keeps only the **copy**: D-04's bound is a
/// question about a grandfathered generation's coverage, and the successor is not
/// on one. So the `all_subscriptions` key is left with a predecessor that stops at
/// the cutover and nothing after it — uncovered from the cutover instant onward,
/// which is exactly the trailing void D-05 exists to prevent and no gap check can
/// see.
///
/// That reframes what the unwind is owed for. Slice 11's AC list files it as
/// *"Slice 7's to build"*, a missing callee; it is also a **latent defect of this
/// surface**, armed and waiting on the same lane D-316's residual waits on. The
/// two arrive together and neither is closed by the lane landing.
///
/// The positive control is the copy's verdict: the guard does fire here, on the
/// key it is for, so a green on the successor's line is not the guard being inert.
#[test]
fn when_the_lane_lands_retirement_cancels_the_cutovers_successor_and_keeps_only_the_copy() {
    use bss_pricing::domain::retirement::{
        GenerationCoverage, KeptReason, PresenceMap, ScheduledWindow, WindowDisposition,
        dispose_windows, strand_free_disposition,
    };
    use bss_pricing::domain::window::{WindowInterval, WindowState};
    use chrono::TimeDelta;

    let successor_price = Uuid::from_u128(0x_5c_a1);
    let copy_price = Uuid::from_u128(0x_5c_a2);
    let successor_window = Uuid::from_u128(0x_5c_b1);
    let copy_window = Uuid::from_u128(0x_5c_b2);

    // Both post-cutover windows are candidates; the shortened predecessor is
    // active and therefore never in this set.
    let scheduled = [
        ScheduledWindow {
            window_id: successor_window,
            price_id: successor_price,
        },
        ScheduledWindow {
            window_id: copy_window,
            price_id: copy_price,
        },
    ];

    // The lane answered: nobody is bound to either row.
    let mut verdicts = dispose_windows(&scheduled, &PresenceMap::resolved(std::iter::empty()));
    assert!(
        verdicts.iter().all(WindowVerdictExt::cancelled),
        "before the guard runs, both are condemned: {verdicts:?}"
    );

    // The generation the copy sits on, with a horizon and a margin, so the bound
    // is computable and the guard can actually decide.
    let generation = GenerationCoverage {
        scope_key: rest_support::publishable_scope_key(
            PlanId::new(Uuid::from_u128(0x_5c_c5)),
            bss_pricing::domain::scope_key::PhaseId::new(Uuid::from_u128(0x_5c_c6)),
            "eu",
        )
        .to_generation(cutover_at())
        .expect("the generation key"),
        windows: vec![(
            copy_window,
            WindowInterval::new(cutover_at(), None, WindowState::Scheduled),
        )],
        grandfather_until: Some(Utc.with_ymd_and_hms(2100, 8, 20, 0, 0, 0).unwrap()),
        margin: Some(TimeDelta::days(31)),
    };
    strand_free_disposition(&mut verdicts, &[generation], now());

    let successor = verdicts
        .iter()
        .find(|v| v.window_id == successor_window)
        .expect("the successor's verdict");
    assert_eq!(
        successor.disposition,
        WindowDisposition::Cancelled,
        "the successor's window is cancelled while the predecessor still ends at the cutover - \
         the `all_subscriptions` key is uncovered from the cutover onward, and this is D-05's \
         trailing void arriving from ordinary D-51 retirement with no unwind involved"
    );

    let copy = verdicts
        .iter()
        .find(|v| v.window_id == copy_window)
        .expect("the copy's verdict");
    assert_eq!(
        copy.disposition,
        WindowDisposition::Kept(KeptReason::GrandfatheredCoverageBound),
        "positive control: the guard does fire, on the generation it is for. It is not inert \
         here - it simply has nothing to say about a key that carries no generation"
    );
}

/// `WindowVerdict::is_cancelled` under a name `all` can take.
trait WindowVerdictExt {
    fn cancelled(&self) -> bool;
}

impl WindowVerdictExt for bss_pricing::domain::retirement::WindowVerdict {
    fn cancelled(&self) -> bool {
        self.is_cancelled()
    }
}

// ---------------------------------------------------------------------------
// (5) The clause that is paid, under another name.
// ---------------------------------------------------------------------------

/// D-05's always-material half needs no trigger of its own, and that is why
/// `Trigger::RetirementUnwindingACutover` having zero producers costs nothing
/// observable.
///
/// D-05 registered retirement-with-a-live-cutover as always material; D-109 then
/// registered retirement **unconditionally**, and `retire_in` declares
/// `Trigger::PlanRetirement` on every path. `MaterialityVerdict` is
/// `Material { reason, tripped } | AutoPublishable` and carries no trigger
/// identity — `Trigger::as_str`'s own doc records that the token reaches no
/// column and no response — so the two declarations are byte-identical verdicts.
///
/// This is the one clause of D-05 that is **done under another name**, and the
/// case says which name. What it does not say is that the trigger is spare: it is
/// the vocabulary entry Slice 5's `inst-mat-registered` enumerates, and D-321
/// clause (3)'s rule is that a correct declaration the design set still owes is
/// not dead code.
#[test]
fn a_retirement_over_a_live_cutover_is_material_under_the_trigger_it_already_declares() {
    let with_cutover = materiality::evaluate(
        &ChangeSet::of_act(Trigger::RetirementUnwindingACutover, std::iter::empty()),
        None,
        None,
    );
    let plain = materiality::evaluate(
        &ChangeSet::of_act(Trigger::PlanRetirement, std::iter::empty()),
        None,
        None,
    );
    assert_eq!(
        with_cutover, plain,
        "the two declarations answer identically, so nothing an operator or an approver can see \
         distinguishes them"
    );
    assert!(
        matches!(
            plain,
            MaterialityVerdict::Material {
                reason: MaterialityReason::AlwaysMaterialTrigger,
                ..
            }
        ),
        "positive control: the verdict `retire_in` actually declares is the material one \
         D-05 asked for: {plain:?}"
    );
}
