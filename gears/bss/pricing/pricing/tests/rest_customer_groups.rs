//! `GET/PUT /bss-pricing/v1/customer-groups/taxonomy`, driven through the real
//! router (`design/09-price-overlays.md` §3 `inst-cg-taxonomy`, §5;
//! `design/05-governance.md`'s endpoint map).
//!
//! # The claim this file exists to make good
//!
//! A first attempt at this surface was briefed as a fifth arm of
//! `api::rest::taxonomies`' `{class}` route, gated on `config`. That was wrong:
//! the design set gives this taxonomy its own route and its own resource,
//! `customer_group`, specifically so it is **not** reachable by every holder of
//! `config × write` — per-payer commercial data is more sensitive than plan or
//! config authoring. `every_route_asks_the_catalogued_pair` in `rest_authz.rs`
//! proves this route asks the right pair; what no allow/deny fixture there can
//! prove is that the two resources are actually **segregated** — that a caller
//! holding one does not, by that grant alone, satisfy the other. That is
//! `a_caller_holding_config_write_but_not_customer_group_write_is_refused`
//! below, armed with a `SelectiveResolver` that answers per-pair rather than
//! uniformly, and its mirror control proves the isolation runs both ways.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::approvals::APPROVAL_APPROVE;
use bss_pricing::api::rest::customer_groups::{
    CUSTOMER_GROUP_MEMBER, CUSTOMER_GROUP_MEMBER_MOVE, CUSTOMER_GROUP_MEMBERS,
    CUSTOMER_GROUP_TAXONOMY,
};
use bss_pricing::authz::{actions, labels};
use bss_pricing::config::JobsConfig;
use bss_pricing::domain::audit::AuditSubjectKind;
use bss_pricing::domain::ports::CatalogVersionRegistryV1;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::jobs::readmodel_warm::ReadModelWarmJob;
use bss_pricing::infra::storage::repo::audit_repo;
use bss_pricing::infra::storage::repo::group_membership_repo;
use bss_pricing::infra::storage::repo::{NewApproval, approval_repo};
use rest_support::{
    Harness, approval_row, approval_rows, audit_rows, body_json, etag_of, membership_row,
    pending_version_refs, problem_code, request, stamp_of, with_headers,
};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

/// The `CatalogAdmin` who configures the taxonomies.
const ADMIN: uuid::Uuid = uuid::Uuid::from_u128(0xca_d1);

/// Read the taxonomy, answering the body and the tag together — the same
/// one-response discipline `rest_taxonomies.rs::read` uses, and for the same
/// reason: a helper that re-read for a tag could hand back one describing a
/// different state than the body.
async fn read(harness: &Harness) -> (serde_json::Value, String) {
    let response = harness
        .allowed_as(ADMIN)
        .send(request("GET", CUSTOMER_GROUP_TAXONOMY, None))
        .await;
    assert_eq!(response.status(), StatusCode::OK, "the GET must answer 200");
    let tag = etag_of(&response).expect("a taxonomy read must carry its entity tag");
    (body_json(response).await, tag)
}

/// `PUT` a value set under a tag.
async fn put(
    harness: &Harness,
    tag: &str,
    values: serde_json::Value,
) -> axum::http::Response<axum::body::Body> {
    harness
        .allowed_as(ADMIN)
        .send(with_headers(
            "PUT",
            CUSTOMER_GROUP_TAXONOMY,
            Some(json!({ "values": values })),
            &[("if-match", tag)],
        ))
        .await
}

/// One **published** overlay scoped to `(customerGroup, value)`, written
/// through the entity — `rest_taxonomies.rs::seed_published_overlay`'s sibling.
/// The authoring route cannot produce this state in one call (a submit opens
/// an always-material approval unit, D-50), and what is under test here is the
/// taxonomy guard, not the overlay lifecycle.
async fn seed_published_overlay(harness: &Harness, value: &str) {
    use sea_orm::ActiveValue::Set;
    use sea_orm::EntityTrait;
    use toolkit_db::secure::{AccessScope, SecureInsertExt};

    let conn = harness.db.conn().expect("conn");
    let row = bss_pricing::infra::storage::entity::price_overlay::ActiveModel {
        price_overlay_id: Set(uuid::Uuid::from_u128(0x0c_9a)),
        revision: Set(1),
        tenant_id: Set(harness.tenant),
        lifecycle_state: Set("published".to_owned()),
        scope_class: Set("customer_group".to_owned()),
        scope_value: Set(value.to_owned()),
        precedence: Set(20),
        effective_from: Set(None),
        effective_to: Set(None),
        tax_basis: Set("delegated_tariffs".to_owned()),
        disclosure: Set("restricted".to_owned()),
        target_ref: Set(json!({"plans": []})),
        row_version: Set(0),
    };
    bss_pricing::infra::storage::entity::price_overlay::Entity::insert(row.clone())
        .secure()
        .scope_with_model(&AccessScope::allow_all(), &row)
        .expect("scope")
        .exec(&conn)
        .await
        .expect("seed a published overlay");
}

