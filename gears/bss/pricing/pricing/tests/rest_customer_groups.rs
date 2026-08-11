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
use bss_pricing::api::rest::customer_groups::CUSTOMER_GROUP_TAXONOMY;
use bss_pricing::authz::{actions, labels};
use rest_support::{Harness, audit_rows, body_json, etag_of, problem_code, request, with_headers};
use serde_json::json;

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
