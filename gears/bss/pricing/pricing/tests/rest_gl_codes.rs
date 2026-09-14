//! `GET /bss-pricing/v1/config/vocabularies/gl-codes` and its per-value routes — the
//! general-ledger codes a tenant declares (D-356) — and what declaring them
//! does to a plan's publish.
//!
//! # What is under test, and why the publish trio comes first
//!
//! The vocabulary's own surface is `rounding-policies`' one document over, and
//! its cases below are that suite's. The three publish cases are the reason the
//! surface exists: a plan whose descriptor `glCode` is **in** the declared set
//! publishes, one whose code is **outside a non-empty** set is refused
//! `GL_CODE_UNKNOWN`, and one on a tenant that declared **nothing** publishes any
//! code at all. The refusal is asserted first, because the empty set means
//! "unconstrained" and an unwired `rule_params` looks exactly like a satisfied
//! vocabulary — refusing is what proves the set reached the rule (the D-254
//! defect class `infra::publish` records having paid for once already).

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::gl_codes::{GL_CODE_VALUES, GL_CODES};
use bss_pricing::domain::lifecycle::LifecycleState;
use bss_pricing::domain::scope_key::PlanId;
use bss_pricing::infra::storage::entity::{gl_code_taxonomy, plan, plan_descriptor_set};
use bss_pricing::infra::storage::repo::NewPriceDraft;
use rest_support::{
    Harness, body_json, etag_of, problem_code, publishable_row, publishable_scope_key,
    seed_publishable_shape, with_headers,
};
use sea_orm::ActiveValue::Set;
use sea_orm::sea_query::Expr;
use sea_orm::{ColumnTrait, Condition, EntityTrait};
use toolkit_db::secure::{AccessScope, SecureInsertExt, SecureUpdateExt};
use uuid::Uuid;

/// The submitting principal.
const SUBMITTER: Uuid = Uuid::from_u128(0x5_7e);

// ---------------------------------------------------------------------------
// What declaring the vocabulary does to publish (D-356).
// ---------------------------------------------------------------------------

/// A publishable plan with one priced, covered row. `seed_publishable_shape`
/// authors the descriptor `glCode` **`4000`**, which is the code every case here
/// declares, omits or ignores.
async fn seed_priced_plan(harness: &Harness, plan_id: Uuid, case_seq: u128) -> String {
    let shape = seed_publishable_shape(harness, plan_id).await;
    let plan = PlanId::new(plan_id);
    let scope = harness.scope();

    let price_id = Uuid::now_v7();
    harness
        .state
        .prices
        .create_draft(
            &scope,
            harness.tenant,
            NewPriceDraft {
                price_id,
                scope_key: publishable_scope_key(plan, shape.phase, "eu"),
                content: publishable_row(),
                created_by: rest_support::SEED_ACTOR,
                created_at_utc: rest_support::at(10),
                correlation_id: Uuid::from_u128(0x_61_c0_de_00 + case_seq),
            },
        )
        .await
        .expect("author the row");

    // `inst-wc-required`: no row publishes without a window on its canonical key.
    let conn = harness.state.db.conn().expect("conn");
    common::schedule_coverage_window(
        &conn,
        &scope,
        harness.tenant,
        price_id,
        rest_support::seed_stamp(),
    )
    .await;
    shape.etag()
}

/// **The vocabulary, end to end**: an undeclared code is refused at publish, and
/// declaring it lets the same plan through.
///
/// The pair matters more than either half, for `rest_rounding_policy`'s reason:
/// a test that only declared the code and published would pass against a rule
/// whose operand nobody loaded.
#[tokio::test]
async fn an_undeclared_gl_code_is_refused_and_declaring_it_lets_the_plan_publish() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    let etag = seed_priced_plan(&harness, plan_id, 1).await;

    // A vocabulary that does not contain the descriptor's `4000`.
    declare_gl_code(&harness, "4010-TAX").await;

    let refused = publish(&harness, plan_id, &etag).await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(refused).await.to_string();
    assert!(
        detail.contains("GL_CODE_UNKNOWN"),
        "the refusal names the vocabulary rule; got {detail}"
    );
    assert!(
        detail.contains("`4000`"),
        "and the code the author typed, so they know what to declare or fix; got {detail}"
    );

    declare_gl_code(&harness, "4000").await;

    let after = publish(&harness, plan_id, &etag).await;
    let status = after.status();
    let body = body_json(after).await;
    // **202 and an opened unit**, not merely "the code is absent": a publish that
    // failed for some other reason would satisfy an absence assertion just as well.
    assert_eq!(
        status,
        StatusCode::ACCEPTED,
        "the declared code clears the rule; got {body}"
    );
    assert_eq!(body["outcome"], serde_json::json!("submitted_for_approval"));
}

