//! `GET/PUT /config/approval-threshold-policy`, driven through the real router.
//!
//! # The positive control comes first, and it is the bootstrap
//!
//! D-10 makes a policy `PUT` an always-material act, so *every* case here is one
//! move away from [`the_first_proposal_is_itself_material_and_a_second_principal_makes_it_effective`]
//! — the full round trip: propose, review, approve, and only then is the policy in
//! force. Without that world a service that opened a unit and never made anything
//! effective would satisfy every refusal in the file, which is this program's own
//! standing warning about a rule that refuses everything passing its refusal test.
//!
//! # The `202` is the whole point, and the `GET` is what proves it
//!
//! A test asserting only that the `PUT` answers 202 cannot tell "the proposal is
//! waiting for a reviewer" from "the proposal was applied and the response is
//! mislabelled". So each proposal case reads the policy back: before the approval
//! the effective policy is unchanged, after it the new version is in force. That
//! pairing is the assertion; the status code is a detail of it.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::threshold_policy::APPROVAL_THRESHOLD_POLICY;
use bss_pricing::domain::approval::ApprovalState;
use rest_support::{Harness, approval_rows, audit_rows, body_json, problem_code, with_headers};
use uuid::Uuid;

/// The principal who proposes. Distinct from [`REVIEWER`] because
/// `chk_pricing_approval_distinct_principals` is a real constraint and a suite
/// that used one identity would prove the round trip only for a store that had
/// stopped enforcing it.
const PROPOSER: Uuid = Uuid::from_u128(0x5_d0);

/// The independent `FinanceReviewer` D-10 requires.
const REVIEWER: Uuid = Uuid::from_u128(0xa_d0);

/// An instant that has **arrived** — the date every case asserting a policy is *in
/// force* has to carry.
///
/// It was `2099-03-01T00:00:00Z` until D-188, on the window suites' convention that
/// a fixture instant should be far enough out that no clock reaches it. That
/// convention is right for a window, whose `effectiveFrom` must be in the future
/// when it is authored, and exactly wrong here: a threshold version is not effective
/// **before** its instant, so a suite dated 2099 asserted "the approved version is
/// the tenant's policy" over a version that must not be. Those assertions passed
/// only because the walk compared the instant against nothing.
const EFFECTIVE_FROM: &str = "2020-01-01T00:00:00Z";

/// An instant that has **not** arrived — the same date the file used to give every
/// case, kept for the two that are about a version whose start is still ahead.
const NOT_YET: &str = "2099-03-01T00:00:00Z";

/// A well-formed proposal body over one currency, starting at [`EFFECTIVE_FROM`].
fn proposal(currency: &str, absolute_minor: i64) -> serde_json::Value {
    dated_proposal(EFFECTIVE_FROM, currency, absolute_minor)
}

/// The same body over an authored start.
fn dated_proposal(effective_from: &str, currency: &str, absolute_minor: i64) -> serde_json::Value {
    serde_json::json!({
        "effective_from": effective_from,
        "entries": [{ "currency": currency, "absolute_minor": absolute_minor }]
    })
}

/// Propose `body` as [`PROPOSER`], have [`REVIEWER`] approve it, and hand back the
/// version number the proposal minted.
async fn propose_and_approve(h: &Harness, body: serde_json::Value) -> u64 {
    let opened = body_json(propose_as(h, PROPOSER, body).await).await;
    let approval_id: Uuid = opened["approval"]["approval_id"]
        .as_str()
        .expect("the unit's id")
        .parse()
        .expect("a uuid");
    assert_eq!(approve(h, approval_id).await.status(), StatusCode::OK);
    opened["proposed"]["version"]
        .as_u64()
        .expect("the 202 names the version it minted")
}

/// `PUT` the body as `principal`.
async fn propose_as(
    h: &Harness,
    principal: Uuid,
    body: serde_json::Value,
) -> axum::http::Response<axum::body::Body> {
    h.allowed_as(principal)
        .send(with_headers(
            "PUT",
            APPROVAL_THRESHOLD_POLICY,
            Some(body),
            &[],
        ))
        .await
}

/// `GET` the policy as `principal`, and hand back the parsed body.
async fn read_policy_as(h: &Harness, principal: Uuid) -> serde_json::Value {
    let response = h
        .allowed_as(principal)
        .send(with_headers("GET", APPROVAL_THRESHOLD_POLICY, None, &[]))
        .await;
    assert_eq!(response.status(), StatusCode::OK, "the read always answers");
    body_json(response).await
}

/// Approve `approval_id` as [`REVIEWER`].
async fn approve(h: &Harness, approval_id: Uuid) -> axum::http::Response<axum::body::Body> {
    h.allowed_as(REVIEWER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{approval_id}/approve"),
            None,
            &[],
        ))
        .await
}

// ---------------------------------------------------------------------------
// The positive control.
// ---------------------------------------------------------------------------

/// **The bootstrap round trip**, and the sentence that makes the whole
/// arrangement reachable.
///
/// A tenant with no policy has everything material (`inst-mat-failsafe`), so its
/// *first* `PUT` is itself an always-material act. It answers 202 with a pending
/// unit, the policy stays unset while that unit is open, and an **independent**
/// principal's approval is what puts it in force. Every clause is asserted,
/// because between them they are D-10.
#[tokio::test]
async fn the_first_proposal_is_itself_material_and_a_second_principal_makes_it_effective() {
    let h = Harness::new().await;

    // Unset is a state, answered 200 — not a 404, and not an error.
    let before = read_policy_as(&h, PROPOSER).await;
    assert!(
        before["effective"].is_null(),
        "a tenant that has never had a version approved has no policy: {before}"
    );
    assert!(before["pending_approval"].is_null());

    let response = propose_as(&h, PROPOSER, proposal("EUR", 500)).await;
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "the proposal opens a unit; it does not apply a diff"
    );
    let opened = body_json(response).await;
    assert_eq!(
        opened["proposed"]["version"], 0,
        "the first version is zero"
    );
    assert_eq!(opened["proposed"]["entries"][0]["currency"], "EUR");
    assert_eq!(opened["proposed"]["entries"][0]["absolute_minor"], 500);
    assert_eq!(opened["approval"]["state"], "submitted");
    assert_eq!(
        opened["approval"]["submitter_principal"],
        PROPOSER.to_string()
    );
    // The verdict on the unit is the evaluator's, over the registered trigger this
    // act is — never a value the surface wrote. D-10 is `inst-mat-registered`, not
    // a threshold comparison, so no configured policy can ever make it otherwise.
    assert_eq!(
        opened["approval"]["materiality"]["reason"],
        "alwaysMaterialTrigger"
    );
    let approval_id: Uuid = opened["approval"]["approval_id"]
        .as_str()
        .expect("the unit's id")
        .parse()
        .expect("a uuid");

    // **Still unset**, and this is the assertion the 202 is only a hint about.
    let during = read_policy_as(&h, PROPOSER).await;
    assert!(
        during["effective"].is_null(),
        "a proposal under review is not the tenant's policy: {during}"
    );
    assert_eq!(
        during["pending_approval"]["approval_id"],
        approval_id.to_string(),
        "and the read names the unit an operator is waiting on"
    );

    assert_eq!(approve(&h, approval_id).await.status(), StatusCode::OK);

    let after = read_policy_as(&h, PROPOSER).await;
    assert_eq!(after["effective"]["version"], 0);
    assert_eq!(after["effective"]["entries"][0]["currency"], "EUR");
    assert_eq!(after["effective"]["entries"][0]["absolute_minor"], 500);
    assert_eq!(after["effective"]["effective_from"], EFFECTIVE_FROM);
    assert!(
        after["pending_approval"].is_null(),
        "the proposal is decided, so nothing is waiting: {after}"
    );
}

