//! Line-first authoring over the real router: `/plans/{planId}/charge-lines` and
//! the market prices nested under a line version.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod charge_line_support;
mod common;
mod rest_support;

#[tokio::test]
async fn a_line_can_be_drafted_before_its_market_prices() {
    let h = rest_support::Harness::new().await;
    let plan_id = uuid::Uuid::now_v7();
    rest_support::seed_draft_plan(&h, plan_id).await;
    let line = charge_line_support::create_flat_line(&h, plan_id).await;
    assert!(line["charge_line_id"].is_string());
    assert!(line["line_version_id"].is_string());
    assert_eq!(line["prices"], serde_json::json!([]));
    assert!(line["structure"].get("currency").is_none());
}

use axum::http::StatusCode;
use charge_line_support::{
    create_flat_line, flat_line_body, flat_price_body, line_path, line_prices_path, lines_path,
    post_line, post_price,
};
use rest_support::{
    Harness, body_json, etag_of, location_of, price_rows, problem_code, refused_by, request,
    seed_draft_plan, with_headers,
};
use uuid::Uuid;

async fn drafted_plan(h: &Harness) -> Uuid {
    let plan_id = Uuid::now_v7();
    seed_draft_plan(h, plan_id).await;
    plan_id
}

fn version_of(line: &serde_json::Value) -> String {
    line["line_version_id"]
        .as_str()
        .expect("a line names its version")
        .to_owned()
}

async fn read_line(h: &Harness, plan_id: Uuid, line_version_id: &str) -> serde_json::Value {
    body_json(
        h.allowed()
            .send(request("GET", &line_path(plan_id, line_version_id), None))
            .await,
    )
    .await
}

/// A three-tier graduated usage line on a metered SKU.
fn tiered_line_body() -> serde_json::Value {
    serde_json::json!({
        "scope_key": {
            "phase": rest_support::seeded_phase().get(),
            "sku_id": rest_support::resource_sku("cloudlets"),
            "price_eligibility": "all_subscriptions",
            "charge_kind": "usage",
            "cohort": null
        },
        "structure": {
            "model_kind": "graduated",
            "billing_granularity": "per_hour",
            "tiers": [
                {"from_qty": 0, "to_qty": 100},
                {"from_qty": 100, "to_qty": 1000},
                {"from_qty": 1000, "to_qty": null}
            ]
        }
    })
}

// ---------------------------------------------------------------------------
// One line, several markets.
// ---------------------------------------------------------------------------