fn codes(body: &serde_json::Value) -> Vec<String> {
    body["values"]
        .as_array()
        .expect("values is an array")
        .iter()
        .map(|v| {
            format!(
                "{}:{}",
                v["value"].as_str().expect("value"),
                v["state"].as_str().expect("state")
            )
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The round trip.
// ---------------------------------------------------------------------------

/// A tenant with no values is answered `200` with an empty list **and a tag**
/// — the bootstrap reads its precondition off this response like every other
/// caller.
#[tokio::test]
async fn a_tenant_with_no_values_reads_200_with_an_empty_list_and_a_tag() {
    let harness = Harness::new().await;

    let (body, tag) = read(&harness).await;

    assert_eq!(codes(&body), Vec::<String>::new());
    assert!(
        !tag.is_empty(),
        "the bootstrap carries a tag like any other"
    );
}

/// The round trip: declare a value, read it back, and see it in the list.
#[tokio::test]
async fn a_put_declares_a_value_and_the_get_reads_it_back() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness).await;

    let response = put(
        &harness,
        &tag,
        json!([{ "value": "gold", "display_name": "Gold" }]),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let (body, _) = read(&harness).await;
    assert_eq!(codes(&body), ["gold:active"]);
    assert_eq!(body["values"][0]["display_name"], "Gold");
}

/// A value the body omits is retired, not deleted, and stays readable.
#[tokio::test]
async fn a_value_the_body_omits_is_retired_and_still_listed() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness).await;
    put(
        &harness,
        &tag,
        json!([
            { "value": "gold", "display_name": "Gold" },
            { "value": "silver", "display_name": "Silver" }
        ]),
    )
    .await;

    let (_, tag) = read(&harness).await;
    let response = put(
        &harness,
        &tag,
        json!([{ "value": "gold", "display_name": "Gold" }]),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let (body, _) = read(&harness).await;
    assert_eq!(codes(&body), ["gold:active", "silver:retired"]);
}

// ---------------------------------------------------------------------------
// The precondition.
// ---------------------------------------------------------------------------

/// A `PUT` with no `If-Match` is refused before anything is written.
#[tokio::test]
async fn a_put_without_if_match_is_refused() {
    let harness = Harness::new().await;

    let response = harness
        .allowed_as(ADMIN)
        .send(request(
            "PUT",
            CUSTOMER_GROUP_TAXONOMY,
            Some(json!({ "values": [{ "value": "gold", "display_name": "Gold" }] })),
        ))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (body, _) = read(&harness).await;
    assert_eq!(codes(&body), Vec::<String>::new(), "nothing was written");
}

/// A stale tag is refused as a lost-update guard, exactly as the four-class
/// taxonomy's `PUT` is.
#[tokio::test]
async fn a_concurrent_whole_set_put_is_refused_rather_than_retiring_the_other_authors_value() {
    let harness = Harness::new().await;
    let (_, shared_tag) = read(&harness).await;

    let first = put(
        &harness,
        &shared_tag,
        json!([{ "value": "gold", "display_name": "Gold" }]),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);

    let second = put(
        &harness,
        &shared_tag,
        json!([{ "value": "silver", "display_name": "Silver" }]),
    )
    .await;

    assert_eq!(second.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(second).await, "STALE_VERSION");

    let (body, _) = read(&harness).await;
    assert_eq!(
        codes(&body),
        ["gold:active"],
        "the first author's value survives and the second's was not applied"
    );
}

// ---------------------------------------------------------------------------
// The retire guard — `inst-cg-taxonomy`'s referential check.
// ---------------------------------------------------------------------------

/// **The `409` this surface's guard exists to produce.** A published
/// `customerGroup`-scoped overlay blocks its value's retirement.
#[tokio::test]
async fn retiring_a_referenced_value_answers_409_with_the_declared_code() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness).await;
    put(
        &harness,
        &tag,
        json!([{ "value": "gold", "display_name": "Gold" }]),
    )
    .await;

    seed_published_overlay(&harness, "gold").await;

    let (_, tag) = read(&harness).await;
    let refused = put(&harness, &tag, json!([])).await;

    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(refused).await, "TAXONOMY_VALUE_IN_USE");

    let (body, _) = read(&harness).await;
    assert_eq!(
        codes(&body),
        ["gold:active"],
        "a refused PUT writes nothing at all"
    );
}

// ---------------------------------------------------------------------------
// The audit half.
// ---------------------------------------------------------------------------

/// One audit record per `PUT`, naming this taxonomy.
#[tokio::test]
async fn a_put_is_audited_once_naming_the_taxonomy() {
    let harness = Harness::new().await;
    let before = audit_rows(&harness).await.len();
    let (_, tag) = read(&harness).await;

    put(
        &harness,
        &tag,
        json!([
            { "value": "gold", "display_name": "Gold" },
            { "value": "silver", "display_name": "Silver" }
        ]),
    )
    .await;

    let rows = audit_rows(&harness).await;
    assert_eq!(
        rows.len(),
        before + 1,
        "one PUT is one audited act, however many values it moved"
    );
    assert_eq!(
        rows.last().expect("a record").subject_ref,
        "taxonomy/customer_group"
    );
}