/// D-61: the reviewer of a policy unit is shown **the content their signature
/// covers**, not a hash.
///
/// The half a 200 on the approve does not prove. `GET /approvals/{id}` renders the
/// pinned content for a plan unit and had to learn a second subject; a policy unit
/// whose detail carried `pinned_content: null` and nothing else would be exactly
/// the hash-blind signature §3's invariant forbids, and it would pass every other
/// test in this file.
#[tokio::test]
async fn a_policy_units_pinned_content_is_readable_by_its_reviewer() {
    let h = Harness::new().await;
    let opened = body_json(propose_as(&h, PROPOSER, proposal("USD", 250)).await).await;
    let approval_id = opened["approval"]["approval_id"]
        .as_str()
        .expect("the unit's id")
        .to_owned();

    let detail = body_json(
        h.allowed_as(REVIEWER)
            .send(with_headers(
                "GET",
                &format!("/bss-pricing/v1/approvals/{approval_id}"),
                None,
                &[],
            ))
            .await,
    )
    .await;

    assert_eq!(detail["approval"]["subject_kind"], "policy");
    assert!(
        detail["pinned_content"].is_null(),
        "a policy unit is not about a plan: {detail}"
    );
    let pinned = &detail["pinned_threshold_policy"];
    assert_eq!(pinned["version"], 0);
    assert_eq!(pinned["effective_from"], EFFECTIVE_FROM);
    assert_eq!(pinned["entries"][0]["currency"], "USD");
    assert_eq!(pinned["entries"][0]["absolute_minor"], 250);
    assert!(
        pinned["entries"][0]["percent_bp"].is_null(),
        "exactly one basis is set"
    );
    assert_eq!(
        detail["content_matches_pin"], true,
        "the re-derivation digests to what was pinned, or the reviewer is being shown a \
         different document from the one they would be signing"
    );
}