/// **Structure is authored once; money is authored per market.** Three markets
/// under one line are one logical line, three stable variants, three monetary
/// versions -- and repricing one of them moves nothing else.
#[tokio::test]
async fn three_markets_are_priced_under_one_line_and_one_edit_moves_one_amount() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let line = create_flat_line(&h, plan_id).await;
    let version = version_of(&line);

    let mut prices = Vec::new();
    for (currency, region, amount, key) in [
        ("EUR", "EU", 900, "m-eur-eu"),
        ("USD", "US", 1_000, "m-usd-us"),
        ("USD", "us-east", 1_100, "m-usd-us-east"),
    ] {
        let response = post_price(
            &h,
            plan_id,
            &version,
            flat_price_body(currency, region, amount),
            key,
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "{currency}/{region}"
        );
        assert!(location_of(&response).is_some(), "a create names its row");
        prices.push((etag_of(&response), body_json(response).await));
    }

    let read = read_line(&h, plan_id, &version).await;
    assert_eq!(
        read["charge_line_id"], line["charge_line_id"],
        "one logical line"
    );
    let listed = read["prices"].as_array().expect("prices");
    assert_eq!(listed.len(), 3, "three monetary versions: {read}");
    let variants: std::collections::BTreeSet<&str> = listed
        .iter()
        .map(|price| price["market_price_id"].as_str().expect("a variant id"))
        .collect();
    assert_eq!(variants.len(), 3, "three distinct market variants");
    assert!(
        listed
            .iter()
            .all(|price| price["line_version_id"] == read["line_version_id"]),
        "every price names the exact structure version it is priced against"
    );

    // Reprice USD/US alone, under its own tag.
    let (tag, usd_us) = &prices[1];
    let price_id = usd_us["price_id"].as_str().expect("id");
    let response = h
        .allowed()
        .send(with_headers(
            "PATCH",
            &format!("/bss-pricing/v1/plans/{plan_id}/prices/{price_id}"),
            Some(serde_json::json!({ "money": { "amount_minor": 1_250 } })),
            &[(
                "if-match",
                tag.as_deref().expect("the create answered a tag"),
            )],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);

    let after = read_line(&h, plan_id, &version).await;
    let amount_of = |market: &serde_json::Value| {
        after["prices"]
            .as_array()
            .expect("prices")
            .iter()
            .find(|price| price["market_price_id"] == market["market_price_id"])
            .map(|price| price["money"]["amount_minor"].clone())
    };
    assert_eq!(amount_of(usd_us), Some(serde_json::json!(1_250)));
    assert_eq!(
        amount_of(&prices[0].1),
        Some(serde_json::json!(900)),
        "EUR/EU untouched"
    );
    assert_eq!(
        amount_of(&prices[2].1),
        Some(serde_json::json!(1_100)),
        "USD/us-east untouched"
    );
    assert_eq!(
        after["structure"], read["structure"],
        "and so is the structure"
    );
    assert_eq!(
        after["row_version"], read["row_version"],
        "a monetary edit does not move the line version's own tag"
    );
    for market in &prices {
        assert_eq!(
            after["prices"]
                .as_array()
                .expect("prices")
                .iter()
                .filter(|price| price["market_price_id"] == market.1["market_price_id"])
                .count(),
            1,
            "the variants are stable across the edit"
        );
    }
}

// ---------------------------------------------------------------------------
// Identity.
// ---------------------------------------------------------------------------

/// A fresh id does not buy a second line on the same eight axes.
#[tokio::test]
async fn a_second_line_on_the_same_axes_is_a_duplicate_whatever_its_id() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    create_flat_line(&h, plan_id).await;

    // A different idempotency key, so this is a *new* request and not a replay.
    let response = post_line(&h, plan_id, flat_line_body("one_time"), "new-line-2").await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(response).await, "DUPLICATE_SCOPE_KEY");

    // A different axis is a different line.
    let other = post_line(&h, plan_id, flat_line_body("recurring"), "new-line-3").await;
    assert_eq!(other.status(), StatusCode::CREATED);
}

/// One market of a line holds one draft price.
#[tokio::test]
async fn a_second_price_on_the_same_market_is_a_duplicate_variant() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let version = version_of(&create_flat_line(&h, plan_id).await);

    let first = post_price(
        &h,
        plan_id,
        &version,
        flat_price_body("USD", "US", 1_000),
        "v-1",
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let again = post_price(
        &h,
        plan_id,
        &version,
        flat_price_body("USD", "US", 2_000),
        "v-2",
    )
    .await;
    assert_eq!(again.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(again).await, "DUPLICATE_SCOPE_KEY");
    assert_eq!(price_rows(&h, plan_id).await.len(), 1);
}

// ---------------------------------------------------------------------------
// Preconditions.
// ---------------------------------------------------------------------------

/// A line's structure is the plan's content: its writes assert the plan revision's
/// tag and move it, so a tag read before somebody else's write is stale.
#[tokio::test]
async fn a_structural_write_under_a_stale_plan_tag_is_refused() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let stale = h.plan_etag(plan_id).await;
    let version = version_of(&create_flat_line(&h, plan_id).await);
    assert_ne!(
        h.plan_etag(plan_id).await,
        stale,
        "drafting a line moved the plan's tag"
    );

    for (method, path, body) in [
        (
            "POST",
            lines_path(plan_id),
            Some(flat_line_body("recurring")),
        ),
        (
            "PATCH",
            line_path(plan_id, &version),
            Some(serde_json::json!({ "structure": { "model_kind": "flat" } })),
        ),
        ("DELETE", line_path(plan_id, &version), None),
    ] {
        let response = h
            .allowed()
            .send(with_headers(
                method,
                &path,
                body,
                &[
                    ("if-match", stale.as_str()),
                    ("idempotency-key", "stale-tag"),
                ],
            ))
            .await;
        assert_eq!(response.status(), StatusCode::CONFLICT, "{method}");
        assert_eq!(problem_code(response).await, "STALE_VERSION", "{method}");
    }
    let lines = body_json(
        h.allowed()
            .send(request("GET", &lines_path(plan_id), None))
            .await,
    )
    .await;
    assert_eq!(
        lines["items"].as_array().expect("items").len(),
        1,
        "nothing moved"
    );
}

