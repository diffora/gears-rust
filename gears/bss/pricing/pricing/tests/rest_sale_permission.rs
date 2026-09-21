//! **The sale gate, end to end, over the real doors** (D-376 / P-D-176).
//!
//! Every other suite that reads the sellability document seeds
//! `sale_sku_ids: Vec::new()` straight into a projected delta, so predicate (6) is
//! `not_evaluable` in all of them and three things go unmeasured: that a publish
//! **freezes** the roster of SKUs a sale includes, that the gate asks the registry's
//! **current** state over that roster, and that flipping one member closes or
//! reopens the sale of everything that includes it while the pricing terms stand.
//!
//! One plan, one market, three lines: its own `offer`, a one-time charge on an
//! unmetered `component`, and a metered usage charge on another. The offer and one
//! component start closed for new sales. **Every authoring act succeeds** — the flag
//! is never read by authoring — and the gate then names both closed SKUs, then one,
//! then none, then one again, with the price rows byte-for-byte unchanged throughout.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod charge_line_support;
mod common;
mod rest_support;

use std::sync::Arc;

use axum::http::StatusCode;
use charge_line_support::{post_line, post_price};
use rest_support::{
    Harness, MutableCatalog, OFFER_SKU, body_json, price_rows, request, resource_sku, with_headers,
};
use uuid::Uuid;

/// The unmetered `component` the fixture catalog lists closed for new sales.
const CLOSED_COMPONENT: Uuid = Uuid::from_u128(5);
const SUBMITTER: Uuid = Uuid::from_u128(0x5_a1e);
const APPROVER: Uuid = Uuid::from_u128(0xa_a1e);

fn metered_component() -> Uuid {
    resource_sku("cloudlets")
}

fn set_sellable(catalog: &MutableCatalog, sku: Uuid, sellable: bool) {
    let mut listing = catalog.listing.lock().expect("fixture mutex");
    let entry = listing
        .iter_mut()
        .find(|entry| entry.sku_id == sku)
        .expect("the fixture lists the SKU");
    entry.sellable = sellable;
}

fn line_body(
    phase: Uuid,
    sku: Uuid,
    charge_kind: &str,
    structure: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "scope_key": {
            "phase": phase, "sku_id": sku,
            "price_eligibility": "all_subscriptions",
            "charge_kind": charge_kind, "cohort": null,
            "dimension_key": ""
        },
        "structure": structure
    })
}

async fn created(h: &Harness, plan_id: Uuid, body: serde_json::Value, key: &str) -> String {
    let response = post_line(h, plan_id, body, key).await;
    let status = response.status();
    let line = body_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "{line}");
    line["line_version_id"].as_str().expect("id").to_owned()
}

async fn priced(
    h: &Harness,
    plan_id: Uuid,
    version: &str,
    money: serde_json::Value,
    key: &str,
) -> String {
    let response = post_price(
        h,
        plan_id,
        version,
        serde_json::json!({
            "currency": "EUR", "region": "EU",
            "money": money,
            "market_policy": {"tax_inclusive": false, "rounding_policy_ref": "half_up"}
        }),
        key,
    )
    .await;
    let status = response.status();
    let price = body_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "{price}");
    price["price_id"].as_str().expect("id").to_owned()
}

async fn schedule_at_publish(h: &Harness, plan_id: Uuid, price_ids: &[String]) {
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
                    ("idempotency-key", &format!("sale-window-{nth}")),
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

/// Submit, approve on an independent principal, publish.
async fn publish(h: &Harness, plan_id: Uuid) {
    let tag = h.plan_etag(plan_id).await;
    let submitted = h
        .allowed_as(SUBMITTER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/plans/{plan_id}/publish"),
            None,
            &[("if-match", tag.as_str())],
        ))
        .await;
    let status = submitted.status();
    let submitted = body_json(submitted).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{submitted}");
    let approval_id = submitted["approval"]["approval_id"]
        .as_str()
        .expect("the unit")
        .to_owned();
    let approved = h
        .allowed_as(APPROVER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{approval_id}/approve"),
            None,
            &[],
        ))
        .await;
    assert_eq!(
        approved.status(),
        StatusCode::OK,
        "{}",
        body_json(approved).await
    );
    let published = h
        .allowed_as(SUBMITTER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/plans/{plan_id}/publish"),
            None,
            &[("if-match", tag.as_str())],
        ))
        .await;
    let status = published.status();
    let published = body_json(published).await;
    assert_eq!(status, StatusCode::OK, "{published}");

    // The registry batches the version, then the projector's sweep freezes the
    // delta and advances the pin frontier — the read side the gate answers from.
    let pending = published["receipt"]["pending_version_ref"]
        .as_str()
        .expect("the receipt names the pending version")
        .to_owned();
    h.registry.commit(&pending, 1);
    bss_pricing::infra::jobs::readmodel_warm::ReadModelWarmJob::new(
        h.db.clone(),
        Arc::clone(&h.registry) as Arc<dyn bss_pricing::domain::ports::CatalogVersionRegistryV1>,
        bss_pricing::config::JobsConfig::default(),
    )
    .run(time::OffsetDateTime::now_utc())
    .await
    .expect("the sweep freezes the published version");
}

