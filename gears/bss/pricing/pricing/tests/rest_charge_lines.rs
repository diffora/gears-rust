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

/// A graduated usage line on a metered SKU. Its ladder is each market's own.
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
            "billing_granularity": "per_hour"
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
    for member in ["model_kind", "package_size", "billing_granularity"] {
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
// The ladder is the market's: bounds and rates together, per currency.
// ---------------------------------------------------------------------------

/// Post one market's ladder under `version` and answer the created price.
async fn post_ladder(
    h: &Harness,
    plan_id: Uuid,
    version: &str,
    (currency, region): (&str, &str),
    tiers: serde_json::Value,
    key: &str,
) -> axum::response::Response {
    post_price(
        h,
        plan_id,
        version,
        serde_json::json!({
            "currency": currency, "region": region,
            "money": { "tiers": tiers }
        }),
        key,
    )
    .await
}

/// **One model, different ladders.** The line says the charge is `graduated`;
/// each market says where its own break-points are and what each band costs.
/// EUR has two tiers and USD three, on different bounds, under one line version.
#[tokio::test]
async fn each_market_of_one_line_carries_its_own_ladder() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let created = post_line(&h, plan_id, tiered_line_body(), "tiered-line").await;
    let status = created.status();
    let line = body_json(created).await;
    assert_eq!(status, StatusCode::CREATED, "{line}");
    let version = version_of(&line);
    assert!(
        line["structure"].get("tiers").is_none(),
        "a line carries no ladder: {line}"
    );

    let eur = serde_json::json!([
        {"from_qty": 0, "to_qty": 100, "rate_nano_minor": 450},
        {"from_qty": 100, "to_qty": null, "rate_nano_minor": 350}
    ]);
    let usd = serde_json::json!([
        {"from_qty": 0, "to_qty": 50, "rate_nano_minor": 500},
        {"from_qty": 50, "to_qty": 500, "rate_nano_minor": 400},
        {"from_qty": 500, "to_qty": null, "rate_nano_minor": 300}
    ]);
    for (market, tiers, key) in [
        (("EUR", "EU"), eur.clone(), "ladder-eur"),
        (("USD", "US"), usd.clone(), "ladder-usd"),
    ] {
        let response = post_ladder(&h, plan_id, &version, market, tiers, key).await;
        let status = response.status();
        let body = body_json(response).await;
        assert_eq!(status, StatusCode::CREATED, "{market:?}: {body}");
    }

    // Each market reads back the ladder it was given, whole.
    let read = read_line(&h, plan_id, &version).await;
    let prices = read["prices"].as_array().expect("prices");
    assert_eq!(prices.len(), 2, "{read}");
    for (currency, expected) in [("EUR", &eur), ("USD", &usd)] {
        let price = prices
            .iter()
            .find(|price| price["currency"] == currency)
            .unwrap_or_else(|| panic!("{currency} is priced: {read}"));
        assert_eq!(&price["money"]["tiers"], expected, "{currency}");
        assert!(
            price["money"].get("tier_rates_nano_minor").is_none(),
            "a rate sits beside the bound it prices: {price}"
        );
    }

    // And the resolved rows differ in shape under the one line version.
    let rows = price_rows(&h, plan_id).await;
    assert_eq!(rows.len(), 2);
    let mut ladders: Vec<Vec<u64>> = rows
        .iter()
        .map(|row| row.row.bands.iter().map(|band| band.from_qty).collect())
        .collect();
    ladders.sort();
    assert_eq!(ladders, vec![vec![0, 50, 500], vec![0, 100]]);
}

/// **A structure edit leaves every market's ladder alone while the line stays
/// tiered.** The ladder is not the line's to move any more, so nothing has to be
/// carried across the edit, and nothing is.
#[tokio::test]
async fn a_structure_edit_of_a_tiered_line_leaves_its_markets_ladders_alone() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let line = body_json(post_line(&h, plan_id, tiered_line_body(), "tiered-line").await).await;
    let version = version_of(&line);
    let tiers = serde_json::json!([
        {"from_qty": 0, "to_qty": 100, "rate_nano_minor": 450},
        {"from_qty": 100, "to_qty": null, "rate_nano_minor": 350}
    ]);
    let priced = post_ladder(&h, plan_id, &version, ("EUR", "EU"), tiers.clone(), "eur").await;
    assert_eq!(priced.status(), StatusCode::CREATED);

    let tag = h.plan_etag(plan_id).await;
    let mut structure = tiered_line_body()["structure"].clone();
    structure["model_kind"] = serde_json::json!("volume");
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
    assert_eq!(edited["structure"]["model_kind"], "volume");
    assert_eq!(edited["prices"][0]["money"]["tiers"], tiers, "{edited}");
}

