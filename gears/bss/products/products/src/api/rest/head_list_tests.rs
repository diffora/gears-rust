//! Authoring collection contract: current heads, query semantics and scoped paging.

use super::{OTHER_TENANT, TENANT, TestHarness, api_state, harness, json_body, new_product};
use crate::{
    api::rest::{products, skus},
    infra::storage::repo,
    test_support::{authed_ctx, flat_in_enforcer, id_matches},
};
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode, header::ETAG},
};
use sea_orm::{ConnectionTrait as _, Database};
use serde_json::{Value, json};
use std::sync::Arc;
use toolkit::api::OpenApiRegistryImpl;
use toolkit::api::openapi_registry::OpenApiInfo;
use toolkit_db::secure::AccessScope;
use tower::ServiceExt as _;
use uuid::Uuid;

/// Seed actual head rows without running a catalog projector.
async fn seed(h: &TestHarness, tenant: Uuid, name: &str, sellable: bool) -> (Uuid, Uuid) {
    let conn = h.db.conn().unwrap();
    let scope = AccessScope::for_tenant(tenant);
    let product_id = Uuid::new_v4();
    let sku_id = Uuid::new_v4();
    let mut product = new_product(product_id, tenant);
    product.name = name.to_owned();
    product.name_normalized = name.to_lowercase();
    product.product_code = Some(format!("P-{name}"));
    repo::insert_product(&conn, &scope, product).await.unwrap();
    repo::insert_sku(
        &conn,
        &scope,
        repo::NewSku {
            sku_id,
            tenant_id: tenant,
            product_id,
            sku_code: format!("S-{name}"),
            region_scope: "eu".to_owned(),
            brand_scope: String::new(),
            created_by: "principal:author-1".to_owned(),
            created_at: crate::test_support::utc(2026, 9, 18, 9, 0, 0),
            cloned_from: None,
            cloned_from_version: None,
            sku_type: "simple".to_owned(),
            sellable,
            plan_tier: "standard".to_owned(),
            metering_unit: None,
            usage_type_ref: None,
        },
    )
    .await
    .unwrap();
    (product_id, sku_id)
}

/// The two production routers, with an independently configurable PDP tenant.
fn app(h: &TestHarness, allowed: Uuid) -> Router {
    app_with_enforcer(h, flat_in_enforcer(allowed))
}

fn app_with_enforcer(h: &TestHarness, enforcer: authz_resolver_sdk::PolicyEnforcer) -> Router {
    let openapi = OpenApiRegistryImpl::new();
    let state = api_state(h);
    products::router(Arc::clone(&state), &openapi)
        .merge(skus::router(state, &openapi))
        .layer(axum::Extension(enforcer))
}

/// Percent-encode filter expressions and opaque cursors exactly as a client does.
fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                char::from(b).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn url(kind: &str, params: &[(&str, &str)]) -> String {
    let query = params
        .iter()
        .map(|(key, value)| format!("{key}={}", encode(value)))
        .collect::<Vec<_>>()
        .join("&");
    format!("/bss-products/v1/{kind}?{query}")
}

async fn get(app: Router, uri: &str, tenant: Option<Uuid>) -> (StatusCode, Value) {
    let mut request = Request::builder().uri(uri);
    if let Some(tenant) = tenant {
        request = request.extension(authed_ctx(tenant));
    }
    let response = app
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    // Missing collection GET currently returns 405 with an empty body.
    if status == StatusCode::METHOD_NOT_ALLOWED {
        return (status, Value::Null);
    }
    (status, json_body(response).await)
}

