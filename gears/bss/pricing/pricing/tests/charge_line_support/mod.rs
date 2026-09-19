//! Line-first authoring helpers, through the real authenticated router.
//!
//! Nothing here touches the database: a helper that inserted a line directly
//! would keep passing after the route stopped being able to author one.

#![allow(dead_code)]

use axum::body::Body;
use axum::http::{Response, StatusCode};
use uuid::Uuid;

use crate::rest_support::{Harness, OFFER_SKU, body_json, seeded_phase, with_headers};

/// `…/plans/{plan_id}/charge-lines`.
pub fn lines_path(plan_id: Uuid) -> String {
    format!("/bss-pricing/v1/plans/{plan_id}/charge-lines")
}

/// `…/plans/{plan_id}/charge-lines/{line_version_id}`.
pub fn line_path(plan_id: Uuid, line_version_id: &str) -> String {
    format!("{}/{line_version_id}", lines_path(plan_id))
}

/// `…/charge-lines/{line_version_id}/prices`.
pub fn line_prices_path(plan_id: Uuid, line_version_id: &str) -> String {
    format!("{}/prices", line_path(plan_id, line_version_id))
}

/// A one-time flat line on the seeded phase and the offer SKU.
pub fn flat_line_body(charge_kind: &str) -> serde_json::Value {
    serde_json::json!({
        "scope_key": {
            "phase": seeded_phase().get(), "sku_id": OFFER_SKU,
            "price_eligibility": "all_subscriptions",
            "charge_kind": charge_kind, "cohort": null,
            "dimension_key": ""
        },
        "structure": {"model_kind": "flat"}
    })
}

/// POST a line under the plan's **current** tag.
pub async fn post_line(
    h: &Harness,
    plan_id: Uuid,
    body: serde_json::Value,
    idempotency_key: &str,
) -> Response<Body> {
    let etag = h.plan_etag(plan_id).await;
    h.allowed()
        .send(with_headers(
            "POST",
            &lines_path(plan_id),
            Some(body),
            &[
                ("if-match", etag.as_str()),
                ("idempotency-key", idempotency_key),
            ],
        ))
        .await
}

/// Draft one flat `one_time` line and hand back its representation.
pub async fn create_flat_line(h: &Harness, plan_id: Uuid) -> serde_json::Value {
    let response = post_line(h, plan_id, flat_line_body("one_time"), "new-line-1").await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "the line must draft: {body}");
    body
}

/// A flat market price body.
pub fn flat_price_body(currency: &str, region: &str, amount_minor: i64) -> serde_json::Value {
    serde_json::json!({
        "currency": currency,
        "region": region,
        "money": {"amount_minor": amount_minor},
        "market_policy": {"tax_inclusive": false}
    })
}

/// POST a market price under a line version.
pub async fn post_price(
    h: &Harness,
    plan_id: Uuid,
    line_version_id: &str,
    body: serde_json::Value,
    idempotency_key: &str,
) -> Response<Body> {
    h.allowed()
        .send(with_headers(
            "POST",
            &line_prices_path(plan_id, line_version_id),
            Some(body),
            &[("idempotency-key", idempotency_key)],
        ))
        .await
}