/// Predicate (6) off the sellability document, evaluated inside the coverage.
async fn registry_predicate(h: &Harness, plan_id: Uuid) -> serde_json::Value {
    // The `at_publish` windows open at the publish instant, so a day later is
    // inside every key's coverage and the horizon predicate has its operand.
    let at = time::OffsetDateTime::now_utc() + time::Duration::days(1);
    let query = format!(
        "currency=EUR&region=EU&at={}",
        bss_pricing::domain::instant::format_rfc3339(at)
    );
    let response = h
        .allowed()
        .send(request(
            "GET",
            &format!("/bss-pricing/v1/plans/{plan_id}/sellability?{query}"),
            None,
        ))
        .await;
    let status = response.status();
    let document = body_json(response).await;
    assert_eq!(status, StatusCode::OK, "{document}");
    document["predicates"]
        .as_array()
        .expect("plan predicates")
        .iter()
        .find(|p| p["ordinal"] == 6)
        .cloned()
        .unwrap_or_else(|| panic!("predicate (6) on the document: {document}"))
}

/// Author and publish the three-line plan of the module doc; answer its plan id
/// and its frozen price rows.
async fn published_three_line_plan(
    h: &Harness,
) -> (Uuid, Vec<bss_pricing::domain::price_record::PriceRecord>) {
    let plan_id = Uuid::now_v7();
    let shape = rest_support::seed_publishable_shape(h, plan_id).await;
    let phase = shape.phase.get();
    let recurring = created(
        h,
        plan_id,
        line_body(
            phase,
            OFFER_SKU,
            "recurring",
            serde_json::json!({
                "model_kind": "flat", "gl_code_ref": "4000",
                "billing_timing": "advance", "billing_anchor_policy": "calendar_month",
                "proration_basis": "calendar_days_actual", "credit_on_downgrade": false
            }),
        ),
        "sale-recurring",
    )
    .await;
    let one_time = created(
        h,
        plan_id,
        line_body(
            phase,
            CLOSED_COMPONENT,
            "one_time",
            serde_json::json!({
                "model_kind": "flat", "gl_code_ref": "4000"
            }),
        ),
        "sale-one-time",
    )
    .await;
    let usage = created(
        h,
        plan_id,
        line_body(
            phase,
            metered_component(),
            "usage",
            serde_json::json!({
                "model_kind": "per_unit", "gl_code_ref": "4000", "billing_granularity": "per_hour"
            }),
        ),
        "sale-usage",
    )
    .await;
    let price_ids = vec![
        priced(
            h,
            plan_id,
            &recurring,
            serde_json::json!({"amount_minor": 9_900}),
            "sale-p-rec",
        )
        .await,
        priced(
            h,
            plan_id,
            &one_time,
            serde_json::json!({"amount_minor": 4_900}),
            "sale-p-one",
        )
        .await,
        priced(
            h,
            plan_id,
            &usage,
            serde_json::json!({"unit_rate_nano_minor": 700_000_000_i64}),
            "sale-p-use",
        )
        .await,
    ];
    schedule_at_publish(h, plan_id, &price_ids).await;
    publish(h, plan_id).await;
    let terms = price_rows(h, plan_id).await;
    assert_eq!(terms.len(), 3);
    (plan_id, terms)
}