/// An empty vocabulary constrains nothing — the opt-in reading (D-356).
///
/// The negative control for the case above, and the single property that let
/// the table land: without it, every plan whose `glCode` nobody had declared
/// would have failed publish on the day of the migration.
#[tokio::test]
async fn a_tenant_with_no_declared_vocabulary_publishes_any_gl_code() {
    let harness = Harness::new().await;
    let plan_id = Uuid::now_v7();
    let etag = seed_priced_plan(&harness, plan_id, 2).await;

    let after = publish(&harness, plan_id, &etag).await;
    assert_eq!(
        after.status(),
        StatusCode::ACCEPTED,
        "declaring nothing is not opting in; got {}",
        body_json(after).await
    );
}

/// Declare one GL code straight at the table.
///
/// Direct because the publish rule's operand is what these cases are about, not
/// the vocabulary surface — `rest_rounding_policy::declare_rounding_value`
/// carries the same reasoning. The surface's own cases are below.
async fn declare_gl_code(harness: &Harness, value: &str) {
    let conn = harness.db.conn().expect("conn");
    let row = gl_code_taxonomy::ActiveModel {
        tenant_id: Set(harness.tenant),
        value: Set(value.to_owned()),
        display_name: Set(format!("fixture {value}")),
        state: Set("active".to_owned()),
    };
    gl_code_taxonomy::Entity::insert(row.clone())
        .secure()
        .scope_with_model(&AccessScope::allow_all(), &row)
        .expect("scope the value")
        .exec(&conn)
        .await
        .expect("declare the value");
}

async fn publish(
    harness: &Harness,
    plan_id: Uuid,
    tag: &str,
) -> axum::http::Response<axum::body::Body> {
    harness
        .allowed_as(SUBMITTER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/plans/{plan_id}/publish"),
            None,
            &[("if-match", tag)],
        ))
        .await
}

// ---------------------------------------------------------------------------
// The vocabulary's own surface (D-356) — `rest_rounding_policy`'s cases, one
// document over.
// ---------------------------------------------------------------------------

async fn read_vocabulary(harness: &Harness) -> (StatusCode, Option<String>, serde_json::Value) {
    let response = harness
        .allowed()
        .send(with_headers("GET", GL_CODES, None, &[]))
        .await;
    let status = response.status();
    let tag = etag_of(&response);
    (status, tag, body_json(response).await)
}

/// `POST …/values` — declare **one** code.
async fn declare(
    harness: &Harness,
    body: serde_json::Value,
) -> axum::http::Response<axum::body::Body> {
    harness
        .allowed()
        .send(with_headers("POST", GL_CODE_VALUES, Some(body), &[]))
        .await
}

/// `GET …/values/{value}` — one code and **its own** tag.
async fn read_value(
    harness: &Harness,
    value: &str,
) -> (StatusCode, Option<String>, serde_json::Value) {
    let response = harness
        .allowed()
        .send(with_headers("GET", &value_path(value), None, &[]))
        .await;
    let status = response.status();
    let tag = etag_of(&response);
    (status, tag, body_json(response).await)
}

/// `PATCH …/values/{value}` under the value's own tag.
async fn patch_value(
    harness: &Harness,
    value: &str,
    body: serde_json::Value,
    tag: &str,
) -> axum::http::Response<axum::body::Body> {
    harness
        .allowed()
        .send(with_headers(
            "PATCH",
            &value_path(value),
            Some(body),
            &[("if-match", tag)],
        ))
        .await
}

fn value_path(value: &str) -> String {
    format!("{GL_CODE_VALUES}/{value}")
}