async fn lists_all_head_states(kind: &str, id_field: &str) {
    let h = harness().await;
    let db = Database::connect(&h.dsn).await.unwrap();
    for state in ["draft", "published", "deprecated", "retired", "discarded"] {
        let (product, sku) = seed(&h, TENANT, state, true).await;
        let id = if kind == "products" { product } else { sku };
        let table = if kind == "products" {
            "products_product"
        } else {
            "products_sku"
        };
        let steps: &[&str] = match state {
            "draft" => &[],
            "published" => &["published"],
            "deprecated" => &["published", "deprecated"],
            "retired" => &["published", "deprecated", "retired"],
            _ => &["discarded"],
        };
        for step in steps {
            db.execute_unprepared(&format!("UPDATE {table} SET lifecycle_state = '{step}', internal_revision = internal_revision + 1 WHERE {}", id_matches(id_field, id))).await.unwrap();
        }
    }
    let (status, body) = get(app(&h, TENANT), &url(kind, &[]), Some(TENANT)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().unwrap();
    let mut lifecycle_states = items
        .iter()
        .map(|v| v["lifecycle_state"].as_str().unwrap())
        .collect::<Vec<_>>();
    lifecycle_states.sort_unstable();
    assert_eq!(
        lifecycle_states,
        ["deprecated", "discarded", "draft", "published", "retired"]
    );
    assert!(
        items
            .iter()
            .all(|v| v[id_field].is_string() && v["internal_revision"].as_i64().unwrap() > 0)
    );
    assert_eq!(body["page_info"]["limit"], 50);
    assert!(body["page_info"]["next_cursor"].is_null());
}

#[tokio::test]
async fn products_list_includes_drafts_and_terminal_heads_without_projection() {
    lists_all_head_states("products", "product_id").await;
}

#[tokio::test]
async fn skus_list_includes_drafts_and_terminal_heads_without_projection() {
    lists_all_head_states("skus", "sku_id").await;
}

#[tokio::test]
async fn lists_apply_product_search_and_sku_parent_classification_filters() {
    let h = harness().await;
    let (product, sku) = seed(&h, TENANT, "Cloud", false).await;
    seed(&h, TENANT, "Other", true).await;
    for filter in [
        "contains(name,'loud')",
        "product_code eq 'P-Cloud'",
        "startswith(name,'C') and lifecycle_state eq 'draft'",
    ] {
        let (status, body) = get(
            app(&h, TENANT),
            &url("products", &[("$filter", filter)]),
            Some(TENANT),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["items"].as_array().unwrap().len(), 1);
        assert_eq!(body["items"][0]["product_id"], product.to_string());
    }
    let filter = format!(
        "product_id eq {product} and contains(sku_code,'Cloud') and lifecycle_state eq 'draft' and sku_type eq 'simple' and sellable eq false"
    );
    let (status, body) = get(
        app(&h, TENANT),
        &url("skus", &[("$filter", &filter)]),
        Some(TENANT),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["sku_id"], sku.to_string());
}

#[tokio::test]
async fn lists_preserve_both_the_caller_tenant_and_pdp_constraints() {
    let h = harness().await;
    seed(&h, TENANT, "Own", true).await;
    seed(&h, OTHER_TENANT, "Foreign", true).await;
    for kind in ["products", "skus"] {
        let (status, body) = get(app(&h, TENANT), &url(kind, &[]), Some(TENANT)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["items"].as_array().unwrap().len(), 1);
        assert_eq!(body["items"][0]["tenant_id"], TENANT.to_string());
        let (status, body) = get(app(&h, OTHER_TENANT), &url(kind, &[]), Some(TENANT)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body["items"],
            json!([]),
            "PDP's foreign scope must not replace the caller tenant"
        );
        let (status, _) = get(app(&h, TENANT), &url(kind, &[]), None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}

#[tokio::test]
async fn lists_page_forward_and_backward_with_tied_order_keys() {
    let h = harness().await;
    for name in ["Alpha", "Beta", "Gamma"] {
        seed(&h, TENANT, name, true).await;
    }
    for (kind, id_field) in [("products", "product_id"), ("skus", "sku_id")] {
        let (status, first) = get(
            app(&h, TENANT),
            &url(
                kind,
                &[("limit", "1"), ("$orderby", "lifecycle_state desc")],
            ),
            Some(TENANT),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let mut ids = vec![first["items"][0][id_field].clone()];
        let mut page = first.clone();
        while let Some(cursor) = page["page_info"]["next_cursor"].as_str() {
            let (status, next) = get(
                app(&h, TENANT),
                &url(kind, &[("limit", "1"), ("cursor", cursor)]),
                Some(TENANT),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{next}");
            ids.push(next["items"][0][id_field].clone());
            assert!(ids.len() <= 3, "pagination must terminate");
            if ids.len() == 2 {
                let prev = next["page_info"]["prev_cursor"].as_str().unwrap();
                let (status, back) = get(
                    app(&h, TENANT),
                    &url(kind, &[("limit", "1"), ("cursor", prev)]),
                    Some(TENANT),
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{back}");
                assert_eq!(back["items"], first["items"]);
            }
            page = next;
        }
        assert_eq!(ids.len(), 3);
        ids.sort_by_key(Value::to_string);
        ids.dedup();
        assert_eq!(ids.len(), 3);
    }
}

#[tokio::test]
async fn lists_reject_invalid_queries_instead_of_silently_ignoring_them() {
    let h = harness().await;
    for kind in ["products", "skus"] {
        for (key, value) in [
            ("status", "draft"),
            ("$filter", "tenant_id eq 'foreign'"),
            ("$filter", "unknown eq 1"),
            ("$orderby", "unknown desc"),
            ("limit", "0"),
            ("limit", "not-a-number"),
            ("cursor", "bad-cursor"),
            ("$select", "name"),
            ("$skip", "10"),
        ] {
            let (status, body) =
                get(app(&h, TENANT), &url(kind, &[(key, value)]), Some(TENANT)).await;
            assert_eq!(
                status,
                StatusCode::BAD_REQUEST,
                "{kind} {key}={value}: {body}"
            );
        }
    }
}

#[tokio::test]
async fn lists_refuse_order_on_nullable_keys() {
    let h = harness().await;
    for (kind, orderby) in [("products", "product_code asc"), ("skus", "sku_type desc")] {
        let (status, body) = get(
            app(&h, TENANT),
            &url(kind, &[("$orderby", orderby)]),
            Some(TENANT),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{kind} {orderby}: {body}");
    }
}

#[tokio::test]
async fn lists_answer_an_empty_page_without_an_etag() {
    let h = harness().await;
    for kind in ["products", "skus"] {
        let response = app(&h, TENANT)
            .oneshot(
                Request::builder()
                    .uri(&url(kind, &[("$top", "1")]))
                    .extension(authed_ctx(TENANT))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers().get(ETAG).is_none(),
            "a collection has no per-item ETag; GET /{kind}/{{id}} is the edit token"
        );
        let body = json_body(response).await;
        assert_eq!(body["items"], json!([]));
        assert_eq!(body["page_info"]["limit"], 1);
        assert!(body["page_info"]["next_cursor"].is_null());
    }
}

#[tokio::test]
async fn list_cursors_are_bound_to_the_filter_tenant_and_collection() {
    let h = harness().await;
    for tenant in [TENANT, OTHER_TENANT] {
        for name in ["Alpha", "Beta"] {
            seed(&h, tenant, name, true).await;
        }
    }
    for kind in ["products", "skus"] {
        let (status, first) =
            get(app(&h, TENANT), &url(kind, &[("limit", "1")]), Some(TENANT)).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        let cursor = first["page_info"]["next_cursor"].as_str().unwrap();
        let (status, body) = get(
            app(&h, TENANT),
            &url(
                kind,
                &[
                    ("cursor", cursor),
                    ("$filter", "lifecycle_state eq 'draft'"),
                ],
            ),
            Some(TENANT),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let (status, body) = get(
            app(&h, OTHER_TENANT),
            &url(kind, &[("cursor", cursor)]),
            Some(OTHER_TENANT),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        let other = if kind == "products" {
            "skus"
        } else {
            "products"
        };
        let (status, body) = get(
            app(&h, TENANT),
            &url(other, &[("cursor", cursor)]),
            Some(TENANT),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    }
}

#[tokio::test]
async fn lists_enforce_the_page_ceiling_and_default_order() {
    let h = harness().await;
    for name in ["Gamma", "Alpha", "Beta"] {
        seed(&h, TENANT, name, true).await;
    }
    for (kind, field, expected) in [
        ("products", "name", json!(["Alpha", "Beta", "Gamma"])),
        ("skus", "sku_code", json!(["S-Alpha", "S-Beta", "S-Gamma"])),
    ] {
        let (status, body) = get(
            app(&h, TENANT),
            &url(kind, &[("limit", "999")]),
            Some(TENANT),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["page_info"]["limit"], 200);
        let values = body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row[field].clone())
            .collect::<Vec<_>>();
        assert_eq!(json!(values), expected);
    }
}

/// A grant for one resource type, optionally restricted to a single head.
struct ReadGrant {
    label: &'static str,
    resource_id: Option<Uuid>,
    constrained: bool,
}

#[async_trait::async_trait]
impl authz_resolver_sdk::AuthZResolverApi for ReadGrant {
    async fn evaluate(
        &self,
        _ctx: toolkit_security::PlatformSecurityContext,
        request: authz_resolver_sdk::models::EvaluationRequest,
    ) -> Result<
        authz_resolver_sdk::models::EvaluationResponse,
        toolkit_canonical_errors::CanonicalError,
    > {
        use authz_resolver_sdk::{
            constraints::{Constraint, InPredicate, Predicate},
            models::{EvaluationResponse, EvaluationResponseContext},
        };
        let mut predicates = vec![Predicate::In(InPredicate::new(
            "owner_tenant_id",
            vec![TENANT],
        ))];
        if let Some(id) = self.resource_id {
            predicates.push(Predicate::In(InPredicate::new("id", vec![id])));
        }
        Ok(EvaluationResponse {
            decision: request.resource.resource_type == self.label
                && request.action.name == "read"
                && request.resource.id.is_none(),
            context: EvaluationResponseContext {
                constraints: if self.constrained {
                    vec![Constraint { predicates }]
                } else {
                    vec![]
                },
                deny_reason: None,
            },
        })
    }
}

#[tokio::test]
async fn lists_enforce_resource_grants_and_fail_closed_without_constraints() {
    let h = harness().await;
    let (product, sku) = seed(&h, TENANT, "Allowed", true).await;
    seed(&h, TENANT, "Hidden", true).await;
    for (kind, label, id_field, id) in [
        (
            "products",
            crate::authz::labels::PRODUCT,
            "product_id",
            product,
        ),
        ("skus", crate::authz::labels::SKU, "sku_id", sku),
    ] {
        for (grant_label, constrained, expected) in [
            (label, true, StatusCode::OK),
            (label, false, StatusCode::FORBIDDEN),
            ("wrong-permission", true, StatusCode::FORBIDDEN),
        ] {
            let enforcer = authz_resolver_sdk::PolicyEnforcer::new(Arc::new(ReadGrant {
                label: grant_label,
                resource_id: Some(id),
                constrained,
            }));
            let router = app_with_enforcer(&h, enforcer);
            let (status, body) = get(router, &url(kind, &[]), Some(TENANT)).await;
            assert_eq!(status, expected, "{body}");
            if expected == StatusCode::OK {
                assert_eq!(body["items"].as_array().unwrap().len(), 1);
                assert_eq!(body["items"][0][id_field], id.to_string());
            }
        }
    }
}

#[tokio::test]
async fn collection_openapi_keeps_create_and_declares_typed_page_responses() {
    let h = harness().await;
    let registry = OpenApiRegistryImpl::new();
    let state = api_state(&h);
    let _router =
        products::router(Arc::clone(&state), &registry).merge(skus::router(state, &registry));
    let doc =
        serde_json::to_value(registry.build_openapi(&OpenApiInfo::default()).unwrap()).unwrap();
    for (kind, schema) in [("products", "ProductView"), ("skus", "SkuView")] {
        let path = &doc["paths"][format!("/bss-products/v1/{kind}")];
        assert!(path["post"].is_object(), "create remains reachable");
        assert_eq!(
            path["get"]["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            format!("#/components/schemas/Page_{schema}")
        );
        assert!(
            doc["components"]["schemas"][schema].is_object(),
            "page item schema resolves"
        );
        let parameters = path["get"]["parameters"].as_array().unwrap();
        for name in ["$filter", "$orderby", "limit", "cursor"] {
            assert!(parameters.iter().any(|p| p["name"] == name));
        }
    }
}