/// A **second** version supersedes the first rather than merging with it.
///
/// The property the "whole policy, not a patch" contract rests on: a currency left
/// out of a later version is a currency with **no** threshold, which is material
/// under `inst-mat-percurrency`'s fail-safe — not a currency that keeps its old
/// value. A union-shaped implementation passes the round-trip test above and fails
/// this one.
#[tokio::test]
async fn a_later_approved_version_replaces_the_earlier_one_entirely() {
    let h = Harness::new().await;

    let first = body_json(propose_as(&h, PROPOSER, proposal("EUR", 500)).await).await;
    let first_id: Uuid = first["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    assert_eq!(approve(&h, first_id).await.status(), StatusCode::OK);

    let second = body_json(propose_as(&h, PROPOSER, proposal("USD", 900)).await).await;
    assert_eq!(
        second["proposed"]["version"], 1,
        "versions are minted in order"
    );
    let second_id: Uuid = second["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");

    // Before the second is approved, the **first** is still in force. The store
    // holds both versions' rows by now, so a reader that resolved on "greatest
    // version" rather than "greatest approved version" answers `USD` here.
    let between = read_policy_as(&h, PROPOSER).await;
    assert_eq!(between["effective"]["version"], 0);
    assert_eq!(between["effective"]["entries"][0]["currency"], "EUR");

    assert_eq!(approve(&h, second_id).await.status(), StatusCode::OK);

    let after = read_policy_as(&h, PROPOSER).await;
    assert_eq!(after["effective"]["version"], 1);
    let entries = after["effective"]["entries"]
        .as_array()
        .expect("an entry list");
    assert_eq!(entries.len(), 1, "the later version is the whole policy");
    assert_eq!(entries[0]["currency"], "USD");
}

// ---------------------------------------------------------------------------
// D-188: the third reason the walk skips a version.
// ---------------------------------------------------------------------------

/// **An approved version whose `effective_from` has not arrived is not yet the
/// tenant's policy.**
///
/// The instant was stored, quantum-checked, `max`-derived across the version's rows
/// with disagreement refused as a corrupt row, carried into the domain value and
/// **hashed into the pin the approver signs** — and compared against nothing. So a
/// version approved with a start a lifetime out governed *today's* publishes from
/// the moment it was approved, and a reviewer who signed "these looser bars start in
/// 2099" had authorized them starting immediately.
///
/// It is the one step of this walk whose direction was **fail-open**: between the
/// approval and the authored start the tenant's policy is the design set's *unset*
/// state, so every change should be material, and instead each was thresholded
/// against a bar nobody had reached yet. Skipping restores the direction by
/// construction — no policy is what makes everything material (`inst-mat-failsafe`).
///
/// The read is taken twice on purpose. The comparison is against "now" at read time,
/// so the same rows answer differently before and after the instant; what must not
/// happen is two answers *at one clock*, which is what a walk that consumed
/// something on its first pass would give.
#[tokio::test]
async fn an_approved_version_whose_start_has_not_arrived_is_not_yet_the_policy() {
    let h = Harness::new().await;
    assert_eq!(
        propose_and_approve(&h, dated_proposal(NOT_YET, "EUR", 500)).await,
        0
    );

    let after = read_policy_as(&h, PROPOSER).await;
    assert!(
        after["effective"].is_null(),
        "a version dated {NOT_YET} does not govern today: {after}"
    );
    assert!(
        after["pending_approval"].is_null(),
        "and it is not pending either - it is approved and simply not in force yet: {after}"
    );
    let again = read_policy_as(&h, PROPOSER).await;
    assert_eq!(
        again, after,
        "the walk is idempotent: one clock, one answer"
    );
}

/// A future-dated version leaves the tenant on the version **already in force**,
/// rather than on nothing.
///
/// The half the case above cannot show. Skipping is fail-safe in the direction that
/// matters — an unreached start must not loosen anything — but it must not *tighten*
/// by discarding a policy the tenant already has, which is what a walk that stopped
/// at the greatest approved version and answered `None` for it would do. The walk
/// carries on to the next version down, exactly as it does for a version whose unit
/// was rejected and for one whose rows no longer match its pin.
#[tokio::test]
async fn a_future_dated_version_leaves_the_tenant_on_the_one_already_in_force() {
    let h = Harness::new().await;
    assert_eq!(propose_and_approve(&h, proposal("EUR", 500)).await, 0);
    assert_eq!(
        read_policy_as(&h, PROPOSER).await["effective"]["version"],
        0,
        "the control: a version whose start has passed is in force"
    );

    assert_eq!(
        propose_and_approve(&h, dated_proposal(NOT_YET, "USD", 900)).await,
        1
    );

    let after = read_policy_as(&h, PROPOSER).await;
    assert_eq!(
        after["effective"]["version"], 0,
        "the greatest approved version is not the effective one while its start is ahead: {after}"
    );
    assert_eq!(
        after["effective"]["entries"][0]["currency"], "EUR",
        "and the policy in force is the older version's whole content, not a merge: {after}"
    );
}

// ---------------------------------------------------------------------------
// The refusals, each one move from the control.
// ---------------------------------------------------------------------------

/// `inst-co-single-pending` on the policy plane: one open proposal per tenant.
///
/// It cannot be the exact-`subject_ref` reading the other units use — a policy
/// unit's ref is the version number it proposes, so every proposal names a
/// different subject and an exact read would find nothing. Two open proposals would
/// leave two reviewers approving two versions whose order of arrival then decides
/// the tenant's thresholds, which is the race the rule exists to close.
#[tokio::test]
async fn a_second_proposal_while_one_is_under_review_is_refused() {
    let h = Harness::new().await;
    assert_eq!(
        propose_as(&h, PROPOSER, proposal("EUR", 500))
            .await
            .status(),
        StatusCode::ACCEPTED
    );

    let response = propose_as(&h, PROPOSER, proposal("USD", 900)).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(response).await, "PENDING_CHANGE_UNIT_EXISTS");

    // And nothing was written: the version rows and the unit commit together, so a
    // refused proposal leaves neither behind. Without this the refusal could be a
    // 409 raised after the rows had landed, and the next proposal would mint a
    // version number over a row set nobody proposed.
    assert_eq!(
        approval_rows(&h).await.len(),
        1,
        "the refused proposal opened no second unit"
    );
    let policy = read_policy_as(&h, PROPOSER).await;
    assert_eq!(policy["pending_approval"]["subject_ref"], "0");
}

