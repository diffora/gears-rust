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

/// An instant far enough out that no suite's clock reaches it — the fixtures are
/// on 2099 by convention, so a proposal's `effective_from` is a fact about the
/// future rather than a value that silently becomes the past.
const EFFECTIVE_FROM: &str = "2099-03-01T00:00:00Z";

/// A well-formed proposal body over one currency.
fn proposal(currency: &str, absolute_minor: i64) -> serde_json::Value {
    serde_json::json!({
        "effective_from": EFFECTIVE_FROM,
        "entries": [{ "currency": currency, "absolute_minor": absolute_minor }]
    })
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
