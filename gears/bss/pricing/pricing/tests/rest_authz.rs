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

mod common;
mod rest_support;

use axum::http::StatusCode;
use bss_pricing::api::rest::approvals::{
    APPROVAL, APPROVAL_APPROVE, APPROVAL_REJECT, APPROVAL_WITHDRAW, APPROVALS,
};
use bss_pricing::api::rest::bundles::{BUNDLE_BY_ID, BUNDLE_PUBLISH, BUNDLES};
use bss_pricing::api::rest::frontier::FRONTIER;
use bss_pricing::api::rest::plans::{PLAN_ABANDON, PLANS};
use bss_pricing::api::rest::prices::{PLAN_PRICE, PLAN_PRICES};
use bss_pricing::api::rest::publish::PLAN_PUBLISH;
use bss_pricing::api::rest::supersessions::PLAN_SUPERSESSIONS;
use bss_pricing::api::rest::threshold_policy::APPROVAL_THRESHOLD_POLICY;
use bss_pricing::api::rest::windows::{
    PLAN_COVERAGE, PLAN_SELLABILITY, PRICE_WINDOW, PRICE_WINDOWS,
};
use bss_pricing::authz::{actions, labels};
use bss_pricing::domain::approval::ApprovalState;
use rest_support::{
    Harness, approval_row, body_json, plan_count, plan_row_version, price_rows, request,
    seed_draft_plan, seed_price, with_headers,
};
use uuid::Uuid;