// ---------------------------------------------------------------------------
// The separate-route claim: `customer_group` and `config` are segregated.
// ---------------------------------------------------------------------------

/// **The whole point of the separate route.** A caller holding `config ×
/// write` — the grant that declares regions, brands, partners and org tiers —
/// does NOT, by that grant alone, satisfy `customer_group × write`.
///
/// Driven with [`rest_support::Harness::selectively_allowed_as`] rather than
/// `allowed_as`: the ordinary fixture grants every pair uniformly, which
/// cannot tell "gated on the right label and this caller lacks it" from
/// "gated on the wrong label and every fixture happens to satisfy it" — the
/// exact blind spot `rest_authz.rs`'s module doc names for the census it
/// carries. This case is the reason that census exists, armed against the
/// specific defect a shared `{class}` route would have reintroduced: a
/// `config`-only caller reaching a resource `05-governance.md` says is more
/// sensitive.
#[tokio::test]
async fn a_caller_holding_config_write_but_not_customer_group_write_is_refused() {
    let harness = Harness::new().await;
    let (_, tag) = read(&harness).await;

    let response = harness
        .selectively_allowed_as(ADMIN, &[(labels::CONFIG, actions::WRITE)])
        .send(with_headers(
            "PUT",
            CUSTOMER_GROUP_TAXONOMY,
            Some(json!({ "values": [{ "value": "gold", "display_name": "Gold" }] })),
            &[("if-match", tag.as_str())],
        ))
        .await;

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "config x write must not satisfy customer_group x write"
    );
    let (body, _) = read(&harness).await;
    assert_eq!(codes(&body), Vec::<String>::new(), "nothing was written");
}

/// The mirror control: a caller holding `customer_group × write` **and
/// nothing else** — not even `config` — succeeds. Without this, the case above
/// could pass merely because `SelectiveResolver` denies everything by
/// default; this is the positive half that proves the pair this route asks
/// for is the one that actually authorizes it.
#[tokio::test]
async fn a_caller_holding_only_customer_group_write_succeeds() {
    let harness = Harness::new().await;
    // The tag comes from the ordinary admin client: what this case is about is
    // the `PUT`'s gate, not the `GET`'s, so the read is not driven through the
    // restricted client.
    let (_, tag) = read(&harness).await;

    let response = harness
        .selectively_allowed_as(ADMIN, &[(labels::CUSTOMER_GROUP, actions::WRITE)])
        .send(with_headers(
            "PUT",
            CUSTOMER_GROUP_TAXONOMY,
            Some(json!({ "values": [{ "value": "gold", "display_name": "Gold" }] })),
            &[("if-match", tag.as_str())],
        ))
        .await;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "customer_group x write alone, with no config grant at all, must authorize the write"
    );
}

// ---------------------------------------------------------------------------
// Task 6: the membership routes, and the publish unit `dod-customer-group`'s
// MUST requires (`design/09-price-overlays.md:472-473`) — audit-only path
// only (`inst-mm-renewal`).
// ---------------------------------------------------------------------------

/// The admin who enrolls payers.
const MEMBERSHIP_ADMIN: uuid::Uuid = uuid::Uuid::from_u128(0xca_d2);

/// Enroll a payer through the real route, returning the parsed body.
///
/// Declares `gold` active first — `GROUP_UNKNOWN` refuses a group the
/// taxonomy never declared, and this suite's own subject is the membership
/// mutation, not the taxonomy gate (that is `retiring_a_referenced_value_
/// answers_409_with_the_declared_code` and its siblings above).
async fn enroll(harness: &Harness, payer_tenant_id: Uuid) -> (StatusCode, serde_json::Value) {
    rest_support::declare_customer_group(harness, "gold").await;
    let response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBERS.replace("{group}", "gold"),
            Some(json!({
                "payer_tenant_id": payer_tenant_id,
                "effective_from": "2026-01-01T00:00:00Z"
            })),
            &[("idempotency-key", "enroll-1")],
        ))
        .await;
    let status = response.status();
    (status, body_json(response).await)
}

/// Run the read-model warm sweep the way `sqlite_read_model.rs`'s own
/// `sweep` helper does — a fresh [`ReadModelWarmJob`] over the harness's own
/// provider and the **same** registry double the route's publish unit
/// requested its handle from, so a ref this test recorded and a ref the
/// sweep resolves are the same act's two halves.
async fn sweep(harness: &Harness, now: chrono::DateTime<chrono::Utc>) -> u64 {
    let job = ReadModelWarmJob::new(
        harness.db.clone(),
        Arc::clone(&harness.registry) as Arc<dyn CatalogVersionRegistryV1>,
        JobsConfig::default(),
    );
    let report = job.run(now).await.expect("the sweep pass runs");
    report.subjects_projected
}