/// No tag at all is not a write either.
#[tokio::test]
async fn a_structural_write_without_a_tag_is_refused() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let response = h
        .allowed()
        .send(with_headers(
            "POST",
            &lines_path(plan_id),
            Some(flat_line_body("one_time")),
            &[("idempotency-key", "no-tag")],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    refused_by(
        &body_json(response).await,
        "invalid_argument",
        "If-Match is required",
    );
}

// ---------------------------------------------------------------------------
// Idempotency.
// ---------------------------------------------------------------------------

/// The same key and body is answered the recorded response -- the same ids -- and
/// the same key with a different body is refused.
#[tokio::test]
async fn both_creates_replay_the_same_ids_and_refuse_a_changed_body() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;

    // The line. The replay carries the tag the first call was made under, which is
    // stale by then: a recorded response is answered before the precondition is
    // judged, which is what makes a retry safe.
    let tag = h.plan_etag(plan_id).await;
    let send_line = |body: serde_json::Value| {
        let tag = tag.clone();
        let client = h.allowed();
        async move {
            client
                .send(with_headers(
                    "POST",
                    &lines_path(plan_id),
                    Some(body),
                    &[("if-match", tag.as_str()), ("idempotency-key", "line-key")],
                ))
                .await
        }
    };
    let first = send_line(flat_line_body("one_time")).await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let first = body_json(first).await;
    let replay = send_line(flat_line_body("one_time")).await;
    assert_eq!(replay.status(), StatusCode::CREATED);
    assert!(
        location_of(&replay).is_some(),
        "a replay names the line too"
    );
    assert_eq!(
        body_json(replay).await,
        first,
        "the recorded line, id for id"
    );
    let changed = send_line(flat_line_body("recurring")).await;
    assert_eq!(changed.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(changed).await, "IDEMPOTENCY_PAYLOAD_MISMATCH");

    // The price.
    let version = version_of(&first);
    let one = post_price(
        &h,
        plan_id,
        &version,
        flat_price_body("USD", "US", 1_000),
        "p-key",
    )
    .await;
    assert_eq!(one.status(), StatusCode::CREATED);
    let one = body_json(one).await;
    let two = post_price(
        &h,
        plan_id,
        &version,
        flat_price_body("USD", "US", 1_000),
        "p-key",
    )
    .await;
    assert_eq!(two.status(), StatusCode::CREATED);
    assert_eq!(body_json(two).await, one, "the recorded price, id for id");
    let other = post_price(
        &h,
        plan_id,
        &version,
        flat_price_body("USD", "US", 9),
        "p-key",
    )
    .await;
    assert_eq!(other.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(other).await, "IDEMPOTENCY_PAYLOAD_MISMATCH");
    assert_eq!(
        price_rows(&h, plan_id).await.len(),
        1,
        "one row, however many retries"
    );
}

// ---------------------------------------------------------------------------
// Authorization and tenancy.
// ---------------------------------------------------------------------------