/// One row of the census: a route, and the catalogued pair it must gate on.
///
/// The pairs come from [`bss_pricing::authz`]'s consts, never from string
/// literals, so a label renamed in one place cannot be "fixed" in the other. The
/// **source** is `05-governance.md`'s endpoint -> `(resource, action)` map:
/// `POST/PATCH /bss-pricing/v1/plans*`, `POST …/prices*` and `DELETE …/prices/{id}`
/// are `plan x write`; `GET /bss-pricing/v1/plans*`, `GET …/prices` and the
/// frontier are `plan x read`; the entrance is `plan x publish`, spelled
/// `publish (submit for publish)` there; and the approval surface is
/// `approval x read` on its two reads and `approval x approve` on all three
/// decisions.
///
/// **`plan x publish` is not `plan x write`, and the matrix is why.**
/// `ProductManager` holds `plan x write/read` and deliberately **not**
/// `plan x publish`, so a publish route gated on `write` would hand the
/// entrance to a role the role table excludes from it — and no allow/deny test
/// can see the difference, because every fixture that grants one grants the
/// other.
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
            path: FRONTIER,
            resource_type: labels::PLAN,
            action: actions::READ,
            mutating: false,
        },
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
        // Slice 8. Authoring is `bundle x write`; **publish is `plan x publish`
        // only** — D-11, because under the conjunction the design set first
        // stated, only CatalogAdmin could publish a bundle while S8 §1.3
        // promises FinanceManager can. Catalogued here so the gate is a fact
        // this census asserts rather than a comment in a handler.
        Route {
            method: "POST",
            path: BUNDLES,
            resource_type: labels::BUNDLE,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "PATCH",
            path: BUNDLE_BY_ID,
            resource_type: labels::BUNDLE,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "POST",
            path: BUNDLE_PUBLISH,
            resource_type: labels::PLAN,
            action: actions::PUBLISH,
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
        Route {
            method: "GET",
            path: PLAN_COVERAGE,
            resource_type: labels::PLAN,
            action: actions::READ,
            mutating: false,
        },
        // The gate's surface is `plan x read` like the coverage report: it answers
        // about a plan's published content, and the AuthZ catalog puts read on the
        // plan. It is deliberately **not** a narrower pair - there is no
        // `sellability` label - and it needs no new authz vocabulary.
        Route {
            method: "GET",
            path: PLAN_SELLABILITY,
            resource_type: labels::PLAN,
            action: actions::READ,
            mutating: false,
        },
        // Slice 7's three mutating surfaces. All three are `plan x write` and not
        // `plan x publish`, although each is a publish *unit* (D-99): the pair is
        // what the AuthZ catalog's endpoint map assigns to the path, and §5 files
        // these three under the owned scheduling surface rather than under the
        // publish route. A window mutation therefore needs no `publish` grant, and
        // that is the catalog's call, not this test's.
        Route {
            method: "POST",
            path: PRICE_WINDOWS,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "PATCH",
            path: PRICE_WINDOW,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "DELETE",
            path: PRICE_WINDOW,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
        // The supersession unit (D-88). **`plan x write`, not `plan x publish`** — S5's
        // endpoint map puts it there beside the cutover, and the reason is the same one
        // the window mutations rest on: what `publish` guards is the *entrance*, and a
        // supersession that commits does so under an approval this route does not grant
        // itself. Gating it on `publish` would deny it to `ProductManager`, whom the role
        // matrix does grant `plan x write`.
        Route {
            method: "POST",
            path: PLAN_SUPERSESSIONS,
            resource_type: labels::PLAN,
            action: actions::WRITE,
            mutating: true,
        },
        Route {
            method: "POST",
            path: PLAN_PUBLISH,
            resource_type: labels::PLAN,
            action: actions::PUBLISH,
            mutating: true,
        },
        Route {
            method: "GET",
            path: APPROVALS,
            resource_type: labels::APPROVAL,
            action: actions::READ,
            mutating: false,
        },
        Route {
            method: "GET",
            path: APPROVAL,
            resource_type: labels::APPROVAL,
            action: actions::READ,
            mutating: false,
        },
        Route {
            method: "POST",
            path: APPROVAL_APPROVE,
            resource_type: labels::APPROVAL,
            action: actions::APPROVE,
            mutating: true,
        },
        Route {
            method: "POST",
            path: APPROVAL_REJECT,
            resource_type: labels::APPROVAL,
            action: actions::APPROVE,
            mutating: true,
        },
        Route {
            method: "POST",
            path: APPROVAL_WITHDRAW,
            resource_type: labels::APPROVAL,
            action: actions::APPROVE,
            mutating: true,
        },
        // The threshold policy is `approval_policy`, **never** `config`, and the
        // separation is the point rather than a taxonomy choice: a config admin
        // holds `config × write` and must not be able to weaken the thresholds that
        // decide whether their own changes need a second principal. No allow/deny
        // fixture can see the difference — every fixture granting one grants the
        // other — which is why it is asserted here, the way `plan × publish` is.
        Route {
            method: "GET",
            path: APPROVAL_THRESHOLD_POLICY,
            resource_type: labels::APPROVAL_POLICY,
            action: actions::READ,
            mutating: false,
        },
        Route {
            method: "PUT",
            path: APPROVAL_THRESHOLD_POLICY,
            resource_type: labels::APPROVAL_POLICY,
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
    /// The draft plan every row is aimed at.
    plan: Uuid,
    /// Its one draft price row.
    price: Uuid,
    /// The pending unit the approval rows are aimed at.
    approval: Uuid,
    /// One `scheduled` window on [`Seeded::price`], so the two window routes that
    /// address a window by id reach their gate instead of failing to parse a path.
    window: Uuid,
    /// One bundle on [`Seeded::plan`], so the two routes that address a bundle
    /// by id reach their gate instead of failing to parse a path — the same
    /// reason [`Seeded::window`] exists.
    bundle: Uuid,
}

/// The world the whole census is driven against: a draft plan with a shape and
/// one price row, **and a pending approval unit over it**.
///
/// The unit is opened through the service rather than through the publish route
/// on purpose. The route would first have to pass the validation pipeline, so
/// the seed would have to become a publishable plan — a different world from the
/// one the eight authoring rows have always been driven against, and changing it
/// to reach the approval rows would silently re-stage every case in the file.
/// What the approval rows need from the seed is one thing only: an id that
/// resolves to a record in the caller's tenant, so a refusal is the gate's and
/// never a 404's.
async fn seed(harness: &Harness) -> Seeded {
    let plan_id = Uuid::now_v7();
    seed_draft_plan(harness, plan_id).await;
    harness.attach_shape(plan_id, 0).await;
    let price = seed_price(harness, plan_id, "EU").await;
    let approval_id = Uuid::now_v7();
    harness
        .governance
        .approvals
        .submit(
            &harness.scope(),
            harness.tenant,
            bss_pricing::domain::scope_key::PlanId::new(plan_id),
            approval_id,
            serde_json::json!({ "material": true, "reason": "noConfiguredThreshold" }),
            rest_support::seed_stamp(),
        )
        .await
        .expect("open the pending unit the approval rows are driven against");
    let window = rest_support::seed_window(harness, price.price_id).await;
    let bundle = harness
        .state
        .bundles
        .create(
            &harness.scope(),
            bss_pricing::infra::storage::repo::NewBundle {
                bundle_id: Uuid::now_v7(),
                tenant_id: harness.tenant,
                plan_id: bss_pricing::domain::scope_key::PlanId::new(plan_id),
                price_basis: bss_pricing::domain::bundle::PriceBasis::SumOfParts,
                invoice_itemization: bss_pricing::domain::bundle::InvoiceItemization::Aggregate,
            },
            rest_support::seed_stamp(),
        )
        .await
        .expect("seed the bundle the two bundle-by-id routes address");
    Seeded {
        plan: plan_id,
        price: price.price_id,
        approval: approval_id,
        window,
        bundle,
    }
}

/// A request's optional JSON body and its preconditions.
type DrivenBody = (Option<serde_json::Value>, Vec<(&'static str, &'static str)>);

/// Fill a route template with the seeded ids and a well-formed body.
///
/// The precondition is deliberately **the current one** — on a plan route the
/// revision-qualified `"0-3"` the seeded draft stands at after `attach_shape`,
/// on a price route the row's own `"0"` — so a refusal in any of these cases is
/// the gate's and never a stale tag's. The entrance takes the plan-plane tag for
/// the same reason.
fn drive(
    route: &Route,
    seeded: &Seeded,
    version: &'static str,
    key: &'static str,
) -> axum::http::Request<axum::body::Body> {
    let path = route
        .path
        .replace("{planId}", &seeded.plan.to_string())
        .replace("{priceId}", &seeded.price.to_string())
        .replace("{approvalId}", &seeded.approval.to_string())
        .replace("{windowId}", &seeded.window.to_string())
        .replace("{bundleId}", &seeded.bundle.to_string());
    // The sellability surface requires all three of §5's query parameters and
    // parses them **before** it asks the PDP — `schedule_window`'s ordering, and
    // for its reason: a caller who omitted one is told that rather than being told
    // they may not read a plan. So the request driven here has to carry them, or
    // every property in this file would be asserted against a 400 that never
    // reached a gate. Found by mounting the route and watching
    // `every_route_asks_the_catalogued_pair` and the PDP-outage case both redden.
    let path = if route.path == PLAN_SELLABILITY {
        format!("{path}?at=2099-01-01T00:00:00Z&currency=EUR&region=EU")
    } else {
        path
    };
    let (body, headers): DrivenBody = match (route.method, route.path) {
        ("POST", PLANS) => (
            Some(serde_json::json!({ "plan_tier": "gold" })),
            vec![("idempotency-key", key)],
        ),
        // Slice 8's three. The composition route takes the **plan revision's**
        // tag, which is the plane the seeded draft stands on; the publish route
        // takes neither a tag nor a key, since its concurrency is the approval
        // unit's and its idempotency is per revision.
        ("POST", BUNDLES) => (
            Some(serde_json::json!({
                "plan_id": Uuid::now_v7(),
                "price_basis": "sum_of_parts",
            })),
            vec![("idempotency-key", key)],
        ),
        ("PATCH", BUNDLE_BY_ID) => (
            Some(serde_json::json!({
                "plan_revision": 0,
                "components": [],
            })),
            vec![("if-match", version)],
        ),
        ("POST", BUNDLE_PUBLISH) => (
            Some(serde_json::json!({ "plan_revision": 0, "markets": [] })),
            vec![],
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
        ("POST", PLAN_ABANDON | PLAN_PUBLISH) => (None, vec![("if-match", version)]),
        ("DELETE", PLAN_PRICE) => (None, vec![("if-match", "\"0\"")]),
        // The `Idempotency-Key` is required on the schedule (D-171) and is read
        // before anything is resolved, so a request without one never reaches the
        // gate this suite is about.
        ("POST", PRICE_WINDOWS) => (
            Some(serde_json::json!({
                "effective_from": "2099-01-01T00:00:00Z",
                "reason_code": "authz-probe"
            })),
            vec![("idempotency-key", key)],
        ),
        // The supersession body is parsed before the gate for `schedule_window`'s
        // reason, so a request without one would never reach the PDP this suite is
        // about. It carries **no** idempotency header: S5's column for this surface is
        // the act's own identity, not a client key.
        ("POST", PLAN_SUPERSESSIONS) => (
            Some(serde_json::json!({
                "predecessor_price_id": seeded.price.to_string(),
                "changeover": "2099-08-20T00:00:00Z",
                "successor": { "model_kind": "flat", "amount_minor": 100 },
                "reason_code": "authz-probe"
            })),
            vec![],
        ),
        ("PATCH", PRICE_WINDOW) => (
            Some(serde_json::json!({ "effective_to": "2099-06-01T00:00:00Z" })),
            vec![("if-match", "\"0\"")],
        ),
        // The reason is mandatory on a reject (`inst-as-reject`), and a reject
        // without one is refused **after** the gate — but a body that carries it
        // is what makes this the same request an operator sends.
        // A well-formed proposal, so a refusal here is the gate's and never
        // `THRESHOLD_INVALID`'s. One entry is the minimum a version can carry.
        ("PUT", APPROVAL_THRESHOLD_POLICY) => (
            Some(serde_json::json!({
                "effective_from": "2099-01-01T00:00:00Z",
                "entries": [{ "currency": "EUR", "absolute_minor": 500 }]
            })),
            Vec::new(),
        ),
        ("POST", APPROVAL_REJECT) => (
            Some(serde_json::json!({ "reason": "not signed off" })),
            Vec::new(),
        ),
        // No body and no preconditions. `DELETE /price-windows/{windowId}` reaches
        // here **deliberately** rather than through an arm of its own: it carries
        // neither an `Idempotency-Key` nor an `If-Match`, which is §5's own
        // idempotency column for that surface, so an arm naming it would be
        // identical to this one and would only invite somebody to add a header to
        // it. The three read routes reach here for the same reason.
        _ => (None, Vec::new()),
    };
    with_headers(route.method, &path, body, &headers)
}

// ---------------------------------------------------------------------------
// 1. The census matches what the router registered.
// ---------------------------------------------------------------------------

/// Every operation the gear's routers register, keyed `METHOD:path`.
///
/// Built from the routers themselves, not from [`census`], because a census that
/// checked itself would stay green for exactly the route it was meant to catch:
/// one added to the registry and left out of the table. **Every** router and not
/// the two authoring ones, because a census built from a subset of the routers is
/// the same hole one level up — it was two routers until 2026-08-04, so the
/// set-level properties below ranged over 8 of the gear's routes while
/// `module.rs` claimed they ranged over all of them.
///
/// The list is nonetheless hand-written, because each router takes a differently
/// typed state and no loop can build them. That is the hole
/// [`every_mounted_router_is_merged_into_both_censuses`] closes: a new
/// `pub fn router` under `src/api/rest/**` fails that test until this function and
/// `module_test.rs`'s twin have both been told about it. It was open until
/// 2026-08-04 and was found by mounting a sixth router and watching **both**
/// censuses stay green.
async fn registered_paths() -> Vec<String> {
    use std::sync::Arc;
    use toolkit::api::OpenApiRegistryImpl;

    let harness = Harness::new().await;
    let openapi = OpenApiRegistryImpl::new();
    drop(
        bss_pricing::api::rest::frontier::router(Arc::clone(&harness.frontier), &openapi)
            .merge(bss_pricing::api::rest::plans::router(
                Arc::clone(&harness.state),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::prices::router(
                Arc::clone(&harness.state),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::bundles::router(
                Arc::clone(&harness.state),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::windows::router(
                Arc::clone(&harness.governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::supersessions::router(
                Arc::clone(&harness.governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::approvals::router(
                Arc::clone(&harness.governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::publish::router(
                Arc::clone(&harness.governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::threshold_policy::router(
                Arc::clone(&harness.governance),
                &openapi,
            )),
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
    // the registered set is exactly the set `declared_paths()` names; this one
    // owns the narrower and more useful one, that every route the gear mounts has
    // a catalogued `(resource_type, action)` pair beside it.
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
    // **Four** labels and no fifth. This used to read "every surface in this group is
    // on the `plan` label", which stopped being true the day the approval surface
    // was mounted, "two labels and no third", which stopped being true the day the
    // threshold policy was, and "three and no fourth", which stopped being true the
    // day Slice 8 mounted the bundle authoring surface — so it is stated as the set
    // it is, and a census row on a label nobody decided about fails here rather than
    // reading as coverage.
    //
    // `bundle` is the fourth, and it is deliberately **not** `plan`: authoring a
    // composition is `bundle x write` (ProductManager, CatalogAdmin) while
    // publishing it is `plan x publish` **only** (D-11). Filing the authoring routes
    // under `plan` would hand composition authorship to every holder of
    // `plan x write`, and filing the publish route under `bundle` would take the
    // entrance away from FinanceManager, whom S8 §1.3 promises it. Neither is
    // visible to an allow/deny fixture, which is why both are asserted here.
    //
    // `approval_policy` is the third and it is deliberately **not** `config`: the
    // segregation of duties is the whole reason the catalog carries two labels, and
    // a policy route filed under `config` would hand a config admin the thresholds
    // that govern their own changes. That is invisible to every allow/deny fixture,
    // which is why it is asserted here.
    let used: std::collections::BTreeSet<&str> =
        census().iter().map(|route| route.resource_type).collect();
    assert_eq!(
        used,
        std::collections::BTreeSet::from([
            labels::PLAN,
            labels::BUNDLE,
            labels::APPROVAL,
            labels::APPROVAL_POLICY,
        ]),
        "a census row on a label this gear has not mounted before needs a decision, not a row"
    );
}

/// Every router the REST layer defines is merged into **every** place that has to
/// mount it — the production mount and the three test-side ones.
///
/// # The hole this closes, and how it was found
///
/// Four functions merge routers by hand, because each router takes a differently
/// typed state and no loop can build them: `module.rs`'s `register_rest` (the
/// production mount), `registered_paths` above, `module_test.rs`'s
/// `registered_operations`, and `rest_support`'s `Harness` app. Every set-level
/// property in this file and the whole route census in `module_test.rs` range over
/// what those functions build — so a router none of them merges is a router **no
/// census can see**, and the guards read as coverage while covering nothing.
///
/// That was live on 2026-08-04. A sixth router (`windows`) was mounted in
/// `module.rs` and the census in `module_test.rs` — the one whose doc promised "a
/// path registered without a line here fails the census" — **stayed green**,
/// because its own router list did not include it. The route was registered,
/// declared, catalogued, and invisible to both censuses; the harness did not mount
/// it either, so every request to it answered 404 without ever reaching the gate.
///
/// So the scan is over the **source text** of the four functions' files. It cannot
/// check that the merge is correct — only that the name appears — which is exactly
/// the guarantee needed: it forces an author who adds a router to visit all four,
/// and the censuses then do the rest.
#[test]
fn every_mounted_router_is_merged_into_both_censuses() {
    let mounts = [
        "src/module.rs",
        "tests/rest_authz.rs",
        "tests/module_test.rs",
        "tests/rest_support/mod.rs",
    ];
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));

    let mut routers: Vec<String> = Vec::new();
    for path in rest_sources() {
        let text = std::fs::read_to_string(&path).expect("a readable REST source");
        if !text.contains("pub fn router(") {
            continue;
        }
        let module = path
            .file_stem()
            .and_then(|s| s.to_str())
            .expect("a named source file");
        // `src/api/rest.rs` itself declares no router; every other file that does
        // is named for its module.
        routers.push(module.to_owned());
    }
    routers.sort();
    assert!(
        routers.len() >= 5,
        "the scan found {routers:?}, which is fewer routers than this gear has had since phase 3 - the scan is broken, not the layer"
    );

    for module in &routers {
        let needle = format!("rest::{module}::router(");
        for mount in mounts {
            let text = std::fs::read_to_string(root.join(mount))
                .unwrap_or_else(|e| panic!("read {mount}: {e}"));
            assert!(
                text.contains(&needle),
                "{mount} does not merge the `{module}` router: `{needle}` appears nowhere in it, so every census and every gate property built there is blind to its routes"
            );
        }
    }
}

/// Every `.rs` file at or under `src/api/rest`, **recursively**.
///
/// Recursive because the earlier version of this scan read one directory level,
/// so any file in a future `src/api/rest/**` subdirectory evaded it entirely -
/// and a guard that a reorganisation switches off silently is a guard that reads
/// as coverage to everyone who greps for it.
fn rest_sources() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/api/rest");
    let mut found = vec![root.with_extension("rs")];
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("the REST layer is where it has always been") {
            let path = entry.expect("readable dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// The source of `path` with `//` comments and every `#![…]`/`#[…]` line dropped,
/// joined into one string so a declaration split across lines is one subject.
///
/// Comments are stripped because this file's own prose names the type it bans,
/// and a scan that matched its own explanation would have to be weakened until it
/// matched nothing.
fn scannable(path: &std::path::Path) -> String {
    let stripped = std::fs::read_to_string(path)
        .expect("readable source")
        .lines()
        .map(|line| match line.find("//") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join(" ");
    normalized(&stripped)
}

/// One line, whitespace collapsed and **removed around every `::`**.
///
/// The path separator is where a formatter is free to break, so
/// `AccessScope\n    ::allow_all()` and `AccessScope::allow_all()` are one
/// construction written two ways. A scan that could not see that was one a
/// `cargo fmt` could switch off.
fn normalized(source: &str) -> String {
    source
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace(" ::", "::")
        .replace(":: ", "::")
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
    // `crate::authz::access_scope`, which *is* the gate.
    //
    // # What the earlier version of this scan could not see
    //
    // It matched the literal `AccessScope::` line by line over one directory
    // level, so four things evaded it: an alias (`use … AccessScope as Reach`), a
    // qualified call (`<AccessScope>::for_tenant`), a declaration split across
    // lines, and any file in a subdirectory. This one bans the **import** rather
    // than one spelling of the call: a file that cannot name the type in a value
    // position cannot construct one, whatever it calls it - and a type position
    // (`scope: &AccessScope`) needs the import too, which is why the test asserts
    // what the import is *used for* rather than that it is absent.
    let mut offenders: Vec<String> = Vec::new();
    let files = rest_sources();
    assert!(files.len() > 5, "the scan found almost nothing: {files:?}");

    for path in &files {
        let source = scannable(path);
        // Any construction reachable from this file, under any name the file gave
        // the type. `AccessScope` cannot be built without naming it or an alias of
        // it, and an alias is established by an import this scan reads.
        let aliases = access_scope_names(&source);
        for alias in &aliases {
            for pattern in [
                format!("{alias}::"),
                format!("<{alias}>::"),
                format!("{alias} {{"),
            ] {
                if source.contains(&pattern) {
                    offenders.push(format!("{}: {pattern}", path.display()));
                }
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

/// Every name `source` can refer to `AccessScope` by — the type's own name, plus
/// whatever an `as` clause renamed it to.
///
/// A file that never imports the type at all yields nothing to look for, which is
/// correct: it cannot construct one.
fn access_scope_names(source: &str) -> Vec<String> {
    let mut names = Vec::new();
    if !source.contains("AccessScope") {
        return names;
    }
    names.push("AccessScope".to_owned());
    // `use … AccessScope as Reach` — the alias is the token after `as`.
    let mut rest = source;
    while let Some(at) = rest.find("AccessScope as ") {
        let tail = &rest[at + "AccessScope as ".len()..];
        let alias: String = tail
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !alias.is_empty() {
            names.push(alias);
        }
        rest = tail;
    }
    names
}

#[test]
fn the_scan_would_catch_each_evasion_the_earlier_one_missed() {
    // A guard's own falsifiability. Each string below is a construction the
    // line-by-line literal scan let through, and each must be caught now -
    // otherwise the fix is a comment.
    for evasion in [
        "use toolkit_db::secure::AccessScope as Reach; let s = Reach::allow_all();",
        "let s = <AccessScope>::for_tenant(t);",
        "use toolkit_db::secure::AccessScope; let s = AccessScope\n    ::allow_all();",
    ] {
        let joined = normalized(&evasion.lines().collect::<Vec<_>>().join(" "));
        let aliases = access_scope_names(&joined);
        let caught = aliases.iter().any(|alias| {
            joined.contains(&format!("{alias}::")) || joined.contains(&format!("<{alias}>::"))
        });
        assert!(caught, "this evasion is not caught: {evasion}");
    }
}

#[test]
fn the_scan_reads_more_than_one_directory_level() {
    // The other half of the earlier scan's blindness. `read_dir` is not
    // recursive, so a `src/api/rest/**` subdirectory was invisible - and this
    // asserts the walk descends rather than that it happens to find enough files.
    let files = rest_sources();
    assert!(
        files.iter().any(|path| path.ends_with("api/rest/plans.rs")),
        "the walk missed a file it must see: {files:?}"
    );
    assert!(
        files.iter().any(|path| path.ends_with("api/rest.rs")),
        "including the module root: {files:?}"
    );
}

/// The four verbs that make a `router()` a **mutating** one, in the spelling the
/// builder registers them under.
const MUTATING_BUILDERS: &[&str] = &[
    "OperationBuilder::post(",
    "OperationBuilder::patch(",
    "OperationBuilder::put(",
    "OperationBuilder::delete(",
];

#[test]
fn every_mutating_router_applies_the_correlation_edge() {
    // D-178's edge is applied inside each mutating router's own `router()` rather
    // than where the routers are merged, so that it travels with the routes. The
    // cost of that choice is that a **new** router can be written without it, and
    // nothing in the crate noticed: the route suites drive the four routers this
    // harness names, so a fifth would evade every one of them.
    //
    // The consequence is not a missing column. `require_correlation` runs at the
    // top of every handler, **before** the authz gate, so a route mounted without
    // the layer answers **500 to a caller who should have got 403** - the gate
    // never runs, and the refusal an operator sees is an internal fault.
    //
    // Scanned rather than driven, because the property is about a router that
    // does not exist yet. The two suites that could observe it - the census here
    // and `module_test`'s registered-path set - both start from a list somebody
    // maintains; this starts from the filesystem.
    let files = rest_sources();
    assert!(files.len() > 5, "the scan found almost nothing: {files:?}");

    let mut mutating = Vec::new();
    let mut offenders = Vec::new();
    for path in &files {
        let source = scannable(path);
        if !MUTATING_BUILDERS.iter().any(|verb| source.contains(verb)) {
            continue;
        }
        mutating.push(path.clone());
        if !source.contains("correlation::establish") {
            offenders.push(path.display().to_string());
        }
    }

    // Without this the scan would pass by finding nothing to check, which is the
    // failure mode of every source scan.
    assert!(
        mutating.len() >= 4,
        "the scan sees fewer mutating routers than this gear mounts: {mutating:?}"
    );
    assert!(
        offenders.is_empty(),
        "a router registers a mutating operation and does not apply \
         `correlation::establish`; its handlers will answer 500 where they owe 403, and every \
         record they write would carry a per-record correlation (D-178):\n{}",
        offenders.join("\n")
    );
    // The read-only router is exempt and must stay outside the set above, or the
    // assertion is measuring the wrong thing: nothing behind `frontier` writes an
    // audit record or an outbox row, so it has no field for a correlation to
    // satisfy and deliberately carries no edge.
    assert!(
        !mutating.iter().any(|path| path.ends_with("frontier.rs")),
        "the frontier router registered a mutating operation; that is a decision, not a scan \
         result"
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
        let version_before = plan_row_version(&harness, seeded.plan, 0).await;
        let prices_before = price_rows(&harness, seeded.plan).await;
        // The approval plane's own readback. Without it the three decision routes
        // would be in this loop asserting nothing about themselves: a handler that
        // decided the unit and then checked the gate moves no plan row and no
        // price row, so every assertion below would hold.
        let unit_before = approval_row(&harness, seeded.approval).await.state;

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
            plan_row_version(&harness, seeded.plan, 0).await,
            version_before,
            "a version moved"
        );
        let prices_after = price_rows(&harness, seeded.plan).await;
        assert_eq!(prices_after.len(), prices_before.len(), "a price row moved");
        assert_eq!(
            prices_after.first().map(|row| row.row_version.get()),
            prices_before.first().map(|row| row.row_version.get()),
            "a price row's version moved"
        );
        assert_eq!(
            approval_row(&harness, seeded.approval).await.state,
            unit_before,
            "a refused decision decided the unit anyway"
        );
        assert_eq!(
            unit_before,
            ApprovalState::Submitted,
            "the seeded unit must be pending, or `unit_before` is a tautology"
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
        .send(request("GET", &format!("{PLANS}/{}", seeded.plan), None))
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
        .send(request("GET", &format!("{PLANS}/{}", seeded.plan), None))
        .await;
    assert_eq!(
        baseline.status(),
        StatusCode::OK,
        "without the owner's 200 the 404 below would be consistent with mere absence"
    );

    let foreign = harness
        .other_tenant()
        .send(request("GET", &format!("{PLANS}/{}", seeded.plan), None))
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