/// **The claim this section exists to prove.** A route-level enrollment is a
/// real publish unit: a pending ref recorded against the membership subject,
/// the audit record written, and the projector able to run on it and land a
/// warm delta — not merely a response that says so.
///
/// To redden this: comment out `record_ref`'s call inside
/// `membership_publish::enroll_in` (or drop the call to it entirely). The row
/// still lands and the `201` still answers — the response body proves
/// nothing here, which is exactly why every assertion below reads the
/// **store** instead. With no ref recorded, `pending_version_refs` returns
/// empty and the sweep has nothing to resolve, so `subjects_projected` reads
/// `0` and the third assertion catches what the first two, read alone, could
/// each still pass past (a ref recorded for the wrong subject id would fail
/// only the first).
#[tokio::test]
async fn a_route_level_enrollment_is_a_real_publish_unit() {
    let harness = Harness::new().await;
    let payer_tenant_id = Uuid::now_v7();

    let (status, body) = enroll(&harness, payer_tenant_id).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    let membership_id = body["membership"]["membership_id"]
        .as_str()
        .expect("membership_id")
        .to_owned();
    let pending_ref = body["pending_version_ref"]
        .as_str()
        .expect("pending_version_ref")
        .to_owned();
    assert!(!pending_ref.is_empty());

    // 1. A pending ref, recorded against exactly this membership subject.
    let refs = pending_version_refs(&harness).await;
    let matched: Vec<_> = refs
        .iter()
        .filter(|r| r.subject_kind == "group_membership" && r.subject_ref == membership_id)
        .collect();
    assert_eq!(
        matched.len(),
        1,
        "expected exactly one pending ref for membership {membership_id}, got {refs:?}"
    );
    assert_eq!(matched[0].pending_ref, pending_ref);

    // 2. The audit record `group_membership_repo::enroll` writes.
    let audits = audit_rows(&harness).await;
    assert!(
        audits
            .iter()
            .any(|a| a.subject_ref == membership_id && a.action == "create"),
        "no audit record named membership {membership_id}: {audits:?}"
    );

    // 3. The projector can run on the ref this route recorded: commit the
    // handle on the registry double and sweep.
    harness.registry.commit(&pending_ref, 9);
    let projected = sweep(&harness, chrono::Utc::now()).await;
    assert_eq!(
        projected, 1,
        "the projector must land exactly one warm delta off this route's own pending ref"
    );
}

/// **The audit-only property.** `inst-mm-renewal`'s default commits directly
/// and opens **no** approval unit.
///
/// Paired with [`a_pending_approval_unit_is_visible_through_the_same_readback`]
/// as the positive control the module doc's TESTS section requires: without
/// it, a broken `approval_rows` that always answered empty would make this
/// test pass for the wrong reason.
///
/// To redden this: give `membership_publish::enroll_in` a call that opens an
/// approval unit (any `approval_repo::open` call over the enrolled
/// membership's subject) before it returns. The route still answers `201`
/// and the publish-unit assertions in the sibling test still hold, which is
/// why this is its own test rather than a fourth assertion bolted onto that
/// one.
#[tokio::test]
async fn enrolling_a_payer_opens_no_approval_unit() {
    let harness = Harness::new().await;
    let before = approval_rows(&harness).await.len();

    let (status, _body) = enroll(&harness, Uuid::now_v7()).await;
    assert_eq!(status, StatusCode::CREATED);

    assert_eq!(
        approval_rows(&harness).await.len(),
        before,
        "the audit-only path must open no approval unit"
    );
}

/// The positive control for the test above: `approval_rows` really does see
/// a unit when one is opened, seeded independently of the membership route
/// through `approval_repo::open` directly (no plan needed — the approval
/// store's own writer takes a bare subject).
///
/// To redden this: remove the `approval_repo::open` call below. Then this
/// test asserts `1 == 0` and fails loudly, which is what proves it was
/// checking something rather than passing by construction.
#[tokio::test]
async fn a_pending_approval_unit_is_visible_through_the_same_readback() {
    let harness = Harness::new().await;
    let conn = harness.db.conn().expect("conn");
    assert_eq!(approval_rows(&harness).await.len(), 0, "clean start");

    approval_repo::open(
        &conn,
        &harness.scope(),
        NewApproval {
            approval_id: Uuid::now_v7(),
            tenant_id: harness.tenant,
            // The approval store's own shape (`<plan_id>/<revision>`), unrelated
            // to any membership route — a bare label tripped `CorruptRow` here
            // during this test's own development, which is `subject_of`'s own
            // fail-closed reading of a subject no writer in this crate produces.
            subject_ref: audit_repo::plan_revision_ref(PlanId::new(Uuid::now_v7()), 0),
            subject_kind: AuditSubjectKind::PlanRevision,
            content_hash: vec![0u8; 32],
            materiality: json!({ "material": true, "reason": "positive-control" }),
            held_keys: std::collections::BTreeSet::new(),
        },
        stamp_of(MEMBERSHIP_ADMIN, chrono::Utc::now()),
    )
    .await
    .expect("seed a pending unit unrelated to any membership route");

    assert_eq!(
        approval_rows(&harness).await.len(),
        1,
        "the readback must see a unit that genuinely exists"
    );
}

