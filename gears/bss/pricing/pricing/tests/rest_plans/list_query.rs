//! Real-router regression tests for the canonical, SQL-paginated plan projection.

use bss_pricing::domain::scope_key::{Cohort, PriceEligibility};

use super::{
    Harness, PLANS, StatusCode, a_unit_over, body_json, etag_of, json, plan_path, request,
    seed_draft_plan, seed_price, seed_price_keyed, seed_publishable_plan, with_headers,
};
use serde_json::Value;
use uuid::Uuid;

/// Fetch a page, preserving the full error body in a failed assertion.
async fn page(h: &Harness, query: &str) -> Value {
    let response = h
        .allowed()
        .send(request("GET", &format!("{PLANS}?{query}"), None))
        .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        body_json(response).await
    );
    body_json(response).await
}

/// Walk one-row pages; cap the walk to detect a repeated cursor without hanging.
async fn walk(h: &Harness, order: &str) -> Vec<Value> {
    let mut query = format!("$orderby={}&limit=1", order.replace(' ', "%20"));
    let mut rows = Vec::new();
    for _ in 0..20 {
        let result = page(h, &query).await;
        rows.extend(result["items"].as_array().unwrap().iter().cloned());
        let Some(cursor) = result["page_info"]["next_cursor"].as_str() else {
            return rows;
        };
        query = format!("cursor={cursor}&limit=1");
    }
    unreachable!("pagination did not terminate")
}

/// Author a name through PATCH, including the automatic successor path.
async fn name(h: &Harness, id: Uuid, label: &str) -> Value {
    let response = h
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(id),
            Some(json!({"shape":{"plan_name":label}})),
            &[("if-match", &h.plan_etag(id).await)],
        ))
        .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "{}",
        body_json(response).await
    );
    body_json(response).await
}