/// Every line route is denied by the gate, with the store unchanged.
#[tokio::test]
async fn every_line_route_is_denied_with_the_store_unchanged() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let version = version_of(&create_flat_line(&h, plan_id).await);
    let tag = h.plan_etag(plan_id).await;
    let before = read_line(&h, plan_id, &version).await;

    for (method, path, body) in [
        (
            "POST",
            lines_path(plan_id),
            Some(flat_line_body("recurring")),
        ),
        ("GET", lines_path(plan_id), None),
        ("GET", line_path(plan_id, &version), None),
        (
            "PATCH",
            line_path(plan_id, &version),
            Some(serde_json::json!({ "structure": { "model_kind": "per_unit" } })),
        ),
        ("DELETE", line_path(plan_id, &version), None),
        (
            "POST",
            line_prices_path(plan_id, &version),
            Some(flat_price_body("USD", "US", 1_000)),
        ),
        ("GET", line_prices_path(plan_id, &version), None),
    ] {
        let response = h
            .denied()
            .send(with_headers(
                method,
                &path,
                body,
                &[("if-match", tag.as_str()), ("idempotency-key", "denied")],
            ))
            .await;
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {path}");
    }
    assert_eq!(
        read_line(&h, plan_id, &version).await,
        before,
        "the line did not move"
    );
    assert_eq!(h.plan_etag(plan_id).await, tag, "nor the plan's tag");
    assert!(
        price_rows(&h, plan_id).await.is_empty(),
        "and no price was filed"
    );
}

/// A line under another plan's URL, or reached by another tenant, answers exactly
/// like an absent one -- and the owner's identical call is the positive control.
#[tokio::test]
async fn a_line_under_another_plans_url_or_another_tenant_is_absent() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let other_plan = drafted_plan(&h).await;
    let version = version_of(&create_flat_line(&h, plan_id).await);
    let absent = Uuid::now_v7().to_string();

    // Wrong nesting: the version exists, under a different plan of the same tenant.
    let other_tag = h.plan_etag(other_plan).await;
    for (method, path, body) in [
        ("GET", line_path(other_plan, &version), None),
        (
            "PATCH",
            line_path(other_plan, &version),
            Some(serde_json::json!({ "structure": { "model_kind": "flat" } })),
        ),
        ("DELETE", line_path(other_plan, &version), None),
        (
            "POST",
            line_prices_path(other_plan, &version),
            Some(flat_price_body("USD", "US", 1_000)),
        ),
    ] {
        let nested = h
            .allowed()
            .send(with_headers(
                method,
                &path,
                body.clone(),
                &[
                    ("if-match", other_tag.as_str()),
                    ("idempotency-key", "nesting"),
                ],
            ))
            .await;
        assert_eq!(
            nested.status(),
            StatusCode::NOT_FOUND,
            "{method} under the wrong plan"
        );
        let nested = body_json(nested).await;
        // And it reads like a version that does not exist at all.
        let missing = h
            .allowed()
            .send(with_headers(
                method,
                &path.replace(&version, &absent),
                body,
                &[
                    ("if-match", other_tag.as_str()),
                    ("idempotency-key", "nesting-absent"),
                ],
            ))
            .await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND, "{method} absent");
        assert_eq!(
            nested["type"],
            body_json(missing).await["type"],
            "{method}: a version elsewhere and no version are one answer"
        );
    }

    // Another tenant, at this tenant's line.
    let tag = h.plan_etag(plan_id).await;
    for (method, path, body) in [
        ("GET", line_path(plan_id, &version), None),
        (
            "PATCH",
            line_path(plan_id, &version),
            Some(serde_json::json!({ "structure": { "model_kind": "flat" } })),
        ),
        ("DELETE", line_path(plan_id, &version), None),
        (
            "POST",
            line_prices_path(plan_id, &version),
            Some(flat_price_body("USD", "US", 1_000)),
        ),
    ] {
        let foreign = h
            .other_tenant()
            .send(with_headers(
                method,
                &path,
                body,
                &[("if-match", tag.as_str()), ("idempotency-key", "foreign")],
            ))
            .await;
        assert_eq!(
            foreign.status(),
            StatusCode::NOT_FOUND,
            "{method} from another tenant"
        );
    }
    assert!(
        price_rows(&h, plan_id).await.is_empty(),
        "nothing was filed"
    );

    // The owner's identical delete lands -- the unpriced line is deletable.
    let own = h
        .allowed()
        .send(with_headers(
            "DELETE",
            &line_path(plan_id, &version),
            None,
            &[("if-match", tag.as_str())],
        ))
        .await;
    assert_eq!(own.status(), StatusCode::NO_CONTENT);
    let gone = h
        .allowed()
        .send(request("GET", &line_path(plan_id, &version), None))
        .await;
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// Each door refuses the other's fields.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_structure_refuses_money_and_a_price_refuses_structure() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;

    for member in ["currency", "amount_minor", "region", "tax_inclusive"] {
        let mut body = flat_line_body("one_time");
        body["structure"][member] = serde_json::json!("USD");
        let response = post_line(&h, plan_id, body, &format!("money-on-line-{member}")).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{member} on a structure"
        );
        refused_by(
            &body_json(response).await,
            "invalid_argument",
            &format!("unknown field `{member}`"),
        );
    }
    // Derived and derived-looking members are refused too, by name.
    for (member, value) in [
        ("meter", serde_json::json!("cloudlets")),
        ("meter", serde_json::Value::Null),
    ] {
        let mut body = flat_line_body("one_time");
        body["structure"][member] = value;
        let response = post_line(&h, plan_id, body, "meter-on-line").await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "an authored {member}"
        );
    }
    let mut body = flat_line_body("one_time");
    body["charge_kinds"] = serde_json::json!(["one_time"]);
    let response = post_line(&h, plan_id, body, "derived-kinds").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    refused_by(
        &body_json(response).await,
        "invalid_argument",
        "unknown field `charge_kinds`",
    );

    let version = version_of(&create_flat_line(&h, plan_id).await);
    for member in ["model_kind", "tiers", "package_size", "billing_granularity"] {
        let mut body = flat_price_body("USD", "US", 1_000);
        body["money"][member] = serde_json::json!("flat");
        let response = post_price(
            &h,
            plan_id,
            &version,
            body,
            &format!("structure-on-price-{member}"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "{member} on a price"
        );
        refused_by(
            &body_json(response).await,
            "invalid_argument",
            &format!("unknown field `{member}`"),
        );
    }
    assert!(
        price_rows(&h, plan_id).await.is_empty(),
        "a refused request writes nothing"
    );
}