/// The move route composes an end and an enroll on one publish unit — both
/// subjects recorded against the **same** pending ref.
#[tokio::test]
async fn a_move_records_both_membership_subjects_against_one_pending_ref() {
    let harness = Harness::new().await;
    let payer_tenant_id = Uuid::now_v7();
    let (status, body) = enroll(&harness, payer_tenant_id).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    let old_membership_id = body["membership"]["membership_id"]
        .as_str()
        .expect("membership_id")
        .to_owned();
    rest_support::declare_customer_group(&harness, "silver").await;

    let response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBER_MOVE
                .replace("{group}", "silver")
                .replace("{payerId}", &payer_tenant_id.to_string()),
            Some(json!({ "effective_from": "2026-06-01T00:00:00Z" })),
            &[("idempotency-key", "move-1")],
        ))
        .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let ended_id = body["ended"]["membership_id"]
        .as_str()
        .expect("ended.membership_id");
    assert_eq!(
        ended_id, old_membership_id,
        "the move ended the payer's prior membership"
    );
    let enrolled_id = body["enrolled"]["membership_id"]
        .as_str()
        .expect("enrolled.membership_id")
        .to_owned();
    let pending_ref = body["pending_version_ref"]
        .as_str()
        .expect("pending_version_ref")
        .to_owned();

    let refs = pending_version_refs(&harness).await;
    let for_pending: Vec<_> = refs
        .iter()
        .filter(|r| r.pending_ref == pending_ref)
        .collect();
    let subjects: std::collections::BTreeSet<&str> =
        for_pending.iter().map(|r| r.subject_ref.as_str()).collect();
    assert!(
        subjects.contains(old_membership_id.as_str()) && subjects.contains(enrolled_id.as_str()),
        "the one handle must carry both membership subjects: {refs:?}"
    );
}

/// The `PATCH` ends a membership under its own `If-Match`, and the same
/// publish unit and audit-only properties hold for it.
#[tokio::test]
async fn adjusting_a_membership_is_also_its_own_publish_unit() {
    let harness = Harness::new().await;
    let (status, body) = enroll(&harness, Uuid::now_v7()).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    let membership_id = body["membership"]["membership_id"]
        .as_str()
        .expect("membership_id")
        .to_owned();
    // There is no `GET` on a membership (§5), so the tag a `PATCH` asserts is
    // the one the create's own response carried — read off the body rather
    // than a second request.
    let row_version = body["membership"]["row_version"]
        .as_u64()
        .expect("row_version");
    let expected_tag = format!("\"{row_version}\"");

    let response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "PATCH",
            &CUSTOMER_GROUP_MEMBER
                .replace("{group}", "gold")
                .replace("{id}", &membership_id),
            Some(json!({ "effective_to": "2026-03-01T00:00:00Z" })),
            &[("if-match", &expected_tag)],
        ))
        .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let pending_ref = body["pending_version_ref"]
        .as_str()
        .expect("pending_version_ref");

    let refs = pending_version_refs(&harness).await;
    assert!(
        refs.iter()
            .any(|r| r.subject_ref == membership_id && r.pending_ref == pending_ref),
        "the PATCH must record its own pending ref: {refs:?}"
    );
}

// ---------------------------------------------------------------------------
// `GROUP_UNKNOWN` (§5:257): `{group}` must be declared and active.
// ---------------------------------------------------------------------------

/// A `{group}` the taxonomy has never declared is refused `GROUP_UNKNOWN`, and
/// nothing lands: no membership row, no pending ref, no audit record.
///
/// **The positive control** is `a_route_level_enrollment_is_a_real_publish_unit`
/// above: it already proves an enrollment into a group `declare_customer_group`
/// made active succeeds end to end, so this refusal is provably about the
/// undeclared group and not about the route being broken in general.
///
/// To redden this: remove the `require_active_group` call from
/// `membership_publish::enroll_in`. The route still answers `201` and the
/// positive control above still passes, which is why this needs its own test
/// rather than a shared one.
#[tokio::test]
async fn enrolling_into_an_undeclared_group_is_refused_group_unknown_and_writes_nothing() {
    let harness = Harness::new().await;
    // No `declare_customer_group` call: the tenant's taxonomy is empty.

    let response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBERS.replace("{group}", "nonexistent"),
            Some(json!({
                "payer_tenant_id": Uuid::now_v7(),
                "effective_from": "2026-01-01T00:00:00Z"
            })),
            &[("idempotency-key", "undeclared-1")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(response).await, "GROUP_UNKNOWN");

    assert_eq!(pending_version_refs(&harness).await, Vec::new());
    assert!(audit_rows(&harness).await.is_empty());
}

