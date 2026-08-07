//! Gear-declaration smoke tests: the capability wiring is real, not decorative.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use bss_pricing::api::rest::state::{AuthoringState, GovernanceState};
use bss_pricing::config::LimitsConfig;
use bss_pricing::infra::approval::ApprovalService;
use bss_pricing::infra::fixture_gate::FixtureGate;
use bss_pricing::infra::publish::PublishService;
use bss_pricing::infra::storage::repo::{
    IdempotencyGate, PinFrontierRepo, PlanRepo, PlanShapeRepo, PriceRepo,
};
use bss_pricing::infra::window::WindowService;
use bss_pricing::module::BssPricingGear;
use toolkit::GearCtx;
use toolkit::api::OpenApiRegistryImpl;
use toolkit::api::operation_builder::ParamLocation;
use toolkit::contracts::{DatabaseCapability, RestApiCapability};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};

#[test]
fn the_gear_declares_the_database_capability() {
    // The `db` capability must resolve to the Foundation chain, not to an empty
    // vec: a gear that declares `db` and hands the platform nothing has tables
    // no migration ever creates.
    let gear = BssPricingGear::default();

    assert!(
        !gear.migrations().is_empty(),
        "the Foundation migration chain must be wired into the db capability"
    );
}

#[test]
fn every_migration_name_is_unique() {
    // The toolkit runner applies migrations in NAME order and rejects a
    // duplicate name outright — so a copy-pasted `DeriveMigrationName` does not
    // merely sort oddly, it aborts the whole chain at boot. Asserting it here
    // means the mistake is caught by `cargo test` rather than by a crash loop
    // in a cluster.
    let gear = BssPricingGear::default();
    let migrations = gear.migrations();
    let names: Vec<String> = migrations.iter().map(|m| m.name().to_owned()).collect();
    let unique: HashSet<&String> = names.iter().collect();

    assert_eq!(
        unique.len(),
        names.len(),
        "duplicate migration name in the chain: {names:?}"
    );
}