/// Declare one code through the surface and assert it landed.
async fn put_one(harness: &Harness, value: &str) {
    let response = declare(
        harness,
        serde_json::json!({ "value": value, "display_name": format!("GL {value}") }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "{}",
        body_json(response).await
    );
}

/// A tenant that has declared nothing reads an empty set **with a tag** — the
/// state every tenant starts in, and the one that constrains nothing.
#[tokio::test]
async fn an_undeclared_vocabulary_reads_empty_with_a_tag() {
    let harness = Harness::new().await;

    let (status, tag, body) = read_vocabulary(&harness).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["values"], serde_json::json!([]));
    assert_eq!(body["resource"], serde_json::json!("gl-codes"));
    assert!(tag.is_some(), "the empty set is a state and carries a tag");
}

/// **The whole-set `PUT` is gone**, and the route answers as a method the
/// resource does not have rather than as an unknown path.
///
/// The case exists because the removal is the point of the change: a `PUT` left
/// standing beside the per-value doors would be a gate with an open door next
/// to it — it can retire any code the body omits, on one principal, with an
/// audit record that cannot say which.
#[tokio::test]
async fn the_whole_set_put_is_gone() {
    let harness = Harness::new().await;

    let response = harness
        .allowed()
        .send(with_headers(
            "PUT",
            GL_CODES,
            Some(serde_json::json!({ "values": [] })),
            &[("if-match", "\"whatever\"")],
        ))
        .await;

    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let (_, _, body) = read_vocabulary(&harness).await;
    assert_eq!(body["values"], serde_json::json!([]));
}

/// A declare round-trips through both reads, carries its **own** tag and a
/// `Location`, and defaults to `active`.
#[tokio::test]
async fn a_declared_value_round_trips_and_defaults_to_active() {
    let harness = Harness::new().await;

    let created = declare(
        &harness,
        serde_json::json!({ "value": "4010-TAX", "display_name": "Sales tax payable" }),
    )
    .await;
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(
        created
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok()),
        Some("/bss-pricing/v1/config/vocabularies/gl-codes/values/4010-TAX")
    );
    let value_tag = etag_of(&created).expect("the value's own tag");
    assert_eq!(
        body_json(created).await,
        serde_json::json!({
            "value": "4010-TAX", "display_name": "Sales tax payable", "state": "active"
        })
    );

    declare(
        &harness,
        serde_json::json!({ "value": "4000-REV", "display_name": "Revenue" }),
    )
    .await;

    let (_, _, body) = read_vocabulary(&harness).await;
    assert_eq!(
        body["values"],
        serde_json::json!([
            { "value": "4000-REV", "display_name": "Revenue", "state": "active" },
            { "value": "4010-TAX", "display_name": "Sales tax payable", "state": "active" }
        ]),
        "ordered by value, active unless said otherwise, and the second declare did not \
         retire the first - which is what the whole-set PUT could not promise"
    );

    let (status, own_tag, one) = read_value(&harness, "4010-TAX").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(one["display_name"], serde_json::json!("Sales tax payable"));
    assert_eq!(
        own_tag.as_deref(),
        Some(value_tag.as_str()),
        "the create's tag and the by-value GET's come from one computation"
    );
}