// ---------------------------------------------------------------------------
// Tier geometry is the line's; tier rates are the market's.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn tier_rates_fill_the_lines_geometry_and_survive_an_edit_of_it() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let created = post_line(&h, plan_id, tiered_line_body(), "tiered-line").await;
    let status = created.status();
    let line = body_json(created).await;
    assert_eq!(status, StatusCode::CREATED, "{line}");
    let version = version_of(&line);
    assert_eq!(
        line["structure"]["meter"], "cloudlets",
        "the meter derives from the SKU"
    );

    // A rate count that is not the tier count cannot be stored against the ladder.
    let short = post_price(
        &h,
        plan_id,
        &version,
        serde_json::json!({
            "currency": "USD", "region": "US",
            "money": { "tier_rates_nano_minor": [500, 400] }
        }),
        "short-ladder",
    )
    .await;
    assert_eq!(short.status(), StatusCode::BAD_REQUEST);
    assert_eq!(problem_code(short).await, "MARKET_TIER_RATE_COUNT_MISMATCH");

    // Two markets, each with its own rates on the one shared ladder.
    for (currency, region, rates, key) in [
        ("USD", "US", [500, 400, 300], "ladder-usd"),
        ("EUR", "EU", [450, 350, 250], "ladder-eur"),
    ] {
        let response = post_price(
            &h,
            plan_id,
            &version,
            serde_json::json!({
                "currency": currency, "region": region,
                "money": { "tier_rates_nano_minor": rates }
            }),
            key,
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED, "{currency}");
    }

    // Move the bounds. Both markets' rates point into this geometry, so the edit
    // has to carry them across rather than be refused by the key between them.
    let tag = h.plan_etag(plan_id).await;
    let mut structure = tiered_line_body()["structure"].clone();
    structure["tiers"] = serde_json::json!([
        {"from_qty": 0, "to_qty": 50},
        {"from_qty": 50, "to_qty": 500},
        {"from_qty": 500, "to_qty": null}
    ]);
    let edited = h
        .allowed()
        .send(with_headers(
            "PATCH",
            &line_path(plan_id, &version),
            Some(serde_json::json!({ "structure": structure })),
            &[("if-match", tag.as_str())],
        ))
        .await;
    let status = edited.status();
    let edited = body_json(edited).await;
    assert_eq!(status, StatusCode::OK, "{edited}");
    assert_eq!(edited["structure"]["tiers"][1]["from_qty"], 50);
    let rates: Vec<&serde_json::Value> = edited["prices"]
        .as_array()
        .expect("prices")
        .iter()
        .map(|price| &price["money"]["tier_rates_nano_minor"])
        .collect();
    assert_eq!(rates.len(), 2);
    assert!(
        rates.contains(&&serde_json::json!([500, 400, 300]))
            && rates.contains(&&serde_json::json!([450, 350, 250])),
        "each market kept its own rates on the moved ladder: {edited}"
    );

    // And the resolved row joins the two halves in quantity order.
    let rows = price_rows(&h, plan_id).await;
    assert_eq!(rows.len(), 2);
    for row in rows {
        let bounds: Vec<u64> = row.row.bands.iter().map(|band| band.from_qty).collect();
        assert_eq!(bounds, vec![0, 50, 500]);
    }
}