/// Every route this gear serves, spelled through the modules' own consts.
///
/// A path registered without a line here fails the census below, which is what
/// stops a route landing without anybody deciding it should — **as long as the
/// router that registers it is merged into `registered_operations`**, which is
/// hand-written and was the hole `rest_authz.rs`'s
/// `every_mounted_router_is_merged_into_both_censuses` now closes. The list is
/// also the only thing pinning those `const`s against the **literals** the
/// `OperationBuilder` calls take: DE0801 validates a literal argument and
/// silently passes a `const` one, so the route-shape rule binds where the
/// literal is, and this binds the two spellings together.
fn declared_paths() -> Vec<(&'static str, &'static str)> {
    use bss_pricing::api::rest::approvals::{
        APPROVAL, APPROVAL_APPROVE, APPROVAL_REJECT, APPROVAL_WITHDRAW, APPROVALS,
    };
    use bss_pricing::api::rest::bundles::{BUNDLE_BY_ID, BUNDLE_PUBLISH, BUNDLES};
    use bss_pricing::api::rest::cutovers::PLAN_CUTOVERS;
    use bss_pricing::api::rest::frontier::FRONTIER;
    use bss_pricing::api::rest::overlays::{
        PRICE_OVERLAY_BY_ID, PRICE_OVERLAY_SUBMIT, PRICE_OVERLAYS,
    };
    use bss_pricing::api::rest::plans::{PLAN, PLAN_ABANDON, PLANS};
    use bss_pricing::api::rest::prices::{PLAN_PRICE, PLAN_PRICES};
    use bss_pricing::api::rest::publish::PLAN_PUBLISH;
    use bss_pricing::api::rest::supersessions::PLAN_SUPERSESSIONS;
    use bss_pricing::api::rest::threshold_policy::APPROVAL_THRESHOLD_POLICY;
    use bss_pricing::api::rest::windows::{
        PLAN_COVERAGE, PLAN_SELLABILITY, PRICE_WINDOW, PRICE_WINDOWS,
    };
    vec![
        ("GET", FRONTIER),
        ("GET", PLAN),
        ("POST", PLANS),
        ("PATCH", PLAN),
        ("POST", PLAN_ABANDON),
        ("POST", PLAN_PRICES),
        ("GET", PLAN_PRICES),
        ("PATCH", PLAN_PRICE),
        ("DELETE", PLAN_PRICE),
        // Slice 8's three (`design/08-bundles.md` §5). The publish answers 202,
        // per `inst-ba-return`: the composition is frozen into the read model by
        // the projector, which the response does not wait for.
        // Slice 9's overlay half (`design/09-price-overlays.md` §5). The `PATCH`
        // is mounted per-resource rather than on the collection §5 spells,
        // because a precondition addresses a resource — the divergence Slice 8
        // reported for its own composition route and this one inherits. The
        // submit answers 202: it opens the always-material approval unit (D-50).
        ("POST", PRICE_OVERLAYS),
        ("GET", PRICE_OVERLAYS),
        ("PATCH", PRICE_OVERLAY_BY_ID),
        ("POST", PRICE_OVERLAY_SUBMIT),
        ("POST", BUNDLES),
        ("PATCH", BUNDLE_BY_ID),
        ("POST", BUNDLE_PUBLISH),
        // Slice 7's two reads: the coverage report an operator remediates from,
        // and the gate's surface over one pinned delta. Two routes and not one
        // because they answer different questions over different row sets - see
        // the `api::rest::windows` module doc.
        ("GET", PLAN_COVERAGE),
        ("GET", PLAN_SELLABILITY),
        // Slice 7's mutating half: the window surfaces §5 declares, each a publish
        // unit (D-99) answering 202, and two of them under D-62's two-person control.
        // The `DELETE` carries no idempotency header, which is §5's own column and is
        // reported as a divergence in `api::rest::windows`.
        ("POST", PRICE_WINDOWS),
        ("PATCH", PRICE_WINDOW),
        ("DELETE", PRICE_WINDOW),
        // Slice 7's interactive repricing: the supersession unit (D-88), one route in a
        // module of its own. It answers 202 on both arms and takes **no** idempotency
        // header, which is S5's own column for it — the act's identity is the key — so
        // it is deliberately absent from `idempotency_key_routes()` below and the
        // divergence its natural key leaves past the commit is reported in
        // `api::rest::supersessions`.
        ("POST", PLAN_SUPERSESSIONS),
        ("POST", PLAN_CUTOVERS),
        // Slice 5's entrance: the publish mount and the approval surface.
        ("POST", PLAN_PUBLISH),
        ("GET", APPROVALS),
        ("GET", APPROVAL),
        ("POST", APPROVAL_APPROVE),
        ("POST", APPROVAL_REJECT),
        ("POST", APPROVAL_WITHDRAW),
        // Slice 5's governance config: the threshold policy a D-10 unit is opened
        // over, and D-185's way back to §6's *unset ⇒ two-person rule always* — the
        // tombstone, authored through the `PUT`'s `retire` marker rather than a verb
        // of its own, so there are two operations here and not three.
        //
        // The `PUT` requires an `If-Match` (D-186) and is therefore in
        // `if_match_routes()` below. The sentence that used to stand here said it
        // carried no precondition "because a tenant's first proposal has no prior
        // version to name and a mandatory `If-Match` would make the bootstrap
        // unreachable" — false, and withdrawn: the `GET` answers 200 with
        // `effective: null`, so there is always a representation to tag.
        ("GET", APPROVAL_THRESHOLD_POLICY),
        ("PUT", APPROVAL_THRESHOLD_POLICY),
    ]
}