/// **The retired case (§5's "declared and then retired" reading).** A
/// `{group}` that was declared and then retired is refused `GROUP_UNKNOWN`
/// exactly as an undeclared one is — retirement guards existing references,
/// it does not bless a new one.
///
/// To redden this: in `membership_publish::require_active_group`, change the
/// `Some(entry) if entry.state == TaxonomyState::Active => Ok(())` arm's
/// negative case to answer `Ok(())` regardless of state (i.e. only refuse
/// `None`). The sibling test above (an **undeclared** group) still catches
/// nothing wrong — its group answers `None`, not `Some(Retired)` — which is
/// exactly why the retired case needs a test of its own rather than trusting
/// the undeclared one to cover it.
#[tokio::test]
async fn enrolling_into_a_retired_group_is_refused_group_unknown_and_writes_nothing() {
    let harness = Harness::new().await;
    rest_support::declare_customer_group(&harness, "silver-tier").await;
    rest_support::retire_customer_group(&harness, "silver-tier").await;

    let response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBERS.replace("{group}", "silver-tier"),
            Some(json!({
                "payer_tenant_id": Uuid::now_v7(),
                "effective_from": "2026-01-01T00:00:00Z"
            })),
            &[("idempotency-key", "retired-1")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(response).await, "GROUP_UNKNOWN");

    assert_eq!(pending_version_refs(&harness).await, Vec::new());
    assert!(audit_rows(&harness).await.is_empty());
}

/// The move route's target group is checked the same way — a payer cannot be
/// moved into a group the taxonomy does not currently declare.
#[tokio::test]
async fn moving_a_payer_into_an_undeclared_group_is_refused_group_unknown_and_writes_nothing() {
    let harness = Harness::new().await;
    let payer_tenant_id = Uuid::now_v7();
    let (status, body) = enroll(&harness, payer_tenant_id).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    let refs_before = pending_version_refs(&harness).await;
    let audits_before = audit_rows(&harness).await.len();

    let response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBER_MOVE
                .replace("{group}", "nonexistent")
                .replace("{payerId}", &payer_tenant_id.to_string()),
            Some(json!({ "effective_from": "2026-06-01T00:00:00Z" })),
            &[("idempotency-key", "move-undeclared-1")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(response).await, "GROUP_UNKNOWN");

    // Nothing moved: the payer's one prior membership is still exactly what
    // it was, and no second pending ref or audit record appeared.
    assert_eq!(pending_version_refs(&harness).await, refs_before);
    assert_eq!(audit_rows(&harness).await.len(), audits_before);
}

// ---------------------------------------------------------------------------
// The material path: `inst-mm-immediate` / `inst-mm-renewal`'s control pair,
// and the approve -> commit lane (Task 7 of the customer-group plane).
// ---------------------------------------------------------------------------

/// The independent second principal every material-path test below decides
/// under — distinct from [`MEMBERSHIP_ADMIN`], which is what
/// `chk_pricing_approval_distinct_principals` and `inst-tp-distinct` both
/// require of an approve.
const MOVE_APPROVER: uuid::Uuid = uuid::Uuid::from_u128(0xca_d3);

/// `POST .../move` with no `immediate` field at all.
///
/// **The negative half of the control pair.** Paired with
/// [`moving_a_payer_immediately_is_material_and_writes_no_membership_row_until_approved`]:
/// one alone proves nothing, because a route that opened a unit on *every*
/// move would still pass a test that only checked the immediate case, and a
/// route that opened one on *no* move would still pass a test that only
/// checked the default case. Together they prove the field is what decides
/// it.
///
/// To redden this: give `move_membership`'s renewal-aligned branch a call
/// that opens an approval unit (any `ApprovalService::submit_membership_move_on`
/// call before it returns). The route still answers `200` and the sibling
/// pending-ref test still holds, which is why this is checked on its own.
#[tokio::test]
async fn moving_a_payer_without_immediate_still_opens_no_approval_unit() {
    let harness = Harness::new().await;
    let payer_tenant_id = Uuid::now_v7();
    let (status, body) = enroll(&harness, payer_tenant_id).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    rest_support::declare_customer_group(&harness, "silver").await;
    let before = approval_rows(&harness).await.len();

    let response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBER_MOVE
                .replace("{group}", "silver")
                .replace("{payerId}", &payer_tenant_id.to_string()),
            Some(json!({ "effective_from": "2026-06-01T00:00:00Z" })),
            &[("idempotency-key", "move-renewal-1")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        approval_rows(&harness).await.len(),
        before,
        "a renewal-aligned move must open no approval unit (inst-mm-renewal)"
    );
}