// ---------------------------------------------------------------------------
// Delete.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_priced_line_is_not_deletable_until_its_prices_are_gone() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let version = version_of(&create_flat_line(&h, plan_id).await);
    let price = post_price(
        &h,
        plan_id,
        &version,
        flat_price_body("USD", "US", 1_000),
        "only",
    )
    .await;
    let price_tag = etag_of(&price).expect("a tag");
    let price_id = body_json(price).await["price_id"]
        .as_str()
        .expect("id")
        .to_owned();

    let delete_line = || async {
        let tag = h.plan_etag(plan_id).await;
        h.allowed()
            .send(with_headers(
                "DELETE",
                &line_path(plan_id, &version),
                None,
                &[("if-match", tag.as_str())],
            ))
            .await
    };
    let refused = delete_line().await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(problem_code(refused).await, "CHARGE_LINE_IN_USE");

    let gone = h
        .allowed()
        .send(with_headers(
            "DELETE",
            &format!("/bss-pricing/v1/plans/{plan_id}/prices/{price_id}"),
            None,
            &[("if-match", price_tag.as_str())],
        ))
        .await;
    assert!(gone.status().is_success(), "{:?}", gone.status());

    let deleted = delete_line().await;
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);
    let lines = body_json(
        h.allowed()
            .send(request("GET", &lines_path(plan_id), None))
            .await,
    )
    .await;
    assert_eq!(
        lines["items"],
        serde_json::json!([]),
        "the logical line went with its only version"
    );
    // The axes are free again.
    let again = post_line(&h, plan_id, flat_line_body("one_time"), "after-delete").await;
    assert_eq!(again.status(), StatusCode::CREATED);
}

/// A line drafted ahead of its market prices is **seen by publish**, and a
/// publish that would freeze it unpriced is refused.
///
/// The case the line-first door made possible and the resolved row plane cannot
/// reach: a line with no monetary version has no `pricing_price` row at all, so
/// a publish subject assembled from rows alone would freeze a line that sells in
/// no market and say nothing. `infra::publish::assemble` reads the revision's
/// line versions beside its rows for exactly this.
#[tokio::test]
async fn a_line_with_no_market_price_blocks_the_publish() {
    let h = Harness::new().await;
    let plan_id = Uuid::now_v7();
    let seeded = rest_support::seed_publishable_plan(&h, plan_id).await;

    let mut body = flat_line_body("one_time");
    body["scope_key"]["phase"] = serde_json::json!(seeded.phase.get());
    let created = post_line(&h, plan_id, body, "unpriced-line").await;
    assert_eq!(created.status(), StatusCode::CREATED);

    let etag = h.plan_etag(plan_id).await;
    let response = h
        .allowed()
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/plans/{plan_id}/publish"),
            None,
            &[("if-match", etag.as_str())],
        ))
        .await;

    assert!(
        response.status().is_client_error(),
        "an unpriced line cannot publish: {}",
        response.status()
    );
    let problem = body_json(response).await;
    assert!(
        problem.to_string().contains("LINE_MARKET_PRICE_MISSING"),
        "the refusal names the line that sells in no market: {problem}"
    );
}