/// Build every mounted router over a connected-but-empty database and hand back
/// what they registered.
///
/// The number of them is deliberately not in this sentence: it said "three" while six
/// were merged, and a count beside a roster is one more thing to keep true. **The
/// sentence then said "a seventh router" while the seventh was already in the chain
/// below** — the same defect one clause over, which is why the ordinal is gone too. The
/// merge chain below is the roster, and `rest_authz.rs`'s
/// `every_mounted_router_is_merged_into_both_censuses` is what stops **the next** router
/// being written without joining it.
///
/// No migrations: nothing here sends a request, and the registration happens
/// while the router is built. What this needs a provider for is that the states
/// hold one.
async fn registered_operations() -> OpenApiRegistryImpl {
    let db = connect_db("sqlite::memory:", ConnectOpts::default())
        .await
        .expect("connect in-memory sqlite");
    let db = DBProvider::<DbError>::new(db);
    let openapi = OpenApiRegistryImpl::new();

    let frontier_state = Arc::new(bss_pricing::api::rest::frontier::ApiState {
        pin_frontier: PinFrontierRepo::new(db.clone()),
    });
    let approvals = ApprovalService::new(db.clone());
    let authoring = Arc::new(AuthoringState {
        approvals: approvals.clone(),
        db: db.clone(),
        plans: PlanRepo::new(db.clone()),
        shapes: PlanShapeRepo::new(db.clone()),
        prices: PriceRepo::new(db.clone()),
        bundles: bss_pricing::infra::storage::repo::BundleRepo::new(db.clone()),
        bundle_service: bss_pricing::infra::bundle::BundleService::new(db.clone()),
        overlays: bss_pricing::infra::storage::repo::OverlayRepo::new(db.clone()),
        idempotency: IdempotencyGate::new(Duration::from_hours(1)),
    });
    // The registry is the fail-closed production default and the fixture gate is
    // loaded from a path that does not exist, which closes it. Neither matters:
    // registration happens while the router is built and nothing here sends a
    // request. Wiring a working pair would be wiring a publish this test does
    // not perform.
    let governance = Arc::new(GovernanceState {
        db: db.clone(),
        idempotency: bss_pricing::infra::storage::repo::IdempotencyGate::new(
            LimitsConfig::default().idempotency_key_ttl(),
        ),
        thresholds: bss_pricing::infra::threshold::ThresholdService::new(db.clone()),
        plans: PlanRepo::new(db.clone()),
        prices: PriceRepo::new(db.clone()),
        approvals,
        overlays: bss_pricing::infra::storage::repo::OverlayRepo::new(db.clone()),
        overlay_publish: bss_pricing::infra::overlay_publish::OverlayPublishService::new(
            db.clone(),
            Arc::new(
                bss_pricing_sdk::catalog_version_registry::UnconfiguredCatalogVersionRegistryV1,
            ),
        ),
        windows: WindowService::new(
            db.clone(),
            Arc::new(
                bss_pricing_sdk::catalog_version_registry::UnconfiguredCatalogVersionRegistryV1,
            ),
        ),
        supersessions: bss_pricing::infra::supersession::SupersessionService::new(
            db.clone(),
            Arc::new(
                bss_pricing_sdk::catalog_version_registry::UnconfiguredCatalogVersionRegistryV1,
            ),
        ),
        cutovers: bss_pricing::infra::cutover::CutoverService::new(
            db.clone(),
            Arc::new(
                bss_pricing_sdk::catalog_version_registry::UnconfiguredCatalogVersionRegistryV1,
            ),
        ),
        publish: PublishService::new(
            db,
            &LimitsConfig::default(),
            FixtureGate::load(std::path::Path::new("/nonexistent/registry.toml")),
            Arc::new(
                bss_pricing_sdk::catalog_version_registry::UnconfiguredCatalogVersionRegistryV1,
            ),
        ),
    });

    drop(
        bss_pricing::api::rest::frontier::router(frontier_state, &openapi)
            .merge(bss_pricing::api::rest::plans::router(
                Arc::clone(&authoring),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::prices::router(
                Arc::clone(&authoring),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::overlays::router(
                Arc::clone(&authoring),
                &openapi,
            ))
            // The submit route publishes (D-234), so it is mounted on the
            // governance state apart from its authoring siblings.
            .merge(bss_pricing::api::rest::overlays::governance_router(
                Arc::clone(&governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::bundles::router(
                Arc::clone(&authoring),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::windows::router(
                Arc::clone(&governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::supersessions::router(
                Arc::clone(&governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::cutovers::router(
                Arc::clone(&governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::approvals::router(
                Arc::clone(&governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::publish::router(
                Arc::clone(&governance),
                &openapi,
            ))
            .merge(bss_pricing::api::rest::threshold_policy::router(
                governance, &openapi,
            )),
    );
    openapi
}

#[tokio::test]
async fn the_registered_route_set_is_exactly_the_declared_paths() {
    // Six groups built repositories, guards, a validator, a commit and a
    // projector, and Slice 5 gave the commit an entrance; this is the whole of
    // what an operator can reach. A route added later without a line in
    // `declared_paths` fails here rather than shipping ungated and
    // undocumented.
    let openapi = registered_operations().await;

    let mut found: Vec<String> = openapi
        .operation_specs
        .iter()
        .map(|entry| entry.key().clone())
        .collect();
    found.sort();
    let mut expected: Vec<String> = declared_paths()
        .into_iter()
        .map(|(method, path)| format!("{method}:{path}"))
        .collect();
    expected.sort();

    assert_eq!(found, expected);
}

#[tokio::test]
async fn every_registered_operation_carries_a_tag_and_a_summary() {
    // DE0205 in a test as well as in a lint: an operation without them is one an
    // operator meets in a generated client with no name and no group.
    let openapi = registered_operations().await;

    for entry in &openapi.operation_specs {
        let spec = entry.value();
        assert!(!spec.tags.is_empty(), "{} carries no tag", entry.key());
        assert!(
            spec.summary.as_ref().is_some_and(|s| !s.trim().is_empty()),
            "{} carries no summary",
            entry.key()
        );
    }
}

#[tokio::test]
async fn no_operation_declares_a_422() {
    // §3.3 states it once for the whole design set: the platform has no 422
    // category, every architectural 422 reaches the wire as a 400 carrying its
    // code, and an endpoint MUST NOT declare a response no path can produce.
    let openapi = registered_operations().await;

    for entry in &openapi.operation_specs {
        for response in &entry.value().responses {
            assert_ne!(
                response.status,
                422,
                "{} declares a 422; no path in this gear can produce one",
                entry.key()
            );
        }
    }
}

/// A config provider that knows about no gear at all.
struct NoConfig;

impl toolkit::config::ConfigProvider for NoConfig {
    fn get_gear_config(&self, _gear: &str) -> Option<&serde_json::Value> {
        None
    }
}

#[tokio::test]
async fn an_unconfigured_gear_reserves_its_prefix_and_answers_404_under_it() {
    // A gear compiled in but absent from `gears:` has no runtime. It must still
    // claim `/bss-pricing/v1` - so another gear cannot take the namespace - and
    // a request under it must be a clean 404 rather than a 500 from a handler
    // reaching for state that was never built.
    let gear = BssPricingGear::default();
    let openapi = OpenApiRegistryImpl::new();
    let ctx = GearCtx::new(
        "bss-pricing",
        uuid::Uuid::new_v4(),
        Arc::new(NoConfig),
        Arc::new(toolkit::client_hub::ClientHub::new()),
        tokio_util::sync::CancellationToken::new(),
    );

    let router = gear
        .register_rest(&ctx, Router::new(), &openapi)
        .expect("an unconfigured gear still registers a router");

    assert_eq!(
        openapi.operation_specs.len(),
        0,
        "an unconfigured gear registers no operation at all"
    );

    let response = tower::ServiceExt::oneshot(
        router,
        axum::http::Request::builder()
            .method("GET")
            .uri("/bss-pricing/v1/plans/00000000-0000-0000-0000-000000000000")
            .body(axum::body::Body::empty())
            .expect("build request"),
    )
    .await
    .expect("the router answers");

    assert_eq!(response.status(), axum::http::StatusCode::NOT_FOUND);
}

/// Every mutating route that declares a precondition header, and which one.
///
/// Transcribed rather than derived from the router, for `declared_paths`' reason:
/// a declaration dropped from a registration is invisible if the expectation is
/// read off the same registration. The count is deliberately not in this
/// sentence — the roster below is the roster, and a number beside it is one more
/// thing to keep true (the doc said "six" against a list of seven for a whole
/// phase).
///
/// **The three approval decisions are deliberately not here.** §5's idempotency
/// cell for them reads *per decision*, and the decision is at-most-once by
/// construction: `approval_repo`'s compare-and-swap carries `state =
/// 'submitted'` in its own predicate, so a retry is refused
/// `APPROVAL_NOT_PENDING` whether or not a header was sent. An approval record
/// carries no version column for an `If-Match` to name, and declaring one would
/// tell a generated client to send a precondition the server cannot test.
///
/// **`DELETE /price-windows/{windowId}` is deliberately not here either**, and it is
/// the one absence a test enforces rather than a doc asserting: §5's Idempotency cell
/// for that surface is **empty**, so it declares neither header, and
/// `the_window_cancel_declares_no_precondition_header` is what keeps a later group
/// from adding one to be helpful.
fn if_match_routes() -> Vec<(&'static str, &'static str)> {
    use bss_pricing::api::rest::plans::{PLAN, PLAN_ABANDON, PLANS};
    use bss_pricing::api::rest::prices::{PLAN_PRICE, PLAN_PRICES};
    use bss_pricing::api::rest::publish::PLAN_PUBLISH;
    use bss_pricing::api::rest::threshold_policy::APPROVAL_THRESHOLD_POLICY;
    use bss_pricing::api::rest::windows::{PRICE_WINDOW, PRICE_WINDOWS};
    vec![
        ("PATCH", PLAN),
        ("POST", PLAN_ABANDON),
        ("PATCH", PLAN_PRICE),
        ("DELETE", PLAN_PRICE),
        // The publish mount: its tag names the revision it freezes **and** that
        // revision's version, which is what the commit's compare-and-swap
        // submits.
        ("POST", PLAN_PUBLISH),
        // Slice 7's window `PATCH`: §5 gives it an **ETag**, and on a window route the
        // tag is the window row's own version (D-141's rule for a price row, applied
        // to the surface that addresses one window by id).
        ("PATCH", PRICE_WINDOW),
        // Slice 5's policy `PUT`: §5's cell is *`ETag` + approval unit* and D-186
        // implements the first half. It is the one entry here whose tag is **not** a
        // row version — the store is append-only and has no version column — so the
        // tag is a digest over the representation the `GET` serves. The declaration
        // matters as much as on the others and for a sharper reason: this resource's
        // `GET` is the only place a tag can be obtained, so a client that does not
        // know to send one cannot write at all.
        ("PUT", APPROVAL_THRESHOLD_POLICY),
        // The creates assert their precondition through the idempotency gate rather
        // than through a version, and are listed under `idempotency_key_routes` too —
        // the window schedule among them, which is §5's own Idempotency cell for it.
        ("POST", PLANS),
        ("POST", PLAN_PRICES),
        ("POST", PRICE_WINDOWS),
    ]
}

/// The creates, each of which requires an `Idempotency-Key` (D-141/D-142, and §5's
/// Idempotency column for the window schedule).
fn idempotency_key_routes() -> Vec<(&'static str, &'static str)> {
    use bss_pricing::api::rest::plans::PLANS;
    use bss_pricing::api::rest::prices::PLAN_PRICES;
    use bss_pricing::api::rest::windows::PRICE_WINDOWS;
    vec![
        ("POST", PLANS),
        ("POST", PLAN_PRICES),
        ("POST", PRICE_WINDOWS),
    ]
}

/// The header parameters one registered operation declares.
fn declared_headers(openapi: &OpenApiRegistryImpl, method: &str, path: &str) -> Vec<String> {
    let key = format!("{method}:{path}");
    let entry = openapi
        .operation_specs
        .get(&key)
        .unwrap_or_else(|| panic!("{key} is not a registered operation"));
    entry
        .value()
        .params
        .iter()
        .filter(|param| matches!(param.location, ParamLocation::Header))
        .map(|param| param.name.to_ascii_lowercase())
        .collect()
}

#[tokio::test]
async fn every_mutating_route_declares_its_precondition_header() {
    // D-171's owed clause: the declarations existed on every mutating route and
    // **no test read the registration's params**, so deleting one failed nothing. A
    // declaration is what a generated client sends; a route whose `If-Match` is
    // undeclared is one whose clients omit the header and are then refused 400 by
    // a server that never told them to send it.
    //
    // The roster is `if_match_routes()` and the count is deliberately not repeated
    // here: it said "six" against a list of seven (five `If-Match` plus the two
    // creates, which assert their precondition through the idempotency gate).
    //
    // Delete one `.param(if_match_param(...))` and exactly this assertion fails.
    let openapi = registered_operations().await;

    for (method, path) in if_match_routes() {
        let headers = declared_headers(&openapi, method, path);
        let expected = if idempotency_key_routes().contains(&(method, path)) {
            "idempotency-key"
        } else {
            "if-match"
        };
        assert!(
            headers.iter().any(|name| name == expected),
            "{method} {path} declares no {expected}: {headers:?}"
        );
    }
}

#[tokio::test]
async fn every_create_declares_its_idempotency_key() {
    // The other half, and it is a different header on a different pair of routes:
    // a create has no version to assert, so its precondition is the gate's key.
    let openapi = registered_operations().await;

    for (method, path) in idempotency_key_routes() {
        let headers = declared_headers(&openapi, method, path);
        assert!(
            headers.iter().any(|name| name == "idempotency-key"),
            "{method} {path} declares no Idempotency-Key: {headers:?}"
        );
    }
}

#[tokio::test]
async fn a_read_route_declares_no_precondition_header() {
    // The negative side, which is what stops the two assertions above from being
    // satisfiable by declaring both headers everywhere. A GET has no precondition
    // to assert, and a declared one would tell a client to send a header the
    // server ignores.
    let openapi = registered_operations().await;

    for (method, path) in [
        ("GET", bss_pricing::api::rest::plans::PLAN),
        ("GET", bss_pricing::api::rest::prices::PLAN_PRICES),
        ("GET", bss_pricing::api::rest::approvals::APPROVALS),
        ("GET", bss_pricing::api::rest::approvals::APPROVAL),
        ("GET", bss_pricing::api::rest::windows::PLAN_COVERAGE),
        ("GET", bss_pricing::api::rest::windows::PLAN_SELLABILITY),
    ] {
        let headers = declared_headers(&openapi, method, path);
        assert!(
            !headers.iter().any(|name| name == "if-match"),
            "{method} {path} declares an If-Match it cannot honour: {headers:?}"
        );
    }
}

#[tokio::test]
async fn the_window_cancel_declares_no_precondition_header() {
    // §5's Idempotency cell for `DELETE /price-windows/{windowId}` is **empty**, and
    // `api::rest::windows` reports that as the design set's call rather than an
    // omission to improve on — the refusal of a second cancellation
    // (`WINDOW_NOT_CANCELLABLE`) is what stands in for a key.
    //
    // A **mutating** route with no declared precondition is otherwise exactly the hole
    // `every_mutating_route_declares_its_precondition_header` closes, so the absence
    // needs its own assertion or it is indistinguishable from a forgotten
    // declaration. Add a header here to be helpful and this reddens.
    let openapi = registered_operations().await;

    let headers = declared_headers(
        &openapi,
        "DELETE",
        bss_pricing::api::rest::windows::PRICE_WINDOW,
    );

    assert!(
        headers.is_empty(),
        "the cancel takes neither an If-Match nor an Idempotency-Key: {headers:?}"
    );
}

/// The **second** route that declares no precondition header, and it needs its own
/// assertion for [`the_window_cancel_declares_no_precondition_header`]'s reason.
///
/// S5's idempotency column for `POST …/supersessions` is `(planId, scope key, changeover
/// instant)` — the act's own identity — not a client key, and the handler takes no
/// `HeaderMap` at all. So a declaration either way would be a header the server ignores.
/// Without this case the absence is indistinguishable from a forgotten declaration, and a
/// later group reading the route as "a create like the other creates" could add an
/// `Idempotency-Key` param with every test still green — handing callers at-most-once
/// semantics they do not have, on a surface whose own module doc argues that wiring that
/// gate here would make the commit arm permanently unreachable.
#[tokio::test]
async fn the_supersession_declares_no_precondition_header() {
    let openapi = registered_operations().await;

    let headers = declared_headers(
        &openapi,
        "POST",
        bss_pricing::api::rest::supersessions::PLAN_SUPERSESSIONS,
    );

    assert!(
        headers.is_empty(),
        "the supersession takes neither an If-Match nor an Idempotency-Key: {headers:?}"
    );
}
