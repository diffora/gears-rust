//! The authz gate, proved over the **set** rather than per route.
//!
//! Everything before this proved the gate one route at a time, which leaves two
//! holes an in-process suite can close and only closes deliberately.
//!
//! The first is a route gated on the **wrong pair**. No allow/deny test can see
//! it: a `plan x read` gate on a mutating route allows exactly the callers a
//! `plan x write` gate would in every fixture that grants both, and denies
//! exactly the ones it would in every fixture that grants neither. Only a
//! resolver that records what it was asked can tell. That is the census below.
//!
//! The second is a route added later **with no gate at all**. A per-route suite
//! grows a hole the moment somebody adds a route and forgets its test; a census
//! driven from the registered path set fails instead.
//!
//! # What this suite cannot prove, and does not pretend to
//!
//! **The role matrix is the PDP's, not this gear's.** Which role holds
//! `plan x write`, and whether `FinanceReviewer` can read what it approves
//! (D-61's reviewability invariant), are decisions in
//! `05-governance.md`'s role table that this gear neither implements nor can
//! observe: it asks a question and binds the answer. Nothing here substitutes
//! for testing that table where it lives.
//!
//! **Nothing here is an isolation-level property.** Everything runs over
//! `sqlite::memory:`, which serializes writers.

#![allow(clippy::expect_used, clippy::unwrap_used)]

mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::plans::{PLAN_ABANDON, PLANS};
use bss_pricing::api::rest::prices::{PLAN_PRICE, PLAN_PRICES};
use bss_pricing::authz::{actions, labels};
use rest_support::{
    Harness, body_json, plan_count, plan_row_version, price_rows, request, seed_draft_plan,
    seed_price, with_headers,
};
use uuid::Uuid;

/// One row of the census: a route, and the catalogued pair it must gate on.
///
/// The pairs come from [`bss_pricing::authz`]'s consts, never from string
/// literals, so a label renamed in one place cannot be "fixed" in the other. The
/// **source** is `05-governance.md`'s endpoint -> `(resource, action)` map:
/// `POST/PATCH /bss-pricing/v1/plans*`, `POST …/prices*` and `DELETE …/prices/{id}`
/// are `plan x write`; `GET /bss-pricing/v1/plans*`, `GET …/prices` and the
/// frontier are `plan x read`.
struct Route {
    /// HTTP method.
    method: &'static str,
    /// The registered path template.
    path: &'static str,
    /// The catalogued resource label.
    resource_type: &'static str,
    /// The catalogued action.
    action: &'static str,
    /// Whether driving it changes state (so a denial has something to observe).
    mutating: bool,
}