/// **Leaving `graduated` / `volume` takes the ladders off.** A `per_unit` line
/// has no ladder to keep, and a band left hanging off one is money no rule reads.
#[tokio::test]
async fn a_line_leaving_the_tiered_kinds_drops_its_markets_ladders() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;
    let line = body_json(post_line(&h, plan_id, tiered_line_body(), "tiered-line").await).await;
    let version = version_of(&line);
    let tiers = serde_json::json!([
        {"from_qty": 0, "to_qty": null, "rate_nano_minor": 450}
    ]);
    let priced = post_ladder(&h, plan_id, &version, ("EUR", "EU"), tiers, "eur").await;
    assert_eq!(priced.status(), StatusCode::CREATED);

    let tag = h.plan_etag(plan_id).await;
    let mut structure = tiered_line_body()["structure"].clone();
    structure["model_kind"] = serde_json::json!("per_unit");
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
    assert!(
        edited["prices"][0]["money"]["tiers"].is_null(),
        "a per_unit line's market carries no ladder: {edited}"
    );
    let rows = price_rows(&h, plan_id).await;
    assert!(rows.iter().all(|row| row.row.bands.is_empty()));
}

/// **The old shape is refused by name, never dropped.** A client that keeps
/// sending the line's `tiers` or the market's positional rates and hears nothing
/// would believe both are still honored.
#[tokio::test]
async fn the_split_ladder_shape_is_refused_by_name() {
    let h = Harness::new().await;
    let plan_id = drafted_plan(&h).await;

    let mut body = tiered_line_body();
    body["structure"]["tiers"] = serde_json::json!([{"from_qty": 0, "to_qty": null}]);
    let response = post_line(&h, plan_id, body, "ladder-on-line").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    refused_by(
        &body_json(response).await,
        "invalid_argument",
        "unknown field `tiers`",
    );

    let line = body_json(post_line(&h, plan_id, tiered_line_body(), "tiered-line").await).await;
    let version = version_of(&line);
    let response = post_price(
        &h,
        plan_id,
        &version,
        serde_json::json!({
            "currency": "USD", "region": "US",
            "money": { "tier_rates_nano_minor": [500, 400, 300] }
        }),
        "positional-rates",
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    refused_by(
        &body_json(response).await,
        "invalid_argument",
        "unknown field `tier_rates_nano_minor`",
    );
    assert!(
        price_rows(&h, plan_id).await.is_empty(),
        "a refused request writes nothing"
    );
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

// ---------------------------------------------------------------------------
// One scenario, end to end, through the real router.
// ---------------------------------------------------------------------------

/// The authoring half of the end-to-end scenario: a plan with a trial that
/// converts to a paid phase, one recurring line per phase, each in EUR/EU and
/// USD/US. Returns the plan, the two line versions and the four price ids.
///
/// The trial is free by an **explicit zero**, not by an absent price.
async fn author_two_phases_two_markets(h: &Harness) -> (Uuid, Vec<String>, Vec<String>) {
    use bss_pricing::domain::plan_shape::{PhaseKind, PlanPhase};
    use bss_pricing::domain::scope_key::{PhaseId, PlanId};

    let plan_id = Uuid::now_v7();
    let shape = rest_support::seed_publishable_shape(h, plan_id).await;
    let paid = shape.phase;
    let trial = PhaseId::new(Uuid::now_v7());
    h.state
        .shapes
        .replace_phases(
            &h.scope(),
            h.tenant,
            PlanId::new(plan_id),
            shape.revision,
            shape.version,
            vec![
                PlanPhase {
                    phase_id: trial,
                    kind: PhaseKind::Trial,
                    display_name: None,
                    ordinal: 0,
                    converts_to_phase_id: Some(paid),
                    phase_duration_days: Some(14),
                },
                PlanPhase {
                    phase_id: paid,
                    kind: PhaseKind::Evergreen,
                    display_name: None,
                    ordinal: 1,
                    converts_to_phase_id: None,
                    phase_duration_days: None,
                },
            ],
            rest_support::seed_stamp(),
        )
        .await
        .expect("a trial that converts to a paid phase");

    // 2. One recurring line per phase, each in EUR/EU and USD/US. The trial is
    //    free by an **explicit zero**, not by an absent price.
    let mut price_ids = Vec::new();
    let mut versions = Vec::new();
    for (nth, (phase, eur, usd)) in [(trial, 0, 0), (paid, 9_900, 10_900)]
        .into_iter()
        .enumerate()
    {
        let mut body = flat_line_body("recurring");
        body["scope_key"]["phase"] = serde_json::json!(phase.get());
        body["structure"]["gl_code_ref"] = serde_json::json!("4000");
        body["structure"]["billing_timing"] = serde_json::json!("advance");
        body["structure"]["billing_anchor_policy"] = serde_json::json!("calendar_month");
        body["structure"]["proration_basis"] = serde_json::json!("calendar_days_actual");
        body["structure"]["credit_on_downgrade"] = serde_json::json!(false);
        let created = post_line(h, plan_id, body, &format!("e2e-line-{nth}")).await;
        let status = created.status();
        let line = body_json(created).await;
        assert_eq!(status, StatusCode::CREATED, "{line}");
        let version = line["line_version_id"].as_str().expect("id").to_owned();
        for (currency, region, amount) in [("EUR", "EU", eur), ("USD", "US", usd)] {
            let mut price = flat_price_body(currency, region, amount);
            price["market_policy"]["rounding_policy_ref"] = serde_json::json!("half_up");
            let priced = post_price(
                h,
                plan_id,
                &version,
                price,
                &format!("e2e-price-{nth}-{currency}"),
            )
            .await;
            let status = priced.status();
            let priced = body_json(priced).await;
            assert_eq!(status, StatusCode::CREATED, "{priced}");
            price_ids.push(priced["price_id"].as_str().expect("id").to_owned());
        }
        versions.push(version);
    }
    (plan_id, versions, price_ids)
}

/// An explicit `at_publish` intention per price: every market of every phase.
async fn schedule_at_publish_windows(h: &Harness, plan_id: Uuid, price_ids: &[String]) {
    for (nth, price_id) in price_ids.iter().enumerate() {
        let scheduled = h
            .allowed()
            .send(with_headers(
                "POST",
                &format!("/bss-pricing/v1/prices/{price_id}/windows"),
                Some(serde_json::json!({
                    "context": {"kind": "draft", "plan_revision": 0},
                    "start": {"kind": "at_publish"},
                    "reason_code": "launch"
                })),
                &[
                    ("if-match", h.plan_etag(plan_id).await.as_str()),
                    ("idempotency-key", &format!("e2e-window-{nth}")),
                ],
            ))
            .await;
        assert_eq!(
            scheduled.status(),
            StatusCode::CREATED,
            "{}",
            body_json(scheduled).await
        );
    }
}

/// Neither the shared structure nor the money moves in place once published,
/// and each refusal leaves the store exactly as it was.
async fn assert_frozen(h: &Harness, plan_id: Uuid, versions: &[String], price_ids: &[String]) {
    // 5. Frozen: neither half moves in place, and each refusal leaves the store
    //    exactly as it was.
    let lines_before = body_json(
        h.allowed()
            .send(request("GET", &lines_path(plan_id), None))
            .await,
    )
    .await;
    let restructure = h
        .allowed()
        .send(with_headers(
            "PATCH",
            &line_path(plan_id, &versions[1]),
            Some(serde_json::json!({ "structure": { "model_kind": "per_unit" } })),
            &[
                ("if-match", h.plan_etag(plan_id).await.as_str()),
                ("idempotency-key", "e2e-restructure"),
            ],
        ))
        .await;
    assert!(
        restructure.status().is_client_error(),
        "a frozen structure version is immutable: {}",
        restructure.status()
    );
    let reprice = h
        .allowed()
        .send(with_headers(
            "PATCH",
            &format!("/bss-pricing/v1/plans/{plan_id}/prices/{}", price_ids[3]),
            Some(serde_json::json!({ "money": { "amount_minor": 1 } })),
            &[("if-match", "\"1\"")],
        ))
        .await;
    assert!(
        reprice.status().is_client_error(),
        "a published monetary version is immutable: {}",
        reprice.status()
    );
    let lines_after = body_json(
        h.allowed()
            .send(request("GET", &lines_path(plan_id), None))
            .await,
    )
    .await;
    assert_eq!(lines_after, lines_before, "both refusals wrote nothing");
    assert_eq!(lines_after["items"].as_array().map(Vec::len), Some(2));
    for item in lines_after["items"].as_array().expect("lines") {
        assert_eq!(item["prices"].as_array().map(Vec::len), Some(2));
    }
}

/// **A trial + paid plan with no plan type, two markets, explicit windows —
/// authored, approved, published and then held frozen.**
///
/// Every step is a real route; the two seeds that are not (the plan shell and
/// its two-phase graph) use the repositories the plan routes themselves call.
/// What is asserted is the contract as a whole rather than any one door:
///
/// 1. the plan says what it charges from its lines (`charge_kinds`), with no
///    type anywhere;
/// 2. each phase carries its **own** complete line, and each line two markets;
/// 3. nothing publishes without an authored window per market per phase, and
///    nothing publishes on one signature;
/// 4. the reviewer is shown the normalized graph they sign for;
/// 5. once published, neither the shared structure nor the money moves in
///    place — each refusal is asserted by code **and** by the store being
///    exactly what it was.
///
/// **What this scenario cannot reach, recorded rather than faked:** a
/// *structural* change of a published line across its markets. No door authors
/// one — `PATCH …/charge-lines/{id}` refuses a frozen version, a plan revision
/// may add rows but not supersede them, and the supersession door is
/// single-market. `inst-sc-simultaneous` guards that state at publish; the unit
/// that would *produce* it lawfully is not built.
#[tokio::test]
async fn a_two_phase_two_market_plan_is_authored_approved_published_and_frozen() {
    const SUBMITTER: Uuid = Uuid::from_u128(0x5_b0);
    const APPROVER: Uuid = Uuid::from_u128(0xa_b0);

    let h = Harness::new().await;
    let (plan_id, versions, price_ids) = author_two_phases_two_markets(&h).await;

    // 3a. No window, no submit.
    let unscheduled = h
        .allowed_as(SUBMITTER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/plans/{plan_id}/publish"),
            None,
            &[("if-match", h.plan_etag(plan_id).await.as_str())],
        ))
        .await;
    assert!(unscheduled.status().is_client_error());
    assert!(
        body_json(unscheduled)
            .await
            .to_string()
            .contains("WINDOW_COVERAGE_MISSING"),
        "every market of every phase owes an authored window"
    );

    schedule_at_publish_windows(&h, plan_id, &price_ids).await;

    // 1. What the plan charges, read off its lines.
    let plan = body_json(
        h.allowed()
            .send(request(
                "GET",
                &format!("/bss-pricing/v1/plans/{plan_id}"),
                None,
            ))
            .await,
    )
    .await;
    assert_eq!(plan["charge_kinds"], serde_json::json!(["recurring"]));
    assert!(plan.get("billing_cycle").is_none());

    // 3b. Submit opens a unit; it does not publish.
    let submit_tag = h.plan_etag(plan_id).await;
    let submitted = h
        .allowed_as(SUBMITTER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/plans/{plan_id}/publish"),
            None,
            &[("if-match", submit_tag.as_str())],
        ))
        .await;
    let status = submitted.status();
    let submitted = body_json(submitted).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{submitted}");
    let approval_id = submitted["approval"]["approval_id"]
        .as_str()
        .expect("the unit")
        .to_owned();

    // 4. The reviewer's document carries the graph.
    let unit = body_json(
        h.allowed_as(APPROVER)
            .send(request(
                "GET",
                &format!("/bss-pricing/v1/approvals/{approval_id}"),
                None,
            ))
            .await,
    )
    .await;
    let pinned = &unit["pinned_content"];
    assert_eq!(pinned["charge_lines"].as_array().map(Vec::len), Some(2));
    assert_eq!(pinned["market_prices"].as_array().map(Vec::len), Some(4));
    assert_eq!(
        pinned["draft_window_entries"].as_array().map(Vec::len),
        Some(4)
    );

    let approved = h
        .allowed_as(APPROVER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{approval_id}/approve"),
            None,
            &[],
        ))
        .await;
    assert_eq!(approved.status(), StatusCode::OK);

    let published = h
        .allowed_as(SUBMITTER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/plans/{plan_id}/publish"),
            None,
            &[("if-match", submit_tag.as_str())],
        ))
        .await;
    let status = published.status();
    let published = body_json(published).await;
    assert_eq!(status, StatusCode::OK, "{published}");

    let rows = price_rows(&h, plan_id).await;
    assert_eq!(rows.len(), 4);
    assert!(
        rows.iter()
            .all(|row| row.lifecycle_state.as_str() == "published"),
        "both phases, both markets"
    );

    assert_frozen(&h, plan_id, &versions, &price_ids).await;
}