/// A repeat declare of the **same** content replays; one naming the same code
/// with **other** content is `409` and points at the `PATCH`.
///
/// This is what replaced `a_value_listed_twice_in_one_body_is_refused`: a body
/// carrying one value cannot list it twice, so the question "what happens when
/// a code is declared twice" moved from the body to the sequence, and both
/// answers are here.
#[tokio::test]
async fn a_second_declare_replays_or_refuses_by_content() {
    let harness = Harness::new().await;
    put_one(&harness, "4000-REV").await;

    let replay = declare(
        &harness,
        serde_json::json!({ "value": "4000-REV", "display_name": "GL 4000-REV" }),
    )
    .await;
    assert_eq!(replay.status(), StatusCode::OK, "the create's replay");

    let refused = declare(
        &harness,
        serde_json::json!({ "value": "4000-REV", "display_name": "Something else" }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let code = problem_code(refused).await;
    assert_eq!(code, "TAXONOMY_VALUE_EXISTS");

    let (_, _, body) = read_vocabulary(&harness).await;
    assert_eq!(
        body["values"][0]["display_name"],
        serde_json::json!("GL 4000-REV"),
        "the refused declare wrote nothing"
    );
}

/// A `PATCH` to `retired` retires **one** code and leaves its sibling alone —
/// the false conflict the per-value door exists to remove, asserted rather
/// than argued.
#[tokio::test]
async fn a_retirement_touches_one_value_and_stays_readable() {
    let harness = Harness::new().await;
    put_one(&harness, "4000-REV").await;
    put_one(&harness, "4010-TAX").await;

    let (_, tag, _) = read_value(&harness, "4010-TAX").await;
    let retired = patch_value(
        &harness,
        "4010-TAX",
        serde_json::json!({ "state": "retired" }),
        &tag.expect("the value's tag"),
    )
    .await;
    assert_eq!(retired.status(), StatusCode::OK);

    let (_, _, body) = read_vocabulary(&harness).await;
    assert_eq!(
        body["values"][0]["state"],
        serde_json::json!("active"),
        "the sibling is untouched: {body}"
    );
    assert_eq!(body["values"][1]["value"], serde_json::json!("4010-TAX"));
    assert_eq!(
        body["values"][1]["state"],
        serde_json::json!("retired"),
        "retired rather than gone: {body}"
    );
}

/// **Two admins editing two different codes do not refuse each other.**
///
/// The reason the whole-set `PUT` went: under it, the second author's tag was
/// stale the moment the first committed, though the two never disagreed about
/// anything. The tag is now the value's own, so the second write lands.
#[tokio::test]
async fn two_values_edited_under_tags_read_together_both_land() {
    let harness = Harness::new().await;
    put_one(&harness, "4000-REV").await;
    put_one(&harness, "4010-TAX").await;

    // Both authors read before either writes.
    let (_, first, _) = read_value(&harness, "4000-REV").await;
    let (_, second, _) = read_value(&harness, "4010-TAX").await;

    let one = patch_value(
        &harness,
        "4000-REV",
        serde_json::json!({ "display_name": "Revenue" }),
        &first.expect("a tag"),
    )
    .await;
    assert_eq!(one.status(), StatusCode::OK);

    let two = patch_value(
        &harness,
        "4010-TAX",
        serde_json::json!({ "display_name": "Tax payable" }),
        &second.expect("a tag"),
    )
    .await;
    assert_eq!(
        two.status(),
        StatusCode::OK,
        "the second author's tag covers their own value only: {}",
        body_json(two).await
    );
}

/// **The set's tag does not satisfy the per-value precondition.**
///
/// The two digests are deliberately over different segments — `gl-codes` and
/// `gl-codes/{value}` — so a client that read the collection and sent that tag
/// back on a value's `PATCH` is refused rather than accidentally admitted. A
/// tag built without the value segment collides with the set's whenever the
/// tenant holds exactly one code, which is every tenant's first day.
#[tokio::test]
async fn the_sets_tag_does_not_satisfy_the_per_value_patch() {
    let harness = Harness::new().await;
    put_one(&harness, "4000-REV").await;

    let (_, set_tag, _) = read_vocabulary(&harness).await;
    let refused = patch_value(
        &harness,
        "4000-REV",
        serde_json::json!({ "display_name": "Revenue" }),
        &set_tag.expect("the set's tag"),
    )
    .await;

    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(refused).await, "STALE_VERSION");
    let (_, _, body) = read_vocabulary(&harness).await;
    assert_eq!(
        body["values"][0]["display_name"],
        serde_json::json!("GL 4000-REV"),
        "the refused write changed nothing"
    );
}

/// A stale **value** tag is refused as a stale tag and writes nothing.
#[tokio::test]
async fn a_stale_value_tag_is_refused() {
    let harness = Harness::new().await;
    put_one(&harness, "4000-REV").await;
    let (_, first, _) = read_value(&harness, "4000-REV").await;
    let first = first.expect("a tag");

    let landed = patch_value(
        &harness,
        "4000-REV",
        serde_json::json!({ "display_name": "Revenue" }),
        &first,
    )
    .await;
    assert_eq!(landed.status(), StatusCode::OK);

    let response = patch_value(
        &harness,
        "4000-REV",
        serde_json::json!({ "display_name": "Something else" }),
        &first,
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    // Which 409: this route answers `TAXONOMY_VALUE_IN_USE` on the same status.
    assert_eq!(problem_code(response).await, "STALE_VERSION");
    let (_, _, body) = read_vocabulary(&harness).await;
    assert_eq!(
        body["values"][0]["display_name"],
        serde_json::json!("Revenue"),
        "the refused write changed nothing"
    );
}

/// A blank value is refused by the surface rather than by a constraint, and a
/// refused declare leaves the set exactly where it was.
#[tokio::test]
async fn a_blank_vocabulary_value_is_refused() {
    let harness = Harness::new().await;
    // The refusal needs something to lose: a readback against the empty default
    // is satisfied by a refusal that wiped the set on its way out.
    put_one(&harness, "4000-REV").await;
    let (_, _, before) = read_vocabulary(&harness).await;
    assert_eq!(before["values"].as_array().map(Vec::len), Some(1));

    let response = declare(
        &harness,
        serde_json::json!({ "value": "   ", "display_name": "nothing" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let (_, _, after) = read_vocabulary(&harness).await;
    assert_eq!(after["values"], before["values"]);
}

/// A code a **published** revision's descriptor set names cannot be retired
/// through the surface, and nothing is written.
///
/// The guard's whole point: retiring under a frozen reference would leave a
/// `CatalogVersion` naming a code no vocabulary declares. The revision is seeded
/// straight at the tables — the publish path is `rest_publish`'s subject — with
/// the descriptor row inserted while the revision is a draft, which is what its
/// append-only trigger admits, and the revision then flipped as `publish_revision`
/// flips it.
#[tokio::test]
async fn a_code_a_published_descriptor_set_names_cannot_be_retired() {
    let harness = Harness::new().await;
    put_one(&harness, "4000-REV").await;
    seed_published_revision_naming(&harness, Uuid::now_v7(), "4000-REV").await;

    let (_, tag, before) = read_vocabulary(&harness).await;
    assert!(tag.is_some());
    let (_, value_tag, _) = read_value(&harness, "4000-REV").await;
    let refused = patch_value(
        &harness,
        "4000-REV",
        serde_json::json!({ "state": "retired" }),
        &value_tag.expect("the value's tag"),
    )
    .await;

    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(refused).await, "TAXONOMY_VALUE_IN_USE");
    let (_, _, after) = read_vocabulary(&harness).await;
    assert_eq!(after["values"], before["values"]);
    assert_eq!(after["values"][0]["state"], serde_json::json!("active"));
}

/// **A code a published descriptor set names can be deprecated, and could not
/// be retired** (D-370).
///
/// The middle state's reason, on the descriptor plane. One fixture, two
/// destinations, two answers — asserted as a pair, because a case showing only
/// the deprecation landing would be satisfied by a retire guard that had
/// stopped firing.
///
/// The second half is the one that makes `deprecated` different from
/// `retired`: after the deprecation the code is out of `active_gl_codes`, so a
/// **new** descriptor naming it fails publish — and the published revision
/// that already names it still blocks the retirement, which is the same guard
/// reading the same reference it read before.
#[tokio::test]
async fn a_referenced_code_can_be_deprecated_where_it_could_not_be_retired() {
    let harness = Harness::new().await;
    put_one(&harness, "4000-REV").await;
    seed_published_revision_naming(&harness, Uuid::now_v7(), "4000-REV").await;

    let (_, tag, _) = read_value(&harness, "4000-REV").await;
    let refused = patch_value(
        &harness,
        "4000-REV",
        serde_json::json!({ "state": "retired" }),
        &tag.expect("the value's tag"),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(refused).await, "TAXONOMY_VALUE_IN_USE");

    let (_, tag, _) = read_value(&harness, "4000-REV").await;
    let deprecated = patch_value(
        &harness,
        "4000-REV",
        serde_json::json!({ "state": "deprecated" }),
        &tag.expect("the value's tag"),
    )
    .await;
    assert_eq!(
        deprecated.status(),
        StatusCode::OK,
        "saying `stop using this` must always be possible: {}",
        body_json(deprecated).await
    );

    let (_, _, body) = read_vocabulary(&harness).await;
    assert_eq!(body["values"][0]["state"], serde_json::json!("deprecated"));

    // And it is still guarded out of retirement, which is the difference
    // between deprecating and retiring stated as an assertion.
    let (_, tag, _) = read_value(&harness, "4000-REV").await;
    let still_refused = patch_value(
        &harness,
        "4000-REV",
        serde_json::json!({ "state": "retired" }),
        &tag.expect("the value's tag"),
    )
    .await;
    assert_eq!(still_refused.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(still_refused).await, "TAXONOMY_VALUE_IN_USE");
}

/// A state token outside the machine is refused, and the refusal names every
/// state the machine has.
///
/// This door builds its message from `TaxonomyState::ALL`, so it was correct
/// across D-370 without an edit — which is the property worth pinning, not the
/// message's current wording.
#[tokio::test]
async fn an_unknown_state_token_is_refused_naming_every_state() {
    let harness = Harness::new().await;

    let refused = declare(
        &harness,
        serde_json::json!({ "value": "4000-REV", "display_name": "R", "state": "withdrawn" }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
    let detail = body_json(refused).await.to_string();
    for token in ["active", "deprecated", "retired"] {
        assert!(
            detail.contains(token),
            "the refusal must name `{token}`: {detail}"
        );
    }
}

/// A code the tenant never declared is `404` on its own route rather than an
/// empty `200`.
#[tokio::test]
async fn an_undeclared_code_is_not_found_on_its_own_route() {
    let harness = Harness::new().await;
    put_one(&harness, "4000-REV").await;

    let (status, _, _) = read_value(&harness, "9999-NOPE").await;

    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Another tenant's vocabulary is invisible, and reads as the empty — hence
/// unconstrained — set.
///
/// A GL code is what an ERP posts a tenant's revenue against; one tenant's
/// catalog must neither see nor be judged by another's chart of accounts.
#[tokio::test]
async fn another_tenants_vocabulary_is_invisible() {
    let harness = Harness::new().await;
    put_one(&harness, "4000-REV").await;

    let response = harness
        .other_tenant()
        .send(with_headers("GET", GL_CODES, None, &[]))
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert!(etag_of(&response).is_some());
    assert_eq!(body_json(response).await["values"], serde_json::json!([]));
}

async fn seed_published_revision_naming(harness: &Harness, plan_id: Uuid, gl_code: &str) {
    let conn = harness.db.conn().expect("conn");
    let plan_row = plan::ActiveModel {
        plan_id: Set(plan_id),
        revision: Set(1),
        tenant_id: Set(harness.tenant),
        lifecycle_state: Set(LifecycleState::Draft.as_str().to_owned()),
        created_by: Set(rest_support::SEED_ACTOR),
        created_at_utc: Set(rest_support::at(9)),
        ..Default::default()
    };
    plan::Entity::insert(plan_row.clone())
        .secure()
        .scope_with_model(&AccessScope::allow_all(), &plan_row)
        .expect("scope")
        .exec(&conn)
        .await
        .expect("seed the plan revision");

    let descriptors = plan_descriptor_set::ActiveModel {
        plan_id: Set(plan_id),
        plan_revision: Set(1),
        tenant_id: Set(harness.tenant),
        invoice_line_template: Set(Some("{plan}".to_owned())),
        gl_code: Set(Some(gl_code.to_owned())),
        itemization_rule: Set(Some("per_charge".to_owned())),
        additional_fields: Set(serde_json::json!({})),
    };
    plan_descriptor_set::Entity::insert(descriptors.clone())
        .secure()
        .scope_with_model(&AccessScope::allow_all(), &descriptors)
        .expect("scope")
        .exec(&conn)
        .await
        .expect("seed the descriptor set");

    let moved = plan::Entity::update_many()
        .secure()
        .scope_with(&AccessScope::allow_all())
        .col_expr(
            plan::Column::LifecycleState,
            Expr::value(LifecycleState::Published.as_str()),
        )
        .filter(
            Condition::all()
                .add(plan::Column::PlanId.eq(plan_id))
                .add(plan::Column::Revision.eq(1_i64)),
        )
        .exec(&conn)
        .await
        .expect("publish the revision");
    assert_eq!(moved.rows_affected, 1, "the seed must have moved one row");
}