fn census() -> Vec<Route> {
    vec![
        Route {
            method: "GET",
            path: PLANS_BY_ID,
            resource_type: labels::PLAN,
            action: actions::READ,
            mutating: false,
        },
        Route {
            method: "POST",
            path: PLANS,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "PATCH",
            path: PLANS_BY_ID,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "POST",
            path: PLAN_ABANDON,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "POST",
            path: PLAN_PRICES,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "GET",
            path: PLAN_PRICES,
            resource_type: labels::PLAN,
            action: actions::READ,
            mutating: false,
        },
        Route {
            method: "PATCH",
            path: PLAN_PRICE,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "DELETE",
            path: PLAN_PRICE,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
    ]
}

/// `plans::PLAN` under a name that does not collide with `labels::PLAN`.
const PLANS_BY_ID: &str = bss_pricing::api::rest::plans::PLAN;

/// The seeded world every case drives against, and the concrete request for a
/// route template.
struct Seeded {
    plan_id: Uuid,
    price_id: Uuid,
}

async fn seed(harness: &Harness) -> Seeded {
    let plan_id = Uuid::now_v7();
    seed_draft_plan(harness, plan_id).await;
    harness.attach_shape(plan_id, 0).await;
    let price = seed_price(harness, plan_id, "EU").await;
    Seeded {
        plan_id,
        price_id: price.price_id,
    }
}

/// A request's optional JSON body and its preconditions.
type DrivenBody = (Option<serde_json::Value>, Vec<(&'static str, &'static str)>);

/// Fill a route template with the seeded ids and a well-formed body.
///
/// The precondition is deliberately **the current one** — on a plan route the
/// revision-qualified `"0-3"` the seeded draft stands at after `attach_shape`,
/// on a price route the row's own `"0"` — so a refusal in any of these cases is
/// the gate's and never a stale tag's.
fn drive(
    route: &Route,
    seeded: &Seeded,
    version: &'static str,
    key: &'static str,
) -> axum::http::Request<axum::body::Body> {
    let path = route
        .path
        .replace("{planId}", &seeded.plan_id.to_string())
        .replace("{priceId}", &seeded.price_id.to_string());
    let (body, headers): DrivenBody = match (route.method, route.path) {
        ("POST", PLANS) => (
            Some(serde_json::json!({ "plan_tier": "gold" })),
            vec![("idempotency-key", key)],
        ),
        ("POST", PLAN_PRICES) => (
            Some(serde_json::json!({
                "scope_key": {
                    "currency": "USD",
                    "region": "US",
                    "phase": rest_support::seeded_phase().get().to_string(),
                    "price_eligibility": "all_subscriptions",
                    "charge_kind": "recurring",
                    "cohort": serde_json::Value::Null
                },
                "content": { "model_kind": "flat", "amount_minor": 100 }
            })),
            vec![("idempotency-key", key)],
        ),
        ("PATCH", p) if p == PLANS_BY_ID => (
            Some(serde_json::json!({ "shape": { "plan_tier": "platinum" } })),
            vec![("if-match", version)],
        ),
        ("PATCH", PLAN_PRICE) => (
            Some(serde_json::json!({ "content": { "model_kind": "flat", "amount_minor": 7 } })),
            vec![("if-match", "\"0\"")],
        ),
        ("POST", PLAN_ABANDON) => (None, vec![("if-match", version)]),
        ("DELETE", PLAN_PRICE) => (None, vec![("if-match", "\"0\"")]),
        _ => (None, Vec::new()),
    };
    with_headers(route.method, &path, body, &headers)
}

// ---------------------------------------------------------------------------
// 1. The census matches what the router registered.
// ---------------------------------------------------------------------------

/// Every operation the three routers register, keyed `METHOD:path`.
///
/// Built from the routers themselves, not from [`census`], because a census that
/// checked itself would stay green for exactly the route it was meant to catch:
/// one added to the registry and left out of the table.
async fn registered_paths() -> Vec<String> {
    use std::sync::Arc;
    use toolkit::api::OpenApiRegistryImpl;

    let harness = Harness::new().await;
    let openapi = OpenApiRegistryImpl::new();
    drop(
        bss_pricing::api::rest::plans::router(Arc::clone(&harness.state), &openapi).merge(
            bss_pricing::api::rest::prices::router(Arc::clone(&harness.state), &openapi),
        ),
    );
    let mut paths: Vec<String> = openapi
        .operation_specs
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    paths.sort();
    paths
}

#[tokio::test]
async fn the_census_covers_every_route_the_routers_register() {
    // A route added later without a census row fails here, which is the whole
    // point of a census. `tests/module_test.rs` owns the wider property - that
    // the registered set is exactly the nine paths, frontier included; this one
    // owns the narrower and more useful one, that every **gated** route the two
    // authoring routers mount has a catalogued pair beside it.
    let registered = registered_paths().await;
    let mut expected: Vec<String> = census()
        .iter()
        .map(|route| format!("{}:{}", route.method, route.path))
        .collect();
    expected.sort();

    assert_eq!(
        registered, expected,
        "the routers register a path the census does not cover, or the other way round"
    );
    assert!(
        census()
            .iter()
            .all(|route| route.resource_type == labels::PLAN),
        "every surface in this group is on the `plan` label; a new label needs a new census row"
    );
}

#[test]
fn no_handler_can_build_an_access_scope_of_its_own() {
    // What makes "the authz gate before the repository" falsifiable rather than a
    // claim about source order.
    //
    // `every_mutating_route_is_denied_with_the_state_unchanged` proves no
    // **write** precedes the gate. It cannot prove that no **read** does, and a
    // read is what leaks a catalog the design set calls commercially sensitive:
    // a repository call taking `AccessScope::allow_all()` inserted above the PEP
    // call leaves every suite in this crate green.
    //
    // The real guarantee is structural - every repository method takes a
    // `&AccessScope`, and the only producer of one on a REST path is
    // `crate::authz::access_scope`, which *is* the gate. This asserts exactly
    // that: no file under `src/api/rest/` may **construct** an `AccessScope`.
    // It is a stronger anchor than banning `allow_all` alone, because
    // `AccessScope::for_tenant` would be just as much of a bypass and reads far
    // more innocent. Type positions (`scope: &AccessScope`) are untouched - the
    // pattern is the `::` of a constructor call.
    let rest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api/rest");
    let mut offenders: Vec<String> = Vec::new();

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&rest)
        .expect("the REST layer is where it has always been")
        .map(|entry| entry.expect("readable dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.push(rest.with_extension("rs"));
    assert!(files.len() > 5, "the scan found almost nothing: {files:?}");

    for path in files {
        let source = std::fs::read_to_string(&path).expect("readable source");
        for (number, line) in source.lines().enumerate() {
            if line.contains("AccessScope::") {
                offenders.push(format!(
                    "{}:{}: {}",
                    path.display(),
                    number + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a handler that builds its own AccessScope has bypassed the PEP; the only producer on \
         a REST path is `crate::authz::access_scope`:\n{}",
        offenders.join("\n")
    );
}

// ---------------------------------------------------------------------------
// 2. A recording PDP: the pair each route actually asked about.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_route_asks_the_catalogued_pair() {
    // The one property no allow/deny test can reach. A mutating route gated on
    // `plan x read` behaves identically under every fixture that grants both and
    // under every fixture that grants neither; only what it ASKED separates them.
    for route in census() {
        let harness = Harness::new().await;
        let seeded = seed(&harness).await;
        let (client, seen) = harness.recording();

        let response = client
            .send(drive(&route, &seeded, "\"0-3\"", "census-key"))
            .await;
        assert!(
            response.status() != StatusCode::UNAUTHORIZED
                && response.status() != StatusCode::FORBIDDEN,
            "{} {} was refused before it reached the gate",
            route.method,
            route.path
        );

        let asked = seen.lock().expect("recorder");
        let first = asked
            .first()
            .unwrap_or_else(|| panic!("{} {} asked the PDP nothing", route.method, route.path));
        assert_eq!(
            first.resource.resource_type, route.resource_type,
            "{} {} gates on the wrong resource label",
            route.method, route.path
        );
        assert_eq!(
            first.action.name, route.action,
            "{} {} gates on the wrong action",
            route.method, route.path
        );
        assert!(
            first.context.require_constraints,
            "{} {} would accept an unconstrained allow",
            route.method, route.path
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Denied, and untouched.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn every_mutating_route_is_denied_with_the_state_unchanged() {
    // "The authz gate before the repository" is a claim about source order until
    // somebody looks at the store. A handler that wrote first and checked second
    // would produce the same 403.
    for route in census().into_iter().filter(|route| route.mutating) {
        let harness = Harness::new().await;
        let seeded = seed(&harness).await;
        let plans_before = plan_count(&harness).await;
        let version_before = plan_row_version(&harness, seeded.plan_id, 0).await;
        let prices_before = price_rows(&harness, seeded.plan_id).await;

        let response = harness
            .denied()
            .send(drive(&route, &seeded, "\"0-3\"", "denied-key"))
            .await;

        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{} {}",
            route.method,
            route.path
        );
        assert_eq!(
            plan_count(&harness).await,
            plans_before,
            "a row was created"
        );
        assert_eq!(
            plan_row_version(&harness, seeded.plan_id, 0).await,
            version_before,
            "a version moved"
        );
        let prices_after = price_rows(&harness, seeded.plan_id).await;
        assert_eq!(prices_after.len(), prices_before.len(), "a price row moved");
        assert_eq!(
            prices_after.first().map(|row| row.row_version.get()),
            prices_before.first().map(|row| row.row_version.get()),
            "a price row's version moved"
        );
    }
}

// ---------------------------------------------------------------------------
// 4. Unavailable fails closed.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_pdp_outage_fails_closed_on_every_route() {
    // A policy engine that cannot answer must never degrade into an allow, and
    // the caller must be told to retry rather than told it is forbidden.
    for route in census() {
        let harness = Harness::new().await;
        let seeded = seed(&harness).await;

        let response = harness
            .unavailable()
            .send(drive(&route, &seeded, "\"0-3\"", "outage-key"))
            .await;

        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{} {} must fail closed, not open",
            route.method,
            route.path
        );
    }
}

// ---------------------------------------------------------------------------
// 5. Constraints are required.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unconstrained_allow_is_refused_on_a_read_and_on_a_write() {
    // An allow with no constraints compiles to a scope that filters nothing,
    // which is every tenant's price book. `require_constraints = true` is what
    // makes that a denial rather than an exposure.
    let harness = Harness::new().await;
    let seeded = seed(&harness).await;

    let read = harness
        .unconstrained()
        .send(request("GET", &format!("{PLANS}/{}", seeded.plan_id), None))
        .await;
    let write = harness
        .unconstrained()
        .send(with_headers(
            "POST",
            PLANS,
            Some(serde_json::json!({ "plan_tier": "gold" })),
            &[("idempotency-key", "unconstrained")],
        ))
        .await;

    assert_eq!(read.status(), StatusCode::FORBIDDEN);
    assert_eq!(write.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        plan_count(&harness).await,
        1,
        "the unconstrained write must have created nothing"
    );
}

// ---------------------------------------------------------------------------
// 6. Cross-tenant, both directions.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_foreign_tenants_plan_reads_exactly_like_an_absent_one() {
    let harness = Harness::new().await;
    let seeded = seed(&harness).await;

    let baseline = harness
        .allowed()
        .send(request("GET", &format!("{PLANS}/{}", seeded.plan_id), None))
        .await;
    assert_eq!(
        baseline.status(),
        StatusCode::OK,
        "without the owner's 200 the 404 below would be consistent with mere absence"
    );

    let foreign = harness
        .other_tenant()
        .send(request("GET", &format!("{PLANS}/{}", seeded.plan_id), None))
        .await;
    let random = harness
        .other_tenant()
        .send(request("GET", &format!("{PLANS}/{}", Uuid::now_v7()), None))
        .await;

    assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    assert_eq!(random.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        body_json(foreign).await.get("type"),
        body_json(random).await.get("type"),
        "the two answers must be indistinguishable, or the surface leaks whose catalog exists"
    );
}

#[tokio::test]
async fn a_write_whose_target_tenant_is_outside_the_scope_is_denied_on_every_write() {
    // `access_scope`'s membership assertion. The degraded flat-`In` PDP decision
    // does not re-check `owner_tenant_id`, so a target outside the compiled
    // scope is refused there or nowhere.
    for route in census().into_iter().filter(|route| route.mutating) {
        let harness = Harness::new().await;
        let seeded = seed(&harness).await;

        let response = harness
            .scope_mismatch()
            .send(drive(&route, &seeded, "\"0-3\"", "mismatch-key"))
            .await;

        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "{} {} accepted a write anchored outside the caller's scope",
            route.method,
            route.path
        );
    }
}