#[tokio::test]
async fn closing_sales_on_any_included_sku_closes_the_plan_and_reopening_reopens_it() {
    let catalog = Arc::new(MutableCatalog::new());
    // Stated, not inherited: the fixture lists every metered SKU closed — the
    // D-46-era "composition-only" convention this change retires — so each flag
    // the case depends on is set here. The offer and the unmetered component
    // start closed; the metered component starts open.
    set_sellable(&catalog, OFFER_SKU, false);
    set_sellable(&catalog, CLOSED_COMPONENT, false);
    set_sellable(&catalog, metered_component(), true);
    let h = Harness::new_with_catalog(catalog.clone()).await;

    // 1–2. Authoring admits every role with either flag, and publish succeeds
    //      with two SKUs closed: the flag is never read by authoring.
    let (plan_id, terms_before) = published_three_line_plan(&h).await;

    // 3. The roster the publish froze is every SKU the sale includes.
    let (_, payload) = frozen_payload(&h, plan_id).await;
    let mut roster: Vec<String> = payload["saleSkuIds"]
        .as_array()
        .expect("saleSkuIds")
        .iter()
        .map(|v| v.as_str().expect("uuid").to_owned())
        .collect();
    roster.sort();
    let mut expected = vec![
        OFFER_SKU.to_string(),
        CLOSED_COMPONENT.to_string(),
        metered_component().to_string(),
    ];
    expected.sort();
    assert_eq!(roster, expected, "the plan's own SKU and every row's SKU");

    // 4. Both closed SKUs are named, not the first.
    let p6 = registry_predicate(&h, plan_id).await;
    assert_eq!(p6["answer"], "failed", "{p6}");
    let detail = p6["detail"].as_str().expect("detail");
    assert!(
        detail.contains(&OFFER_SKU.to_string()) && detail.contains(&CLOSED_COMPONENT.to_string()),
        "{detail}"
    );
    assert!(
        !detail.contains(&metered_component().to_string()),
        "the open component is not named: {detail}"
    );

    // 5. Reopening the offer alone is not enough: the component still closes the sale.
    set_sellable(&catalog, OFFER_SKU, true);
    let p6 = registry_predicate(&h, plan_id).await;
    assert_eq!(p6["answer"], "failed", "{p6}");
    let detail = p6["detail"].as_str().expect("detail");
    assert!(
        !detail.contains(&OFFER_SKU.to_string()) && detail.contains(&CLOSED_COMPONENT.to_string()),
        "{detail}"
    );

    // 6. With every included SKU open the registry predicate is satisfied — with
    //    no pricing re-publish in between.
    set_sellable(&catalog, CLOSED_COMPONENT, true);
    let p6 = registry_predicate(&h, plan_id).await;
    assert_eq!(p6["answer"], "satisfied", "{p6}");

    // 7. Closing the *open* component — the one priced by a usage row — closes
    //    new sales again, and names it.
    set_sellable(&catalog, metered_component(), false);
    let p6 = registry_predicate(&h, plan_id).await;
    assert_eq!(p6["answer"], "failed", "{p6}");
    assert!(
        p6["detail"]
            .as_str()
            .expect("detail")
            .contains(&metered_component().to_string())
    );

    // 8. Nothing of the frozen terms moved for any of it.
    assert_eq!(
        price_rows(&h, plan_id).await,
        terms_before,
        "permission is not price"
    );
}

// **Not here: the same through a containing bundle.** Attempted 2026-09-21 and
// removed rather than left pretending. The composition write over this published
// member lands, but the bundle *plan*'s own publish is refused
// `PHASE_CHARGE_LINES_EMPTY` — a `sum_of_parts` bundle plan carries no charge
// lines by design (S8 C-1) and no publish path exempts it — and
// `bundle_sellability_tests::nothing_in_this_crate_reaches_the_bundle_conjunction`
// records that the bundle gate has no producer in this crate. So the composition
// half of the projector's roster (`infra::read_model`: the included SKU, the
// member plan's SKU and its rows' SKUs) is derived but unmeasured end to end.
// Recorded on D-376 as owed.

/// The frozen plan payload the sale gate reads, at its pinned version.
async fn frozen_payload(h: &Harness, plan_id: Uuid) -> (u64, serde_json::Value) {
    use bss_pricing::domain::read_model::SubjectRef;
    use bss_pricing::infra::storage::repo::{pin_frontier_repo, read_model_repo};

    let conn = h.db.conn().expect("conn");
    let frontier = pin_frontier_repo::read_at(&conn, &h.scope(), h.tenant)
        .await
        .expect("read the frontier")
        .expect("a pin-eligible version");
    let stored = read_model_repo::delta_at(
        &conn,
        &h.scope(),
        h.tenant,
        &SubjectRef::Plan(plan_id),
        frontier.catalog_version,
    )
    .await
    .expect("read the frozen delta")
    .expect("the published plan is projected");
    (stored.catalog_version.get(), stored.payload)
}
