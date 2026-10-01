//! D-469 (asks 30 and the contracts notes, phase 9 plan rev 2 M1 and W1): the served spec says
//! what the doors do. Every op declares 503, as products declares it on all of its ops: every door
//! answers 503 when the policy decision point cannot answer. The texts name
//! `REGISTRY_UNAVAILABLE` on exactly the ops that read Products hard. The item create, the item
//! PATCH, the revision PATCH and the revision delete name what they refuse. The `ETag` of every
//! answer that sets one is pinned with the route census, `tests/module_test.rs`.
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
    assert_eq!(all.len(), 52, "the route census holds 52 ops");
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
    let item_patch = description(&api, "patch", "/bss-pricing/v1/plan-items/{id}");
    for code in [
        "BODY_UNEXPECTED",
        "ITEM_ENTRY_MISSING",
        "ITEM_BOOK_FOREIGN",
        "ITEM_ENTRY_SKU_MISMATCH",
        "NOT_DRAFT_AUTHOR",
        "404 for an unknown item",
        "ENTRY_NOT_FOUND",
        "REVISION_NOT_DRAFT",
        "STALE_REVISION",
    ] {
        assert!(
            item_patch.contains(code),
            "the item PATCH names {code}: {item_patch}"
        );
    }
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

/// D-470 (ask 42): the counts op declares its 503, its narrowing, its answer and its refusals;
/// the list names its order, its light read and their refusals.
#[tokio::test]
async fn the_unit_reads_say_how_they_count_and_order() {
    let api = served().await;
    let path = "/bss-pricing/v1/approval-units/counts";
    let counts = &api["paths"][path]["get"];
    assert!(
        !counts["responses"]["503"]["content"]["application/problem+json"].is_null(),
        "{counts}"
    );
    let names = |op: &Value| -> Vec<String> {
        op["parameters"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["in"] == "query")
            .map(|p| p["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(names(counts), ["state", "kind", "ref_id", "book_id"]);
    let text = description(&api, "get", path);
    for said in [
        "by_state",
        "by_kind",
        "total",
        "one grouped statement",
        "UNIT_STATE_INVALID",
        "QUERY_INVALID",
    ] {
        assert!(text.contains(said), "the counts say {said}: {text}");
    }
    let schema = &counts["responses"]["200"]["content"]["application/json"]["schema"]["$ref"];
    assert_eq!(
        schema, "#/components/schemas/PricingApprovalUnitCounts",
        "{counts}"
    );
    let shape = &api["components"]["schemas"]["PricingApprovalUnitCounts"]["properties"];
    for field in ["by_state", "by_kind", "total"] {
        assert!(!shape[field].is_null(), "{field}: {shape}");
    }
    let list = &api["paths"]["/bss-pricing/v1/approval-units"]["get"];
    let listed = names(list);
    for name in ["$orderby", "impact"] {
        assert!(listed.iter().any(|n| n == name), "{name}: {listed:?}");
    }
    let text = description(&api, "get", "/bss-pricing/v1/approval-units");
    for said in [
        "submitted_at desc",
        "ORDER_WITH_CURSOR",
        "INVALID_ORDERBY_FIELD",
        "impact=false",
    ] {
        assert!(text.contains(said), "the list says {said}: {text}");
    }
}

/// D-471 (ask 28): every unit carries `caller_can_approve`, a required boolean whose text says it
/// judges Approve only and not the grant; the list's and the card's texts name it.
#[tokio::test]
async fn every_unit_says_whether_its_reader_may_approve_it() {
    let api = served().await;
    for path in [
        "/bss-pricing/v1/approval-units",
        "/bss-pricing/v1/approval-units/{id}",
    ] {
        let text = description(&api, "get", path);
        assert!(text.contains("caller_can_approve"), "{path}: {text}");
    }
    let unit = &api["components"]["schemas"]["PricingApprovalUnitDto"];
    let flag = &unit["properties"]["caller_can_approve"];
    assert_eq!(flag["type"], "boolean", "{flag}");
    assert!(
        unit["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "caller_can_approve"),
        "{unit}"
    );
    let said = flag["description"].as_str().unwrap_or_default();
    assert!(
        said.contains("Approve only") && said.contains("403"),
        "the flag says what it judges: {said}"
    );
}

/// A property of a component schema, found on the schema or on one part of its `allOf` (a
/// flattened DTO).
fn property(api: &Value, schema: &str, name: &str) -> Value {
    let schema = &api["components"]["schemas"][schema];
    std::iter::once(schema)
        .chain(schema["allOf"].as_array().into_iter().flatten())
        .find_map(|part| part["properties"].get(name).cloned())
        .unwrap_or(Value::Null)
}

/// D-472 and D-473 (asks 26 and 37): the three entry reads name `next_price`, which has
/// `current_price`'s schema; the book's entries list declares `as_of` and says what the date
/// judges, what it refuses — another day than today without the money's grant included — and that
/// a price outside the book's validity is not sellable.
#[tokio::test]
async fn the_entry_reads_say_what_they_headline_and_on_which_day() {
    let api = served().await;
    for path in [
        "/bss-pricing/v1/price-book-entries/{id}",
        "/bss-pricing/v1/price-books/{id}/entries",
        "/bss-pricing/v1/price-book-entries",
    ] {
        let text = description(&api, "get", path);
        assert!(
            text.contains("next_price") && text.contains("version_no"),
            "{path}: {text}"
        );
    }
    for schema in ["PricingPriceBookEntryReadDto", "PricingSkuEntryDto"] {
        let (mut next, mut current) = (
            property(&api, schema, "next_price"),
            property(&api, schema, "current_price"),
        );
        for field in [&mut next, &mut current] {
            if let Some(fields) = field.as_object_mut() {
                fields.remove("description");
            }
        }
        assert!(!next.is_null(), "{schema} carries next_price");
        assert_eq!(
            next, current,
            "{schema}: next_price has current_price's schema"
        );
        let said = api["components"]["schemas"][schema]["description"]
            .as_str()
            .unwrap_or_default();
        assert!(said.contains("next_price"), "{schema}: {said}");
    }
    let list = "/bss-pricing/v1/price-books/{id}/entries";
    let as_of = api["paths"][list]["get"]["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "as_of")
        .cloned()
        .unwrap_or(Value::Null);
    assert_eq!(
        (as_of["in"].as_str(), as_of["required"].as_bool()),
        (Some("query"), Some(false)),
        "{as_of}"
    );
    let text = description(&api, "get", list);
    for said in [
        "as_of",
        "DATE_INVALID",
        "QUERY_INVALID",
        "not sellable",
        "valid_from",
        "valid_until",
        // D-473 amended (phase 9 review R1): another day than today takes the money's grant.
        "PRICE_BOOK_READ_REQUIRED",
        "an as_of other than today",
    ] {
        assert!(text.contains(said), "the list says {said}: {text}");
    }
}