/// A withdrawn proposal frees the tenant to propose again — the escape hatch
/// `inst-as-void` names, on this plane.
///
/// Paired with the refusal above deliberately: a rule that refused a second
/// proposal *forever* would pass that test, and a tenant whose first proposal was a
/// mistake would have no policy and no way to get one.
#[tokio::test]
async fn a_withdrawn_proposal_frees_the_tenant_to_propose_again() {
    let h = Harness::new().await;
    let opened = body_json(propose_as(&h, PROPOSER, proposal("EUR", 500)).await).await;
    let approval_id = opened["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .to_owned();

    let withdrawn = h
        .allowed_as(PROPOSER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{approval_id}/withdraw"),
            Some(serde_json::json!({ "reason": "proposed the wrong currency" })),
            &[],
        ))
        .await;
    assert_eq!(withdrawn.status(), StatusCode::OK);

    let second = propose_as(&h, PROPOSER, proposal("USD", 900)).await;
    assert_eq!(second.status(), StatusCode::ACCEPTED);
    let body = body_json(second).await;
    assert_eq!(
        body["proposed"]["version"], 1,
        "the withdrawn version's number stays consumed, as D-145 keeps a revision's"
    );

    // The withdrawn version never becomes effective, however many versions follow
    // it: `effective_version` walks greatest-first and asks the approval store, and
    // a voided unit is not an approval.
    let approval_id: Uuid = body["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    assert_eq!(approve(&h, approval_id).await.status(), StatusCode::OK);
    let policy = read_policy_as(&h, PROPOSER).await;
    assert_eq!(policy["effective"]["version"], 1);
    assert_eq!(policy["effective"]["entries"][0]["currency"], "USD");
}

/// A **rejected** proposal never takes effect, and the tenant keeps the policy it
/// had.
///
/// The immaterial-twin of the round trip: the same act, decided the other way. A
/// reader that made a version effective on "a unit exists over it" rather than on
/// "a unit *approved* it" passes the control and fails here.
#[tokio::test]
async fn a_rejected_proposal_never_becomes_effective() {
    let h = Harness::new().await;
    let first = body_json(propose_as(&h, PROPOSER, proposal("EUR", 500)).await).await;
    let first_id: Uuid = first["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    assert_eq!(approve(&h, first_id).await.status(), StatusCode::OK);

    let second = body_json(propose_as(&h, PROPOSER, proposal("USD", 900)).await).await;
    let second_id = second["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let rejected = h
        .allowed_as(REVIEWER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{second_id}/reject"),
            Some(serde_json::json!({ "reason": "too loose for this market" })),
            &[],
        ))
        .await;
    assert_eq!(rejected.status(), StatusCode::OK);

    let policy = read_policy_as(&h, PROPOSER).await;
    assert_eq!(
        policy["effective"]["version"], 0,
        "the rejected version's rows are in the store and are not the policy: {policy}"
    );
    assert_eq!(policy["effective"]["entries"][0]["currency"], "EUR");
}

/// The proposer cannot approve their own proposal — `inst-tp-distinct`, on the
/// plane whose whole subject is the two-person rule.
///
/// Worth its own case rather than inherited from the approval suite: this is the
/// one unit type where a single principal completing the loop would let them
/// configure the thresholds that decide whether their *other* changes need a
/// second principal.
#[tokio::test]
async fn the_proposer_cannot_approve_their_own_proposal() {
    let h = Harness::new().await;
    let opened = body_json(propose_as(&h, PROPOSER, proposal("EUR", 500)).await).await;
    let approval_id = opened["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .to_owned();
    let before = audit_rows(&h).await.len();

    let response = h
        .allowed_as(PROPOSER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{approval_id}/approve"),
            None,
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert_eq!(problem_code(response).await, "SELF_APPROVAL_FORBIDDEN");

    // The attempt is recorded (`inst-tp-selfaudit`), and on the **policy** segment.
    let after = audit_rows(&h).await;
    assert_eq!(after.len(), before + 1);
    let denial = after.last().expect("the record");
    assert_eq!(denial.action, "deny");
    assert_eq!(denial.subject_kind, "policy");
    assert_eq!(
        denial.chain_id,
        bss_pricing::infra::storage::repo::audit_repo::policy_chain(),
        "a policy act belongs to the policy aggregate's segment, never to a plan's"
    );

    // And the policy is still unset, which is the state that matters: a refused
    // self-approval that had nonetheless made the version effective would be the
    // whole rule defeated by a 403 with no teeth.
    assert!(read_policy_as(&h, PROPOSER).await["effective"].is_null());
}

/// Every shape rule the surface owns reaches the wire as `THRESHOLD_INVALID`
/// (400), and **nothing is written**.
///
/// One request per rule. The `assert` on the store is what distinguishes a refusal
/// from a rollback that did not happen: the empty and duplicate cases are refused
/// *inside* the proposal's transaction, after `latest_version` has been read, so a
/// missing rollback would consume a version number and leave the next proposal
/// minting over it.
#[tokio::test]
async fn every_shape_rule_is_threshold_invalid_and_writes_nothing() {
    let h = Harness::new().await;
    let cases: Vec<(&str, serde_json::Value)> = vec![
        (
            "a code that is not ISO 4217",
            serde_json::json!({ "effective_from": EFFECTIVE_FROM,
                "entries": [{ "currency": "EURO", "absolute_minor": 1 }] }),
        ),
        // The three shapes the D-185 marker adds. A body carrying both a retirement
        // and a threshold set names two different versions, and the operator could
        // not be told which one a reviewer is about to sign; the two halves of a
        // proposal are each required on their own because an absent one is not an
        // empty one.
        (
            "a retirement carrying entries",
            serde_json::json!({ "retire": true,
                "entries": [{ "currency": "EUR", "absolute_minor": 1 }] }),
        ),
        (
            "a retirement carrying a start, which is not schedulable",
            serde_json::json!({ "retire": true, "effective_from": EFFECTIVE_FROM }),
        ),
        (
            "a proposal with no start",
            serde_json::json!({ "entries": [{ "currency": "EUR", "absolute_minor": 1 }] }),
        ),
        (
            "neither basis",
            serde_json::json!({ "effective_from": EFFECTIVE_FROM,
                "entries": [{ "currency": "EUR" }] }),
        ),
        (
            "both bases",
            serde_json::json!({ "effective_from": EFFECTIVE_FROM,
                "entries": [{ "currency": "EUR", "absolute_minor": 1, "percent_bp": 1 }] }),
        ),
        (
            "a negative absolute threshold",
            serde_json::json!({ "effective_from": EFFECTIVE_FROM,
                "entries": [{ "currency": "EUR", "absolute_minor": -1 }] }),
        ),
        (
            "a zero percent threshold",
            serde_json::json!({ "effective_from": EFFECTIVE_FROM,
                "entries": [{ "currency": "EUR", "percent_bp": 0 }] }),
        ),
        (
            "a percent threshold above 100%",
            serde_json::json!({ "effective_from": EFFECTIVE_FROM,
                "entries": [{ "currency": "EUR", "percent_bp": 10001 }] }),
        ),
        (
            "no entries at all",
            serde_json::json!({ "effective_from": EFFECTIVE_FROM, "entries": [] }),
        ),
        (
            "one currency twice",
            serde_json::json!({ "effective_from": EFFECTIVE_FROM, "entries": [
                { "currency": "EUR", "absolute_minor": 1 },
                { "currency": "EUR", "absolute_minor": 2 }
            ] }),
        ),
    ];

    for (what, body) in cases {
        let response = propose_as(&h, PROPOSER, body).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{what} must be refused"
        );
        assert_eq!(problem_code(response).await, "THRESHOLD_INVALID", "{what}");
    }

    assert!(
        approval_rows(&h).await.is_empty(),
        "a refused proposal opens no unit"
    );
    let policy = read_policy_as(&h, PROPOSER).await;
    assert!(policy["effective"].is_null());
    assert!(policy["pending_approval"].is_null());

    // The version counter was never consumed: the next well-formed proposal is
    // still version zero. This is the rollback assertion, and it is the reason the
    // two version-owned rules are driven through the route rather than only as unit
    // tests — `parse_entries` refuses before the transaction opens, but
    // `ThresholdVersion::new` refuses inside it.
    let accepted = body_json(propose_as(&h, PROPOSER, proposal("EUR", 500)).await).await;
    assert_eq!(accepted["proposed"]["version"], 0);
}

/// An unquantized `effective_from` is refused, and by the same boundary check
/// every other authored instant in this gear passes (D-144).
///
/// Its own code, not `THRESHOLD_INVALID`: the entry set is well formed and the
/// field the operator must move is `effective_from`, so folding it into the entry
/// -set code would name a field they cannot fix.
#[tokio::test]
async fn a_sub_millisecond_effective_from_is_refused_on_its_own_code() {
    let h = Harness::new().await;
    let response = propose_as(
        &h,
        PROPOSER,
        serde_json::json!({
            "effective_from": "2099-03-01T00:00:00.0005Z",
            "entries": [{ "currency": "EUR", "absolute_minor": 500 }]
        }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(response).await, "TIMESTAMP_PRECISION_EXCEEDED");
    assert!(approval_rows(&h).await.is_empty());
}

// ---------------------------------------------------------------------------
// The gate.
// ---------------------------------------------------------------------------

/// Both verbs are gated, and a denial changes nothing.
///
/// `tests/rest_authz.rs` owns the property that each verb asks for its
/// **catalogued** pair; what this adds is the observability twin the census cannot
/// give — that the refused `PUT` left no version and no unit behind.
#[tokio::test]
async fn a_denied_caller_reads_nothing_and_writes_nothing() {
    let h = Harness::new().await;

    let read = h
        .denied()
        .send(with_headers("GET", APPROVAL_THRESHOLD_POLICY, None, &[]))
        .await;
    assert_eq!(read.status(), StatusCode::FORBIDDEN);

    let write = h
        .denied()
        .send(with_headers(
            "PUT",
            APPROVAL_THRESHOLD_POLICY,
            Some(proposal("EUR", 500)),
            &[],
        ))
        .await;
    assert_eq!(write.status(), StatusCode::FORBIDDEN);

    assert!(
        approval_rows(&h).await.is_empty(),
        "the refused write opened no unit"
    );
    assert!(read_policy_as(&h, PROPOSER).await["effective"].is_null());
}

/// A tenant reads **its own** policy and never another's.
///
/// The scope axis, driven rather than asserted off the query builder: the store is
/// keyed `(tenant_id, version, currency)` and every read goes through `SecureORM`,
/// so a cross-tenant leak would present as one tenant's approved thresholds
/// governing another's publishes.
#[tokio::test]
async fn one_tenants_approved_policy_is_not_another_tenants() {
    let h = Harness::new().await;
    let opened = body_json(propose_as(&h, PROPOSER, proposal("EUR", 500)).await).await;
    let approval_id: Uuid = opened["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    assert_eq!(approve(&h, approval_id).await.status(), StatusCode::OK);
    assert_eq!(
        read_policy_as(&h, PROPOSER).await["effective"]["version"],
        0
    );

    let other = h
        .other_tenant()
        .send(with_headers("GET", APPROVAL_THRESHOLD_POLICY, None, &[]))
        .await;
    // The other tenant's caller is authorized for its own tenant only, so it either
    // never reaches the store or reads an empty one. Both are correct and the
    // assertion is the one that matters either way: it does not read **this**
    // tenant's policy.
    if other.status() == StatusCode::OK {
        let body = body_json(other).await;
        assert!(
            body["effective"].is_null(),
            "another tenant's policy is not this one's: {body}"
        );
    } else {
        assert_eq!(other.status(), StatusCode::FORBIDDEN);
    }
}

/// A decided unit's state is what the store holds, not only what the response
/// said.
///
/// The store-side twin of the round trip: `pricing_approval` is append-only with a
/// one-way flip, so a response that reported `approved` while the row stayed
/// `submitted` would leave the policy unreachable and every subsequent proposal
/// refused `PENDING_CHANGE_UNIT_EXISTS`.
#[tokio::test]
async fn the_approved_units_row_holds_the_decision_and_names_both_principals() {
    let h = Harness::new().await;
    let opened = body_json(propose_as(&h, PROPOSER, proposal("EUR", 500)).await).await;
    let approval_id: Uuid = opened["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    assert_eq!(approve(&h, approval_id).await.status(), StatusCode::OK);

    let rows = approval_rows(&h).await;
    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.subject_kind, "policy");
    assert_eq!(row.subject_ref, "0", "the ref is the version number");
    assert_eq!(row.state, ApprovalState::Approved.as_str());
    assert_eq!(row.submitter_principal, PROPOSER);
    assert_eq!(row.approver_principal, Some(REVIEWER));
}

/// **A version widened after it was approved stops being effective.**
///
/// The one mutation of an approved version the store still permits, and the
/// reason `effective_version` matches on the **pin** rather than on the version
/// number. `pricing_approval_threshold` refuses every `UPDATE` and every `DELETE`,
/// but its primary key is `(tenant_id, version, currency)` — so an `INSERT` of a
/// currency the version did not have succeeds, and a resolver keyed on "version 0
/// has an approved unit" would hand the tenant a policy an approver never saw.
///
/// The statement only this guard can refuse is exactly that insert. No neighbour
/// shadows it: the unit is still `approved`, the version still exists, every CHECK
/// and both triggers are satisfied, and `a_rejected_proposal_never_becomes_effective`
/// and `a_withdrawn_proposal_frees_the_tenant_to_propose_again` both stay green
/// with the content match removed — which is how this test came to be written, the
/// match having had a removal proof of **zero** until it existed.
///
/// Fail-safe in the direction that matters: the widened version drops out and the
/// tenant falls back to no policy, so everything is material again. A widening
/// cannot loosen anything.
#[tokio::test]
async fn a_version_widened_after_approval_stops_being_the_effective_policy() {
    let h = Harness::new().await;
    let opened = body_json(propose_as(&h, PROPOSER, proposal("EUR", 500)).await).await;
    let approval_id: Uuid = opened["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    assert_eq!(approve(&h, approval_id).await.status(), StatusCode::OK);
    assert_eq!(
        read_policy_as(&h, PROPOSER).await["effective"]["version"],
        0,
        "the control: the approved version is in force"
    );

    // The widening. `open_version` is what would perform it — it appends rows for a
    // `(tenant, version)` and the primary key only refuses a currency the version
    // already has — so the insert is driven through the repository rather than
    // around it: what is being demonstrated is that the *store* has no answer here,
    // and a future caller that re-used a version number is exactly how it happens.
    let conn = h.db.conn().expect("a connection");
    bss_pricing::infra::storage::repo::threshold_repo::open_version(
        &conn,
        &h.scope(),
        h.tenant,
        0,
        chrono::DateTime::parse_from_rfc3339(EFFECTIVE_FROM)
            .expect("a real instant")
            .with_timezone(&chrono::Utc),
        &[bss_pricing::infra::storage::repo::ThresholdEntryRow {
            currency: "USD".to_owned(),
            absolute_minor: Some(1),
            percent_bp: None,
        }],
        rest_support::seed_stamp(),
    )
    .await
    .expect("the store permits an insert of a currency the version did not have");

    let after = read_policy_as(&h, PROPOSER).await;
    assert!(
        after["effective"].is_null(),
        "a version whose rows no longer digest to what its approver signed is not the \
         tenant's policy: {after}"
    );
}

/// **M-4: the sixth unasserted `repo_failure` arm — `approval_repo::open` losing the
/// primary key inside `open_policy_unit`.**
///
/// `RepoError::ConcurrentMutation` → `DomainError::ConcurrentMutation` → **409
/// `CONCURRENT_MUTATION`**, and nothing asserted that this path could produce it. It is
/// reachable rather than theoretical: the pending-unit guard is a read-then-write, and
/// the trail append shares **one constant segment per tenant**
/// (`audit_repo::policy_chain` — a threshold policy has exactly one aggregate), so two
/// proposals racing in one tenant contend on the audit segment's `seq` even when their
/// ids differ. `sqlite::memory:` serializes writers and can stage neither race, so what
/// is asserted here is the arm's **mapping from this call site**, over the collision the
/// service can be handed directly.
///
/// It is service-level for the reason `publish_tests::an_approved_record_naming_one_principal_twice_is_refused_here`
/// is: the route mints `Uuid::now_v7()` per request, so no HTTP caller can collide, and
/// a test that could only drive the route could not reach the arm at all.
#[tokio::test]
async fn a_policy_unit_whose_id_is_taken_is_a_retriable_conflict_and_not_a_storage_failure() {
    let h = Harness::new().await;
    let entry = bss_pricing::domain::materiality::ThresholdEntry {
        currency: bss_pricing::domain::money::CurrencyCode::new("EUR").expect("a valid code"),
        basis: bss_pricing::domain::materiality::ThresholdBasis::Absolute { minor: 500 },
    };
    let effective_from = EFFECTIVE_FROM.parse().expect("an RFC 3339 instant");
    let taken = Uuid::now_v7();
    let materiality = serde_json::json!({ "material": true, "reason": "alwaysMaterialTrigger" });

    // The first proposal takes the id, and is decided so the pending-unit guard is not
    // what answers the second — the refusal under test must be the store's.
    h.governance
        .thresholds
        .propose(
            &h.scope(),
            h.tenant,
            taken,
            effective_from,
            vec![entry.clone()],
            materiality.clone(),
            rest_support::stamp_of(PROPOSER, rest_support::at(9)),
        )
        .await
        .expect("the first proposal opens its unit");
    assert_eq!(approve(&h, taken).await.status(), StatusCode::OK);

    let refused = h
        .governance
        .thresholds
        .propose(
            &h.scope(),
            h.tenant,
            taken,
            effective_from,
            vec![entry],
            materiality,
            rest_support::stamp_of(PROPOSER, rest_support::at(10)),
        )
        .await
        .expect_err("the primary key refuses the second unit under one id");

    // Asserted by equality off the typed error, both halves: the variant, and the wire
    // pair a client branches on. A `DomainError::Internal` here would render 500 and
    // tell an operator to page somebody about a race whose whole remedy is to retry.
    assert!(
        matches!(
            refused,
            bss_pricing::domain::error::DomainError::ConcurrentMutation(_)
        ),
        "a lost primary key is contention, not a storage failure: {refused:?}"
    );
    let canonical = toolkit::api::canonical_prelude::CanonicalError::from(refused);
    assert_eq!(canonical.status_code(), StatusCode::CONFLICT.as_u16());
    match canonical {
        toolkit::api::canonical_prelude::CanonicalError::Aborted { ctx, .. } => assert_eq!(
            ctx.reason, "CONCURRENT_MUTATION",
            "read off the typed context and compared by equality: a `contains` cannot see a code \
             with something appended to it"
        ),
        other => panic!("expected a 409 conflict, got {other:?}"),
    }

    // And the version that could not open its unit did not land either: the proposal's
    // two halves are one transaction, so a policy nobody is reviewing cannot exist.
    let after = read_policy_as(&h, PROPOSER).await;
    assert_eq!(
        after["effective"]["version"], 0,
        "the approved version is still the only one: {after}"
    );
}

// ---------------------------------------------------------------------------
// D-185: the way back to the fail-safe.
// ---------------------------------------------------------------------------

/// Propose the tombstone as `principal`, through the **`PUT`** — the same door every
/// other version is authored by.
///
/// A tombstone is an appended version whose content is "no thresholds", not a
/// deletion: the store is append-only and nothing is removed. So it is authored the
/// way versions are authored, with a positive marker in the body. A `DELETE` would
/// have said the opposite of what happens, answered 202 with a pending unit, and
/// added a verb §5 does not declare for this path.
async fn retire_as(h: &Harness, principal: Uuid) -> axum::http::Response<axum::body::Body> {
    h.allowed_as(principal)
        .send(with_headers(
            "PUT",
            APPROVAL_THRESHOLD_POLICY,
            Some(serde_json::json!({ "retire": true })),
            &[],
        ))
        .await
}

/// **D-185 — a tenant can go back to "unset", and it takes two people to do it.**
///
/// §6 makes *unset ⇒ the two-person rule always* the G1 fail-safe and every tenant
/// starts there. Until this decision it was also a state no tenant could **return**
/// to: the policy is per-currency rows keyed `(tenant, version, currency)`, so "no
/// thresholds" would have to be a version with zero rows — which the authoring door
/// refuses (`THRESHOLD_INVALID`) and which, if admitted, `latest_version` could not
/// see and no pin could cover. The only way back was a version of absurdly high
/// bars, which is not the same rule, reads nothing like it to an auditor, and stops
/// being true the day a currency is added.
///
/// So the way back is a **tombstone**: a version that positively says *this tenant
/// has no thresholds*, minted with the next number, pinned, and approved by an
/// independent principal like every other policy diff (D-10). Every clause of that
/// sentence is asserted here, because between them they are the decision:
///
/// * the retirement answers **202** with a pending unit, not 200 with a diff applied;
/// * the old policy is **still in force** while that unit is open — a tenant must not
///   be able to revert the two-person rule single-handed;
/// * once approved the tombstone is the **effective** version, and it is `entries: []`
///   rather than `effective: null`, which is what distinguishes an authored *none*
///   from a tenant that never configured anything.
#[tokio::test]
async fn a_tenant_returns_to_the_fail_safe_through_a_tombstone_a_second_principal_approves() {
    let h = Harness::new().await;

    // A configured policy, in force. Without it the tombstone would be indistinguishable
    // from the bootstrap and this case would prove nothing.
    assert_eq!(propose_and_approve(&h, proposal("EUR", 500)).await, 0);
    let configured = read_policy_as(&h, PROPOSER).await;
    assert_eq!(configured["effective"]["entries"][0]["currency"], "EUR");

    let response = retire_as(&h, PROPOSER).await;
    assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "retiring the policy opens a unit; it does not apply a diff"
    );
    let opened = body_json(response).await;
    assert_eq!(
        opened["proposed"]["version"], 1,
        "the tombstone is a version like any other, minted with the next number: {opened}"
    );
    assert_eq!(
        opened["proposed"]["entries"]
            .as_array()
            .map(Vec::len)
            .expect("the proposal renders an entry list"),
        0,
        "and it is the version that says **none**: {opened}"
    );
    assert_eq!(
        opened["approval"]["materiality"]["reason"], "alwaysMaterialTrigger",
        "it rides the same always-material unit every other policy diff rides (D-10)"
    );
    let approval_id: Uuid = opened["approval"]["approval_id"]
        .as_str()
        .expect("the unit's id")
        .parse()
        .expect("a uuid");

    // The clause that makes the whole thing safe: the thresholds are still the
    // tenant's policy while the retirement is under review.
    let during = read_policy_as(&h, PROPOSER).await;
    assert_eq!(
        during["effective"]["version"], 0,
        "a retirement under review is not yet the tenant's policy: {during}"
    );
    assert_eq!(during["effective"]["entries"][0]["currency"], "EUR");

    // The reviewer is shown what they are signing — "no thresholds", not a hash and
    // not a plan.
    let detail = body_json(
        h.allowed_as(REVIEWER)
            .send(with_headers(
                "GET",
                &format!("/bss-pricing/v1/approvals/{approval_id}"),
                None,
                &[],
            ))
            .await,
    )
    .await;
    assert_eq!(detail["approval"]["subject_kind"], "policy");
    assert_eq!(detail["pinned_threshold_policy"]["version"], 1);
    assert_eq!(
        detail["pinned_threshold_policy"]["entries"]
            .as_array()
            .map(Vec::len)
            .expect("the pinned document renders an entry list"),
        0,
        "the approver signs 'no thresholds', distinguishably from any particular set: {detail}"
    );
    assert_eq!(
        detail["content_matches_pin"], true,
        "and the re-derivation digests to what was pinned: {detail}"
    );

    assert_eq!(approve(&h, approval_id).await.status(), StatusCode::OK);

    let after = read_policy_as(&h, PROPOSER).await;
    assert!(
        !after["effective"].is_null(),
        "the tombstone is a version **in force**, not the absence of one - an authored, \
         pinnable, approved 'none' rather than the bootstrap: {after}"
    );
    assert_eq!(after["effective"]["version"], 1);
    assert_eq!(
        after["effective"]["entries"]
            .as_array()
            .map(Vec::len)
            .expect("the effective version renders an entry list"),
        0,
        "and it is empty, which is what makes every change material again: {after}"
    );

    // **The clause the wire cannot show**, and the one the decision turns on:
    // `effective_version` answers `Some` — there *is* a version in force — while
    // `effective_policy` answers `None`, which is what makes `materiality::evaluate`
    // answer `noConfiguredThreshold`. The two functions disagree on purpose, and a
    // reader that took the `GET`'s non-null `effective` as "this tenant has
    // thresholds" would have the fail-safe switched off by a rendering.
    let conn = h.db.conn().expect("a scoped connection");
    let policy = bss_pricing::infra::threshold::effective_policy(&conn, &h.scope(), h.tenant)
        .await
        .expect("the effective policy reads");
    assert!(
        policy.is_none(),
        "an effective tombstone configures nothing, so every change is material again: \
         {policy:?}"
    );
    let version = bss_pricing::infra::threshold::effective_version(&conn, &h.scope(), h.tenant)
        .await
        .expect("the effective version reads")
        .expect("the tombstone is in force");
    assert_eq!(version.version(), 1);
    assert!(
        version.is_tombstone(),
        "and the version in force is the one that says none: {version:?}"
    );
}

/// **The second run — every direction of the round trip, because a one-way tombstone
/// is a trap door.**
///
/// The decision is only worth having if the state it reaches is an ordinary one. Three
/// transitions have to be defined and none of them is implied by the others:
///
/// * a **tombstone superseded by a real policy** — a tenant that retired its
///   thresholds can configure them again, or the fail-safe is a state with no exit and
///   the gear has traded one trap for another;
/// * a **real policy superseded by a tombstone**, which the case above asserts;
/// * a **tombstone proposed twice** — the second is a version like any other, not a
///   no-op and not a refusal, because the store is append-only history and a tenant
///   re-asserting "still no thresholds" has authored a fact an auditor can read.
///
/// The version numbers are asserted at every step, because they are the shared
/// sequence: a tombstone that did not consume a number would let the next proposal
/// mint the one it holds, leaving a single version carrying both an authored
/// retirement and an authored entry set — a version neither approver signed.
#[tokio::test]
async fn a_tombstone_is_superseded_by_a_real_policy_and_can_be_authored_again_after_it() {
    let h = Harness::new().await;

    assert_eq!(propose_and_approve(&h, proposal("EUR", 500)).await, 0);

    // Retire it: version 1.
    let retired = body_json(retire_as(&h, PROPOSER).await).await;
    assert_eq!(retired["proposed"]["version"], 1);
    let retired_id: Uuid = retired["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    assert_eq!(approve(&h, retired_id).await.status(), StatusCode::OK);

    // **A real policy after a tombstone**: version 2, and it takes effect. A
    // `latest_version` blind to the tombstone would have minted 1 again here, and the
    // two versions would then share one number.
    let reconfigured = propose_and_approve(&h, proposal("USD", 900)).await;
    assert_eq!(
        reconfigured, 2,
        "the tombstone consumed version 1, so the next proposal is 2"
    );
    let back = read_policy_as(&h, PROPOSER).await;
    assert_eq!(back["effective"]["version"], 2);
    assert_eq!(back["effective"]["entries"][0]["currency"], "USD");
    assert_eq!(
        back["effective"]["entries"][0]["absolute_minor"], 900,
        "a retired policy can be configured again - the fail-safe is a state, not a trap door: \
         {back}"
    );

    // **And a tombstone again**: version 3, distinct from version 1, which is what
    // makes the history readable rather than idempotent.
    let again = body_json(retire_as(&h, PROPOSER).await).await;
    assert_eq!(
        again["proposed"]["version"], 3,
        "a second retirement is a second version, not a no-op: {again}"
    );
    let again_id: Uuid = again["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    assert_eq!(approve(&h, again_id).await.status(), StatusCode::OK);

    let end = read_policy_as(&h, PROPOSER).await;
    assert_eq!(end["effective"]["version"], 3);
    assert_eq!(
        end["effective"]["entries"]
            .as_array()
            .map(Vec::len)
            .expect("an entry list"),
        0
    );

    // The whole sequence is still there: nothing was deleted by a `DELETE`.
    let conn = h.db.conn().expect("a scoped connection");
    let versions = bss_pricing::infra::storage::repo::threshold_repo::versions_desc(
        &conn,
        &h.scope(),
        h.tenant,
    )
    .await
    .expect("the version list reads");
    assert_eq!(
        versions,
        vec![3, 2, 1, 0],
        "one sequence across both tables, greatest first - the walk's whole contract: {versions:?}"
    );
}

/// **The refusal of an empty `PUT` stays, and the tombstone is not a way round it.**
///
/// The two halves are one decision. An empty entry set writes zero rows, so
/// `latest_version` cannot see it, `read_version` cannot tell it from a version nobody
/// proposed, and an approval pin would cover nothing — which is why it is refused at
/// the door where the operator hears about it. A tombstone is the *positive* marker
/// that fixes the capability without relaxing the refusal, and a build that admitted
/// the empty `PUT` "because retirement is expressible now" would have re-opened the
/// hole the tombstone exists to close from the other side.
#[tokio::test]
async fn an_empty_put_is_still_refused_even_though_retirement_is_now_expressible() {
    let h = Harness::new().await;
    assert_eq!(propose_and_approve(&h, proposal("EUR", 500)).await, 0);

    let response = propose_as(
        &h,
        PROPOSER,
        serde_json::json!({ "effective_from": EFFECTIVE_FROM, "entries": [] }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(response).await, "THRESHOLD_INVALID");

    // And it wrote nothing: no version was minted, so the retirement that follows is
    // still version 1.
    let still = read_policy_as(&h, PROPOSER).await;
    assert_eq!(
        still["effective"]["version"], 0,
        "a refused proposal consumes no version number: {still}"
    );
    let retired = body_json(retire_as(&h, PROPOSER).await).await;
    assert_eq!(retired["proposed"]["version"], 1);
}

/// One pending policy unit per tenant, **whichever door opened it**.
///
/// `inst-co-single-pending`'s per-tenant reading is `find_pending_policy_unit`, which
/// is about the unit's subject kind and not about which route opened it. So a
/// retirement while a configuration is under review is refused, and the converse too —
/// and the converse is the one worth asserting, because a reviewer looking at
/// "configure EUR at 500" while a retirement of the whole policy sat beside it would
/// be deciding one of two proposals whose order of arrival then decides the tenant's
/// policy.
#[tokio::test]
async fn a_retirement_and_a_proposal_cannot_both_be_under_review() {
    let h = Harness::new().await;

    // A proposal is open; the retirement is refused.
    let opened = body_json(propose_as(&h, PROPOSER, proposal("EUR", 500)).await).await;
    let opened_id: Uuid = opened["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    let refused = retire_as(&h, PROPOSER).await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(refused).await, "PENDING_CHANGE_UNIT_EXISTS");
    assert_eq!(approve(&h, opened_id).await.status(), StatusCode::OK);

    // A retirement is open; the proposal is refused.
    let retiring = body_json(retire_as(&h, PROPOSER).await).await;
    let retiring_id: Uuid = retiring["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");
    let refused = propose_as(&h, PROPOSER, proposal("USD", 900)).await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(refused).await, "PENDING_CHANGE_UNIT_EXISTS");

    // And the refused proposal left the retirement the only thing under review, on the
    // version it minted — not on one the refused proposal had already consumed.
    let during = read_policy_as(&h, PROPOSER).await;
    assert_eq!(
        during["pending_approval"]["approval_id"],
        retiring_id.to_string()
    );
    assert_eq!(retiring["proposed"]["version"], 1);
}

/// **A retirement under review is not the tenant's policy, and a rejected one never
/// becomes it.**
///
/// The direction that matters most on this route: the whole safety of D-185 is that
/// the tombstone rides an approval, so a rejected retirement must leave the thresholds
/// exactly where they were. A build that wrote the tombstone row and treated the unit
/// as advisory passes every case above and fails this one.
#[tokio::test]
async fn a_rejected_retirement_leaves_the_thresholds_in_force() {
    let h = Harness::new().await;
    assert_eq!(propose_and_approve(&h, proposal("EUR", 500)).await, 0);

    let retiring = body_json(retire_as(&h, PROPOSER).await).await;
    let retiring_id: Uuid = retiring["approval"]["approval_id"]
        .as_str()
        .expect("id")
        .parse()
        .expect("a uuid");

    let rejected = h
        .allowed_as(REVIEWER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{retiring_id}/reject"),
            Some(serde_json::json!({ "reason": "not yet - the audit is next month" })),
            &[],
        ))
        .await;
    assert_eq!(rejected.status(), StatusCode::OK);

    let after = read_policy_as(&h, PROPOSER).await;
    assert_eq!(
        after["effective"]["version"], 0,
        "a rejected retirement is not a retirement: {after}"
    );
    assert_eq!(after["effective"]["entries"][0]["currency"], "EUR");

    // The rejected version's row is still in the store — append-only — and it is
    // still not the policy, which is the pair `effective_version` has to get right:
    // the greatest version is 1 and the greatest **approved** one is 0.
    let conn = h.db.conn().expect("a scoped connection");
    let latest = bss_pricing::infra::storage::repo::threshold_repo::latest_version(
        &conn,
        &h.scope(),
        h.tenant,
    )
    .await
    .expect("the latest version reads");
    assert_eq!(
        latest,
        Some(1),
        "the tombstone row is still there; what it lacks is an approval"
    );
    let states: Vec<String> = approval_rows(&h)
        .await
        .into_iter()
        .map(|record| record.state)
        .collect();
    assert_eq!(states.len(), 2, "both units are on the record: {states:?}");
    assert!(
        states.contains(&ApprovalState::Rejected.as_str().to_owned()),
        "the retirement's unit is rejected: {states:?}"
    );
    assert!(
        !audit_rows(&h).await.is_empty(),
        "and the decision is on the trail"
    );
}