/// **The positive half of the control pair.** `immediate: true` makes the
/// move material: the route answers `202`, an approval unit opens carrying
/// `alwaysMaterialTrigger` — read back from the **store**, never from the
/// response body (F-1: a route that echoes its own literal proves nothing
/// about what the store holds) — and, `inst-mm-pending`'s own promise, no
/// `pricing_group_membership` row exists yet.
///
/// To redden this: delete the `if request.immediate == Some(true)` branch
/// from `move_membership`, folding the immediate call into the ordinary
/// audit-only path. The response then answers `200` with a committed move
/// instead of `202`, and `approval_rows` stays at `before`.
#[tokio::test]
async fn moving_a_payer_immediately_is_material_and_writes_no_membership_row_until_approved() {
    let harness = Harness::new().await;
    let payer_tenant_id = Uuid::now_v7();
    rest_support::declare_customer_group(&harness, "gold").await;
    let before = approval_rows(&harness).await.len();

    let response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBER_MOVE
                .replace("{group}", "gold")
                .replace("{payerId}", &payer_tenant_id.to_string()),
            Some(json!({
                "effective_from": "2026-06-01T00:00:00Z",
                "immediate": true
            })),
            &[("idempotency-key", "move-immediate-1")],
        ))
        .await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
    assert_eq!(body["outcome"], "submitted_for_approval");
    assert!(body["moved"].is_null(), "nothing has committed yet");

    assert_eq!(
        approval_rows(&harness).await.len(),
        before + 1,
        "an immediate re-resolution must open exactly one approval unit"
    );
    let approval_id: Uuid = body["approval"]["approval_id"]
        .as_str()
        .expect("approval.approval_id")
        .parse()
        .expect("a UUID");
    let stored = approval_row(&harness, approval_id).await;
    assert_eq!(stored.subject_kind, AuditSubjectKind::Membership);
    assert_eq!(
        stored.materiality["reason"], "alwaysMaterialTrigger",
        "an immediate re-resolution must be stored as an always-material trigger: {:?}",
        stored.materiality
    );

    // `inst-mm-pending`: nothing is written to the membership plane before
    // approval.
    let conn = harness.db.conn().expect("conn");
    let intervals = group_membership_repo::intervals_for_payer(
        &conn,
        &harness.scope(),
        harness.tenant,
        payer_tenant_id,
    )
    .await
    .expect("read the payer's intervals");
    assert!(
        intervals.is_empty(),
        "no membership row may exist before the unit is approved"
    );
}

/// **The approve -> commit path works end to end, at three calls and not
/// two.** Once [`MOVE_APPROVER`] — independent of the submitter — approves
/// the unit the first call opened, retrying the identical move (a fresh
/// `Idempotency-Key`, since this is a new attempt rather than a replay of
/// the first) finds the approved unit and commits: `200`, a real membership
/// row, and the D-06 publish unit's pending handle. A **third** call —
/// `ApprovalState::Approved` is terminal, so the unit this call finds is the
/// same one the second call already applied — must answer the same way
/// rather than trying to re-apply the move: this is the replay guard's own
/// test, and every earlier version of this suite stopped at two calls, which
/// is exactly why the guard's absence was invisible.
///
/// To redden this (the commit half): in `move_membership_immediate`,
/// short-circuit the `approved_unit` branch to always fall through to "open
/// a new unit" instead of committing. The second call then answers `202`
/// again (a second, distinct approval unit) instead of `200` with a
/// committed move, and the membership-row assertion after it fails.
///
/// To redden this (the replay guard): remove the `already_applied` check
/// from `ApprovalService::commit_membership_move_in`. The third call then
/// re-enters `move_payer_in` for a proposal already applied, which tries to
/// end the membership row the second call just created at the exact instant
/// it starts — `end_membership` refuses that `MembershipIntervalEmpty`, and
/// the third call answers a 4xx interval error instead of replaying `200`.
#[tokio::test]
async fn an_immediately_moved_payer_commits_once_a_second_principal_approves() {
    let harness = Harness::new().await;
    let payer_tenant_id = Uuid::now_v7();
    rest_support::declare_customer_group(&harness, "gold").await;

    let submit_response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBER_MOVE
                .replace("{group}", "gold")
                .replace("{payerId}", &payer_tenant_id.to_string()),
            Some(json!({
                "effective_from": "2026-06-01T00:00:00Z",
                "immediate": true
            })),
            &[("idempotency-key", "move-immediate-commit-1")],
        ))
        .await;
    assert_eq!(submit_response.status(), StatusCode::ACCEPTED);
    let submit_body = body_json(submit_response).await;
    let approval_id: Uuid = submit_body["approval"]["approval_id"]
        .as_str()
        .expect("approval.approval_id")
        .parse()
        .expect("a UUID");

    let approve_response = harness
        .allowed_as(MOVE_APPROVER)
        .send(request(
            "POST",
            &APPROVAL_APPROVE.replace("{approvalId}", &approval_id.to_string()),
            None,
        ))
        .await;
    assert_eq!(
        approve_response.status(),
        StatusCode::OK,
        "body: {:?}",
        body_json(approve_response).await
    );

    let commit_response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBER_MOVE
                .replace("{group}", "gold")
                .replace("{payerId}", &payer_tenant_id.to_string()),
            Some(json!({
                "effective_from": "2026-06-01T00:00:00Z",
                "immediate": true
            })),
            &[("idempotency-key", "move-immediate-commit-2")],
        ))
        .await;
    let status = commit_response.status();
    let body = body_json(commit_response).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["outcome"], "committed");
    assert_eq!(body["moved"]["enrolled"]["group_value"], "gold");
    assert_eq!(
        body["moved"]["enrolled"]["payer_tenant_id"],
        payer_tenant_id.to_string()
    );
    assert!(
        !body["moved"]["pending_version_ref"]
            .as_str()
            .expect("pending_version_ref")
            .is_empty()
    );
    let committed_membership_id = body["moved"]["enrolled"]["membership_id"].clone();

    // **The third call.** `ApprovalState::Approved` is terminal, so this
    // finds the same approved unit the second call already applied. It must
    // answer idempotently — the same membership, no new row, no interval
    // refusal — rather than trying to re-apply a proposal that already
    // landed.
    let replay_response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "POST",
            &CUSTOMER_GROUP_MEMBER_MOVE
                .replace("{group}", "gold")
                .replace("{payerId}", &payer_tenant_id.to_string()),
            Some(json!({
                "effective_from": "2026-06-01T00:00:00Z",
                "immediate": true
            })),
            &[("idempotency-key", "move-immediate-commit-3")],
        ))
        .await;
    let replay_status = replay_response.status();
    let replay_body = body_json(replay_response).await;
    assert_eq!(
        replay_status,
        StatusCode::OK,
        "a third call over an already-committed unit must answer idempotently, not refuse an \
         interval error: body: {replay_body}"
    );
    assert_eq!(replay_body["outcome"], "committed");
    assert_eq!(
        replay_body["moved"]["enrolled"]["membership_id"], committed_membership_id,
        "a replay must report the membership the second call already created, not a new one"
    );

    let conn = harness.db.conn().expect("conn");
    let intervals = group_membership_repo::intervals_for_payer(
        &conn,
        &harness.scope(),
        harness.tenant,
        payer_tenant_id,
    )
    .await
    .expect("read the payer's intervals");
    assert_eq!(
        intervals.len(),
        1,
        "the approved move must actually write the membership row, and the replay must not \
         write a second one"
    );
}

