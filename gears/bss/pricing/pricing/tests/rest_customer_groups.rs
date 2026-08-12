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
use bss_pricing::infra::storage::repo::{NewApproval, approval_repo};
use rest_support::{
    Harness, approval_rows, audit_rows, body_json, etag_of, pending_version_refs, problem_code,
    request, stamp_of, with_headers,
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
async fn enroll(harness: &Harness, payer_tenant_id: Uuid) -> (StatusCode, serde_json::Value) {
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