/// The line list pages by **logical line**: a page of two is two lines however
/// many markets hang under them, the walk visits every line exactly once, and a
/// page that ends on the last line says so rather than sending the client for an
/// empty one.
#[tokio::test]
async fn the_line_list_pages_by_logical_line_and_stops_at_the_exact_boundary() {
    let h = Harness::new().await;
    let plan_id = Uuid::now_v7();
    seed_draft_plan(&h, plan_id).await;

    // Three lines; the first carries two markets, so a row-counting page of two
    // would end inside it.
    let mut versions = Vec::new();
    // Three distinct lines: two kinds, and a second eligibility class of one of
    // them — an axis of the line key, so it is a line of its own.
    for (nth, (kind, eligibility)) in [
        ("one_time", "all_subscriptions"),
        ("recurring", "all_subscriptions"),
        ("recurring", "new_subscriptions_only"),
    ]
    .into_iter()
    .enumerate()
    {
        let mut body = flat_line_body(kind);
        body["scope_key"]["price_eligibility"] = serde_json::json!(eligibility);
        let created = post_line(&h, plan_id, body, &format!("page-{nth}")).await;
        let status = created.status();
        let body = body_json(created).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        versions.push(body["line_version_id"].as_str().expect("id").to_owned());
    }
    for (nth, (currency, region)) in [("EUR", "EU"), ("USD", "US")].into_iter().enumerate() {
        let priced = post_price(
            &h,
            plan_id,
            &versions[0],
            flat_price_body(currency, region, 1_000),
            &format!("page-price-{nth}"),
        )
        .await;
        assert_eq!(priced.status(), StatusCode::CREATED);
    }

    let read = |query: String| {
        let h = &h;
        async move {
            let response = h
                .allowed()
                .send(request(
                    "GET",
                    &format!("{}?{query}", lines_path(plan_id)),
                    None,
                ))
                .await;
            assert_eq!(response.status(), StatusCode::OK);
            body_json(response).await
        }
    };

    let first = read("limit=2".to_owned()).await;
    assert_eq!(first["items"].as_array().unwrap().len(), 2, "{first}");
    let cursor = first["page_info"]["next_cursor"]
        .as_str()
        .expect("a third line is still ahead")
        .to_owned();
    let second = read(format!("limit=2&cursor={cursor}")).await;
    assert_eq!(second["items"].as_array().unwrap().len(), 1, "{second}");
    assert!(
        second["page_info"]["next_cursor"].is_null(),
        "the last page names no successor: {second}"
    );

    let mut seen: Vec<String> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .map(|item| item["line_version_id"].as_str().unwrap().to_owned())
        .collect();
    seen.sort();
    let mut expected = versions.clone();
    expected.sort();
    assert_eq!(seen, expected, "every line exactly once");
    let two_market_line = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(second["items"].as_array().unwrap())
        .find(|item| item["line_version_id"] == versions[0])
        .expect("the priced line");
    assert_eq!(
        two_market_line["prices"].as_array().unwrap().len(),
        2,
        "a page boundary never cuts a line's markets"
    );

    // The exact boundary: a page of three over three lines is the last page.
    let exact = read("limit=3".to_owned()).await;
    assert_eq!(exact["items"].as_array().unwrap().len(), 3);
    assert!(exact["page_info"]["next_cursor"].is_null(), "{exact}");

    // And a page of zero never advances, so it is refused.
    let zero = h
        .allowed()
        .send(request(
            "GET",
            &format!("{}?limit=0", lines_path(plan_id)),
            None,
        ))
        .await;
    assert_eq!(zero.status(), StatusCode::BAD_REQUEST);
}
