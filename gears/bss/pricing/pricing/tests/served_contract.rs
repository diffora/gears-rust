//! D-469 (asks 30 and the contracts notes, phase 9 plan rev 2 M1 and W1): the served spec says
//! what the doors do. Every op declares 503, as products declares it on all of its ops: every door
//! answers 503 when the policy decision point cannot answer. The texts name
//! `REGISTRY_UNAVAILABLE` on exactly the ops that read Products hard. The item create, the revision
//! PATCH and the revision delete name what they refuse. The `ETag` of every answer that sets one is
//! pinned with the route census, `tests/module_test.rs`.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use serde_json::Value;
use std::collections::BTreeSet;
use toolkit::api::OpenApiInfo;

pub mod rest_support;

/// The ops that read Products hard: a Products that cannot answer is their 503
/// `REGISTRY_UNAVAILABLE` (the census of run 9.2, plan rev 2 M1).
const HARD_READS: &[(&str, &str)] = &[
    ("get", "/bss-pricing/v1/plan-revisions/{id}/checks"),
    ("post", "/bss-pricing/v1/plan-revisions/{id}/items"),
    ("post", "/bss-pricing/v1/plan-revisions/{id}/submit"),
    ("post", "/bss-pricing/v1/approval-units/{id}/approve"),
    ("post", "/bss-pricing/v1/price-books/{id}/entries"),
    ("get", "/bss-pricing/v1/resolve"),
    ("post", "/bss-pricing/v1/prices/{id}/submit"),
    ("post", "/bss-pricing/v1/price-books/{id}/publish-changes"),
];

async fn served() -> Value {
    let harness = rest_support::Harness::new().await.unwrap();
    let (_, openapi) = harness.router(axum::Router::new()).unwrap();
    serde_json::to_value(openapi.build_openapi(&OpenApiInfo::default()).unwrap()).unwrap()
}
/// Every pricing op of the served spec: `(method, path, op)`.
fn ops(api: &Value) -> Vec<(String, String, Value)> {
    let mut out = Vec::new();
    for (path, methods) in api["paths"].as_object().unwrap() {
        if !path.starts_with("/bss-pricing/") {
            continue;
        }
        for (method, op) in methods.as_object().unwrap() {
            if ["get", "post", "put", "patch", "delete"].contains(&method.as_str()) {
                out.push((method.clone(), path.clone(), op.clone()));
            }
        }
    }
    out
}
fn set(pairs: &[(&str, &str)]) -> BTreeSet<(String, String)> {
    pairs
        .iter()
        .map(|(m, p)| ((*m).to_owned(), (*p).to_owned()))
        .collect()
}
fn description(api: &Value, method: &str, path: &str) -> String {
    api["paths"][path][method]["description"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn every_op_declares_its_503() {
    let api = served().await;
    let all = ops(&api);
    assert_eq!(all.len(), 51, "the route census holds 51 ops");
    let missing: Vec<_> = all
        .iter()
        .filter(|(_, _, op)| {
            op["responses"]["503"]["content"]["application/problem+json"].is_null()
        })
        .map(|(m, p, _)| format!("{m} {p}"))
        .collect();
    assert!(missing.is_empty(), "no 503 declared on: {missing:?}");
}

#[tokio::test]
async fn registry_unavailable_is_named_on_exactly_the_ops_that_read_products_hard() {
    let api = served().await;
    let named: BTreeSet<(String, String)> = ops(&api)
        .into_iter()
        .filter(|(_, _, op)| {
            op["description"]
                .as_str()
                .is_some_and(|d| d.contains("REGISTRY_UNAVAILABLE"))
        })
        .map(|(m, p, _)| (m, p))
        .collect();
    assert_eq!(named, set(HARD_READS));
    for (method, path) in [
        ("post", "/bss-pricing/v1/approval-units/{id}/reject"),
        ("patch", "/bss-pricing/v1/plan-items/{id}"),
    ] {
        assert!(
            !description(&api, method, path).contains("Products cannot answer"),
            "{method} {path} never answers a Products 503"
        );
    }
}

#[tokio::test]
async fn the_texts_name_what_the_doors_refuse() {
    let api = served().await;
    let create = description(&api, "post", "/bss-pricing/v1/plan-revisions/{id}/items");
    for code in [
        "BODY_UNEXPECTED",
        "ITEM_ENTRY_MISSING",
        "ITEM_BOOK_FOREIGN",
        "ITEM_ENTRY_SKU_MISMATCH",
        "ITEM_SKU_DEPRECATED",
        "ITEM_BUNDLE_SKU",
        "REVISION_ITEMS_TOO_MANY",
        "NOT_DRAFT_AUTHOR",
        "ENTRY_NOT_FOUND",
        "REVISION_NOT_DRAFT",
        "ITEM_SKU_TAKEN",
        "IDEMPOTENCY_CONFLICT",
        "SKU_FENCED",
        "SKU_RETIRING",
        "SKU_DRAFT",
        "REGISTRY_UNAVAILABLE",
    ] {
        assert!(
            create.contains(code),
            "the item create names {code}: {create}"
        );
    }
    let patch = description(&api, "patch", "/bss-pricing/v1/plan-revisions/{id}");
    assert!(
        patch.contains("same SKU, charge kind, period and model"),
        "{patch}"
    );
    assert!(patch.contains("book_id omitted or null leaves"), "{patch}");
    let delete = description(&api, "delete", "/bss-pricing/v1/plan-revisions/{id}");
    assert!(delete.contains("STALE_REVISION"), "{delete}");
    for path in ["/bss-pricing/v1/plans", "/bss-pricing/v1/plans/{id}/clone"] {
        let text = description(&api, "post", path);
        assert!(
            text.contains("PLAN_CODE_INVALID") && text.contains("1 to 32 characters"),
            "{path}: {text}"
        );
    }
}