/// Sorting by name across pages, with **equal** keys as the hard case.
///
/// The NULL half of this case is gone with D-382: every plan carries a name,
/// so the fixture states five of them rather than leaving two for the sort to
/// place. Two pairs are deliberately equal — the tie is what a cursor loses
/// when the sort is not total, and it is the half that survived.
#[tokio::test]
async fn names_and_equal_sort_keys_are_not_lost_between_pages() {
    let h = Harness::new().await;
    let ids: Vec<_> = (1..=5).map(Uuid::from_u128).collect();
    for id in &ids {
        seed_draft_plan(&h, *id).await;
    }
    name(&h, ids[0], "Beta").await;
    name(&h, ids[1], "Alpha").await;
    name(&h, ids[2], "Alpha").await;
    // Named rather than left at the seeder's default: a fixture that inherits
    // its sort key from a helper is one a change to that helper re-orders
    // silently.
    name(&h, ids[3], "Zeta").await;
    name(&h, ids[4], "Zeta").await;
    for (order, expected) in [
        (
            "plan_name asc",
            vec![ids[1], ids[2], ids[0], ids[3], ids[4]],
        ),
        (
            // Descending on the name; each tie broken by `plan_id` ascending,
            // which is the stable order the walk appends.
            "plan_name desc",
            vec![ids[3], ids[4], ids[0], ids[1], ids[2]],
        ),
        (
            "plan_name asc,plan_id desc",
            vec![ids[2], ids[1], ids[0], ids[4], ids[3]],
        ),
    ] {
        let rows = walk(&h, order).await;
        assert_eq!(
            rows.iter()
                .map(|r| r["plan_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            expected.iter().map(Uuid::to_string).collect::<Vec<_>>(),
            "{order}"
        );
    }
    let search = page(&h, "$filter=contains(plan_name,'ALP')").await;
    assert_eq!(search["items"].as_array().unwrap().len(), 2);
    let null_filter = h
        .allowed()
        .send(request(
            "GET",
            &format!("{PLANS}?$filter=plan_name%20eq%20null"),
            None,
        ))
        .await;
    assert_eq!(
        null_filter.status(),
        StatusCode::BAD_REQUEST,
        "the existing toolkit does not admit null literals for String fields - a statement \
         about the filter grammar, and since D-382 there is no null-named plan for such a \
         filter to have found anyway"
    );
    name(&h, ids[0], "literal_%!").await;
    assert_eq!(
        page(&h, "$filter=contains(plan_name,'_%25!')").await["items"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "LIKE wildcards must be literal"
    );
}

/// `sku_id` and `plan_tier` select the plan that carries the value.
///
/// The list's filter translator maps every field to a column by hand
/// (`Field::SkuId => col(plan::Column::SkuId)`, `Field::PlanTier =>
/// col(plan::Column::PlanTier)`), and these two had no case at all — so the two
/// arms could be swapped, or either pointed at a neighbouring column, and the
/// authoring list would answer the wrong rows with every suite green. Both
/// directions are asserted: the value selects its own plan **and** excludes the
/// other, because a translator that ignored the predicate would pass a
/// one-sided check on a one-plan fixture.
#[tokio::test]
async fn sku_id_and_plan_tier_predicates_select_the_plan_that_carries_the_value() {
    let h = Harness::new().await;
    let (gold, silver) = (Uuid::now_v7(), Uuid::now_v7());
    // A SKU the registry serves: re-pointing a plan is a new binding, and the
    // role rule reads the registry before it lands.
    let sku = crate::rest_support::OTHER_OFFER_SKU;
    seed_draft_plan(&h, gold).await;
    seed_draft_plan(&h, silver).await;

    // Both seed as `plan_tier: gold` with no sku; move one of them through the
    // surface so the two rows differ on exactly these columns.
    let patched = h
        .allowed()
        .send(with_headers(
            "PATCH",
            &plan_path(silver),
            Some(json!({"shape": {"plan_tier": "silver", "sku_id": sku}})),
            &[("if-match", &h.plan_etag(silver).await)],
        ))
        .await;
    assert_eq!(
        patched.status(),
        StatusCode::OK,
        "{}",
        body_json(patched).await
    );

    let only = |page: &Value, expected: Uuid| {
        let items = page["items"].as_array().expect("items");
        assert_eq!(items.len(), 1, "{page}");
        assert_eq!(items[0]["plan_id"], json!(expected), "{page}");
    };

    only(&page(&h, "$filter=plan_tier%20eq%20'silver'").await, silver);
    only(&page(&h, "$filter=plan_tier%20eq%20'gold'").await, gold);
    only(
        &page(&h, &format!("$filter=sku_id%20eq%20{sku}")).await,
        silver,
    );
    assert_eq!(
        page(&h, "$filter=plan_tier%20eq%20'bronze'").await["items"],
        json!([]),
        "a tier nothing carries selects nothing"
    );
}

#[tokio::test]
async fn filtering_uses_shown_revision_and_creation_time_survives_a_successor() {
    let h = Harness::new().await;
    let id = Uuid::now_v7();
    seed_draft_plan(&h, id).await;
    let original = name(&h, id, "Old title").await;
    assert!(original.get("created_at_utc").is_none());
    h.publish(id, 0).await;
    let successor = name(&h, id, "New title").await;
    assert_eq!(successor["revision"], 1);
    assert_eq!(successor["created_at"], original["created_at"]);
    assert_ne!(
        successor["revision_created_at"],
        original["revision_created_at"]
    );
    for filter in [
        "plan_name%20eq%20'Old%20title'",
        "lifecycle_state%20eq%20'published'",
        "lifecycle_state%20eq%20'superseded'",
    ] {
        assert_eq!(
            page(&h, &format!("$filter={filter}")).await["items"],
            json!([]),
            "{filter}"
        );
    }
    let result = page(&h, "$filter=plan_name%20eq%20'New%20title'").await;
    assert_eq!(result["items"][0]["created_at"], original["created_at"]);
    assert_eq!(
        result["items"][0]["revision_created_at"],
        successor["revision_created_at"]
    );
    for order in ["created_at asc", "created_at desc", "lifecycle_state asc"] {
        let rows = walk(&h, order).await;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["plan_id"], json!(id));
    }
    let created = original["created_at"].as_str().unwrap();
    let exact = page(&h, &format!("$filter=created_at%20eq%20{created}")).await;
    assert_eq!(exact["items"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn price_count_sorts_before_pagination_and_membership_predicates_are_independent() {
    use bss_pricing::infra::storage::entity::{charge_line_version, market_price, price};
    use sea_orm::ActiveValue::Set;
    use sea_orm::{ColumnTrait, Condition, EntityTrait};
    use toolkit_db::secure::{SecureEntityExt, SecureInsertExt, SecureUpdateExt};
    let h = Harness::new().await;
    let ids: Vec<_> = (10..13).map(Uuid::from_u128).collect();
    for id in &ids {
        seed_draft_plan(&h, *id).await;
    }
    seed_price(&h, ids[1], "EU").await;
    seed_price(&h, ids[2], "EU").await;
    // **The two matches must live on different *lines*, not different rows.**
    // `model_kind` is shared calculation structure, so every market of one line
    // reads the same one — a fixture that set `per_unit` on a second row of the
    // seeded line would move the first row's model too, and the plan would hold
    // no `flat` at all. `new_subscriptions_only` is an axis of the charge line,
    // so it buys a second line; that line gets the `EUR` market and the
    // `per_unit` model while the seeded line keeps `flat` and `USD`.
    let second = seed_price_keyed(
        &h,
        ids[2],
        "US",
        PriceEligibility::NewSubscriptionsOnly,
        Cohort::None,
    )
    .await;
    let conn = h.db.conn().unwrap();
    let stored = price::Entity::find()
        .secure()
        .scope_with(&h.scope())
        .filter(Condition::all().add(price::Column::PriceId.eq(second.price_id)))
        .one(&conn)
        .await
        .unwrap()
        .unwrap();
    let graph = bss_pricing::infra::storage::repo::price_join::load_graph(
        &conn,
        &h.scope(),
        stored.tenant_id,
        stored,
    )
    .await
    .unwrap();
    // A market is an identity row, so the currency is not edited: a second
    // market is added on that second line and the draft row repointed at it.
    let eur_market = Uuid::now_v7();
    let eur = market_price::ActiveModel {
        tenant_id: Set(graph.market.tenant_id),
        market_price_id: Set(eur_market),
        charge_line_id: Set(graph.market.charge_line_id),
        currency: Set("EUR".to_owned()),
        region: Set(graph.market.region.clone()),
    };
    market_price::Entity::insert(eur.clone())
        .secure()
        .scope_with_model(&h.scope(), &eur)
        .expect("scope the second market")
        .exec(&conn)
        .await
        .unwrap();
    price::Entity::update_many()
        .secure()
        .scope_with(&h.scope())
        .col_expr(
            price::Column::MarketPriceId,
            sea_orm::sea_query::Expr::value(eur_market),
        )
        .filter(Condition::all().add(price::Column::PriceId.eq(second.price_id)))
        .exec(&conn)
        .await
        .unwrap();
    charge_line_version::Entity::update_many()
        .secure()
        .scope_with(&h.scope())
        .col_expr(
            charge_line_version::Column::ModelKind,
            sea_orm::sea_query::Expr::value("per_unit"),
        )
        .filter(
            Condition::all()
                .add(charge_line_version::Column::LineVersionId.eq(graph.version.line_version_id)),
        )
        .exec(&conn)
        .await
        .unwrap();
    for (order, expected) in [
        ("price_row_count asc", vec![0, 1, 2]),
        ("price_row_count desc", vec![2, 1, 0]),
    ] {
        assert_eq!(
            walk(&h, order)
                .await
                .iter()
                .map(|r| r["price_row_count"].as_u64().unwrap())
                .collect::<Vec<_>>(),
            expected
        );
    }
    let result = page(
        &h,
        "$filter=model_kind%20eq%20'flat'%20and%20currency%20in%20('EUR','JPY')",
    )
    .await;
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["plan_id"], json!(ids[2]));
    assert_eq!(
        result["items"][0]["model_kinds"],
        json!(["flat", "per_unit"])
    );
    assert_eq!(result["items"][0]["currencies"], json!(["EUR", "USD"]));
    let negated = page(&h, "$filter=not(currency%20eq%20'EUR')").await;
    assert_eq!(negated["items"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn pending_filter_tracks_the_same_units_as_badges() {
    let h = Harness::new().await;
    let active = Uuid::now_v7();
    let idle = Uuid::now_v7();
    let seeded = seed_publishable_plan(&h, active).await;
    seed_draft_plan(&h, idle).await;
    let unit = a_unit_over(&h, active, &seeded.etag()).await;
    let result = page(&h, "$filter=has_pending_approvals%20eq%20true").await;
    assert_eq!(result["items"].as_array().unwrap().len(), 1);
    assert_eq!(result["items"][0]["plan_id"], json!(active));
    assert_eq!(
        result["items"][0]["pending_approvals"][0]["approval_id"],
        json!(unit)
    );
    assert_eq!(
        page(&h, "$filter=has_pending_approvals%20eq%20false").await["items"][0]["plan_id"],
        json!(idle)
    );
    let response = h
        .allowed_as(super::SUBMITTER)
        .send(with_headers(
            "POST",
            &format!("/bss-pricing/v1/approvals/{unit}/withdraw"),
            None,
            &[],
        ))
        .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        page(&h, "$filter=has_pending_approvals%20eq%20true").await["items"],
        json!([])
    );
    assert_eq!(
        page(&h, "$filter=has_pending_approvals%20ne%20true").await["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn typed_sort_cursor_rejects_malformed_nullable_keys_and_changed_filters() {
    use toolkit_odata::CursorV1;
    let h = Harness::new().await;
    for n in 1..=3 {
        seed_draft_plan(&h, Uuid::from_u128(n)).await;
    }
    let first = page(&h, "$orderby=plan_name%20desc&limit=1").await;
    let token = first["page_info"]["next_cursor"].as_str().unwrap();
    let mut cursor = CursorV1::decode(token).unwrap();
    cursor.k[0] = "not-a-json-option".to_owned();
    let malformed = h
        .allowed()
        .send(request(
            "GET",
            &format!("{PLANS}?cursor={}", cursor.encode().unwrap()),
            None,
        ))
        .await;
    assert_eq!(malformed.status(), StatusCode::BAD_REQUEST);
    let mismatch = h
        .allowed()
        .send(request(
            "GET",
            &format!("{PLANS}?cursor={token}&$filter=lifecycle_state%20eq%20'draft'"),
            None,
        ))
        .await;
    assert_eq!(mismatch.status(), StatusCode::BAD_REQUEST);
    let detail = h
        .allowed()
        .send(request("GET", &plan_path(Uuid::from_u128(1)), None))
        .await;
    assert!(etag_of(&detail).is_some());
}

/// A real resource-ID predicate, not a flat tenant-only test permission.
struct PinnedPlanRead {
    tenant: Uuid,
    id: Uuid,
}

#[async_trait::async_trait]
impl authz_resolver_sdk::AuthZResolverApi for PinnedPlanRead {
    async fn evaluate(
        &self,
        _ctx: toolkit_security::PlatformSecurityContext,
        _req: authz_resolver_sdk::models::EvaluationRequest,
    ) -> Result<
        authz_resolver_sdk::models::EvaluationResponse,
        toolkit_canonical_errors::CanonicalError,
    > {
        use authz_resolver_sdk::constraints::{Constraint, InPredicate, Predicate};
        use authz_resolver_sdk::models::{EvaluationResponse, EvaluationResponseContext};
        use toolkit_security::pep_properties;
        Ok(EvaluationResponse {
            decision: true,
            context: EvaluationResponseContext {
                constraints: vec![Constraint {
                    predicates: vec![
                        Predicate::In(InPredicate::new(
                            pep_properties::OWNER_TENANT_ID,
                            vec![self.tenant],
                        )),
                        Predicate::In(InPredicate::new(pep_properties::RESOURCE_ID, vec![self.id])),
                    ],
                }],
                deny_reason: None,
            },
        })
    }
}

#[tokio::test]
async fn resource_pinned_plan_reads_keep_child_aggregates_without_leaking_other_plans() {
    let h = Harness::new().await;
    let visible = Uuid::now_v7();
    let hidden = Uuid::now_v7();
    let seeded = seed_publishable_plan(&h, visible).await;
    let pending = a_unit_over(&h, visible, &seeded.etag()).await;
    seed_draft_plan(&h, hidden).await;
    for region in ["EU", "US", "AU"] {
        seed_price(&h, hidden, region).await;
    }
    let client = h.client_as(
        std::sync::Arc::new(PinnedPlanRead {
            tenant: h.tenant,
            id: visible,
        }),
        Some((h.tenant, super::SUBMITTER)),
    );
    for query in [
        "$orderby=price_row_count%20desc",
        "$filter=HAS_PENDING_APPROVALS%20eq%20true",
        "$filter=currency%20eq%20'EUR'",
    ] {
        let response = client
            .send(request("GET", &format!("{PLANS}?{query}"), None))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{}",
            body_json(response).await
        );
        let result = body_json(response).await;
        assert_eq!(
            result["items"].as_array().unwrap().len(),
            1,
            "{query}: {result}"
        );
        let item = &result["items"][0];
        assert_eq!(item["plan_id"], json!(visible));
        assert_eq!(item["price_row_count"], 1);
        assert_eq!(item["currencies"], json!(["EUR"]));
        assert_eq!(item["pending_approvals"][0]["approval_id"], json!(pending));
    }
}

// ---------------------------------------------------------------------------
// Derived charge kinds — what replaced the authored plan type.
// ---------------------------------------------------------------------------

/// Draft one flat line of `kind` through the real route and hand back its
/// `line_version_id`.
async fn draft_line(h: &Harness, plan: Uuid, kind: &str, key: &str) -> String {
    use crate::charge_line_support::{flat_line_body, post_line};

    let response = post_line(h, plan, flat_line_body(kind), key).await;
    let status = response.status();
    let body = body_json(response).await;
    assert_eq!(status, StatusCode::CREATED, "the line must draft: {body}");
    body["line_version_id"]
        .as_str()
        .expect("the line version")
        .to_owned()
}

/// The kinds are read off the plan's **logical lines**: three markets of one
/// line are one member, a plan with two kinds lists both in **wire-token**
/// order, a plan with no line lists none, and nothing anywhere reduces the set
/// to one category or carries the removed cycle.
#[tokio::test]
async fn charge_kinds_are_derived_from_logical_lines_and_filter_by_membership() {
    use crate::charge_line_support::{flat_price_body, lines_path, post_price};

    let h = Harness::new().await;
    let one_kind = Uuid::from_u128(0x71);
    let two_kinds = Uuid::from_u128(0x72);
    let empty = Uuid::from_u128(0x73);
    for id in [one_kind, two_kinds, empty] {
        seed_draft_plan(&h, id).await;
    }

    // One line, three markets.
    let version = draft_line(&h, one_kind, "one_time", "kinds-a").await;
    for (nth, (currency, region)) in [("EUR", "EU"), ("USD", "US"), ("USD", "us-east")]
        .into_iter()
        .enumerate()
    {
        let priced = post_price(
            &h,
            one_kind,
            &version,
            flat_price_body(currency, region, 1_000),
            &format!("kinds-a-price-{nth}"),
        )
        .await;
        assert_eq!(
            priced.status(),
            StatusCode::CREATED,
            "{}",
            body_json(priced).await
        );
    }
    // Two kinds, declared recurring-first so a sort by declaration order and a
    // sort by wire token disagree.
    draft_line(&h, two_kinds, "recurring", "kinds-b-1").await;
    draft_line(&h, two_kinds, "one_time", "kinds-b-2").await;

    let listed = page(&h, "$orderby=plan_id").await;
    let by_id = |id: Uuid| -> Value {
        listed["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["plan_id"] == id.to_string())
            .cloned()
            .unwrap_or_else(|| panic!("plan {id} is on the page: {listed}"))
    };
    assert_eq!(by_id(one_kind)["charge_kinds"], json!(["one_time"]));
    assert_eq!(
        by_id(two_kinds)["charge_kinds"],
        json!(["one_time", "recurring"]),
        "wire-token order, not enum declaration order"
    );
    assert_eq!(by_id(empty)["charge_kinds"], json!([]));
    assert!(by_id(one_kind).get("billing_cycle").is_none());

    // The single read agrees with the page, and carries no plan type either.
    let single = body_json(
        h.allowed()
            .send(request("GET", &plan_path(one_kind), None))
            .await,
    )
    .await;
    assert_eq!(single["charge_kinds"], json!(["one_time"]));
    assert!(single.get("billing_cycle").is_none());

    // The line list counts logical lines: one item, three nested prices.
    let lines = body_json(
        h.allowed()
            .send(request("GET", &lines_path(one_kind), None))
            .await,
    )
    .await;
    assert_eq!(lines["items"].as_array().unwrap().len(), 1, "{lines}");
    assert_eq!(
        lines["items"][0]["prices"].as_array().unwrap().len(),
        3,
        "{lines}"
    );

    // Membership, both polarities.
    let recurring = page(&h, "$filter=charge_kind%20eq%20'recurring'").await;
    let ids = |result: &Value| -> Vec<String> {
        result["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["plan_id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(ids(&recurring), vec![two_kinds.to_string()]);
    let no_one_time = page(&h, "$filter=charge_kind%20ne%20'one_time'").await;
    assert_eq!(
        ids(&no_one_time),
        vec![empty.to_string()],
        "`ne` is \"holds no line of this kind\", so the plan with no line at all matches"
    );
}

/// A token outside the three kinds, and the removed cycle filter, are **refused**
/// rather than ignored: a filter that silently matched everything would hand a
/// caller the whole catalogue as the answer to a question it did not ask.
#[tokio::test]
async fn an_unknown_charge_kind_and_the_removed_cycle_filter_are_refused() {
    let h = Harness::new().await;
    seed_draft_plan(&h, Uuid::from_u128(0x74)).await;

    for query in [
        "$filter=charge_kind%20eq%20'one_time_setup'",
        "$filter=charge_kind%20eq%20'hybrid'",
        "$filter=billing_cycle%20eq%20'monthly'",
    ] {
        let response = h
            .allowed()
            .send(request("GET", &format!("{PLANS}?{query}"), None))
            .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`{query}` must be refused: {}",
            body_json(response).await
        );
    }
}