// ---------------------------------------------------------------------------
// `PATCH .../members/{id}`: `{group}` is checked, not decorative
// (`api::rest::prices::row_of_plan`'s shape).
// ---------------------------------------------------------------------------

/// A `PATCH` whose `{group}` disagrees with the addressed membership's own
/// stored group is refused exactly like an absent membership, and the row is
/// untouched — no new pending ref, no row version moved.
///
/// Paired with `adjusting_a_membership_is_also_its_own_publish_unit` as the
/// positive control: that test `PATCH`es the **same** membership through its
/// **correct** group and succeeds, which is what proves this refusal is about
/// the mismatch and not about the route being broken in general.
///
/// To redden this: remove the `membership_of_group` call from
/// `adjust_membership`. The positive-control PATCH still succeeds (it never
/// exercises a mismatch), which is why the pairing is necessary rather than
/// decorative — a broken removal would silently pass the paired test too if
/// the paired test were the only one changed to catch it.
#[tokio::test]
async fn patching_a_membership_through_the_wrong_group_is_refused_and_writes_nothing() {
    let harness = Harness::new().await;
    let (status, body) = enroll(&harness, Uuid::now_v7()).await;
    assert_eq!(status, StatusCode::CREATED, "body: {body}");
    let membership_id: Uuid = body["membership"]["membership_id"]
        .as_str()
        .expect("membership_id")
        .parse()
        .expect("a UUID");
    rest_support::declare_customer_group(&harness, "wrong-group").await;
    let refs_before = pending_version_refs(&harness).await;
    let before = membership_row(&harness, membership_id)
        .await
        .expect("the enrolled row exists");

    let response = harness
        .allowed_as(MEMBERSHIP_ADMIN)
        .send(with_headers(
            "PATCH",
            &CUSTOMER_GROUP_MEMBER
                .replace("{group}", "wrong-group")
                .replace("{id}", &membership_id.to_string()),
            Some(json!({ "effective_to": "2026-03-01T00:00:00Z" })),
            &[("if-match", "\"0\"")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let after = membership_row(&harness, membership_id)
        .await
        .expect("the row still exists, untouched");
    assert_eq!(
        after.effective_to, before.effective_to,
        "effective_to moved"
    );
    assert_eq!(after.row_version, before.row_version, "row_version moved");
    assert_eq!(
        pending_version_refs(&harness).await,
        refs_before,
        "a refused PATCH must record no pending ref"
    );
}
