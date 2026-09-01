//! Tests for the Product read door and the Product create door.
//!
//! # What this file reaches, and what it does not
//!
//! Every case below drives a real request through the composed router with
//! `tower::ServiceExt::oneshot`, the idiom `gear.rs`'s own mount test uses,
//! rather than calling a handler as a plain async function — the property
//! under test in each case is what the *router* does with a request, which a
//! direct call would not exercise at all.
//!
//! The read-door cases run against a real in-memory-shaped `SQLite` mirror
//! and a real [`PolicyEnforcer`] built over a fake [`AuthZResolverClient`]
//! (the same technique `crate::authz::authz_tests` uses) — nothing about
//! either door needed a live database server or a live PDP deployment to
//! prove.
//!
//! **Not reached here:** a row that exists but lies outside the caller's
//! authorized tenant (as opposed to an id with no row at all). Both are
//! documented to answer the identical `404` (this module's own doc, and
//! [`crate::infra::storage::repo::find_product`]'s), and the door adds no
//! logic between that repository call and the `Ok(None)` branch that would
//! tell the two apart — but exercising it here would mean layering a second
//! tenant onto this file's harness for a property already pinned at the
//! layer that actually implements it:
//! `infra::storage::repo_tests::a_row_belonging_to_another_tenant_is_not_visible_through_a_foreign_scope`.
//! Reproducing that setup here would only prove `find_product` still behaves
//! as its own suite already requires, at the cost of a second PDP fake
//! (`allowed` pinned to a tenant the inserted row is deliberately outside
//! of) this file does not otherwise need.
//!
//! # The create-door harness: a real file, not `:memory:`
//!
//! [`harness`] backs its `SQLite` mirror with a real, uniquely named
//! temporary file rather than `sqlite::memory:`. The create door's own tests
//! need a **second, independent** connection into the same database — one to
//! count the outbox tables' rows (`raw_i64`), one to read an audit row's
//! `error_code` back (`raw_string_opt`), and one, for the
//! `AUDIT_UNAVAILABLE` seam, to drop `products_audit_log` out from under a
//! running door (`drop_table`) — and `sqlite::memory:` hands every new
//! connection its own empty database, which would make all three see
//! nothing. A real file is the one dial this file has to turn to get a
//! second, independently-opened connection that still sees what the door
//! just wrote — `toolkit_db::secure::DBRunner` is sealed exactly to keep
//! downstream code from reaching a raw executor through the production path
//! (see that trait's own doc), so these three helpers open their own
//! `sea_orm::Database::connect`, entirely outside it, the same way
//! `account-management`'s own Postgres test harness keeps one auxiliary
//! `ddl_conn` for the corruption/introspection its integrity-check suite
//! needs. [`TestHarness::drop`] best-effort deletes the file afterward.
//!
//! Every test still pins `max_conns: 1` / `min_conns: 1` on the *production*
//! pool — not because a file needs it the way `:memory:` did, but because it
//! keeps this file's harness shape recognizable next to
//! `infra::storage::repo_tests::harness`'s, and avoids `SQLite`'s
//! one-writer-at-a-time limit turning a stray second production connection
//! into contention no test here wants to reason about.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use core::fmt::Write as _;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse as _;
use chrono::{TimeZone, Utc};
use sea_orm::{ConnectionTrait, Database};
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use tower::ServiceExt as _;
use uuid::Uuid;

use super::router;
use crate::api::rest::ApiState;
use crate::api::rest::preconditions;
use crate::config::ProductsConfig;
use crate::domain::canonical;
use crate::domain::concurrency::InternalRevision;
use crate::domain::error::DomainError;
use crate::domain::governance::{
    ApprovalDisposition, ApprovalId, EntityRef, GateMode, GateVerdict, GovernanceGate,
};
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{self, NewProduct};
use crate::test_support::{
    audit_action, audit_error_code, authed_ctx, drop_table, enqueued_event_count,
    enqueued_event_envelope, flat_in_enforcer, id_matches, idempotency_rows_for, raw_i64,
    raw_string_opt, table_columns,
};

const TENANT: Uuid = Uuid::from_u128(0xd0_01);
const BRAND: Uuid = Uuid::from_u128(0xd0_02);
/// A second tenant, for the case that proves the key's own `tenant_id`
/// component is load-bearing rather than decorative.
const OTHER_TENANT: Uuid = Uuid::from_u128(0xd0_03);

/// Everything a test in this file needs: the database, and the started
/// outbox pipeline the create door enqueues `ProductCreated` against. Both
/// are built over the *same* `Db`/file — see this module's doc for why a
/// door's `enqueue` call would otherwise target tables a second, unrelated
/// connection's pipeline created.
struct TestHarness {
    /// The DSN backing `db` — kept so a test can open its own auxiliary
    /// `sea_orm::Database::connect` into the identical file (see this
    /// module's doc, "The create-door harness: a real file, not
    /// `:memory:`").
    dsn: String,
    db: DBProvider<DbError>,
    outbox: Arc<Outbox>,
    /// Held only so the pipeline's background tasks are not cancelled
    /// (`OutboxHandle`'s own doc: dropping it cancels them) before a test
    /// finishes — no test here relies on those tasks draining anything.
    #[allow(dead_code)]
    _outbox_handle: OutboxHandle,
}

impl Drop for TestHarness {
    /// Best-effort: a leaked temp file here fails no test, and a failed
    /// removal (the file already gone, a transient permission error) is not
    /// worth turning into a panic during unwind.
    fn drop(&mut self) {
        if let Some(rest) = self.dsn.strip_prefix("sqlite://") {
            let path = rest.split('?').next().unwrap_or(rest);
            std::fs::remove_file(path).ok();
        }
    }
}

/// A uniquely named path under the OS temp directory, so concurrently
/// running tests in this file never share a database.
fn unique_sqlite_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "bss-products-tests-{label}-{}.sqlite3",
        Uuid::new_v4()
    ))
}

/// Build a fresh file-backed `SQLite` mirror, migrated with both this
/// gear's own Foundation tables and the toolkit outbox's own tables under
/// [`events::OUTBOX_TABLE_PREFIX`] — the same prefix `gear.rs` builds its
/// own running pipeline with (see `api/rest.rs`'s own doc, which now records
/// where that wiring **is**; the gap it used to describe was closed) — with
/// [`events::QUEUE_NAME`] registered and the
/// **same** [`events::PendingBrokerProducer`] the running gear declares.
///
/// The handler is not an incidental choice. An earlier version of this
/// harness registered a stub answering `Success`, whose doc claimed
/// "nothing in this file drains the queue" — untrue, because `.start()`
/// runs the processor, and a `Success` marks the message delivered and
/// hands it to the vacuum. The enqueue assertion below then read zero rows
/// for a create that had enqueued one. Registering the production handler
/// both fixes the count and makes these tests exercise the configuration
/// the gear boots with **on its no-broker arm** rather than a friendlier one.
///
/// That qualifier is not decoration. Since the producer landed, `Gear::init`
/// registers this queue with `PendingBrokerProducer` only when the `ClientHub`
/// carries no `EventBrokerApi`; with one, the processor is the SDK's producer
/// and the sink is `EventSink::Broker`. So every event assertion in this suite
/// measures one of two arms, and the other is exercised only by
/// `infra::broker::broker_tests`' `MockBroker` case — never through a door.
async fn harness() -> TestHarness {
    let path = unique_sqlite_path("db");
    // `?mode=rwc` is what creates the file. Without it sqlx opens an
    // existing database only, and a fresh per-test path has none yet —
    // every test in this file failed with SQLite error 14 until this was
    // added.
    let dsn = format!("sqlite://{}?mode=rwc", path.display());
    let opts = ConnectOpts {
        max_conns: Some(1),
        min_conns: Some(1),
        ..Default::default()
    };
    let db = connect_db(&dsn, opts)
        .await
        .expect("connect the file-backed sqlite mirror");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("run this gear's own migrator");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX)
            .expect("OUTBOX_TABLE_PREFIX is a fixed, valid identifier"),
    )
    .await
    .expect("run the outbox facility's own migrator");

    let outbox_handle = Outbox::builder(db.clone())
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .expect("OUTBOX_TABLE_PREFIX is a fixed, valid identifier")
        .queue(events::QUEUE_NAME, Partitions::of(events::PARTITIONS))
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .expect("start the outbox pipeline");
    let outbox = Arc::clone(outbox_handle.outbox());

    TestHarness {
        dsn,
        db: DBProvider::<DbError>::new(db),
        outbox,
        _outbox_handle: outbox_handle,
    }
}

fn new_product(product_id: Uuid, tenant_id: Uuid) -> NewProduct {
    NewProduct {
        product_id,
        tenant_id,
        brand_id: BRAND,
        name: "Fibre 500".to_owned(),
        name_normalized: "fibre 500".to_owned(),
        product_code: Some("FIBRE-500".to_owned()),
        region_scope: "eu".to_owned(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: Utc.with_ymd_and_hms(2026, 8, 29, 9, 0, 0).unwrap(),
        cloned_from: None,
        cloned_from_version: None,
    }
}

/// Build the router under test, layered with a `flat_in_enforcer` allowed
/// for `tenant` — the shape every test below shares.
fn app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    // The `idempotency_retention_hours` inside is the value `gear.rs`
    // resolves from the operator's file; the tests configure nothing, so the
    // typed default is what an unconfigured boot would carry here.
    let state = api_state(harness);
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// `POST /bss-products/v1/products` with `body`, authenticated as `tenant`
/// and carrying **no** `Idempotency-Key` — the keyless shape every
/// pre-idempotency case in this file already used, kept so those cases keep
/// exercising the skip rather than the claim.
async fn post_create_product(
    app: Router,
    tenant: Uuid,
    body: &serde_json::Value,
) -> axum::http::Response<Body> {
    post_create_product_with_headers(app, tenant, body, &[]).await
}

/// [`post_create_product`] with `headers` set on the request — the one dial
/// the idempotency cases below turn. A slice of pairs rather than a
/// `HeaderMap` argument: every case sets one or two literal headers, and the
/// pairs read at the call site as the request a client actually sent.
async fn post_create_product_with_headers(
    app: Router,
    tenant: Uuid,
    body: &serde_json::Value,
    headers: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri("/bss-products/v1/products")
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .extension(authed_ctx(tenant));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.oneshot(
        request
            .body(Body::from(body.to_string()))
            .expect("build the create request"),
    )
    .await
    .expect("the router answers")
}

/// `POST /bss-products/v1/products` carrying `key` as its `Idempotency-Key`.
async fn post_create_product_with_key(
    app: Router,
    tenant: Uuid,
    body: &serde_json::Value,
    key: &str,
) -> axum::http::Response<Body> {
    post_create_product_with_headers(app, tenant, body, &[("Idempotency-Key", key)]).await
}

/// The digest this door would take of `body` — computed the way the door
/// computes it, by parsing the same wire `JSON` into the same `DTO` and
/// calling the same function, so a case seeding a stored digest cannot
/// silently disagree with the door about which fields the operand carries.
fn digest_of(body: &serde_json::Value) -> Vec<u8> {
    let request: super::CreateProductRequest =
        serde_json::from_value(body.clone()).expect("the case's own body parses as the create DTO");
    super::payload_digest(&request)
}

/// Seed a **live, unanswered** claim for this door's own endpoint under
/// `client_key`, recorded against `payload_hash`: the in-flight state, and
/// the only way to reach it now that a committed create answers its own
/// claim.
///
/// `payload_hash` is a parameter rather than a fixed literal because the
/// digest decides which refusal the seeded state produces: a duplicate whose
/// digest **matches** is refused `IDEMPOTENCY_KEY_IN_FLIGHT`, and one that
/// differs is refused `IDEMPOTENCY_CONFLICT` — "a payload mismatch stays
/// `IDEMPOTENCY_CONFLICT` in either state"
/// (`design/01-foundation.md` §3.2 `inst-fd-idem-claim-inflight`). An earlier
/// version seeded one fixed literal that no case's body ever hashed to, so
/// the in-flight case it fed was really the mismatch case wearing the wrong
/// name.
///
/// The claim goes through the repository's own `claim_idempotency_key` on
/// the harness's production connection, which is checked back in when this
/// function returns — the door's own transaction needs it, and this file's
/// pool is pinned to one connection.
async fn seed_live_claim(harness: &TestHarness, client_key: &str, payload_hash: &[u8]) {
    let conn = harness
        .db
        .conn()
        .expect("checkout the pinned production connection");
    let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
    let now = Utc::now();
    let held = repo::claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "/bss-products/v1/products",
        client_key,
        payload_hash,
        now,
        now + chrono::TimeDelta::hours(24),
    )
    .await
    .expect("seed the live claim this case collides with");
    assert_eq!(
        held,
        repo::IdempotencyClaim::Claimed,
        "this case's own premise: the key is held and unanswered"
    );
}

/// Seed an `answered` idempotency row for this door's own endpoint under
/// `client_key`, recorded against `payload_hash`, carrying a `201` and a
/// recognizable body.
///
/// Both steps run through the repository's own functions — the claim
/// through `claim_idempotency_key`, the transition through
/// `answer_idempotency_key` — so the row these cases read is written by the
/// code path production writes it with. An earlier version wrote the
/// transition by hand, because no answer-writer existed.
///
/// The stored body is deliberately **not** a rendered `ProductView`: a
/// replay must serve the stored bytes rather than re-render the entity, and
/// a recognizable body is what tells the two apart in
/// `an_answered_key_replays_its_stored_response_even_though_the_retry_carries_a_precondition`.
async fn seed_answered_claim(harness: &TestHarness, client_key: &str, payload_hash: &[u8]) {
    let conn = harness
        .db
        .conn()
        .expect("checkout the pinned production connection");
    let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
    let now = Utc::now();
    let outcome = repo::claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "/bss-products/v1/products",
        client_key,
        payload_hash,
        now,
        now + chrono::TimeDelta::hours(24),
    )
    .await
    .expect("seed the claim this case answers");
    assert_eq!(
        outcome,
        repo::IdempotencyClaim::Claimed,
        "this case's own premise: the seeded key was free"
    );

    let answered = repo::answer_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "/bss-products/v1/products",
        client_key,
        201,
        json!({ "replayed": "the stored answer" }),
    )
    .await
    .expect("record the answer this case replays");
    assert_eq!(
        answered,
        repo::IdempotencyAnswer::Recorded,
        "this case's own premise: the claim it just took was still held"
    );
}

/// A `GET` under the reserved prefix for an unknown id reaches **this**
/// door rather than the previous slice's blanket `404`.
///
/// `gear.rs`'s own mount test proves an unmounted path under
/// `/bss-products/v1` answers a blank axum `404` from the empty router this
/// gear mounts when it has nothing to serve. This test proves the opposite
/// for the path this slice registers: the request reaches [`super::router`]'s
/// `get_product` handler and is refused **there**, for the handler's own
/// reason (no authenticated `SecurityContext` on the request), which is a
/// `401` — not the routing layer's `404`. A router that never actually
/// registered `/bss-products/v1/products/{id}` (a typo'd path, a method
/// mismatch, a nesting bug) would fall through to that same blank `404`
/// instead, which is exactly the regression this test would catch.
#[tokio::test]
async fn a_get_for_an_unknown_id_reaches_this_door_rather_than_the_empty_routers_404() {
    let harness = harness().await;
    let app = app_for(&harness, Uuid::now_v7());

    let response = app
        .oneshot(
            Request::builder()
                .uri(format!("/bss-products/v1/products/{}", Uuid::now_v7()))
                .body(Body::empty())
                .expect("build the probe request"),
        )
        .await
        .expect("the router answers");

    assert_eq!(
        response.status(),
        StatusCode::UNAUTHORIZED,
        "the path is registered, so an unauthenticated GET reaches this door's own 401 refusal \
         rather than an unmounted path's blanket 404"
    );
}

/// A hit answers `200` with an `ETag` that round-trips through
/// [`preconditions::if_match`], and a miss for an unrelated id answers
/// `404` — the property this whole door exists for: without a working
/// `ETag` an author has no `If-Match` to send a later mutating door.
#[tokio::test]
async fn the_hit_carries_a_round_tripping_etag_and_the_miss_is_404() {
    let harness = harness().await;
    let conn = harness
        .db
        .conn()
        .expect("checkout the pinned production connection");
    let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
    let product_id = Uuid::now_v7();
    let inserted = repo::insert_product(&conn, &scope, new_product(product_id, TENANT))
        .await
        .expect("insert product");
    assert_eq!(
        inserted.internal_revision, 1,
        "a freshly created head starts at internal_revision 1 (this test's own premise)"
    );

    let app = app_for(&harness, TENANT);
    let ctx = authed_ctx(TENANT);

    // The miss: an id with no row at all, under a caller authorized for its
    // own tenant. Answered `404`, the same status `find_product`'s own doc
    // says an out-of-scope row would get — see this file's module doc for
    // why the out-of-scope case itself is not reproduced here.
    let miss = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/bss-products/v1/products/{}", Uuid::now_v7()))
                .extension(ctx.clone())
                .body(Body::empty())
                .expect("build the miss request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(
        miss.status(),
        StatusCode::NOT_FOUND,
        "an id with no row must answer 404"
    );

    // The hit: the id just inserted, under the same caller.
    let hit = app
        .oneshot(
            Request::builder()
                .uri(format!("/bss-products/v1/products/{product_id}"))
                .extension(ctx)
                .body(Body::empty())
                .expect("build the hit request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(
        hit.status(),
        StatusCode::OK,
        "the inserted id must be readable"
    );

    let etag_header = hit
        .headers()
        .get(axum::http::header::ETAG)
        .expect("a 200 from this door always carries an ETag")
        .to_str()
        .expect("the ETag header is ASCII")
        .to_owned();

    // Round-trip: the header this door just set must parse back to exactly
    // the revision the row was inserted with. A door that emitted
    // `published_version` instead of `internal_revision` (the wrong operand
    // `domain::concurrency`'s own doc warns against), or that hand-rolled
    // the tag instead of calling `preconditions::etag`, would still answer
    // `200` and still carry *an* `ETag` header — only this assertion catches
    // it.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        axum::http::HeaderValue::from_str(&etag_header).expect("etag header value"),
    );
    let parsed = preconditions::if_match(&headers).expect("the door's own ETag must parse back");
    assert_eq!(
        parsed,
        InternalRevision::new(inserted.internal_revision),
        "the ETag this door set must name the row's own internal_revision"
    );
}

/// A well-formed create persists a `draft` row with `published_version = 0`
/// and `internal_revision = 1`, and answers with the created view —
/// `dod-create-doors`' own baseline, and the one case every other create-door
/// test in this file is a variation on.
#[tokio::test]
async fn a_well_formed_create_persists_a_draft_row_and_answers_with_the_created_view() {
    let harness = harness().await;
    let app = app_for(&harness, TENANT);

    let response = post_create_product(
        app,
        TENANT,
        &json!({
            "brand_id": BRAND,
            "name": "Fibre 500",
            "product_code": "FIBRE-500",
            "region_scope": "eu",
        }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a well-formed create must be admitted"
    );
    // `ProductView` derives `Serialize` only (`#[toolkit_macros::api_dto(response)]`
    // adds no `Deserialize`, matching every other response DTO on this
    // surface) — read the body as generic JSON rather than the type itself.
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read the response body");
    let view: serde_json::Value = serde_json::from_slice(&body).expect("the response body is JSON");

    assert_eq!(
        view["tenant_id"],
        serde_json::json!(TENANT),
        "the row belongs to the caller's own tenant"
    );
    assert_eq!(view["brand_id"], serde_json::json!(BRAND));
    assert_eq!(view["name"], serde_json::json!("Fibre 500"));
    assert_eq!(view["product_code"], serde_json::json!("FIBRE-500"));
    assert_eq!(
        view["lifecycle_state"],
        serde_json::json!("draft"),
        "a freshly created head is always draft"
    );
    assert_eq!(
        view["published_version"],
        serde_json::json!(0),
        "a draft has never been published"
    );
    assert_eq!(
        view["internal_revision"],
        serde_json::json!(1),
        "the first admitted write starts the revision counter at 1"
    );
    assert_eq!(view["region_scope"], serde_json::json!("eu"));

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(
        persisted, 1,
        "the row this door reports created must actually be the one row now in storage"
    );
}

/// Exactly one outbox row is enqueued, in the create door's own mutation
/// transaction — and no content row is written, because this gear defines
/// no content table yet (Phase 4 predates the 02/03 content slices this
/// door's own doc names). What this test can and does assert is the
/// observable half of that guarantee: one `ProductCreated` row lands in the
/// outbox, and the only other row this create touches is the entity's own —
/// in particular, a *successful* create writes no audit row at all (P-D-21:
/// a committed act's own event, not an audit row, is its record).
#[tokio::test]
async fn exactly_one_outbox_row_is_enqueued_and_no_content_row_is_written() {
    let harness = harness().await;
    let app = app_for(&harness, TENANT);

    let response = post_create_product(
        app,
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    // Counted on `_body`, not `_incoming`. `_incoming` is a **staging**
    // table: the running sequencer drains it, assigning per-partition
    // sequence numbers and moving the row onward, so a count taken after the
    // response has raced the pipeline and reads zero. `_body` holds the
    // payload for the message's whole life and is reclaimed only by the
    // vacuum, which never runs here because
    // `events::PendingBrokerProducer` never acks. Counting the staging table
    // was this test's own first version and it failed for exactly that
    // reason.
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let enqueued = raw_i64(
        &harness.dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table}"),
    )
    .await;
    assert_eq!(
        enqueued, 1,
        "exactly one ProductCreated row must be enqueued for one create"
    );
    let payload_type = raw_string_opt(
        &harness.dsn,
        &format!("SELECT payload_type AS v FROM {body_table}"),
    )
    .await;
    assert_eq!(
        payload_type.as_deref(),
        Some(events::PRODUCT_CREATED_PAYLOAD_TYPE),
        "the enqueued row must carry the ProductCreated payload type, not some other event's"
    );

    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(
        audit_rows, 0,
        "a successful create writes no audit row: its event is its record (P-D-21)"
    );
}

/// A second create colliding on the normalized name within the same
/// `(tenant, brand)` is refused `DUPLICATE_NAME`, an audit row records the
/// refusal, and the entity row is not persisted.
#[tokio::test]
async fn a_duplicate_name_within_the_same_tenant_and_brand_is_refused_and_audited() {
    let harness = harness().await;

    let first = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::CREATED,
        "the first create must succeed"
    );

    // A whitespace/case variant of the same name — this is also what proves
    // the collision is judged on `name_normalized`, not the raw string.
    let second = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "  FIBRE 500  " }),
    )
    .await;
    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "a normalized-name collision within the same (tenant, brand) is a 409"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(
        persisted, 1,
        "the losing create must not leave a second row behind"
    );

    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(audit_rows, 1, "the refusal must be audited exactly once");
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(error_code.as_deref(), Some("DUPLICATE_NAME"));
}

/// A create colliding on `product_code` is refused `DUPLICATE_CODE`, with
/// the same three assertions as the name collision above.
#[tokio::test]
async fn a_duplicate_product_code_is_refused_and_audited() {
    let harness = harness().await;

    let first = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500", "product_code": "FIBRE-500" }),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::CREATED,
        "the first create must succeed"
    );

    let second = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "A Different Name", "product_code": "FIBRE-500" }),
    )
    .await;
    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "a product_code collision within the same tenant is a 409"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(
        persisted, 1,
        "the losing create must not leave a second row behind"
    );

    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(audit_rows, 1, "the refusal must be audited exactly once");
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(error_code.as_deref(), Some("DUPLICATE_CODE"));
}

/// A caller-supplied `id` is refused `VALIDATION` naming `id`, not silently
/// dropped (F-6): [`super::CreateProductRequest`] carries an explicit,
/// optional `id` field precisely so this can be a named refusal rather than
/// an unreachable one, and no row is persisted.
#[tokio::test]
async fn a_caller_supplied_id_is_refused_validation() {
    let harness = harness().await;
    let app = app_for(&harness, TENANT);
    let caller_supplied_id = Uuid::now_v7();

    let response = post_create_product(
        app,
        TENANT,
        &json!({
            "id": caller_supplied_id,
            "brand_id": BRAND,
            "name": "Fibre 500",
        }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "VALIDATION renders as an architectural 422, wire 400 (no transport override)"
    );
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read the response body");
    let view: serde_json::Value = serde_json::from_slice(&body).expect("the response body is JSON");
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("VALIDATION"),
        "a caller-supplied id must be refused VALIDATION, not silently accepted"
    );
    assert_eq!(
        view["context"]["violations"][0]["subject"],
        json!("id"),
        "the refusal must name the offending field"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(
        persisted, 0,
        "a refused create must not have left a row behind"
    );
}

/// F-1: a shape `VALIDATION` refusal — not only the code-reservation
/// conflicts — writes its own audit row, carrying `error_code = VALIDATION`
/// and no `subject_id` (the refusal is pre-mint).
#[tokio::test]
async fn a_validation_refusal_is_audited() {
    let harness = harness().await;
    let app = app_for(&harness, TENANT);

    let response =
        post_create_product(app, TENANT, &json!({ "brand_id": BRAND, "name": "   " })).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(
        audit_rows, 1,
        "a VALIDATION refusal must be audited exactly once, the same as a duplicate-conflict \
         refusal"
    );
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(error_code.as_deref(), Some("VALIDATION"));
    let subject_id = raw_string_opt(
        &harness.dsn,
        "SELECT subject_id AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        subject_id, None,
        "a pre-mint refusal must carry no subject_id: nothing was minted to name"
    );
}

/// F-2: `product_code` is trimmed and a blank-after-trim value collapses to
/// `None` before the insert. Two unrelated creates that both send
/// `product_code: ""` must both succeed with a `NULL` stored code, never
/// collide on `uq_products_product_code` (partial `WHERE product_code IS
/// NOT NULL`).
#[tokio::test]
async fn two_creates_with_blank_product_code_both_succeed_with_a_null_stored_code() {
    let harness = harness().await;

    let first = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500", "product_code": "" }),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::CREATED,
        "an empty product_code must not be treated as a reservable value"
    );
    let first_view: serde_json::Value = {
        let body = axum::body::to_bytes(first.into_body(), 64 * 1024)
            .await
            .expect("read the response body");
        serde_json::from_slice(&body).expect("the response body is JSON")
    };
    assert_eq!(
        first_view["product_code"],
        serde_json::Value::Null,
        "a blank product_code must collapse to null on the wire, not persist as \"\""
    );

    let second = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "A Different Name", "product_code": "  " }),
    )
    .await;
    assert_eq!(
        second.status(),
        StatusCode::CREATED,
        "a second, unrelated create with a whitespace-only product_code must not collide with \
         the first"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(persisted, 2, "both creates must have been admitted");
    let null_codes = raw_i64(
        &harness.dsn,
        "SELECT COUNT(*) AS v FROM products_product WHERE product_code IS NULL",
    )
    .await;
    assert_eq!(
        null_codes, 2,
        "both rows must store product_code as NULL, not an empty or whitespace string"
    );
}

/// When the refusal's audit row cannot be written, the door answers
/// `AUDIT_UNAVAILABLE`, not the domain refusal it would otherwise have
/// reported (`repo::write_refusal_audit`'s own contract; the "100%
/// write-path audit" NFR this discipline exists for).
///
/// The seam: `products_audit_log` is dropped, via an auxiliary connection,
/// between the setup create (which must still succeed — it writes no audit
/// row) and the colliding one, so only the refusal's own insert — never the
/// mutation's — fails.
#[tokio::test]
async fn an_unwritable_refusal_audit_answers_audit_unavailable_not_the_domain_refusal() {
    let harness = harness().await;

    let first = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::CREATED,
        "the setup create must succeed"
    );

    drop_table(&harness.dsn, "products_audit_log").await;

    let second = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
    )
    .await;

    assert_eq!(
        second.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unwritable refusal audit must answer 503 AUDIT_UNAVAILABLE, not the domain \
         refusal (DUPLICATE_NAME) the mutation actually reached"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(
        persisted, 1,
        "the losing create's row still must not have been persisted"
    );
}

/// A create carrying an `Idempotency-Key` persists the entity **and** an
/// `answered` idempotency row keyed by the concrete endpoint, in one
/// transaction.
///
/// This is `dod-idempotency-store`'s baseline at the door: the row exists,
/// it names the **concrete resource path** and not a route template
/// (P-D-42), and it sits beside the entity row the same transaction wrote —
/// a claim written on a runner of its own could not be told from this by a
/// count alone, which is why
/// `a_rolled_back_mutation_frees_the_key_for_a_later_create` exists beside
/// this case.
///
/// The state asserted is `answered`, not `claimed`: the create committed,
/// and a committed create answers its own claim in the transaction that
/// took it (§3.2 `inst-fd-idem-claim-write`). `claimed` is the state of a
/// key whose act is still in flight, and it survives a commit nowhere —
/// which is exactly why `a_second_create_on_a_live_key_is_refused_in_flight`
/// has to seed one rather than produce it with a first create.
#[tokio::test]
async fn a_create_with_an_idempotency_key_persists_the_entity_and_an_answered_row() {
    let harness = harness().await;

    let response = post_create_product_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
        "author-retry-1",
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a keyed create is admitted exactly like a keyless one"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(persisted, 1, "the entity row is written");

    let claims = idempotency_rows_for(&harness.dsn, "author-retry-1").await;
    assert_eq!(claims, 1, "the key is claimed exactly once");

    let state = raw_string_opt(
        &harness.dsn,
        "SELECT state AS v FROM products_idempotency WHERE client_key = 'author-retry-1'",
    )
    .await;
    assert_eq!(
        state.as_deref(),
        Some("answered"),
        "the committed create wrote its own answer into the claim it took"
    );

    let endpoint = raw_string_opt(
        &harness.dsn,
        "SELECT endpoint AS v FROM products_idempotency WHERE client_key = 'author-retry-1'",
    )
    .await;
    assert_eq!(
        endpoint.as_deref(),
        Some("/bss-products/v1/products"),
        "endpoint is the concrete resource path, never a route template (P-D-42)"
    );
}

/// A create **without** the header succeeds and writes **no** idempotency
/// row: the phase is skipped, not failed (P-D-34).
///
/// The rule is one word in `dod-idempotency-store` — "skipping" — and it is
/// exactly the kind a later edit inverts by making the header mandatory,
/// which would break every existing caller of this door at once. The
/// assertion is on the row count rather than on the status alone: a door
/// that claimed a key under some placeholder for the missing header would
/// still answer `201` here.
#[tokio::test]
async fn a_create_without_an_idempotency_key_succeeds_and_claims_nothing() {
    let harness = harness().await;

    let response = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a keyless create proceeds normally"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(persisted, 1, "the entity row is written");

    let claims = raw_i64(
        &harness.dsn,
        "SELECT COUNT(*) AS v FROM products_idempotency",
    )
    .await;
    assert_eq!(
        claims, 0,
        "a keyless request claims nothing at all: the phase is skipped, not failed"
    );
}

/// A second create on a key a live claim already holds is refused
/// `IDEMPOTENCY_KEY_IN_FLIGHT`, writes no second entity row, and is audited.
///
/// The refusal executes nothing, which is the property the code exists for,
/// and it audits through the same `crate::api::rest::audit_refusal_and_report`
/// every other refusal on this door uses — an idempotency refusal is a
/// refusal, and a fourth path would be one more place the
/// answer-only-after-the-row-commits discipline could be forgotten.
///
/// **The live claim is seeded, not produced by a first create**, and that is
/// a consequence of the answer write rather than a convenience: a create
/// that commits answers its own claim in the same transaction, so no
/// committed act leaves a `claimed` row behind. A key is `claimed` exactly
/// while its act is in flight, which at this door means a claim taken on
/// another connection whose transaction has not finished — which is what
/// `repo::claim_idempotency_key` on the harness's own connection reproduces
/// here. Before the answer write existed this case could seed the state with
/// an ordinary successful create, and that it no longer can is the whole
/// point of the change.
///
/// The create below sends a **different name** from nothing at all — there
/// is no first Product — so the `409` asserted cannot be the
/// `DUPLICATE_NAME` this door also answers with a `409`: the audited
/// `error_code` is what tells the two apart.
///
/// **The seeded claim is recorded against this very body's digest**, because
/// the in-flight refusal belongs to "a duplicate whose payload hash matches
/// the claimed key's" (§3.2 `inst-fd-idem-claim-inflight`). An earlier
/// version seeded a fixed literal the retry could never hash to, so what it
/// asserted as in-flight was in fact a payload mismatch; the case below is
/// that mismatch, given its own name and its own expected code.
#[tokio::test]
async fn a_second_create_on_a_live_key_is_refused_in_flight_and_audited() {
    let harness = harness().await;

    let body = json!({ "brand_id": BRAND, "name": "Fibre 900" });
    seed_live_claim(&harness, "author-retry-2", &digest_of(&body)).await;

    let second =
        post_create_product_with_key(app_for(&harness, TENANT), TENANT, &body, "author-retry-2")
            .await;
    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "a duplicate under a live claim is a 409"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(
        persisted, 0,
        "the refused duplicate must not have created a Product"
    );

    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(audit_rows, 1, "the refusal is audited exactly once");
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("IDEMPOTENCY_KEY_IN_FLIGHT"),
        "the audited code names the idempotency refusal, not the 409 this door also raises for a \
         duplicate name"
    );
}

/// A second create on a key a live claim holds, **carrying a different
/// payload**, is refused `IDEMPOTENCY_CONFLICT` rather than in flight.
///
/// The pair to the case above, and the half the door could not previously
/// answer at all: `repo::IdempotencyClaim::InFlight` carried no digest, so
/// no comparison against a live claim was structurally possible and every
/// duplicate was told in flight whatever it sent. §3.2
/// `inst-fd-idem-claim-inflight` is explicit that a payload mismatch "stays
/// `IDEMPOTENCY_CONFLICT` **in either state**" — against a stored answer and
/// against a live claim alike.
///
/// The distinction is not cosmetic. `IDEMPOTENCY_KEY_IN_FLIGHT` tells a
/// client its own request is racing itself and that retrying is the right
/// move; `IDEMPOTENCY_CONFLICT` tells it the key is already spoken for by a
/// *different* act and that retrying will never work. Answering the first
/// for the second invites a retry loop that is refused until the key expires.
///
/// The audited `error_code` is the assertion, not the status: both refusals
/// answer `409`, which is exactly why the code is what tells them apart.
#[tokio::test]
async fn a_second_create_on_a_live_key_under_a_different_payload_is_refused_conflict() {
    let harness = harness().await;

    let held = json!({ "brand_id": BRAND, "name": "Fibre 900" });
    seed_live_claim(&harness, "author-retry-2b", &digest_of(&held)).await;

    let second = post_create_product_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "A Different Product Entirely" }),
        "author-retry-2b",
    )
    .await;
    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "a different payload under a live key is a 409, as the in-flight refusal also is"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(persisted, 0, "the refused act wrote nothing");

    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("IDEMPOTENCY_CONFLICT"),
        "a mismatching payload is a conflict in either state, never an in-flight refusal"
    );

    let claims = raw_i64(
        &harness.dsn,
        "SELECT COUNT(*) AS v FROM products_idempotency",
    )
    .await;
    assert_eq!(
        claims, 1,
        "the refusal owns nothing: the seeded claim is still the only row on the key"
    );
}

/// **A rolled-back mutation frees the key.** This is the load-bearing case
/// of the whole slice.
///
/// P-D-42 makes the claim `INSERT` join the guarded mutation's transaction
/// precisely so that a mutation which rolls back takes its claim with it,
/// with no release step anywhere. Wire the claim onto a runner of its own —
/// a second `state.db.transaction(..)`, or `state.db.conn()` before the
/// mutation — and everything else in this file still passes: the claim is
/// still written, the entity is still created, the duplicate is still
/// refused. Only this case fails, and it fails in the direction that matters
/// in production: a key locked for its whole retention window against an act
/// that never happened, so the client's honest retry is refused forever.
///
/// Both halves are asserted, because either alone is satisfiable by a wrong
/// wiring: that the refused mutation left **no** claim behind, and that a
/// later create on that same key **succeeds**.
#[tokio::test]
async fn a_rolled_back_mutation_frees_the_key_for_a_later_create() {
    let harness = harness().await;

    // The setup act, keyless, so it holds no key of its own: it exists only
    // to make the next create collide on `uq_products_product_name` and roll
    // back after its claim was taken.
    let setup = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::CREATED);

    // The mutation that fails *after* the claim: same normalized name, so
    // the entity insert inside the transaction that already claimed
    // `author-retry-3` raises the duplicate-name conflict and the whole
    // transaction rolls back.
    let refused = post_create_product_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
        "author-retry-3",
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::CONFLICT,
        "the colliding create is refused DUPLICATE_NAME"
    );

    // First half: the claim rolled back with the mutation. A claim taken on
    // a runner of its own would still be sitting here.
    let stranded = idempotency_rows_for(&harness.dsn, "author-retry-3").await;
    assert_eq!(
        stranded, 0,
        "a refused mutation stores nothing, claim included (P-D-38, P-D-42): the key is free"
    );

    // Second half: the freed key is usable. This is what the client
    // experiences, and a stranded claim would refuse it
    // IDEMPOTENCY_KEY_IN_FLIGHT instead.
    let retry = post_create_product_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 900" }),
        "author-retry-3",
    )
    .await;
    assert_eq!(
        retry.status(),
        StatusCode::CREATED,
        "the same key claims again after the earlier mutation rolled back"
    );
    assert_eq!(
        idempotency_rows_for(&harness.dsn, "author-retry-3").await,
        1,
        "the retry's own claim is the only row the key ever committed"
    );
    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(
        persisted, 2,
        "the setup Product and the retry's, and nothing from the refusal"
    );
}

/// The same client key under two different tenants both claim.
///
/// `tenant_id` is the first component of the composite key, and a store
/// keyed on `(endpoint, client_key)` alone would let one tenant's key
/// silently refuse another's act — a cross-tenant denial of service that no
/// caller could diagnose. Both creates must be admitted and both claims must
/// exist.
#[tokio::test]
async fn the_same_key_under_two_tenants_both_claim() {
    let harness = harness().await;

    let first = post_create_product_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
        "shared-key",
    )
    .await;
    assert_eq!(first.status(), StatusCode::CREATED);

    let second = post_create_product_with_key(
        app_for(&harness, OTHER_TENANT),
        OTHER_TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
        "shared-key",
    )
    .await;
    assert_eq!(
        second.status(),
        StatusCode::CREATED,
        "another tenant's identical key is a different key entirely"
    );

    assert_eq!(
        idempotency_rows_for(&harness.dsn, "shared-key").await,
        2,
        "two tenants, two claims, one client key"
    );
    let tenants = raw_i64(
        &harness.dsn,
        "SELECT COUNT(DISTINCT tenant_id) AS v FROM products_idempotency \
         WHERE client_key = 'shared-key'",
    )
    .await;
    assert_eq!(
        tenants, 2,
        "the two claims differ in their tenant component"
    );
}

/// The same client key against a **different endpoint** claims too.
///
/// `endpoint` is the key's middle component, and P-D-42 puts the concrete
/// resource path there so that two acts on different resources under one
/// client key cannot share a key and replay each other's outcome. The other
/// endpoint here is one of the three reserved non-`HTTP` lane names
/// (`internal:cascade-leg`), seeded through the repository's own claim path
/// — which also shows this door coexisting with the lanes rather than only
/// with its sibling doors.
#[tokio::test]
async fn the_same_key_against_a_different_endpoint_also_claims() {
    let harness = harness().await;
    let conn = harness
        .db
        .conn()
        .expect("checkout the pinned production connection");
    let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
    let now = Utc::now();
    let outcome = repo::claim_idempotency_key(
        &conn,
        &scope,
        TENANT,
        "internal:cascade-leg",
        "shared-key",
        b"a-lane's-own-digest",
        now,
        now + chrono::TimeDelta::hours(24),
    )
    .await
    .expect("the lane's claim is taken");
    assert_eq!(
        outcome,
        repo::IdempotencyClaim::Claimed,
        "this case's own premise: the lane holds the key first"
    );

    let response = post_create_product_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
        "shared-key",
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the same key on a different endpoint is a different key entirely"
    );

    assert_eq!(
        idempotency_rows_for(&harness.dsn, "shared-key").await,
        2,
        "two endpoints, two claims, one client key"
    );
}

/// An `answered` key whose stored digest matches replays the stored response
/// and executes nothing — **even though this retry carries a precondition**
/// the original did not.
///
/// Two properties in one case, because they share a setup and neither is
/// meaningful without the other:
///
/// - The replay itself (§3.2 `inst-fd-idem-replay-outcome`): the stored
///   status and body come back and no Product is created.
/// - The precondition does not participate in the digest (P-D-34). The
///   stored digest was computed from the body alone; this retry sends the
///   identical body **plus** an `If-Match`, and is still recognised as the
///   same request. A door that folded the header into the digest would
///   answer `IDEMPOTENCY_CONFLICT` here — which is exactly what a client
///   refused `STALE_REVISION` and retrying with a fresher tag would meet.
///
/// The `answered` row is seeded rather than produced by a first create so
/// that what is measured here is the replay-plus-precondition property and
/// nothing else: a first create through the door would bring its own entity
/// row, its own outbox row and its own audit trail, and the two counts this
/// case asserts to be zero would then be asserting the *difference* a replay
/// makes rather than that it writes nothing at all. The seeding runs through
/// `repo::answer_idempotency_key`, the production answer-writer, so the row
/// read here is written by the code path production writes it with
/// (`seed_answered_claim`'s own doc).
#[tokio::test]
async fn an_answered_key_replays_its_stored_response_even_though_the_retry_carries_a_precondition()
{
    let harness = harness().await;
    let body = json!({ "brand_id": BRAND, "name": "Fibre 500" });
    seed_answered_claim(&harness, "author-retry-4", &digest_of(&body)).await;

    let response = post_create_product_with_headers(
        app_for(&harness, TENANT),
        TENANT,
        &body,
        &[("Idempotency-Key", "author-retry-4"), ("If-Match", "\"7\"")],
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the replay reproduces the stored status, not a freshly computed one"
    );
    let replayed = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read the replayed body");
    let replayed: serde_json::Value =
        serde_json::from_slice(&replayed).expect("the replayed body is JSON");
    assert_eq!(
        replayed,
        json!({ "replayed": "the stored answer" }),
        "the body is the stored one, byte for byte, not a re-rendered view"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(
        persisted, 0,
        "a replay executes nothing: no entity row, no event, no second act"
    );
    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(
        audit_rows, 0,
        "a replay is not a refusal and audits nothing of its own"
    );
}

/// An `answered` key arriving with a **different** payload is refused
/// `IDEMPOTENCY_CONFLICT`, audited, and executes nothing.
///
/// The pair to the replay above, and the reason the digest is stored at all:
/// without the comparison the store would answer a different act with
/// another act's outcome — the silent no-op §3.2 `inst-fd-idem-conflict`
/// refuses by name.
#[tokio::test]
async fn an_answered_key_under_a_different_payload_is_refused_conflict_and_audited() {
    let harness = harness().await;
    let original = json!({ "brand_id": BRAND, "name": "Fibre 500" });
    seed_answered_claim(&harness, "author-retry-5", &digest_of(&original)).await;

    let response = post_create_product_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "A Different Product Entirely" }),
        "author-retry-5",
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "a different payload under a live key is a 409"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(persisted, 0, "the refused act wrote nothing");
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("IDEMPOTENCY_CONFLICT"),
        "the refusal is audited under its own code"
    );
}

/// A present but blank `Idempotency-Key` is refused `VALIDATION` and
/// audited, rather than silently treated as absent.
///
/// The skip belongs to a request that asked for no key at all. A caller that
/// sent the header and got a `201` would reasonably read its act as
/// protected at-most-once when nothing keyed it, which is the one way this
/// door could mislead a client about a guarantee it did not give.
#[tokio::test]
async fn a_blank_idempotency_key_is_refused_validation_rather_than_skipped() {
    let harness = harness().await;

    let response = post_create_product_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
        "   ",
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an unusable key is a shape refusal, not a silent skip"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(persisted, 0, "the refused create wrote no Product");
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("VALIDATION"),
        "the refusal rides the same VALIDATION code every other shape refusal here does"
    );
}

/// Read a response body as generic JSON, for the cases that compare one
/// response against another rather than against a literal.
async fn json_body(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read the response body");
    serde_json::from_slice(&bytes).expect("the response body is JSON")
}

/// **A retry after a committed create replays the original response, and
/// executes nothing.** This is the case the whole idempotency store exists
/// for.
///
/// Every other case in this file exercises a half of the mechanism: the
/// claim, the in-flight refusal, the conflict, the replay of a seeded
/// answer. This one drives the client's own sequence end to end — a create
/// that succeeds, then the identical request under the same key, which is
/// what a client sends after a timeout it never learned the outcome of. Its
/// answer must be the original `201` and the original body, not a second
/// Product and not the `IDEMPOTENCY_KEY_IN_FLIGHT` a store with no
/// answer-writer refuses it with.
///
/// Both "executes nothing" halves are asserted on storage rather than on
/// the status alone: **no second entity row** and **no second outbox row**.
/// A door that re-ran the mutation and merely happened to answer `201`
/// would pass a status-only assertion while duplicating the act and the
/// event.
#[tokio::test]
async fn a_retry_after_a_committed_create_replays_the_original_response() {
    let harness = harness().await;
    let body = json!({ "brand_id": BRAND, "name": "Fibre 500" });

    let first =
        post_create_product_with_key(app_for(&harness, TENANT), TENANT, &body, "author-retry-6")
            .await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let original = json_body(first).await;

    let state = raw_string_opt(
        &harness.dsn,
        "SELECT state AS v FROM products_idempotency WHERE client_key = 'author-retry-6'",
    )
    .await;
    assert_eq!(
        state.as_deref(),
        Some("answered"),
        "the committed create answered its own claim, in the transaction that took it"
    );

    let retry =
        post_create_product_with_key(app_for(&harness, TENANT), TENANT, &body, "author-retry-6")
            .await;
    assert_eq!(
        retry.status(),
        StatusCode::CREATED,
        "the retry replays the original status; a store that never answered its claim would \
         refuse this 409 IDEMPOTENCY_KEY_IN_FLIGHT"
    );
    assert_eq!(
        json_body(retry).await,
        original,
        "the replay reproduces the original body, the created view included, not a second \
         Product's"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await;
    assert_eq!(
        persisted, 1,
        "the retry executed nothing: no second entity row"
    );
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let enqueued = raw_i64(
        &harness.dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table}"),
    )
    .await;
    assert_eq!(
        enqueued, 1,
        "the retry executed nothing: no second ProductCreated row"
    );
    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(
        audit_rows, 0,
        "a replay is neither an act nor a refusal, so it audits nothing"
    );
    assert_eq!(
        idempotency_rows_for(&harness.dsn, "author-retry-6").await,
        1,
        "one key, one row: the retry answered from it rather than claiming again"
    );
}

/// The stored answer is the response the door actually gave: status `201`
/// and the created view, recorded under the key the caller sent.
///
/// The replay case above proves the two agree by comparing responses; this
/// one reads the columns directly, so a regression that stored, say, a `200`
/// or an empty object is named at the column rather than diagnosed from a
/// mismatched replay.
#[tokio::test]
async fn the_stored_answer_is_the_status_and_body_the_door_returned() {
    let harness = harness().await;

    let response = post_create_product_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
        "author-retry-7",
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);
    let answered = json_body(response).await;

    let status = raw_i64(
        &harness.dsn,
        "SELECT response_status AS v FROM products_idempotency \
         WHERE client_key = 'author-retry-7'",
    )
    .await;
    assert_eq!(
        status, 201,
        "the stored status is the status the door answered"
    );

    let stored = raw_string_opt(
        &harness.dsn,
        "SELECT response_body AS v FROM products_idempotency \
         WHERE client_key = 'author-retry-7'",
    )
    .await
    .expect("an answered row carries a body; the CHECK admits no other shape");
    let stored: serde_json::Value = serde_json::from_str(&stored).expect("the stored body is JSON");
    assert_eq!(
        stored, answered,
        "the stored body is the body the door answered, not a re-rendered view of the row"
    );
}

/// The state both doors and both direct-call cases build their router or
/// their handler call from. Extracted from [`app_for`] so the governance-gate
/// case, which calls `publish_product_under_gate` directly rather than
/// through the router, builds exactly the same state a routed request would.
fn api_state(harness: &TestHarness) -> Arc<ApiState> {
    Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
    })
}

/// Insert one `draft` Product head straight through the repository — the
/// starting state every publish and discard case below needs, and one no
/// door of this slice's own can be blamed for getting wrong.
async fn seed_draft(harness: &TestHarness, product_id: Uuid) -> repo::ProductRecord {
    let conn = harness
        .db
        .conn()
        .expect("checkout the pinned production connection");
    let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
    repo::insert_product(&conn, &scope, new_product(product_id, TENANT))
        .await
        .expect("seed the draft this case acts on")
}

/// Read one Product head back through the repository.
///
/// Through `find_product` rather than through this file's `raw_i64`: the
/// head's key is a `Uuid`, and a raw `SQLite` predicate over one would have
/// to guess how the driver stored it. The counters these cases assert are
/// read off the typed record instead.
async fn head_of(harness: &TestHarness, product_id: Uuid) -> repo::ProductRecord {
    let conn = harness
        .db
        .conn()
        .expect("checkout the pinned production connection");
    let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
    repo::find_product(&conn, &scope, TENANT, product_id)
        .await
        .expect("read the head back")
        .expect("the head this case acted on exists")
}

/// `POST /bss-products/v1/products/{id}/{act}` with the supplied headers.
///
/// `act` is `publish` or `discard`, and it is a parameter for the reason the
/// door pair is one slice: every case below drives the same request shape
/// against the two paths, and a second copy of this helper would only be a
/// copy to keep in sync.
async fn post_head_act(
    app: Router,
    tenant: Uuid,
    product_id: Uuid,
    act: &str,
    headers: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut request = Request::builder()
        .method("POST")
        .uri(format!("/bss-products/v1/products/{product_id}/{act}"))
        .extension(authed_ctx(tenant));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.oneshot(
        request
            .body(Body::empty())
            .expect("build the head-act request"),
    )
    .await
    .expect("the router answers")
}

/// The `If-Match` value a caller holding `revision` would send — built the
/// way the read door builds the `ETag` it hands out, so a case cannot
/// silently disagree with the door about the tag's shape.
fn if_match(revision: i64) -> String {
    preconditions::etag(InternalRevision::new(revision))
}

/// How many frozen version rows exist.
async fn version_rows(dsn: &str) -> i64 {
    raw_i64(dsn, "SELECT COUNT(*) AS v FROM products_entity_version").await
}

/// A gate host that refuses everything, with a reason a test can recognise.
///
/// **Without this double the `APPROVAL_REQUIRED` path is unreachable.** The
/// gear's only registered host is
/// `crate::domain::governance::NoMaterialityPolicyGate`, which authorizes
/// under `Gate` mode — not permissively, but because no materiality policy
/// is registered yet — so no request through the router can produce a
/// refusal. Slice 05's host is the first one that can, and this double
/// stands in for it so the door's refusal branch is exercised before that
/// slice lands rather than after it breaks.
struct RefusingGate;

impl GovernanceGate for RefusingGate {
    fn evaluate(
        &self,
        _subject: EntityRef,
        _expected_revision: InternalRevision,
        _mode: GateMode,
    ) -> Result<GateVerdict, DomainError> {
        Ok(GateVerdict::Refused {
            reason: "this double refuses every act".to_owned(),
        })
    }
}

/// **A first publish freezes exactly one version row and moves *both*
/// counters by exactly one.**
///
/// Both counters are asserted, and that is the point of the case rather than
/// thoroughness for its own sake: the head-row guard bumps
/// `internal_revision` on every admitted `UPDATE` without exception, so a
/// door that issued two statements — say, one to freeze-and-bump the version
/// and another to write the state — would move `internal_revision` by two
/// while `published_version` still read `1`. A case asserting only
/// `published_version` passes against exactly that defect, and the `ETag` a
/// client then holds would skip a value the door never returned.
#[tokio::test]
async fn a_first_publish_freezes_one_version_row_and_moves_both_counters_by_exactly_one() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    let seeded = seed_draft(&harness, product_id).await;
    assert_eq!(
        (seeded.internal_revision, seeded.published_version),
        (1, 0),
        "this case's own premise: a freshly created head is at revision 1, version 0"
    );

    let response = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a well-formed publish of a draft must be admitted"
    );
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::ETAG)
            .and_then(|tag| tag.to_str().ok()),
        Some(if_match(2).as_str()),
        "the response carries the ETag of the revision the act committed, so the caller's next \
         verb has a precondition to send"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(
        head.published_version, 1,
        "the publish moved published_version to 1"
    );
    assert_eq!(
        head.internal_revision, 2,
        "internal_revision moved by exactly one: one act, one head-row UPDATE, one bump"
    );
    assert_eq!(
        head.lifecycle_state.as_str(),
        "published",
        "the draft -> published edge is taken by the same UPDATE"
    );

    assert_eq!(
        version_rows(&harness.dsn).await,
        1,
        "exactly one frozen version row for one publish"
    );
    let frozen_version = raw_i64(
        &harness.dsn,
        "SELECT published_version AS v FROM products_entity_version",
    )
    .await;
    assert_eq!(
        frozen_version, 1,
        "the frozen row is keyed at the version the act produced, not at the one the head \
         carried before it"
    );

    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let payload_type = raw_string_opt(
        &harness.dsn,
        &format!("SELECT payload_type AS v FROM {body_table}"),
    )
    .await;
    assert_eq!(
        payload_type.as_deref(),
        Some("ProductPublished"),
        "the publish enqueues ProductPublished in its own transaction"
    );
    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(
        audit_rows, 0,
        "a successful publish writes no audit row: its event is its record (P-D-21)"
    );
}

/// **The frozen row's `content_digest` is the digest of the rendering the
/// row itself stores.**
///
/// Read back from storage and **recomputed**, rather than compared against
/// the value the door computed in memory: the property slice 10's restore
/// drill depends on is that the digest can be re-verified from the row
/// alone, and a case that called the same helper on the same in-memory value
/// the door hashed would assert the door against itself and would still pass
/// if the door stored a *different* rendering than the one it hashed.
#[tokio::test]
async fn the_frozen_rows_digest_is_the_digest_of_the_rendering_the_row_stores() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let stored_content = raw_string_opt(
        &harness.dsn,
        "SELECT content AS v FROM products_entity_version",
    )
    .await
    .expect("a frozen row always carries its rendering");
    let stored_digest = raw_string_opt(
        &harness.dsn,
        "SELECT hex(content_digest) AS v FROM products_entity_version",
    )
    .await
    .expect("a frozen row always carries its digest");

    let mut recomputed = String::new();
    for byte in canonical::content_digest(&stored_content) {
        write!(recomputed, "{byte:02X}").expect("writing to a String cannot fail");
    }
    assert_eq!(
        stored_digest, recomputed,
        "the stored digest must be SHA-256 over the stored rendering, byte for byte"
    );

    // The rendering itself is §4.3's: keys sorted, an absent value written
    // `null` rather than omitted, and no whitespace at all. The seed carries
    // a `product_code`, so the null this asserts is not that one — it is
    // proof the roster reaches a field the value did carry, in sorted
    // position.
    //
    // `published_version` is **absent**, not `1`. It is the frozen row's own
    // key column, so the content would restate the key inside the payload
    // the key keys; `super::PRODUCT_CONTENT_ROSTER`'s doc carries the
    // argument. The row still says which version it is — the key column
    // does, and the query above selects on it.
    let seeded = new_product(product_id, TENANT);
    assert_eq!(
        stored_content,
        format!(
            "{{\"brand_id\":\"{}\",\"brand_scope\":\"\",\
             \"cloned_from\":null,\"cloned_from_version\":null,\
             \"created_at\":\"2026-08-29T09:00:00.000000Z\",\"created_by\":\"{}\",\
             \"name\":\"{}\",\"name_normalized\":\"{}\",\"product_code\":\"{}\",\
             \"product_id\":\"{product_id}\",\
             \"region_scope\":\"{}\",\"tenant_id\":\"{}\"}}",
            seeded.brand_id,
            seeded.created_by,
            seeded.name,
            seeded.name_normalized,
            seeded
                .product_code
                .clone()
                .expect("the seed carries a code"),
            seeded.region_scope,
            TENANT,
        ),
        "the frozen content is the roster's fields, sorted, and nothing else: no \
         lifecycle_state, no internal_revision, no updated_at and no published_version"
    );
    for excluded in EXCLUDED_FROM_FROZEN_CONTENT {
        assert!(
            !stored_content.contains(excluded),
            "{excluded} is excluded from a frozen row's content \
             (super::PRODUCT_CONTENT_ROSTER's doc argues each of the four); together the four \
             are what makes `the same content produces the same digest` true, which is what \
             lets a reader answer `did the content change between N and N+1` by comparing two \
             rows' digests. Stored rendering was {stored_content}"
        );
    }

    let digest_version = raw_i64(
        &harness.dsn,
        "SELECT digest_version AS v FROM products_entity_version",
    )
    .await;
    assert_eq!(
        i32::try_from(digest_version).expect("the column holds an i32"),
        canonical::DIGEST_VERSION,
        "the row records the scheme its digest was computed under, so a later bump stays \
         checkable from the row alone"
    );
}

/// **A re-publish moves the version and leaves the state alone.**
///
/// `inst-fd-publish-freeze`: *"a re-publish changes the version, never the
/// state"*. The second publish takes no edge — `published -> published` is
/// not in the machine's edge list and is not supposed to be — so a door that
/// wrote `lifecycle_state = 'published'` unconditionally would still pass a
/// state assertion here while failing the same rule for a `deprecated` head,
/// which is why [`repo::publish_product_head`] decides the edge with a
/// `CASE` over the row image rather than from a caller's argument.
#[tokio::test]
async fn a_republish_moves_the_version_again_and_leaves_the_state_published() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let first = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);

    let second = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(2))],
    )
    .await;
    assert_eq!(
        second.status(),
        StatusCode::OK,
        "a published head is publishable again as version N+1"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(head.published_version, 2, "the second publish is version 2");
    assert_eq!(
        head.internal_revision, 3,
        "the second act moved the revision by exactly one again"
    );
    assert_eq!(
        head.lifecycle_state.as_str(),
        "published",
        "a re-publish takes no edge and leaves the state where it was"
    );
    assert_eq!(
        version_rows(&harness.dsn).await,
        2,
        "two publishes, two frozen rows: the history is append-only"
    );
}

/// **A stale `If-Match` is refused `STALE_REVISION`, it is audited, and
/// nothing is written.**
///
/// The "nothing is written" half is asserted on storage, not inferred from
/// the status: the freeze runs *before* the head-row `UPDATE` can report
/// that it matched no row, so a door that returned its refusal as an
/// ordinary outcome rather than as an error would **commit** the frozen row
/// it had already written — leaving a version nobody published and, worse,
/// satisfying the head-row guard's prerequisite for a later bump.
#[tokio::test]
async fn a_publish_with_a_stale_if_match_is_refused_and_writes_nothing() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(7))],
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "a stale precondition is refused STALE_REVISION, a 409"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(
        (head.internal_revision, head.published_version),
        (1, 0),
        "neither counter moved"
    );
    assert_eq!(
        head.lifecycle_state.as_str(),
        "draft",
        "the head is still a draft"
    );
    assert_eq!(
        version_rows(&harness.dsn).await,
        0,
        "no version row was frozen for a publish that never happened"
    );

    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("STALE_REVISION"),
        "the refusal wrote its own audit row, naming the code it refused with"
    );
    let action = raw_string_opt(&harness.dsn, "SELECT action AS v FROM products_audit_log").await;
    assert_eq!(
        action.as_deref(),
        Some("publish"),
        "the audit row records the act that was refused, not the create door's token"
    );
}

/// **A publish with no `If-Match` at all is refused `VALIDATION`**, and the
/// row is audited too — the second of the two distinct refusals this file
/// pins an audit row for.
///
/// `VALIDATION` rather than a bare `400`: the request parsed, so the bare
/// 400 this gear reserves for a malformed request does not apply (P-D-33,
/// `preconditions`' own doc).
#[tokio::test]
async fn a_publish_without_if_match_is_refused_validation_and_audited() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[],
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an absent precondition rides VALIDATION, which renders 400"
    );

    assert_eq!(
        version_rows(&harness.dsn).await,
        0,
        "a request refused before the transaction opens writes no version row"
    );
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("VALIDATION"),
        "every refusal audits, this one included"
    );
}

/// **A publish on a terminal head is refused `ENTITY_TERMINAL`.**
///
/// The terminal head is produced by this slice's own discard door rather
/// than written by hand, so the case also proves the two doors compose: what
/// discard leaves behind is exactly what publish must refuse.
#[tokio::test]
async fn a_publish_on_a_terminal_head_is_refused_entity_terminal() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let discarded = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "discard",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(
        discarded.status(),
        StatusCode::OK,
        "this case's own premise: the draft discards cleanly"
    );

    let response = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(2))],
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "no head write is admitted on a discarded entity: ENTITY_TERMINAL, a 409"
    );
    assert_eq!(
        version_rows(&harness.dsn).await,
        0,
        "the refused publish froze nothing"
    );
    let codes = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        codes.as_deref(),
        Some("ENTITY_TERMINAL"),
        "the terminal refusal is audited under its own code"
    );
}

/// **A gate that answers no refuses `APPROVAL_REQUIRED` and nothing is
/// written.**
///
/// Driven through `publish_product_under_gate` directly rather than through
/// the router, because the host the router wires is the only one this gear
/// registers and it never refuses — see [`RefusingGate`]. Everything else
/// about the call is what a routed request would produce: the same
/// [`ApiState`], the same enforcer, the same authenticated context and the
/// same headers.
///
/// `inst-fd-gate-rejection` is the property under test: a rejection flips no
/// state, freezes nothing and emits nothing.
#[tokio::test]
async fn a_gate_that_answers_no_refuses_approval_required_and_writes_nothing() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let state = api_state(&harness);
    let enforcer = flat_in_enforcer(TENANT);
    let ctx = authed_ctx(TENANT);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        if_match(1)
            .parse()
            .expect("the ETag is a valid header value"),
    );

    let gate: Arc<dyn GovernanceGate + Send + Sync> = Arc::new(RefusingGate);
    let refusal = super::publish_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        &gate,
        GateMode::Gate,
    )
    .await
    .expect_err("a refusing gate must refuse the publish");
    assert_eq!(
        refusal.into_response().status(),
        StatusCode::FORBIDDEN,
        "APPROVAL_REQUIRED is a 403"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(
        (head.internal_revision, head.published_version),
        (1, 0),
        "a rejection flips no state and moves no counter"
    );
    assert_eq!(
        head.lifecycle_state.as_str(),
        "draft",
        "a first-publish entity stays draft when the gate refuses"
    );
    assert_eq!(
        version_rows(&harness.dsn).await,
        0,
        "the gate refuses before the transaction opens, so nothing is frozen"
    );
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("APPROVAL_REQUIRED"),
        "the gate's refusal is audited like every other"
    );
}

/// A gate host that cannot **reach** an answer, as distinct from one that
/// answers no.
///
/// [`RefusingGate`] is the ceremony saying no; this is the record store
/// behind the ceremony failing to be read. `crate::domain::governance`'s own
/// contract keeps the two apart and says why: a host failure "must not be
/// reported as `APPROVAL_REQUIRED`, which would tell an operator an approval
/// was missing when none was ever looked at".
///
/// **Without this double the branch is unreachable.**
/// `NoMaterialityPolicyGate` is infallible, so no request through the router
/// can produce an `Err` from `evaluate`. Slice 05's store-backed host is the
/// first one that can, and until it lands this double is the only way to
/// exercise the route the door takes.
struct FailingGate;

impl GovernanceGate for FailingGate {
    fn evaluate(
        &self,
        _subject: EntityRef,
        _expected_revision: InternalRevision,
        _mode: GateMode,
    ) -> Result<GateVerdict, DomainError> {
        Err(DomainError::AuditUnavailable(
            "this double cannot reach its record store".to_owned(),
        ))
    }
}

/// **A gate host that fails is an internal failure, not a domain refusal.**
///
/// The door has two `Err`s to route out of the gate step and they are two
/// different kinds of thing: `into_authorization`'s is the ceremony's no
/// (`APPROVAL_REQUIRED`, a 403, audited as a refusal) and `evaluate`'s is
/// the host failing to reach an answer at all. This door mapped **both** to
/// `HeadActError::Refused` until the fix, so a record-store read that failed
/// was answered 4xx and written into the audit trail as a decision the
/// domain had made.
///
/// What this case pins is that the two do not share an answer: a failing
/// host answers 5xx and writes **no audit row**. The audit row is the
/// discriminating half, deliberately. The double answers `AuditUnavailable`
/// — the taxonomy value a real store-read failure would carry — and that
/// value renders 503 under **either** door, so the status assertion states
/// the intent while the audit-row count is what actually reddens against the
/// old code, where `map_err(HeadActError::Refused)` put the host's error on
/// the refusal path and the trail recorded a judgement nobody made.
///
/// Driven through `publish_product_under_gate` for [`RefusingGate`]'s stated
/// reason — the host the router wires cannot fail — and everything else
/// about the call is what a routed request would produce.
#[tokio::test]
async fn a_gate_host_that_fails_is_an_internal_failure_not_a_refusal() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let state = api_state(&harness);
    let enforcer = flat_in_enforcer(TENANT);
    let ctx = authed_ctx(TENANT);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        if_match(1)
            .parse()
            .expect("the ETag is a valid header value"),
    );

    let gate: Arc<dyn GovernanceGate + Send + Sync> = Arc::new(FailingGate);
    let failure = super::publish_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        &gate,
        GateMode::Gate,
    )
    .await
    .expect_err("a host that cannot answer must not publish");
    let response = failure.into_response();
    assert!(
        response.status().is_server_error(),
        "a host that could not reach an answer is infrastructure, so it answers 5xx; a 4xx \
         would tell the caller its own request was at fault. Answered {}",
        response.status()
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(
        (head.internal_revision, head.published_version),
        (1, 0),
        "the transaction rolls back with the failure: no counter moves"
    );
    assert_eq!(
        version_rows(&harness.dsn).await,
        0,
        "nothing is frozen behind a gate that never answered"
    );
    assert_eq!(
        raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await,
        0,
        "a host failure is not a domain decision, so it leaves no refusal row behind. This is \
         the assertion that reddens against the old door: map_err(HeadActError::Refused) sent \
         the host's error down the refusal path, which audits, and the trail would then read \
         as though the domain had judged this act and said no"
    );
}

/// Blank the seeded head's `name` behind the door's back, and move
/// `internal_revision` with it as the head-row guard requires.
///
/// This is the only lever that produces the state
/// `inst-fd-publish-revalidate` is written for: an entity that **was**
/// publishable when it was authored and is not any more. No door in this
/// slice can write a blank name — the create door refuses one — so the
/// re-run's fail-closed branch is unreachable through the gear's own
/// surface, and a raw `UPDATE` through an auxiliary connection is what
/// stands in for the later slice that will move the row.
///
/// No `WHERE`: every case using this seeds exactly one Product, and a
/// predicate over a `Uuid` would have to guess how the driver stored it.
/// `internal_revision + 1` is not decoration either — the head table's
/// `trg_products_product_internal_revision` aborts an `UPDATE` that does not
/// move it by exactly one, so the corruption has to be an admitted write.
async fn blank_the_only_products_name(dsn: &str) {
    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection to corrupt the head");
    conn.execute_unprepared(
        "UPDATE products_product SET name = '   ', internal_revision = internal_revision + 1",
    )
    .await
    .expect("the head-row guard admits a bucket-iii write on a non-terminal draft");
    conn.close().await.ok();
}

/// **A publish whose re-validation fails is refused `INCOMPLETE_ENTITY`, not
/// `VALIDATION`.**
///
/// `inst-fd-publish-revalidate`: *"an entity that stopped being publishable
/// since approval fails closed `INCOMPLETE_ENTITY`/rule-named code"*. The
/// distinction is not bookkeeping. A publish carries **no request body** —
/// `bodiless_payload_digest` is built on exactly that fact — so a
/// `VALIDATION` answer names a field (`name`) of a request that had no
/// fields, and tells the caller to fix a payload it never sent. The row is
/// what is wrong.
///
/// The two codes render the same wire **status** (both 400), which is
/// precisely why this case asserts the `type` in the problem body and the
/// audited `error_code` rather than the status: a status assertion would
/// have passed against the defect. Against the old door this case reddens
/// twice, on `VALIDATION` in both places.
#[tokio::test]
async fn a_publish_whose_revalidation_fails_is_refused_incomplete_entity() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;
    blank_the_only_products_name(&harness.dsn).await;

    // Revision 2, not 1: the corruption above is itself an admitted head
    // write, so the caller's precondition has to be the post-corruption one
    // or this case would measure STALE_REVISION instead.
    let response = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(2))],
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "INCOMPLETE_ENTITY is an architectural 422, wire 400"
    );
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read the response body");
    let view: serde_json::Value = serde_json::from_slice(&body).expect("the response body is JSON");
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("INCOMPLETE_ENTITY"),
        "a publish carries no body, so VALIDATION would name a field of a request that had \
         none; the row stopped being publishable and that is what the code must say"
    );

    assert_eq!(
        version_rows(&harness.dsn).await,
        0,
        "a re-validation that fails closed freezes nothing"
    );
    let head = head_of(&harness, product_id).await;
    assert_eq!(
        (head.internal_revision, head.published_version),
        (2, 0),
        "the refusal rolls the act back: the only revision move is the corruption's own"
    );
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("INCOMPLETE_ENTITY"),
        "the audit row records the code the caller was answered (P-D-37), so it moves with it"
    );
}

/// **A discard of a never-published draft succeeds; a discard of a published
/// head is refused.**
///
/// The refusal is `ILLEGAL_TRANSITION` rather than a validation failure, and
/// the two halves are one case because the second is only meaningful against
/// the first: the same request that is admitted from `draft` is refused from
/// `published`, so what changed is the row's state and nothing else.
#[tokio::test]
async fn a_draft_discards_and_a_published_head_does_not() {
    let harness = harness().await;
    let draft_id = Uuid::now_v7();
    seed_draft(&harness, draft_id).await;

    let discarded = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        draft_id,
        "discard",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(discarded.status(), StatusCode::OK);

    let head = head_of(&harness, draft_id).await;
    assert_eq!(
        head.lifecycle_state.as_str(),
        "discarded",
        "the draft is discarded, terminally"
    );
    assert_eq!(
        head.internal_revision, 2,
        "the transition bumps the revision exactly once, through its own single UPDATE"
    );
    assert_eq!(
        head.published_version, 0,
        "a discard never touches published_version"
    );
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let payload_type = raw_string_opt(
        &harness.dsn,
        &format!("SELECT payload_type AS v FROM {body_table}"),
    )
    .await;
    assert_eq!(
        payload_type.as_deref(),
        Some("ProductDiscarded"),
        "the discard enqueues ProductDiscarded"
    );

    // The published head: same request, different starting state.
    let published_id = Uuid::now_v7();
    {
        let conn = harness
            .db
            .conn()
            .expect("checkout the pinned production connection");
        let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
        let mut second = new_product(published_id, TENANT);
        second.name = "Fibre 900".to_owned();
        second.name_normalized = "fibre 900".to_owned();
        second.product_code = Some("FIBRE-900".to_owned());
        repo::insert_product(&conn, &scope, second)
            .await
            .expect("seed the second draft");
    }
    let published = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        published_id,
        "publish",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(published.status(), StatusCode::OK);

    let refused = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        published_id,
        "discard",
        &[("If-Match", &if_match(2))],
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::CONFLICT,
        "a discard is legal only from draft at published_version 0: ILLEGAL_TRANSITION, a 409"
    );
    assert_eq!(
        head_of(&harness, published_id)
            .await
            .lifecycle_state
            .as_str(),
        "published",
        "the refused discard left the published head exactly where it was"
    );
}

/// **After a discard, the next Product may take the discarded one's
/// `product_code` and name.**
///
/// Both reservations release by the discard's own `UPDATE`, the two partial
/// unique indexes excluding `discarded` rows — there is no release step to
/// forget, and this case is what would notice if either index's predicate
/// were narrowed to exclude only `retired`, or if a later edit added a
/// release statement that did the job twice.
#[tokio::test]
async fn a_discarded_products_name_and_code_are_free_for_the_next_product() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    let seeded = seed_draft(&harness, product_id).await;

    // The premise: while the draft lives, the name and the code are taken.
    let blocked = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({
            "brand_id": BRAND,
            "name": seeded.name.clone(),
            "product_code": seeded.product_code.clone(),
        }),
    )
    .await;
    assert_eq!(
        blocked.status(),
        StatusCode::CONFLICT,
        "this case's own premise: a live draft holds its name and its code"
    );

    let discarded = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "discard",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(discarded.status(), StatusCode::OK);

    let admitted = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({
            "brand_id": BRAND,
            "name": seeded.name,
            "product_code": seeded.product_code,
        }),
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::CREATED,
        "the discard released both reservations by its own write, with no release step"
    );
}

/// **A replayed publish returns the stored answer and does not publish
/// twice.**
///
/// The case the idempotency store exists for, at this door: a client that
/// never learned the outcome retries under the same key, and must be served
/// the original `200` rather than publishing version 2. Both "executes
/// nothing" halves are asserted on storage — no second frozen row and no
/// second event — because a door that re-ran the act and merely happened to
/// answer `200` would pass a status-only assertion while duplicating the
/// version history.
#[tokio::test]
async fn a_replayed_publish_serves_the_stored_answer_and_does_not_publish_twice() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let first = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(1)), ("Idempotency-Key", "publish-1")],
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    let original = json_body(first).await;

    let state = raw_string_opt(
        &harness.dsn,
        "SELECT state AS v FROM products_idempotency WHERE client_key = 'publish-1'",
    )
    .await;
    assert_eq!(
        state.as_deref(),
        Some("answered"),
        "the committed publish answered its own claim, in the transaction that took it"
    );
    let endpoint = raw_string_opt(
        &harness.dsn,
        "SELECT endpoint AS v FROM products_idempotency WHERE client_key = 'publish-1'",
    )
    .await;
    assert_eq!(
        endpoint.as_deref(),
        Some(format!("/bss-products/v1/products/{product_id}/publish").as_str()),
        "the key names the concrete resource path, id and all, never the route template (P-D-42)"
    );

    // The retry carries the *original* precondition, which is what a client
    // that never saw the first response would still hold.
    let retry = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(1)), ("Idempotency-Key", "publish-1")],
    )
    .await;
    assert_eq!(
        retry.status(),
        StatusCode::OK,
        "the retry replays the original status rather than being refused for the now-stale \
         precondition"
    );
    assert_eq!(
        json_body(retry).await,
        original,
        "the replay reproduces the original body"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(
        (head.published_version, head.internal_revision),
        (1, 2),
        "the retry executed nothing: neither counter moved a second time"
    );
    assert_eq!(
        version_rows(&harness.dsn).await,
        1,
        "the retry executed nothing: no second frozen version row"
    );
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let enqueued = raw_i64(
        &harness.dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table}"),
    )
    .await;
    assert_eq!(
        enqueued, 1,
        "the retry executed nothing: no second ProductPublished row"
    );
}

/// The four head columns §4.3 keeps **out** of a frozen row's content, as
/// this file states them independently of the door.
///
/// Spelled here rather than read out of
/// [`super::PRODUCT_CONTENT_ROSTER`]'s own module because that is what makes
/// the case below a **drift** test: the roster is a literal list, and the
/// only way to catch a literal list that forgot a column is to derive the
/// answer from something the roster does not control — the executed schema,
/// plus this short, independently authored exclusion set.
///
/// Two of the four are §4.3's own words (`lifecycle_state`,
/// `internal_revision`; P-D-24 and P-D-35). The other two are readings the
/// door states and argues, because §4.3 enumerates its exclusions as a
/// closed list of four columns plus the metadata map and names neither:
/// `updated_at` is P-D-35's own stated criterion applied to a column the
/// enumeration does not list, and `published_version` is the version row's
/// own key column, which the content would otherwise restate inside the
/// payload the key keys. See [`super::PRODUCT_CONTENT_ROSTER`]'s doc for
/// both arguments and for the additions the design set is owed. §4.3's other two exclusions,
/// `deprecation_provenance` and `replaced_by_sku_id`, are deliberately
/// absent: neither is a column of `products_product` at this commit, and
/// naming one would make the `is a real column` assertion below fail for the
/// wrong reason.
const EXCLUDED_FROM_FROZEN_CONTENT: [&str; 4] = [
    "internal_revision",
    "lifecycle_state",
    "published_version",
    "updated_at",
];

/// **[`super::PRODUCT_CONTENT_ROSTER`] is `products_product`'s own columns
/// minus [`EXCLUDED_FROM_FROZEN_CONTENT`]** — §4.3's rule, measured against
/// the schema the migration chain executed.
///
/// The roster is the third copy of this column list (the two migrations hold
/// the others), and slices 02 and 03 add content columns. Nothing else in
/// this suite would notice a roster that forgot one: a forgotten column
/// simply never reaches the digest, every existing case still passes, and
/// the loss surfaces years later as a restore that cannot reproduce a
/// version. This case is what notices.
///
/// The three assertions are not one assertion written three ways. The middle
/// one — that each excluded name **is** a real column — is what stops the
/// exclusion set from quietly becoming decorative: an exclusion naming a
/// column that does not exist subtracts nothing, and the equality would then
/// hold for a roster that is simply the whole table.
#[tokio::test]
async fn the_product_content_roster_is_the_head_table_minus_the_excluded_columns() {
    let harness = harness().await;
    let columns = table_columns(&harness.dsn, "products_product").await;

    for excluded in EXCLUDED_FROM_FROZEN_CONTENT {
        assert!(
            columns.contains(&excluded.to_owned()),
            "{excluded} must be a real column of products_product for its exclusion to \
             subtract anything; the executed schema has {columns:?}"
        );
        assert!(
            !super::PRODUCT_CONTENT_ROSTER.contains(&excluded),
            "section 4.3 excludes {excluded} from a frozen row's content, so the roster must \
             not name it"
        );
    }

    let mut expected: Vec<String> = columns
        .into_iter()
        .filter(|column| !EXCLUDED_FROM_FROZEN_CONTENT.contains(&column.as_str()))
        .collect();
    expected.sort();
    let mut roster: Vec<String> = super::PRODUCT_CONTENT_ROSTER
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    roster.sort();

    assert_eq!(
        roster, expected,
        "section 4.3 scopes a frozen row's content as the publish-time entity minus its named \
         exclusions, so the roster is the head table's columns minus those and nothing else. A \
         slice that adds a content column to products_product adds it here too, and bumps \
         canonical::DIGEST_VERSION with it"
    );
}

/// **[`super::product_content`] writes exactly
/// [`super::PRODUCT_CONTENT_ROSTER`]'s names** — no extra key, and, the part
/// that matters, no missing one.
///
/// The drift case above compares the *roster* to the executed schema.
/// Nothing compared the *builder* to the roster, and
/// `canonical::Absence::Null` is what makes that gap silent: a roster name
/// the builder forgot is rendered `null` rather than refused, so a builder
/// that dropped `name` would freeze `"name":null`, digest cleanly, and pass
/// every other case in this file — the digest case included, since that one
/// re-hashes whatever was stored rather than judging what it says. Slice
/// 10's restore drill would then reproduce a Product with no name and call
/// it a match.
///
/// The fixture's `product_code` is **present**, and that is a premise the
/// case asserts rather than assumes: `product_code` is the one roster field
/// the builder legitimately omits from the map when the head carries none
/// (that omission is what exercises `Absence::Null`), so against a
/// code-less fixture this equality would hold for a builder that had dropped
/// the field entirely and the case would prove one name less than it looks
/// like it does.
#[tokio::test]
async fn the_product_content_builder_writes_exactly_the_roster() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    let mut head = seed_draft(&harness, product_id).await;
    assert!(
        head.product_code.is_some(),
        "this case's own premise: the optional roster field is present, or the equality below \
         would hold for a builder that had dropped it"
    );
    // The lineage pair joined the roster with P-D-76 and is optional on the
    // same terms as `product_code`, so the same premise applies: populate it
    // here, or the equality would hold for a builder that dropped both names.
    // The builder is a pure function over the record, so setting the pair on
    // the read-back copy is the fixture, not a bypass.
    head.cloned_from = Some(Uuid::now_v7());
    head.cloned_from_version = Some(3);

    let content = super::product_content(&head);
    let mut written: Vec<&str> = content
        .as_object()
        .expect("the builder renders a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    written.sort_unstable();
    let mut roster = super::PRODUCT_CONTENT_ROSTER;
    roster.sort_unstable();

    assert_eq!(
        written, roster,
        "the builder and the roster are one field set stated twice, and only this assertion \
         holds them equal: Absence::Null renders a name the builder forgot as null instead of \
         failing, so the omission would reach storage and no other case would notice"
    );
}

/// The JSON body of the newest enqueued outbox row carrying `payload_type`.
///
/// The `payload` column is a `BLOB`; `CAST(.. AS TEXT)` is what lets
/// [`raw_string_opt`]'s single-text-column shape read it. Filtering by
/// `payload_type` keeps this off the `ProductCreated` row any seeded create
/// enqueued, and the descending order is what makes a second publish's body
/// readable rather than the first one's.
async fn enqueued_event_body(dsn: &str, payload_type: &str) -> serde_json::Value {
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let payload = raw_string_opt(
        dsn,
        &format!(
            "SELECT CAST(payload AS TEXT) AS v FROM {body_table} \
             WHERE payload_type = '{payload_type}' ORDER BY id DESC LIMIT 1"
        ),
    )
    .await
    .expect("the enqueued row carries a payload");
    let envelope: serde_json::Value =
        serde_json::from_str(&payload).expect("the door enqueues a JSON envelope");
    // The door wraps every body in `events::EventEnvelope`; §4.5's five
    // fields live under `data`. Unwrapped here rather than at each call site
    // so a test that asks for "the body" keeps reading the body.
    envelope["data"].clone()
}

/// **`ProductPublished` carries `publishedVersion`, and it is the version the
/// act produced.**
///
/// §4.5: every one of the eight Foundation events carries the same body core,
/// and `ProductPublished`/`SkuPublished` **additionally** carry
/// `publishedVersion` — which slice 06 reads as content and slice 08's
/// projector keys on. A body without it is a body those two consumers cannot
/// use.
///
/// The **value** is asserted, not merely the key's presence. A door that
/// hard-coded a zero, or that announced the pre-act `N` the head carried
/// before the publish, would satisfy an existence check and would still point
/// 06 and 08 at the wrong version. So this reads `published_version` off the
/// head after the act and requires the event to agree — and, to separate the
/// two candidate numbers, it publishes **twice**: after a re-publish the
/// pre-act value is `1` and the post-act value is `2`, so a door announcing
/// `N` fails here even though it would pass on a first publish only if it
/// also happened to be off by one in the other direction.
#[tokio::test]
async fn the_published_event_carries_the_post_act_published_version() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let first = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK, "this case's premise");

    let body = enqueued_event_body(&harness.dsn, "ProductPublished").await;
    assert_eq!(
        body["publishedVersion"],
        json!(head_of(&harness, product_id).await.published_version),
        "the event announces the version the act wrote, read back off the head"
    );
    assert_eq!(
        body["publishedVersion"],
        json!(1),
        "a first publish produces version 1"
    );
    // The core is still there, unchanged: `publishedVersion` is *additional
    // to* the core (§4.5), not a replacement for any of it.
    assert_eq!(body["entityKind"], json!("product"));
    assert_eq!(body["entityId"], json!(product_id.to_string()));
    assert_eq!(body["tenantId"], json!(TENANT.to_string()));
    assert_eq!(body["lifecycleState"], json!("published"));
    assert_eq!(body["internalRevision"], json!(2));

    let second = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(2))],
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK, "a re-publish is admitted");

    let after = enqueued_event_body(&harness.dsn, "ProductPublished").await;
    assert_eq!(
        after["publishedVersion"],
        json!(head_of(&harness, product_id).await.published_version),
        "the re-publish announces 2, the version it produced, not the 1 the head carried when \
         it began"
    );
    assert_eq!(after["publishedVersion"], json!(2));
}

/// **A `ProductDiscarded` body carries no `publishedVersion` at all.**
///
/// §4.5 puts the field on the two `*Published` events and on no other, which
/// is the whole reason it lives on `events::PublishedEventBody` rather than
/// becoming a sixth field of `events::EventBodyCore`. A discard writes no
/// version row and moves no version counter, so any number it announced would
/// be one nothing produced.
#[tokio::test]
async fn a_discarded_event_carries_no_published_version() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "discard",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK, "this case's premise");

    let body = enqueued_event_body(&harness.dsn, "ProductDiscarded").await;
    assert_eq!(
        body.get("publishedVersion"),
        None,
        "section 4.5 names publishedVersion on the two *Published events and on no other"
    );
    assert_eq!(body["lifecycleState"], json!("discarded"));
    assert_eq!(body["internalRevision"], json!(2));
}

/// A gate host that records the mode it was asked in and names a record
/// either way — the double the `PreAuthorized` seam needs.
///
/// The two arms differ the way `inst-fd-gate-mode-gate` and
/// `inst-fd-gate-mode-preauthorized` say a real host's must: under `Gate` it
/// names a record **to consume**, under `PreAuthorized` one already
/// **verified**. `NoMaterialityPolicyGate` can do neither — it holds no
/// record store, so it names no record under `Gate` and refuses outright
/// under `PreAuthorized` — which is why the seam is unreachable without a
/// double.
struct RecordingGate {
    approval: ApprovalId,
    asked: std::sync::Mutex<Vec<GateMode>>,
}

impl RecordingGate {
    fn new(approval: ApprovalId) -> Self {
        Self {
            approval,
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Every mode this host was asked in, in order.
    fn modes(&self) -> Vec<GateMode> {
        self.asked
            .lock()
            .expect("no case poisons this lock")
            .clone()
    }
}

impl GovernanceGate for RecordingGate {
    fn evaluate(
        &self,
        _subject: EntityRef,
        _expected_revision: InternalRevision,
        mode: GateMode,
    ) -> Result<GateVerdict, DomainError> {
        self.asked
            .lock()
            .expect("no case poisons this lock")
            .push(mode);
        let disposition = match mode {
            GateMode::Gate => ApprovalDisposition::Consume(self.approval),
            GateMode::PreAuthorized(id) => ApprovalDisposition::Verified(id),
        };
        Ok(GateVerdict::authorized(
            disposition,
            false,
            "this double authorizes and records the mode it was asked in".to_owned(),
        ))
    }
}

/// **A publish driven in `PreAuthorized` mode reaches the host in that mode,
/// publishes, and consumes nothing.**
///
/// This is the seam `dod-publish-door` (**P-D-30**) requires the door to
/// have: *"the door MUST take a gate mode as an explicit argument ... under
/// `PreAuthorized(approvalId)` it verifies the named record and does not
/// consume it, which is what lets `04-lifecycle`'s scheduled-publish runner
/// drive this same door"*. Until the mode became an argument,
/// `GateMode::PreAuthorized` was a type with no call path anywhere in the
/// gear, so the runner 04 will ship had nothing to arrive through.
///
/// Three things are asserted, and the first is the one that fails against a
/// door that hard-codes the mode: the host was asked in
/// `PreAuthorized(approval)` and in nothing else. A case asserting only that
/// the publish succeeded would pass against a door that quietly substituted
/// `Gate`, since this double authorizes under both.
///
/// **"Consumes nothing" is asserted as far as this slice can reach.** The
/// consume flip itself is `inst-fd-publish-consume`'s and belongs to slice
/// 05's record store, which does not exist here — there is nothing in this
/// gear a flip could write to. What is provable now is the property the flip
/// will be written against: the verdict the door acted on names the record
/// for `approval_ref` and offers **no** id for consumption, and the frozen
/// version row carries that id. `approval_to_consume()` answering `None` is
/// what makes "nothing is consumed under `PreAuthorized`" a property of the
/// type rather than a rule a future door has to remember.
#[tokio::test]
async fn a_preauthorized_publish_reaches_the_host_in_that_mode_and_consumes_nothing() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let approval = ApprovalId::new(Uuid::now_v7());
    let recorder = Arc::new(RecordingGate::new(approval));
    let gate: Arc<dyn GovernanceGate + Send + Sync> = Arc::clone(&recorder) as _;

    let state = api_state(&harness);
    let enforcer = flat_in_enforcer(TENANT);
    let ctx = authed_ctx(TENANT);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        if_match(1)
            .parse()
            .expect("the ETag is a valid header value"),
    );

    let response = super::publish_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        &gate,
        GateMode::PreAuthorized(approval),
    )
    .await
    .expect("a verified pre-authorization must publish");
    assert_eq!(
        response.into_response().status(),
        StatusCode::OK,
        "the scheduled-publish path drives the ordinary door to the ordinary answer"
    );

    assert_eq!(
        recorder.modes(),
        vec![GateMode::PreAuthorized(approval)],
        "the door passed the caller's mode through unchanged, and asked exactly once; a door \
         that substituted GateMode::Gate would still have published, which is why the mode \
         itself is the assertion"
    );

    // The disposition the door acted on, asked of the same host the door
    // asked: `Verified` names the record for `approval_ref` and offers
    // nothing to spend.
    let verdict = recorder
        .evaluate(
            EntityRef {
                tenant_id: TENANT,
                entity_kind: bss_products_sdk::models::EntityKind::Product,
                entity_id: product_id,
            },
            InternalRevision::new(1),
            GateMode::PreAuthorized(approval),
        )
        .expect("this double never fails to reach an answer");
    let GateVerdict::Authorized(authorization) = verdict else {
        panic!("this double authorizes under PreAuthorized")
    };
    assert_eq!(
        authorization.approval_to_consume(),
        None,
        "nothing is consumed under PreAuthorized (inst-fd-publish-consume), and that is a \
         property of ApprovalDisposition rather than a rule the door remembers"
    );
    assert_eq!(
        authorization.approval_ref(),
        Some(approval),
        "a PreAuthorized act still records which approval stands behind the frozen version"
    );

    let matched = raw_i64(
        &harness.dsn,
        &format!(
            "SELECT COUNT(*) AS v FROM products_entity_version WHERE approval_ref = '{approval}' \
             OR hex(approval_ref) = '{}'",
            approval.get().simple().to_string().to_uppercase()
        ),
    )
    .await;
    assert_eq!(
        matched, 1,
        "the frozen row carries the verified record's id in approval_ref, which is the one \
         accessor this act reads off the verdict"
    );
}

/// **The governance-gate phase runs on the discard door: a host that says no
/// refuses `APPROVAL_REQUIRED` and writes nothing.**
///
/// §3.1's `inst-fd-pipeline-gate-phase` puts the phase at *every* mutating
/// door, passing trivially where the act is ungated (**P-D-34**), and §1.1
/// makes governance a phase *inside* the pipeline rather than a path around
/// it. Under the gear's own host a discard is ungated and the phase is
/// invisible, so this double is the only way to tell a phase that passes
/// from a phase that was never asked — which is exactly the distinction that
/// stops mattering the day slice 05 registers a ceremony on a transition.
///
/// The status alone would not separate those two worlds either way, so the
/// assertions are the problem body's own code and the audit row's
/// `error_code`.
#[tokio::test]
async fn a_gate_that_answers_no_refuses_the_discard_and_writes_nothing() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let state = api_state(&harness);
    let enforcer = flat_in_enforcer(TENANT);
    let ctx = authed_ctx(TENANT);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        if_match(1)
            .parse()
            .expect("the ETag is a valid header value"),
    );

    let gate: Arc<dyn GovernanceGate + Send + Sync> = Arc::new(RefusingGate);
    let refusal =
        super::discard_product_under_gate(&state, &enforcer, &ctx, product_id, &headers, &gate)
            .await
            .expect_err("a refusing gate must refuse the discard, which proves it was asked");

    let response = refusal.into_response();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "APPROVAL_REQUIRED is the gate's own 403"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(
        head.lifecycle_state.as_str(),
        "draft",
        "a rejection flips no state (inst-fd-gate-rejection), on this door as on publish"
    );
    assert_eq!(
        head.internal_revision, 1,
        "the refusal rolled the transaction back, so no head-row UPDATE landed"
    );

    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("APPROVAL_REQUIRED"),
        "the discard door's gate refusal audits under its own code, like every other refusal"
    );
}

/// **The discard door's gate phase passes trivially under the gear's own
/// host**: a routed discard, which wires `NoMaterialityPolicyGate`,
/// succeeds.
///
/// The other half of the pair. `inst-fd-pipeline-gate-phase` asks for both
/// halves at once — the phase runs *and* it costs an ungated act nothing —
/// and a case proving only the refusal would leave a door that refuses every
/// discard indistinguishable from a correct one.
#[tokio::test]
async fn the_discard_doors_gate_phase_passes_trivially_under_the_default_host() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "discard",
        &[("If-Match", &if_match(1))],
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the default host authorizes naming no record, so an ungated discard pays nothing for \
         the phase"
    );
    let head = head_of(&harness, product_id).await;
    assert_eq!(head.lifecycle_state.as_str(), "discarded");
    assert_eq!(
        raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await,
        0,
        "a success is not a refusal and audits nothing (P-D-21)"
    );
}

/// [`RefusingGate`] that also counts how many times it was asked.
///
/// The count is the operand the precedence case below needs and the plain
/// refusal cannot give: "the state phase is judged before the gate" is a claim
/// about **whether the gate is consulted at all**, and a door that asked it
/// and then discarded the answer would look identical from the response.
struct CountingRefusingGate {
    asked: std::sync::atomic::AtomicUsize,
}

impl CountingRefusingGate {
    const fn new() -> Self {
        Self {
            asked: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn asked(&self) -> usize {
        self.asked.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl GovernanceGate for CountingRefusingGate {
    fn evaluate(
        &self,
        _subject: EntityRef,
        _expected_revision: InternalRevision,
        _mode: GateMode,
    ) -> Result<GateVerdict, DomainError> {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(GateVerdict::Refused {
            reason: "this double refuses every act it is asked about".to_owned(),
        })
    }
}

/// **The state phase outranks the governance gate on the Product publish
/// door: a head the state phase refuses answers the state phase's code and
/// the gate is never asked.**
///
/// `crate::domain::validation::Phase::ordered()` puts `State` ahead of
/// `GovernanceGate`, and the consequence is not cosmetic: a caller told to
/// seek an approval for an act that is not legal at all has been sent to
/// obtain something that would not help, and the audit trail records an
/// approval question where the machine's own refusal belongs. This door asked
/// the gate ahead of `transition::guard` until the ordering was corrected; it
/// now asks it last, as `skus::run_publish` and both discard doors already
/// did.
///
/// # What this case does and does not discriminate, measured
///
/// It pins the rule for the one state-phase refusal a publish can reach —
/// terminality — and it is honest to say that this refusal was ordered
/// correctly **before** the fix as well: `transition::check_head_write` has
/// always run ahead of the gate, and only `transition::guard` moved.
///
/// A case that discriminated the swap itself would need a head that fails the
/// **edge** and would be refused by the gate, and no such head exists on this
/// door. `check_head_write` refuses `retired` and `discarded` first, and on
/// the three states that survive it — `draft`, `published`, `deprecated` —
/// `super::published_state_after` yields either the admitted
/// `draft -> published` edge or the same-value diagonal, both of which
/// `transition::guard` admits. So `guard` cannot refuse a publish at this
/// commit and the two orderings are observationally equal; the swap is a
/// compliance fix against `Phase::ordered()` and against the SKU door, not a
/// behaviour fix. This case is what will notice if a later slice widens
/// `published_state_after` or `ADMITTED_EDGES` and the ordering silently
/// stops holding.
///
/// The gate's own call count is asserted rather than only the response,
/// because both refusals render a status this suite already sees: a status
/// assertion alone would pass against a door that asked the gate, got its
/// `no`, and happened to report the terminal refusal anyway.
#[tokio::test]
async fn the_state_phase_outranks_the_gate_on_the_publish_door() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let discarded = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "discard",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(
        discarded.status(),
        StatusCode::OK,
        "this case's own premise: the draft discards cleanly, leaving a terminal head"
    );

    let state = api_state(&harness);
    let enforcer = flat_in_enforcer(TENANT);
    let ctx = authed_ctx(TENANT);
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        if_match(2)
            .parse()
            .expect("the ETag is a valid header value"),
    );

    let counter = Arc::new(CountingRefusingGate::new());
    let gate: Arc<dyn GovernanceGate + Send + Sync> = Arc::clone(&counter) as _;
    let refusal = super::publish_product_under_gate(
        &state,
        &enforcer,
        &ctx,
        product_id,
        &headers,
        &gate,
        GateMode::Gate,
    )
    .await
    .expect_err("a terminal head is refused whatever the gate would have said");
    assert_eq!(
        refusal.into_response().status(),
        StatusCode::CONFLICT,
        "ENTITY_TERMINAL is a 409; APPROVAL_REQUIRED would have been a 403, which is what makes \
         the status readable here at all"
    );

    assert_eq!(
        counter.asked(),
        0,
        "the state phase refused, so the gate was never consulted -- the property the phase \
         order exists to give, and the one a status assertion cannot see"
    );

    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("ENTITY_TERMINAL"),
        "the audited code names the rule that actually refused, not the approval question that \
         was never asked"
    );
}

/// `PATCH /bss-products/v1/products/{id}` with `body` and `headers` — the
/// save door's request shape.
///
/// A separate helper from [`post_head_act`] rather than a sixth parameter on
/// it: a save is the only door of the three that carries a request body, and
/// folding a body into the bodiless helper would let a later case send one to
/// `publish` without the compiler minding.
async fn patch_product(
    app: Router,
    tenant: Uuid,
    product_id: Uuid,
    body: &serde_json::Value,
    headers: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut request = Request::builder()
        .method("PATCH")
        .uri(format!("/bss-products/v1/products/{product_id}"))
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .extension(authed_ctx(tenant));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    app.oneshot(
        request
            .body(Body::from(body.to_string()))
            .expect("build the save request"),
    )
    .await
    .expect("the router answers")
}

/// [`patch_product`] carrying only an `If-Match` built from `revision` — the
/// shape most cases below send.
async fn save_at(
    harness: &TestHarness,
    product_id: Uuid,
    revision: i64,
    body: &serde_json::Value,
) -> axum::http::Response<Body> {
    patch_product(
        app_for(harness, TENANT),
        TENANT,
        product_id,
        body,
        &[("If-Match", &if_match(revision))],
    )
    .await
}

/// **A bucket-iii save on a `draft` head is admitted and moves
/// `internal_revision` by exactly one.**
///
/// `name` and `name_normalized` are one bucket-iii field in §4.1 — the second
/// is *"the same field's index operand"* — so both are asserted: a door that
/// wrote the display name alone would leave `uq_products_product_name` keyed
/// to a name the row no longer carries, and the next author would collide
/// with a name nobody holds.
///
/// "By exactly one" is the assertion the head-row guard makes load-bearing:
/// it refuses any `UPDATE` whose `internal_revision` is not `OLD + 1`, so a
/// save split across two statements would move it twice and the `ETag` this
/// door just handed back would skip a value it never returned.
#[tokio::test]
async fn a_bucket_iii_save_on_a_draft_is_admitted_and_bumps_the_revision_once() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = save_at(&harness, product_id, 1, &json!({ "name": "Fibre 900" })).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a bucket-iii save on a non-terminal head is an ordinary admitted write"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(head.name, "Fibre 900", "the routed column was written");
    assert_eq!(
        head.name_normalized, "fibre 900",
        "the index operand moved with the field it is derived from"
    );
    assert_eq!(
        head.internal_revision, 2,
        "one admitted UPDATE moves the revision by exactly one"
    );
    assert_eq!(head.published_version, 0, "a save moves no version counter");
    assert_eq!(
        head.lifecycle_state.as_str(),
        "draft",
        "a save takes no edge"
    );
    assert_eq!(
        version_rows(&harness.dsn).await,
        0,
        "a save writes no products_entity_version row"
    );
}

/// **A bucket-iii save on a `published` head is admitted too, writes no
/// version row and does not move `published_version`.**
///
/// §4.1 is explicit that a published Product *can* be renamed and that the
/// rename comes out as version N+1 under governance rather than forcing
/// retire-and-clone. The two negative assertions are what separate this door
/// from the publish door: the head is the authoring surface in every
/// non-terminal state (`inst-fd-transition-guard`), and the version row is
/// the publish act's alone. A save that froze a row here would key it at a
/// `published_version` the head never took.
#[tokio::test]
async fn a_bucket_iii_save_on_a_published_head_writes_no_version_row() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let published = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(
        published.status(),
        StatusCode::OK,
        "this case's own premise: the draft publishes"
    );
    assert_eq!(
        version_rows(&harness.dsn).await,
        1,
        "this case's own premise: the publish froze exactly one version"
    );

    let response = save_at(&harness, product_id, 2, &json!({ "name": "Fibre 900" })).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a published Product can be renamed (section 4.1)"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(head.name, "Fibre 900");
    assert_eq!(
        head.internal_revision, 3,
        "the save bumped the revision the publish left at 2"
    );
    assert_eq!(
        head.published_version, 1,
        "a save does not move published_version"
    );
    assert_eq!(
        head.lifecycle_state.as_str(),
        "published",
        "a save takes no edge, so the state the publish set stands"
    );
    assert_eq!(
        version_rows(&harness.dsn).await,
        1,
        "still the publish's one row: a save freezes nothing"
    );
}

/// **A bucket-i save before first publish is admitted; after first publish
/// the same field is `ILLEGAL_FIELD_MUTATION`.**
///
/// One case for both halves on purpose: the refusal alone passes against a
/// door that refuses every bucket-i write, and the admission alone passes
/// against one that admits every bucket-i write. Only the pair pins
/// `inst-fd-bucket-i-refusal`'s actual rule, which is keyed to
/// `published_version`.
///
/// The assertion is the **problem body's own code**, not the status: §3.3
/// renders `ILLEGAL_FIELD_MUTATION`, `STALE_REVISION` and `ENTITY_TERMINAL`
/// all as `409`, so a status-only assertion here would pass against three
/// different refusals.
#[tokio::test]
async fn a_bucket_i_save_is_admitted_before_first_publish_and_refused_after_it() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let admitted = save_at(
        &harness,
        product_id,
        1,
        &json!({ "product_code": "FIBRE-900" }),
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "published_version is 0, so identity is still writable"
    );
    assert_eq!(
        head_of(&harness, product_id).await.product_code.as_deref(),
        Some("FIBRE-900"),
        "the bucket-i column was written"
    );

    let published = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(2))],
    )
    .await;
    assert_eq!(
        published.status(),
        StatusCode::OK,
        "this case's own premise: the draft publishes"
    );

    let refused = save_at(
        &harness,
        product_id,
        3,
        &json!({ "product_code": "FIBRE-000" }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let view = json_body(refused).await;
    assert_eq!(
        view["context"]["reason"],
        json!("ILLEGAL_FIELD_MUTATION"),
        "a bucket-i write after first publish is refused by its own rule, not by a stale \
         precondition or a terminal state"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(
        head.product_code.as_deref(),
        Some("FIBRE-900"),
        "the refused save wrote nothing"
    );
    assert_eq!(
        head.internal_revision, 3,
        "and moved no counter, so the caller's ETag still stands"
    );
    assert_eq!(
        audit_error_code(&harness.dsn).await.as_deref(),
        Some("ILLEGAL_FIELD_MUTATION"),
        "the refusal wrote its own audit row"
    );
    assert_eq!(
        audit_action(&harness.dsn).await.as_deref(),
        Some("save"),
        "recorded under this act's own token, not the publish door's"
    );
}

/// **A field no registry row names is refused by the fail-closed miss, and a
/// good field beside it still saves.**
///
/// P-D-50: *"a published-state column carrying no tag means it was added
/// without registering one, and the head door refuses the write under the
/// pipeline's own posture rather than routing to a default bucket"*.
/// `deprecation_provenance` is exactly that shape — a column §4.3 names and
/// `products_product` does not carry at this commit — so it resolves to no
/// row and `crate::domain::bucket::classify` refuses it.
///
/// The positive control is the point of the second half: a door that refused
/// every field would pass the first assertion alone.
#[tokio::test]
async fn an_unregistered_field_is_refused_by_the_fail_closed_miss() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let refused = save_at(
        &harness,
        product_id,
        1,
        &json!({ "deprecation_provenance": "operator" }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(refused).await["context"]["reason"],
        json!("ILLEGAL_FIELD_MUTATION"),
        "a miss is a refusal, never a default bucket"
    );
    assert_eq!(
        head_of(&harness, product_id).await.internal_revision,
        1,
        "the refused save wrote nothing at all"
    );

    let admitted = save_at(&harness, product_id, 1, &json!({ "name": "Fibre 900" })).await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "the positive control: this door does not refuse everything"
    );
}

/// **A save whose fourth field is refused applies none of the first three.**
///
/// Routing runs over the **whole** request before any column is written, so a
/// `PATCH` naming one admitted field and one refused one is refused whole. A
/// door that routed and wrote field by field would leave the head carrying
/// half a request the caller was told had failed — a worse outcome than the
/// refusal, and one no status assertion catches.
#[tokio::test]
async fn a_save_with_one_refused_field_applies_none_of_the_others() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let published = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(published.status(), StatusCode::OK);

    let refused = save_at(
        &harness,
        product_id,
        2,
        &json!({ "name": "Fibre 900", "product_code": "FIBRE-000" }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(refused).await["context"]["reason"],
        json!("ILLEGAL_FIELD_MUTATION")
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(
        head.name, "Fibre 500",
        "the admitted field in the same request was not applied either"
    );
    assert_eq!(
        head.product_code.as_deref(),
        Some("FIBRE-500"),
        "nor, obviously, the refused one"
    );
    assert_eq!(head.internal_revision, 2, "and no counter moved");
}

/// **A stale `If-Match` is `STALE_REVISION` and writes nothing**, with its
/// own audit row under this act's token.
#[tokio::test]
async fn a_save_with_a_stale_if_match_is_refused_and_writes_nothing() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = save_at(&harness, product_id, 7, &json!({ "name": "Fibre 900" })).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(response).await["context"]["reason"],
        json!("STALE_REVISION"),
        "the body's code is what tells this refusal from the two other 409s"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(head.name, "Fibre 500", "nothing was written");
    assert_eq!(head.internal_revision, 1, "and no counter moved");
    assert_eq!(
        audit_error_code(&harness.dsn).await.as_deref(),
        Some("STALE_REVISION")
    );
    assert_eq!(
        audit_action(&harness.dsn).await.as_deref(),
        Some("save"),
        "the second of the two refusals this file pins the action token on"
    );
}

/// **A save with no `If-Match` at all is refused `VALIDATION`.**
///
/// `VALIDATION` rather than `STALE_REVISION`: the caller pinned nothing, so
/// there is nothing to be stale. `preconditions::if_match`'s own doc gives
/// the wording, and the save door is the verb it was written for.
#[tokio::test]
async fn a_save_without_if_match_is_refused_validation() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = patch_product(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        &json!({ "name": "Fibre 900" }),
        &[],
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an absent precondition rides VALIDATION, which renders 400"
    );
    assert_eq!(
        head_of(&harness, product_id).await.name,
        "Fibre 500",
        "nothing was written"
    );
    assert_eq!(
        audit_error_code(&harness.dsn).await.as_deref(),
        Some("VALIDATION"),
        "every refusal audits, this one included"
    );
}

/// **A save on a terminal head is `ENTITY_TERMINAL`** — the rule that reaches
/// every head write and not only a transition (`inst-fd-terminal`, P-D-25
/// widened by P-D-32).
///
/// The terminal head is produced by this gear's own discard door rather than
/// written by hand, so the case also proves the doors compose.
#[tokio::test]
async fn a_save_on_a_terminal_head_is_refused_entity_terminal() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let discarded = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "discard",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(
        discarded.status(),
        StatusCode::OK,
        "this case's own premise: the draft discards cleanly"
    );

    let response = save_at(&harness, product_id, 2, &json!({ "name": "Fibre 900" })).await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        json_body(response).await["context"]["reason"],
        json!("ENTITY_TERMINAL"),
        "no head write is admitted on a discarded entity"
    );
    assert_eq!(
        head_of(&harness, product_id).await.name,
        "Fibre 500",
        "and the terminal row is unchanged"
    );
}

/// **A replayed save under the same key returns the stored answer and does
/// not save twice.**
///
/// The case the store exists for, on this door: a client whose save committed
/// and whose response was lost retries under the key it still holds and with
/// the `If-Match` it still holds — a precondition that is stale **by
/// construction**, since the act it never learned about moved the revision. A
/// door that judged the precondition before the claim would refuse this retry
/// `STALE_REVISION` and never reach the stored answer.
///
/// "Does not save twice" is asserted on the revision rather than on the
/// status: a door that re-ran the mutation and happened to answer `200` would
/// pass a status-only assertion while bumping the revision a second time.
#[tokio::test]
async fn a_replayed_save_serves_the_stored_answer_and_does_not_save_twice() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let body = json!({ "name": "Fibre 900" });
    let headers = [
        ("If-Match", if_match(1)),
        ("Idempotency-Key", "save-key-1".to_owned()),
    ];
    let sent: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();

    let first = patch_product(app_for(&harness, TENANT), TENANT, product_id, &body, &sent).await;
    assert_eq!(first.status(), StatusCode::OK);
    let original = json_body(first).await;

    let retry = patch_product(app_for(&harness, TENANT), TENANT, product_id, &body, &sent).await;
    assert_eq!(
        retry.status(),
        StatusCode::OK,
        "the retry replays the stored answer rather than being refused for its now-stale \
         precondition"
    );
    assert_eq!(
        json_body(retry).await,
        original,
        "byte for byte the first answer, not a re-render"
    );

    assert_eq!(
        head_of(&harness, product_id).await.internal_revision,
        2,
        "one save, one bump: the replay executed nothing"
    );
    assert_eq!(
        idempotency_rows_for(&harness.dsn, "save-key-1").await,
        1,
        "one claim, answered in the mutation's own transaction"
    );
}

/// **A save naming no field at all is refused `VALIDATION`.**
///
/// A bare `internal_revision` bump is a write with no content that still
/// invalidates every `ETag` a client holds, so it is refused at the door
/// rather than admitted as a no-op. `VALIDATION` because the request body is
/// what is wrong, and the caller can fix it.
#[tokio::test]
async fn a_save_naming_no_field_is_refused_validation() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = save_at(&harness, product_id, 1, &json!({})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        head_of(&harness, product_id).await.internal_revision,
        1,
        "no bump for a request that named nothing"
    );
}

/// **A save storing a scope column with an empty token is refused
/// `VALIDATION`, and a well-formed list beside it saves.**
///
/// The refusal is load-bearing beyond this row. `skus::recheck_parent_
/// containment` parses the **parent Product's** stored `region_scope` on
/// every child publish and answers a `500` where it does not parse, so a
/// Product save that stored `"eu,,us"` would turn one caller's `200` into an
/// operator alarm on a different entity's door. Both create doors refuse the
/// same shape --
/// [`a_create_naming_a_scope_with_an_empty_token_is_refused_validation`] is
/// the Product one's own case, and it is cited here rather than asserted in
/// passing because this comment claimed it before it was true: `create_sku`
/// parsed its scope inputs from the day it shipped and `create_product` did
/// not. This door is the other way a stored scope can change.
#[tokio::test]
async fn a_save_storing_a_scope_with_an_empty_token_is_refused_validation() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let refused = save_at(
        &harness,
        product_id,
        1,
        &json!({ "region_scope": "eu,,us" }),
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "an empty token between separators is refused rather than silently filtered"
    );
    let head = head_of(&harness, product_id).await;
    assert_eq!(head.region_scope, "eu", "nothing was written");
    assert_eq!(head.internal_revision, 1, "and no counter moved");

    let admitted = save_at(&harness, product_id, 1, &json!({ "region_scope": "eu,us" })).await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "the positive control: a well-formed list is an ordinary bucket-iii save"
    );
    assert_eq!(
        head_of(&harness, product_id).await.region_scope,
        "eu,us",
        "and it is stored as the caller spelled it"
    );
}

/// Build a router carrying **both** entity kinds' doors, composed the way
/// `gear.rs` composes them — `products::router` merged with `skus::router`
/// over one `ApiState`.
///
/// Every other case in this file uses [`app_for`], which mounts the Product
/// doors alone; the containment cases below cannot. The property they test
/// is a cross-entity one — a Product save judged against the SKUs living
/// under it — and its trigger is three ordinary requests across the two
/// doors. Seeding the child through the repository instead would prove the
/// check runs against a row this file wrote, not against a child a caller
/// could actually create, and the create door is exactly what decides which
/// scope a child stores.
fn both_doors_app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    let state = api_state(harness);
    let openapi = OpenApiRegistryImpl::new();
    router(Arc::clone(&state), &openapi)
        .merge(crate::api::rest::skus::router(state, &openapi))
        .layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// `POST /bss-products/v1/products` through the real door, answering the
/// minted id. The one request the containment cases start from.
async fn create_product_scoped(harness: &TestHarness, region_scope: &str) -> Uuid {
    let response = post_create_product(
        both_doors_app_for(harness, TENANT),
        TENANT,
        &json!({
            "brand_id": BRAND,
            "name": "Fibre 900",
            "region_scope": region_scope,
        }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the case's own premise: the parent it narrows was created"
    );
    let view = json_body(response).await;
    view["product_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the create view names the minted id")
}

/// `POST /bss-products/v1/skus` through the real door, answering the minted
/// id and the `ETag` a publish or discard sends back as `If-Match`.
async fn create_sku_scoped(
    harness: &TestHarness,
    parent_id: Uuid,
    sku_code: &str,
    region_scope: &str,
) -> (Uuid, String) {
    let response = both_doors_app_for(harness, TENANT)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bss-products/v1/skus")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .extension(authed_ctx(TENANT))
                .body(Body::from(
                    json!({
                        "product_id": parent_id,
                        "sku_code": sku_code,
                        "region_scope": region_scope,
                    })
                    .to_string(),
                ))
                .expect("build the SKU create request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the case's own premise: the child is contained at create and admitted"
    );
    let etag = response
        .headers()
        .get(axum::http::header::ETAG)
        .expect("a create answers an ETag")
        .to_str()
        .expect("the ETag is ASCII")
        .to_owned();
    let view = json_body(response).await;
    let sku_id = view["sku_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the create view names the minted id");
    (sku_id, etag)
}

/// `POST /bss-products/v1/skus/{id}/{act}` with `If-Match`, asserted
/// admitted — the setup step, never the property under test.
async fn sku_head_act(harness: &TestHarness, sku_id: Uuid, act: &str, if_match: &str) {
    let response = both_doors_app_for(harness, TENANT)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/bss-products/v1/skus/{sku_id}/{act}"))
                .header(axum::http::header::IF_MATCH, if_match)
                .extension(authed_ctx(TENANT))
                .body(Body::empty())
                .expect("build the SKU head-act request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the case's own premise: the child reached the state it acts from"
    );
}

/// [`save_at`] over the router carrying both entity kinds' doors.
async fn save_at_both_doors(
    harness: &TestHarness,
    product_id: Uuid,
    revision: i64,
    body: &serde_json::Value,
) -> axum::http::Response<Body> {
    patch_product(
        both_doors_app_for(harness, TENANT),
        TENANT,
        product_id,
        body,
        &[("If-Match", &if_match(revision))],
    )
    .await
}

/// The `error_code` of the one **refusal** row in the audit log.
///
/// Narrowed to `error_code IS NOT NULL` where [`audit_error_code`] is not:
/// the containment cases drive several admitted acts before the refusal they
/// assert, and each of those writes its own audit row.
async fn refusal_error_code(dsn: &str) -> Option<String> {
    raw_string_opt(
        dsn,
        "SELECT error_code AS v FROM products_audit_log WHERE error_code IS NOT NULL LIMIT 1",
    )
    .await
}

/// Narrow a Product's `region_scope` **out of band**, around every door.
///
/// Used by exactly one case, and only to manufacture a state no door will
/// produce once the containment check exists: a live child already outside
/// its parent. See that case's own doc for why the state has to be reached
/// this way.
async fn narrow_region_out_of_band(dsn: &str, product_id: Uuid, region: &str) {
    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection to narrow the parent");
    let result = conn
        .execute_unprepared(&format!(
            "UPDATE products_product SET region_scope = '{region}', \
             internal_revision = internal_revision + 1 WHERE {}",
            id_matches("product_id", product_id)
        ))
        .await
        .expect("the head-row guard admits a bucket-iii write on a non-terminal head");
    assert_eq!(
        result.rows_affected(),
        1,
        "the parent this case narrows must exist, or its premise never held"
    );
    conn.close().await.ok();
}

/// **A `PATCH` narrowing a Product's scope out from under a `published`
/// child is refused `SCOPE_NOT_CONTAINED`, and writes nothing.**
///
/// §4.1: `region_scope` and `brand_scope` are bucket iii *"in both
/// directions, widening and narrowing alike, so a narrowing that would
/// orphan a live child meets `fr-parent-child-integrity`'s fail-closed check
/// in the registered-validators phase, ahead of the governance gate"*.
///
/// **Three ordinary requests, no out-of-band write**, which is the whole
/// point: nothing in the sequence is a misuse. The create door admits the
/// child because it is contained *at create*; the head-row guard and
/// [`super::bucket::classify`] admit the narrowing because bucket iii is
/// mutable in both directions. Without this check the third request answers
/// `200` and leaves a `published` SKU scoped `us` under a parent scoped
/// `eu` — and the **child** pays: its next save or re-publish is refused
/// `SCOPE_NOT_CONTAINED` on a request that changed nothing about it.
///
/// **The code is the assertion, not the status.** `SCOPE_NOT_CONTAINED`,
/// `VALIDATION` and `ILLEGAL_FIELD_MUTATION` all render wire `400` here, so
/// a status assertion would pass against a door refusing for an unrelated
/// reason — and would have passed against the defect had the narrowing been
/// refused as, say, a malformed scope.
#[tokio::test]
async fn a_narrowing_that_would_orphan_a_published_child_is_refused_scope_not_contained() {
    let harness = harness().await;
    let product_id = create_product_scoped(&harness, "eu,us").await;
    let (sku_id, etag) = create_sku_scoped(&harness, product_id, "SKU-500", "us").await;
    sku_head_act(&harness, sku_id, "publish", &etag).await;

    let before = head_of(&harness, product_id).await;
    let refused = save_at_both_doors(
        &harness,
        product_id,
        before.internal_revision,
        &json!({ "region_scope": "eu" }),
    )
    .await;

    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "SCOPE_NOT_CONTAINED is one of the taxonomy's architectural 422s, rendered wire 400"
    );
    let view = json_body(refused).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("SCOPE_NOT_CONTAINED"),
        "the refusal names containment; the code, not the status, is what tells this refusal \
         from every other 400 this door can raise"
    );

    let after = head_of(&harness, product_id).await;
    assert_eq!(
        after.region_scope, "eu,us",
        "the parent's scope is unchanged: the refusal is raised before the head-row UPDATE, on \
         the same transaction, so nothing lands"
    );
    assert_eq!(
        after.internal_revision, before.internal_revision,
        "and no counter moved, which is what would have invalidated every ETag a client holds"
    );
    assert_eq!(
        refusal_error_code(&harness.dsn).await.as_deref(),
        Some("SCOPE_NOT_CONTAINED"),
        "the refusal is audited under the code it raised"
    );
}

/// **The positive control: the same narrowing is admitted when the child
/// stays contained.**
///
/// Identical to the case above in every respect but one — the child is
/// scoped `eu` rather than `us`, so the parent's move to `eu` leaves it
/// inside. Without this beside it, a door that refused *every* narrowing
/// would pass the refusal case, and §4.1 asks for bucket-iii mutability in
/// both directions, not for a scope column frozen the moment a child exists.
#[tokio::test]
async fn the_same_narrowing_is_admitted_when_the_published_child_stays_contained() {
    let harness = harness().await;
    let product_id = create_product_scoped(&harness, "eu,us").await;
    let (sku_id, etag) = create_sku_scoped(&harness, product_id, "SKU-500", "eu").await;
    sku_head_act(&harness, sku_id, "publish", &etag).await;

    let before = head_of(&harness, product_id).await;
    let admitted = save_at_both_doors(
        &harness,
        product_id,
        before.internal_revision,
        &json!({ "region_scope": "eu" }),
    )
    .await;

    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "a narrowing that orphans nothing is an ordinary bucket-iii save"
    );
    let after = head_of(&harness, product_id).await;
    assert_eq!(after.region_scope, "eu", "and it is stored");
    assert_eq!(
        after.internal_revision,
        before.internal_revision + 1,
        "moving the revision by exactly one, as any admitted save does"
    );
}

/// **A `discarded` child cannot be orphaned, so a narrowing past it is
/// admitted.**
///
/// The exclusion is deliberate and lives in
/// [`repo::find_non_terminal_skus_of_product`]'s own statement: a terminal
/// child is out of use — no door will write to it again, the head-write
/// guard sees to that — so refusing its parent's save on its account would
/// make a tidy retirement permanently load-bearing. The child here is
/// scoped `us` and would fail the check outright were it live, which is what
/// makes this a test of the exclusion rather than of nothing.
#[tokio::test]
async fn a_narrowing_past_a_discarded_child_alone_is_admitted() {
    let harness = harness().await;
    let product_id = create_product_scoped(&harness, "eu,us").await;
    let (sku_id, etag) = create_sku_scoped(&harness, product_id, "SKU-500", "us").await;
    sku_head_act(&harness, sku_id, "discard", &etag).await;

    let before = head_of(&harness, product_id).await;
    let admitted = save_at_both_doors(
        &harness,
        product_id,
        before.internal_revision,
        &json!({ "region_scope": "eu" }),
    )
    .await;

    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "the only child is discarded, so there is no live child to orphan"
    );
    assert_eq!(
        head_of(&harness, product_id).await.region_scope,
        "eu",
        "and the narrowing is stored"
    );
}

/// **A widening is admitted, with a live child under the parent.**
///
/// §4.1 makes the column bucket iii *"in both directions"*, and only one of
/// those directions can orphan anything. A guard that ran the check and
/// judged it the wrong way round — or that refused any scope save at all
/// while a child exists — passes every case above and fails this one.
#[tokio::test]
async fn a_widening_save_is_admitted_with_a_live_child_under_the_parent() {
    let harness = harness().await;
    let product_id = create_product_scoped(&harness, "eu").await;
    let (sku_id, etag) = create_sku_scoped(&harness, product_id, "SKU-500", "eu").await;
    sku_head_act(&harness, sku_id, "publish", &etag).await;

    let before = head_of(&harness, product_id).await;
    let admitted = save_at_both_doors(
        &harness,
        product_id,
        before.internal_revision,
        &json!({ "region_scope": "eu,us" }),
    )
    .await;

    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "widening cannot put a contained child outside its parent"
    );
    assert_eq!(
        head_of(&harness, product_id).await.region_scope,
        "eu,us",
        "and the widening is stored as the caller spelled it"
    );
}

/// **A save touching neither scope column runs no child scan, and succeeds
/// against a child that would fail one.**
///
/// This is what proves the guard is *conditional* rather than a blanket
/// refusal: the parent here is already narrower than its live child, so a
/// door that scanned the children on every save would refuse an unrelated
/// `name` change — a refusal the caller neither caused nor can act on.
///
/// The offending state is reached **out of band**, by
/// [`narrow_region_out_of_band`], and that is unavoidable rather than a
/// shortcut: with the check in place no door will produce it, which is the
/// whole of what the case above asserts. What can still produce it is a
/// narrowing committed before this check existed — rows a deployment will
/// carry — so the state is real, and a blanket scan would strand every one
/// of those Products against every future save of any column.
#[tokio::test]
async fn a_save_touching_no_scope_column_runs_no_child_scan() {
    let harness = harness().await;
    let product_id = create_product_scoped(&harness, "eu,us").await;
    let (sku_id, etag) = create_sku_scoped(&harness, product_id, "SKU-500", "us").await;
    sku_head_act(&harness, sku_id, "publish", &etag).await;
    narrow_region_out_of_band(&harness.dsn, product_id, "eu").await;

    let before = head_of(&harness, product_id).await;
    assert_eq!(
        before.region_scope, "eu",
        "the case's own premise: the live child is scoped `us` and is already outside its parent"
    );

    let admitted = save_at_both_doors(
        &harness,
        product_id,
        before.internal_revision,
        &json!({ "name": "Fibre 900 Plus" }),
    )
    .await;

    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "a save naming neither scope column cannot orphan anything, so no child is read"
    );
    let after = head_of(&harness, product_id).await;
    assert_eq!(
        after.name, "Fibre 900 Plus",
        "the routed column was written"
    );
    assert_eq!(
        after.region_scope, "eu",
        "and the scope this save never named is untouched"
    );
}

/// **A save stores `name` trimmed, exactly as the create door stores it.**
///
/// `create_product` computes `raw_name.trim().to_owned()` and stores that, so
/// a door that stored the caller's padding would make one operator-facing
/// column depend on which door wrote it — `Fibre` from a `POST`, `  Fibre  `
/// from a `PATCH`. Uniqueness would not notice: `name::normalize` trims and
/// collapses whatever it is handed, which is exactly why a status assertion
/// or a collision assertion cannot see this. The stored value, read back off
/// the head, is the only thing that can.
///
/// The divergence would also outlive the request: the read door serves the
/// stored value and the next publish freezes it verbatim into
/// `products_entity_version` content, so it would land in a `content_digest`.
#[tokio::test]
async fn a_save_stores_the_name_trimmed_as_the_create_door_does() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let response = save_at(&harness, product_id, 1, &json!({ "name": "  Fibre  " })).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a padded name is an ordinary bucket-iii save, not a refusal"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(
        head.name, "Fibre",
        "the stored value is trimmed, so it does not depend on which door wrote it"
    );
    assert_eq!(
        head.name_normalized,
        crate::domain::name::normalize("Fibre"),
        "and the index operand is derived from the value the row actually carries"
    );
}

/// **A create naming a scope column with an empty token is refused
/// `VALIDATION`, and a body wrong in two places reports both.**
///
/// `create_sku` has parsed its scope inputs since it shipped; this door did
/// not, and no `CHECK` stands in for it — `m20260829_000002` declares
/// `region_scope text NOT NULL DEFAULT ''` with no predicate. A `201` here
/// is what arms
/// [`a_poisoned_parent_scope_cannot_be_planted_through_the_create_door`]
/// below.
///
/// The second body is the collection assertion (P-D-37): a blank `name` and
/// an unparseable `brand_scope` in one request report as two violations, not
/// as whichever the door happened to judge first.
#[tokio::test]
async fn a_create_naming_a_scope_with_an_empty_token_is_refused_validation() {
    let harness = harness().await;

    let refused = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 900", "region_scope": "eu,,us" }),
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "an empty token between separators is refused rather than stored verbatim"
    );
    let view = json_body(refused).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("VALIDATION"),
        "the code is the discriminator, not the status"
    );
    assert_eq!(
        view["context"]["violations"][0]["subject"],
        json!("region_scope"),
        "and it names the column the caller can fix"
    );
    assert_eq!(
        raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_product").await,
        0,
        "the refused create left no row behind"
    );

    let both_wrong = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "   ", "brand_scope": ",acme" }),
    )
    .await;
    assert_eq!(both_wrong.status(), StatusCode::BAD_REQUEST);
    let view = json_body(both_wrong).await;
    assert_eq!(
        view["context"]["violations"].as_array().map(Vec::len),
        Some(2),
        "a body wrong in two places reports both violations, not the first (P-D-37)"
    );

    let admitted = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 900", "region_scope": "eu,us" }),
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::CREATED,
        "the positive control: a well-formed list is an ordinary create"
    );
}

/// `POST /bss-products/v1/products` over the both-doors router, answering the
/// minted id — [`create_product_scoped`] with the name as a parameter, which
/// the case below needs because it creates two parents in one tenant and
/// brand and `uq_products_product_name` admits only one of any name.
async fn create_named_product(harness: &TestHarness, name: &str, region_scope: &str) -> Uuid {
    let response = post_create_product(
        both_doors_app_for(harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": name, "region_scope": region_scope }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the case's own premise: the parent it acts on was created"
    );
    json_body(response).await["product_id"]
        .as_str()
        .and_then(|id| Uuid::parse_str(id).ok())
        .expect("the create view names the minted id")
}

/// **The two-request detonation is closed at the door that plants the
/// charge.**
///
/// The failure this closes needs no misuse at all. Before the fix:
///
/// 1. `POST /products {"region_scope": "eu,,us"}` answered `201` and stored
///    the value verbatim.
/// 2. `POST /skus {"product_id": P}` answered `500` — *"parent Product's
///    stored `region_scope` contains an empty token"* — because both
///    `create_sku` and `skus::recheck_parent_containment` parse the parent's
///    **stored** scope and treat a failure as gear-borne corruption.
///
/// So a caller planted a poison value with a success and detonated it on a
/// **different entity's** door, as a `5xx` plus a false operator alarm: the
/// provenance inversion `RepoError::CorruptRow`'s own doc rules out.
///
/// The case measures both halves. First it establishes that the detonation is
/// real, by writing the poison **out of band** — the only way to reach that
/// state now — and reading the child door's `500` back. Then it drives the
/// sequence as a caller would and asserts step 1 never yields the `201` the
/// charge needs, and that an ordinary parent still takes an ordinary child.
/// Without the first half the case would pass against a door whose `500` had
/// simply been renumbered.
#[tokio::test]
async fn a_poisoned_parent_scope_cannot_be_planted_through_the_create_door() {
    let harness = harness().await;

    // -- The detonation, established against a parent no door produced. --
    let armed = create_named_product(&harness, "Fibre 900", "eu,us").await;
    narrow_region_out_of_band(&harness.dsn, armed, "eu,,us").await;
    let detonated = both_doors_app_for(&harness, TENANT)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bss-products/v1/skus")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .extension(authed_ctx(TENANT))
                .body(Body::from(
                    json!({ "product_id": armed, "sku_code": "SKU-1" }).to_string(),
                ))
                .expect("build the SKU create request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(
        detonated.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "an unparseable *stored* parent scope is gear-borne corruption and answers 5xx: that is \
         the alarm the create door must not let a caller arm"
    );

    // -- The plant, refused. --
    let planted = post_create_product(
        both_doors_app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 950", "region_scope": "eu,,us" }),
    )
    .await;
    assert_ne!(
        planted.status(),
        StatusCode::CREATED,
        "step 1 of the sequence is refused, so no caller can reach step 2's 500"
    );

    // -- The positive control: an ordinary parent still takes an ordinary
    // child, so the refusal above is the scope token and not the door. --
    let parent = create_named_product(&harness, "Fibre 950", "eu,us").await;
    let admitted = both_doors_app_for(&harness, TENANT)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/bss-products/v1/skus")
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .extension(authed_ctx(TENANT))
                .body(Body::from(
                    json!({ "product_id": parent, "sku_code": "SKU-2" }).to_string(),
                ))
                .expect("build the SKU create request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(
        admitted.status(),
        StatusCode::CREATED,
        "the positive control: a parent whose stored scope parses takes a child"
    );
}

/// **A save that moves only `brand_id` and loses the name index's race names
/// the value that collided.**
///
/// `uq_products_product_name` keys on `(tenant_id, brand_id, name_normalized)`
/// and `brand_id` is bucket i — writable before first publish — so a brand
/// move can collide on the *name* index while the request names no name.
/// Reading `save.name` alone answers *"...already holds the name "* with an
/// empty value, naming nothing the caller could look for. The name the row
/// would carry is what collided, so the message names it.
///
/// The message is the assertion here rather than the code: the code was
/// already right, and a case asserting only `DUPLICATE_NAME` passes against
/// the defect.
#[tokio::test]
async fn a_brand_only_save_that_collides_on_the_name_index_names_the_name() {
    let harness = harness().await;
    let other_brand = Uuid::now_v7();

    let holder = post_create_product(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "brand_id": other_brand, "name": "Fibre 500" }),
    )
    .await;
    assert_eq!(
        holder.status(),
        StatusCode::CREATED,
        "this case's own premise: the name is held in the brand the mover moves to"
    );

    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;
    let mover = head_of(&harness, product_id).await;
    assert_eq!(
        mover.name, "Fibre 500",
        "this case's premise: the two rows carry the same name in different brands"
    );

    let refused = save_at(
        &harness,
        product_id,
        mover.internal_revision,
        &json!({ "brand_id": other_brand }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    let view = json_body(refused).await;
    assert_eq!(
        view["context"]["reason"],
        json!("DUPLICATE_NAME"),
        "a brand move that collides on the name index is that index's refusal"
    );
    assert!(
        view["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("Fibre 500")),
        "and the message names the value that collided, not an empty string: {:?}",
        view["detail"]
    );
}

/// **A save whose `name` is blank is refused `VALIDATION` even when it also
/// names a bucket-i column on a published head.**
///
/// §3.1's `inst-fd-fail-closed`: *"the phases run in the order above and the
/// run stops at the first failing phase"*, and `Phase::ordered()` puts
/// `Shape` ahead of `State`. Both operands the shape rules read — `name` and
/// `brand_id` — are present the moment the payload parses, so nothing in this
/// phase depends on the routing's output and the run has no reason to reach
/// the bucket rule at all.
///
/// **The code is the assertion, not the status**: `VALIDATION` renders `400`
/// and `ILLEGAL_FIELD_MUTATION` renders `409`, but the pair below is chosen
/// precisely so that a door running the phases in the wrong order still
/// refuses — with the wrong code, naming a rule the caller did not reach.
#[tokio::test]
async fn a_blank_name_beside_a_bucket_i_column_stops_at_the_shape_phase() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let published = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(1))],
    )
    .await;
    assert_eq!(
        published.status(),
        StatusCode::OK,
        "this case's own premise: the draft publishes, so bucket i is now closed"
    );

    let refused = save_at(
        &harness,
        product_id,
        head_of(&harness, product_id).await.internal_revision,
        &json!({ "name": "   ", "brand_id": Uuid::now_v7() }),
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "the shape phase fails first, and its refusal is the architectural 422 rendered 400"
    );
    let view = json_body(refused).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("VALIDATION"),
        "the run stops at the first failing phase, which is `shape` and not `state`"
    );

    let head = head_of(&harness, product_id).await;
    assert_eq!(head.name, "Fibre 500", "the refused save wrote nothing");
    assert_eq!(
        head.brand_id, BRAND,
        "and the bucket-i column it also named is untouched"
    );
}

/// **The two entry points refuse each other's tokens, and neither writes.**
///
/// `enqueue` accepting a `*Published` token emitted a body with no
/// `publishedVersion` — a shape §4.5 calls incomplete — and nothing noticed
/// until the SDK's typed events forced the distinction. The two guards that
/// close it were, until this case, guards whose removal reddened nothing:
/// exactly what review wave B landed the case below for, two commits earlier,
/// in this same file.
///
/// Both directions, because they are separate guards on separate functions.
#[tokio::test]
async fn the_two_enqueue_entry_points_refuse_each_others_tokens() {
    let harness = harness().await;
    let sink = crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox));
    let conn = harness
        .db
        .conn()
        .expect("checkout the pinned production connection");
    let product_id = Uuid::now_v7();
    let core = events::EventBodyCore {
        tenant_id: TENANT,
        entity_kind: events::EntityKind::Product.as_str(),
        entity_id: product_id,
        internal_revision: 1,
        lifecycle_state: "draft",
    };

    let refused = events::enqueue(
        &sink,
        &conn,
        product_id,
        events::PRODUCT_PUBLISHED_PAYLOAD_TYPE,
        &core,
        Uuid::now_v7(),
    )
    .await
    .expect_err("a publish token has a publishedVersion this entry point cannot carry");
    assert!(
        matches!(&refused, events::EventsError::PublishNeedsVersion(t)
                 if t == events::PRODUCT_PUBLISHED_PAYLOAD_TYPE),
        "the refusal must name the token and the reason, not a schema lookup: {refused}"
    );

    let refused = events::enqueue_published(
        &sink,
        &conn,
        product_id,
        events::PRODUCT_CREATED_PAYLOAD_TYPE,
        &core,
        7,
        Uuid::now_v7(),
    )
    .await
    .expect_err(
        "a core-only token must not be given a publishedVersion the design does not put on it",
    );
    assert!(
        matches!(&refused, events::EventsError::NotAPublishEvent(t)
                 if t == events::PRODUCT_CREATED_PAYLOAD_TYPE),
        "and the twin guard must name its own condition: {refused}"
    );

    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    assert_eq!(
        raw_i64(
            &harness.dsn,
            &format!("SELECT COUNT(*) AS v FROM {body_table}")
        )
        .await,
        0,
        "neither refusal may leave a row behind: both guards run before anything is written"
    );
}

/// **An event whose payload type carries no schema reference is refused, and
/// nothing reaches the outbox.**
///
/// `EventsError::UnregisteredSchema` was, until this case, a guard whose
/// removal reddened nothing: replace `enqueue_body`'s `ok_or_else` with
/// `.unwrap_or("")` and every other test in the crate stays green, while every
/// event the gear emits announces an empty schema reference — the one failure
/// `schema_ref_for`'s own doc says a schema reference exists to prevent.
///
/// Two halves, and the first is what makes the second mean anything: a count
/// of zero taken on a harness that cannot write is not evidence. A registered
/// event is enqueued first and read back, and only then is the unregistered
/// token attempted — same outbox, same runner, same core.
///
/// It lives in this file rather than beside `events.rs` because the second
/// assertion needs a real migrated outbox to count rows in, and this suite's
/// harness is the only one in the crate that has one.
#[tokio::test]
async fn an_unregistered_payload_type_is_refused_and_enqueues_nothing() {
    /// Close to a real token on purpose: a lookup written with `starts_with`
    /// would admit this one and the case would prove nothing.
    const UNREGISTERED: &str = "ProductCreatedV2";

    let harness = harness().await;
    // The interim sink, which is what this harness's pipeline runs: the guard
    // under test is `events`' own, on the path a no-broker deployment takes.
    let sink = crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox));
    // One checkout for both calls: the harness pins `max_conns: 1`, and the
    // row counts below read their own auxiliary connection into the same file
    // rather than this one.
    let conn = harness
        .db
        .conn()
        .expect("checkout the pinned production connection");
    let product_id = Uuid::now_v7();
    let core = events::EventBodyCore {
        tenant_id: TENANT,
        entity_kind: events::EntityKind::Product.as_str(),
        entity_id: product_id,
        internal_revision: 1,
        lifecycle_state: "draft",
    };

    events::enqueue(
        &sink,
        &conn,
        product_id,
        events::PRODUCT_CREATED_PAYLOAD_TYPE,
        &core,
        Uuid::now_v7(),
    )
    .await
    .expect("a registered payload type must enqueue");
    assert_eq!(
        enqueued_event_count(&harness.dsn, events::PRODUCT_CREATED_PAYLOAD_TYPE).await,
        1,
        "this case's own premise: the harness can write an outbox body row at all"
    );

    let refused = events::enqueue(
        &sink,
        &conn,
        product_id,
        UNREGISTERED,
        &core,
        Uuid::now_v7(),
    )
    .await
    .expect_err("a payload type outside the roster must never be enqueued");

    assert!(
        matches!(&refused, events::EventsError::UnregisteredSchema(token) if token == UNREGISTERED),
        "the refusal must be the schema guard naming its own token, not a serialization or \
         storage failure that would happen to be red here too: {refused}"
    );
    assert_eq!(
        enqueued_event_count(&harness.dsn, UNREGISTERED).await,
        0,
        "a refused event must leave no body row behind: the schema reference is resolved before \
         anything is written precisely so the act rolls back instead"
    );
    assert_eq!(
        enqueued_event_count(&harness.dsn, events::PRODUCT_CREATED_PAYLOAD_TYPE).await,
        1,
        "and the refusal must not have disturbed the row the control wrote"
    );
}

/// **A save enqueues exactly one `ProductHeadSaved` row, and its body carries
/// the revision the act committed and the state the head is actually in.**
///
/// `design/01-foundation.md`'s `inst-fd-save-txn` makes the outbox row a
/// clause of the save itself, *"in the same transaction"*, and §4.5 puts
/// `ProductHeadSaved` in the roster of eight Foundation events. Nothing else
/// in this suite reads the outbox after a save: with the enqueue deleted
/// every other save case stays green, because they all assert the row the
/// `UPDATE` wrote and none of them assert the announcement a consumer
/// subscribes to.
///
/// Three things are pinned here that a weaker case would miss:
///
/// - The **literal** `"ProductHeadSaved"`. The token is the string a consumer
///   subscribes on, so a rename is a broken subscription rather than a
///   refactor, and a test written against the constant would rename with it
///   in silence.
/// - `internalRevision` **after** the bump. The pre-act value is `1` and the
///   committed value is `2`, so a door announcing the record it read before
///   the write fails here; it is also compared against the head read back,
///   which is what P-D-29's *"the value as committed by the act"* means.
/// - `lifecycleState` read off the head rather than assumed. §4.5 calls it
///   *"the discriminator a consumer of `*HeadSaved` needs, since a save lands
///   on a `draft`, `published` or `deprecated` head alike"*, so the case
///   saves **twice**: once on a `draft` head and once on the same head after
///   it has published. A hard-coded `"draft"` passes the first and fails the
///   second, and the count moving `1 -> 2` is what says each save announces
///   once rather than the publish's row being miscounted as a save's.
#[tokio::test]
async fn a_save_enqueues_one_product_head_saved_carrying_the_committed_revision_and_state() {
    let harness = harness().await;
    let product_id = Uuid::now_v7();
    seed_draft(&harness, product_id).await;

    let first = save_at(&harness, product_id, 1, &json!({ "name": "Fibre 900" })).await;
    assert_eq!(
        first.status(),
        StatusCode::OK,
        "this case's own premise: the bucket-iii save on a draft is admitted"
    );

    assert_eq!(
        enqueued_event_count(&harness.dsn, "ProductHeadSaved").await,
        1,
        "one admitted save enqueues exactly one ProductHeadSaved row, no more and no fewer"
    );

    let head = head_of(&harness, product_id).await;
    let body = enqueued_event_body(&harness.dsn, "ProductHeadSaved").await;
    assert_eq!(body["entityKind"], json!("product"));
    assert_eq!(body["entityId"], json!(product_id.to_string()));
    assert_eq!(body["tenantId"], json!(TENANT.to_string()));
    assert_eq!(
        body["internalRevision"],
        json!(head.internal_revision),
        "the event announces the revision as committed by the act, read back off the head"
    );
    assert_eq!(
        body["internalRevision"],
        json!(2),
        "and that is the post-bump 2, not the 1 the head carried when the door began"
    );
    assert_eq!(
        body["lifecycleState"],
        json!("draft"),
        "a save takes no edge, so the state is the one the head was already in"
    );

    let published = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        product_id,
        "publish",
        &[("If-Match", &if_match(2))],
    )
    .await;
    assert_eq!(
        published.status(),
        StatusCode::OK,
        "this case's own premise: the draft publishes"
    );

    let second = save_at(&harness, product_id, 3, &json!({ "name": "Fibre 901" })).await;
    assert_eq!(
        second.status(),
        StatusCode::OK,
        "section 4.1 admits a bucket-iii save on a published head"
    );

    assert_eq!(
        enqueued_event_count(&harness.dsn, "ProductHeadSaved").await,
        2,
        "the second save announced once too, and the publish's own row is not one of these"
    );
    assert_eq!(
        enqueued_event_count(&harness.dsn, "ProductPublished").await,
        1,
        "the control on the filter: the publish's row is there under its own token"
    );

    let head = head_of(&harness, product_id).await;
    let body = enqueued_event_body(&harness.dsn, "ProductHeadSaved").await;
    assert_eq!(
        body["internalRevision"],
        json!(head.internal_revision),
        "the newest row is the second save's, and it announces its own committed revision"
    );
    assert_eq!(body["internalRevision"], json!(4));
    assert_eq!(
        body["lifecycleState"],
        json!("published"),
        "the discriminator is read off the head, so a save on a published head says so"
    );
}

/// **The envelope P-D-01 requires comes out of the door**, not merely out of
/// the struct that models it.
///
/// `events_tests` proves `EventEnvelope` renders its four fields, three of
/// them P-D-01 obligations; that is a statement about a type. This is the
/// statement about the *door*: a create
/// that built its body correctly and enqueued it unwrapped would leave that
/// unit test green and this one red.
///
/// **`actorRef` is checked against the identity map, not merely for
/// presence.** The pseudonymous ref is the one value here a door could
/// plausibly satisfy by minting a fresh `UUID` — it is a `UUID` either way,
/// and every "is it there" assertion would pass. So this reads the ref slice
/// 10's map actually minted for the acting principal and requires the
/// envelope to carry that one.
#[tokio::test]
async fn a_created_events_envelope_carries_the_four_obligations_from_the_door() {
    let harness = harness().await;
    let app = app_for(&harness, TENANT);

    let response = post_create_product(
        app,
        TENANT,
        &json!({ "brand_id": BRAND, "name": "Fibre 500" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let envelope = enqueued_event_envelope(&harness.dsn, "ProductCreated").await;

    assert_eq!(
        envelope["schemaRef"], "bss-products.ProductCreated.v1.0.0",
        "the envelope must announce the versioned schema its body is shaped by"
    );
    let event_id = envelope["eventId"]
        .as_str()
        .expect("the interim envelope must carry its own event id");
    assert!(
        Uuid::parse_str(event_id).is_ok(),
        "the event id must be a UUID, not a placeholder: {event_id}"
    );

    // `hex()`, not `CAST(.. AS TEXT)`: `SQLite` stores a `UUID` as a 16-byte
    // BLOB, and casting those bytes to text yields invalid UTF-8 rather than
    // the hyphenated form. Both sides are normalised to bare upper-case hex so
    // the comparison is about the value and not about either rendering.
    // Scoped, and the scope is *proved*: an unqualified `LIMIT 1` over this
    // table reads a row by position, so it would keep passing if the door
    // minted a second ref for someone else and the wrong one happened to sort
    // first. The acting principal's own id is not knowable here — `authed_ctx`
    // mints a fresh subject per call — so the tenant is the scope, and the
    // count assertion is what makes reading one row from it exact.
    assert_eq!(
        raw_i64(
            &harness.dsn,
            &format!(
                "SELECT COUNT(*) AS v FROM products_identity_ref WHERE {}",
                id_matches("tenant_id", TENANT)
            ),
        )
        .await,
        1,
        "this create must have minted exactly one identity ref for this tenant"
    );
    let minted = raw_string_opt(
        &harness.dsn,
        &format!(
            "SELECT hex(actor_ref) AS v FROM products_identity_ref WHERE {}",
            id_matches("tenant_id", TENANT)
        ),
    )
    .await
    .expect("the create resolved an actor ref through the identity map");
    let carried = envelope["actorRef"]
        .as_str()
        .expect("the envelope must carry the acting principal's ref")
        .replace('-', "")
        .to_uppercase();
    assert_eq!(
        carried, minted,
        "the envelope must carry the ref the identity map minted, not one of its own"
    );

    // No causing event: an operator request caused this one. Omitted rather
    // than echoing the correlation id, which is the distinction the pair
    // exists to draw.
    assert!(
        envelope.get("causationId").is_none(),
        "an operator-caused event must name no causing event"
    );
    // And no correlation id, honestly: this suite installs no `OTel`
    // subscriber, so there is no ambient trace to read. A door that minted
    // one per event would show up right here as a value that correlates
    // nothing.
    assert!(
        envelope.get("correlationId").is_none(),
        "an untraced request must leave the correlation id off the wire"
    );

    // The body still reads exactly as §4.5 writes it, one level in.
    assert_eq!(envelope["data"]["entityKind"], "product");
    assert_eq!(envelope["data"]["internalRevision"], 1);
    assert_eq!(envelope["data"]["lifecycleState"], "draft");
}

/// The clone door (`inst-cn-door`, P-D-62/75/76), driven through the real
/// router like every other door case in this file.
///
/// (`dod-clone-tests`' marker arrives when its blocking row resolves.)
mod clone_door_tests {
    use super::*;

    pub(super) async fn post_clone(
        app: Router,
        tenant: Uuid,
        product_id: Uuid,
        body: serde_json::Value,
        headers: &[(&str, &str)],
    ) -> axum::http::Response<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/bss-products/v1/products/{product_id}/clone"))
            .header("content-type", "application/json")
            .extension(authed_ctx(tenant));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        app.oneshot(
            request
                .body(Body::from(body.to_string()))
                .expect("build the clone request"),
        )
        .await
        .expect("the router answers")
    }

    pub(super) async fn view_of(response: axum::http::Response<Body>) -> serde_json::Value {
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("read the response body");
        serde_json::from_slice(&body).expect("the response body is JSON")
    }

    /// A draft source clones with the first-free suggestions: `-copy-1` on
    /// both the name and the code, a fresh id, `draft` at revision 1, and
    /// the lineage pair written in the creating statement with the
    /// head-read sentinel (`cloned_from_version` NULL — P-D-76).
    #[tokio::test]
    async fn a_draft_source_clones_with_first_free_suggestions() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        let source = seed_draft(&harness, source_id).await;

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED, "a clone is a 201");
        let view = view_of(response).await;
        assert_eq!(
            view["name"],
            serde_json::json!(format!("{}-copy-1", source.name)),
            "the suggested name is the first free of the -copy family"
        );
        assert_eq!(
            view["product_code"],
            serde_json::json!(format!(
                "{}-copy-1",
                source
                    .product_code
                    .as_deref()
                    .expect("the seed carries a code")
            )),
            "the suggested code walks the same family"
        );
        assert_eq!(view["lifecycle_state"], serde_json::json!("draft"));
        assert_eq!(view["internal_revision"], serde_json::json!(1));
        assert_eq!(view["published_version"], serde_json::json!(0));

        let clone_id = Uuid::parse_str(view["product_id"].as_str().expect("id"))
            .expect("the clone's id is a uuid");
        let clone_head = head_of(&harness, clone_id).await;
        assert_eq!(
            clone_head.cloned_from,
            Some(source_id),
            "lineage names the immediate source, written in the creating statement"
        );
        assert_eq!(
            clone_head.cloned_from_version, None,
            "a draft source is read at its head: the version sentinel is NULL (P-D-76)"
        );
    }

    /// The walk: with `-copy-1` taken, the next clone lands `-copy-2` — the
    /// index arbitrates and the loop moves only on the exact conflict its
    /// candidate owns (P-D-62).
    #[tokio::test]
    async fn a_second_clone_walks_to_copy_two() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        let source = seed_draft(&harness, source_id).await;

        let first = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[],
        )
        .await;
        assert_eq!(first.status(), StatusCode::CREATED);

        let second = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[],
        )
        .await;
        assert_eq!(
            second.status(),
            StatusCode::CREATED,
            "the walk finds the next free"
        );
        let view = view_of(second).await;
        assert_eq!(
            view["name"],
            serde_json::json!(format!("{}-copy-2", source.name)),
            "the second clone of one source suggests -copy-2, not a refusal"
        );
    }

    /// A published source is read from its **frozen version**, never the
    /// head's pending edits: after a publish and a head rename, the clone
    /// carries the frozen name and pins `cloned_from_version = 1`.
    #[tokio::test]
    async fn a_published_source_clones_from_its_frozen_version() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        let source = seed_draft(&harness, source_id).await;

        let publish = post_head_act(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            "publish",
            &[("If-Match", &if_match(1))],
        )
        .await;
        assert_eq!(
            publish.status(),
            StatusCode::OK,
            "premise: the source publishes"
        );

        // Rename the head after the freeze: the clone must not see this.
        let save = app_for(&harness, TENANT)
            .oneshot(
                Request::builder()
                    .method("PATCH")
                    .uri(format!("/bss-products/v1/products/{source_id}"))
                    .header("content-type", "application/json")
                    .header("If-Match", if_match(2))
                    .extension(authed_ctx(TENANT))
                    .body(Body::from(
                        serde_json::json!({"name": "Renamed After Freeze"}).to_string(),
                    ))
                    .expect("build the save request"),
            )
            .await
            .expect("the router answers");
        assert_eq!(
            save.status(),
            StatusCode::OK,
            "premise: the head rename lands"
        );

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let view = view_of(response).await;
        assert_eq!(
            view["name"],
            serde_json::json!(format!("{}-copy-1", source.name)),
            "the clone reads the frozen name, never the head's pending edit"
        );

        let clone_id = Uuid::parse_str(view["product_id"].as_str().expect("id"))
            .expect("the clone's id is a uuid");
        let clone_head = head_of(&harness, clone_id).await;
        assert_eq!(
            clone_head.cloned_from_version,
            Some(1),
            "the lineage pins exactly the frozen version the content was read at"
        );
    }

    /// A `discarded` source is refused `CLONE_SOURCE_DISCARDED` (P-D-75) —
    /// not `ENTITY_TERMINAL` (the clone writes nothing to the source) and
    /// not a 404 (the row is addressable).
    #[tokio::test]
    async fn a_discarded_source_is_refused_with_the_minted_code() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        seed_draft(&harness, source_id).await;
        let discard = post_head_act(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            "discard",
            &[("If-Match", &if_match(1))],
        )
        .await;
        assert_eq!(
            discard.status(),
            StatusCode::OK,
            "premise: the draft discards"
        );

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[],
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "the source's state refuses the act"
        );
        let view = view_of(response).await;
        assert_eq!(
            view["context"]["reason"],
            serde_json::json!("CLONE_SOURCE_DISCARDED"),
            "the refusal is the minted code, not ENTITY_TERMINAL and not a 404"
        );
    }

    /// An operator-supplied name that collides is the ordinary
    /// `DUPLICATE_NAME` — only the *suggested* name walks (P-D-62).
    #[tokio::test]
    async fn an_overridden_name_collision_is_the_ordinary_refusal() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        let source = seed_draft(&harness, source_id).await;

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({ "name": source.name }),
            &[],
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "an overridden collision does not walk"
        );
        let view = view_of(response).await;
        assert_eq!(
            view["context"]["reason"],
            serde_json::json!("DUPLICATE_NAME")
        );
    }

    /// A keyed retry replays the first clone rather than making a second
    /// (P-D-75: what a crash-retrying caller needs to not double-clone).
    #[tokio::test]
    async fn a_keyed_retry_replays_the_first_clone() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        seed_draft(&harness, source_id).await;

        let first = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[("Idempotency-Key", "clone-retry-1")],
        )
        .await;
        assert_eq!(first.status(), StatusCode::CREATED);
        let first_view = view_of(first).await;

        let retry = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[("Idempotency-Key", "clone-retry-1")],
        )
        .await;
        assert_eq!(
            retry.status(),
            StatusCode::CREATED,
            "the replay reproduces the stored status"
        );
        let retry_view = view_of(retry).await;
        assert_eq!(
            retry_view["product_id"], first_view["product_id"],
            "one key, one clone: the retry is the first answer, not a second act"
        );
    }
}

// ---------------------------------------------------------------------------
// The family act (`inst-cn-children`, P-D-72, P-D-79)

mod family_clone_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use authz_resolver_sdk::constraints::{Constraint, InPredicate, Predicate};
    use authz_resolver_sdk::models::{
        DenyReason, EvaluationRequest, EvaluationResponse, EvaluationResponseContext,
    };
    use authz_resolver_sdk::{AuthZResolverClient, AuthZResolverError, PolicyEnforcer};
    use toolkit_security::pep_properties;

    use crate::infra::storage::repo::NewSku;

    use super::clone_door_tests::{post_clone, view_of};

    use super::*;

    /// Seed one SKU under `parent_id` through the repository, contained in
    /// the parent's `eu` region scope.
    async fn seed_child(harness: &TestHarness, parent_id: Uuid, sku_code: &str) -> Uuid {
        let conn = harness
            .db
            .conn()
            .expect("checkout the pinned production connection");
        let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
        let sku_id = Uuid::now_v7();
        repo::insert_sku(
            &conn,
            &scope,
            NewSku {
                sku_id,
                tenant_id: TENANT,
                product_id: parent_id,
                sku_code: sku_code.to_owned(),
                region_scope: "eu".to_owned(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: Utc.with_ymd_and_hms(2026, 8, 29, 9, 30, 0).unwrap(),
                cloned_from: None,
                cloned_from_version: None,
            },
        )
        .await
        .expect("seed the child SKU");
        sku_id
    }

    /// The SKU rows of one Product, keyed by their `cloned_from`.
    async fn children_of(harness: &TestHarness, product_id: Uuid) -> Vec<repo::SkuRecord> {
        let conn = harness
            .db
            .conn()
            .expect("checkout the pinned production connection");
        let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
        repo::find_skus_of_product(&conn, &scope, TENANT, product_id)
            .await
            .expect("read the new parent's children")
    }

    /// A product clone is the family act (P-D-79): every non-discarded
    /// child clones under the new parent — one per state read rule — the
    /// receipt carries one `created` entry per child, and each clone's
    /// lineage names its own source SKU, never the parent act (P-D-72).
    #[tokio::test]
    async fn a_family_clone_lands_with_a_per_child_receipt() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        seed_draft(&harness, source_id).await;
        let child_a = seed_child(&harness, source_id, "FAM-A").await;
        let child_b = seed_child(&harness, source_id, "FAM-B").await;

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let view = view_of(response).await;
        let receipt = view["children"]
            .as_array()
            .expect("the family act answers a per-child receipt");
        assert_eq!(receipt.len(), 2, "one entry per attempted child");
        for entry in receipt {
            assert_eq!(
                entry["disposition"],
                serde_json::json!("created"),
                "both children land"
            );
        }

        let new_parent = Uuid::parse_str(view["product_id"].as_str().expect("id"))
            .expect("the clone's id is a uuid");
        let clones = children_of(&harness, new_parent).await;
        assert_eq!(clones.len(), 2, "both clones hang off the new parent");
        let mut sources: Vec<Option<Uuid>> = clones.iter().map(|row| row.cloned_from).collect();
        sources.sort();
        let mut expected = vec![Some(child_a), Some(child_b)];
        expected.sort();
        assert_eq!(
            sources, expected,
            "each child's lineage names its own source SKU (P-D-72)"
        );
        for row in &clones {
            assert_eq!(
                row.product_id, new_parent,
                "the parent link is remapped, never copied"
            );
        }
    }

    /// A childless source degenerates to a family of zero: the receipt is
    /// present and empty (P-D-79).
    #[tokio::test]
    async fn a_childless_source_answers_an_empty_receipt() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        seed_draft(&harness, source_id).await;

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let view = view_of(response).await;
        assert_eq!(
            view["children"],
            serde_json::json!([]),
            "the receipt field is the act's shape, not a children-only extra"
        );
    }

    /// A resolver that answers `true` once and denies every later
    /// evaluation: the door's first gate (product x write) passes, the
    /// second (sku x write) is refused.
    struct SecondCallDenied {
        calls: AtomicUsize,
    }

    #[async_trait]
    impl AuthZResolverClient for SecondCallDenied {
        async fn evaluate(
            &self,
            _req: EvaluationRequest,
        ) -> Result<EvaluationResponse, AuthZResolverError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(EvaluationResponse {
                decision: call == 0,
                context: EvaluationResponseContext {
                    constraints: vec![Constraint {
                        predicates: vec![Predicate::In(InPredicate::new(
                            pep_properties::OWNER_TENANT_ID,
                            vec![TENANT],
                        ))],
                    }],
                    deny_reason: (call > 0).then(|| DenyReason {
                        error_code: "no-sku-grant".to_owned(),
                        details: None,
                    }),
                },
            })
        }
    }

    /// The product clone door spends BOTH grants unconditionally (P-D-79):
    /// a caller holding `product x write` but not `sku x write` is refused
    /// before anything is read or written — even for a childless source.
    #[tokio::test]
    async fn the_family_act_spends_both_grants() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        seed_draft(&harness, source_id).await;

        let state = api_state(&harness);
        let openapi = OpenApiRegistryImpl::new();
        let enforcer = PolicyEnforcer::new(Arc::new(SecondCallDenied {
            calls: AtomicUsize::new(0),
        }));
        let app = router(state, &openapi).layer(axum::Extension(enforcer));

        let response = post_clone(app, TENANT, source_id, serde_json::json!({}), &[]).await;
        assert_eq!(
            response.status(),
            StatusCode::FORBIDDEN,
            "the second gate is spent at the door, ahead of the child count"
        );
    }

    /// The crash window (P-D-72, P-D-79): a committed-but-unanswered claim
    /// whose `entity_ref` names the parent resumes the children phase on
    /// the same-key retry — already-cloned sources are skipped and
    /// receipted `created` with their existing ids, the remainder is
    /// cloned, and the answer is stored at completion.
    #[tokio::test]
    async fn a_committed_unanswered_claim_resumes_the_family() {
        let harness = harness().await;
        let source_id = Uuid::now_v7();
        seed_draft(&harness, source_id).await;
        let child_a = seed_child(&harness, source_id, "RES-A").await;
        let child_b = seed_child(&harness, source_id, "RES-B").await;

        // The crashed first attempt, reconstructed exactly as its parent
        // transaction committed it: the claim, the parent with its lineage,
        // the entity_ref stamp — plus one of the two children, cloned
        // before the crash.
        let conn = harness
            .db
            .conn()
            .expect("checkout the pinned production connection");
        let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
        let endpoint = format!("/bss-products/v1/products/{source_id}/clone");
        let digest = crate::domain::idempotency::payload_digest(&serde_json::json!({}));
        let now = Utc::now();
        let claimed = repo::claim_idempotency_key(
            &conn,
            &scope,
            TENANT,
            &endpoint,
            "resume-key",
            &digest,
            now,
            now + chrono::Duration::hours(24),
        )
        .await
        .expect("seed the crashed claim");
        assert_eq!(claimed, repo::IdempotencyClaim::Claimed, "premise");

        let parent_id = Uuid::now_v7();
        let mut parent = new_product(parent_id, TENANT);
        parent.name = "Fibre 500-copy-1".to_owned();
        parent.name_normalized = "fibre 500-copy-1".to_owned();
        parent.product_code = Some("FIBRE-500-copy-1".to_owned());
        parent.cloned_from = Some(source_id);
        repo::insert_product(&conn, &scope, parent)
            .await
            .expect("seed the crashed act's parent");
        repo::stamp_idempotency_entity_ref(
            &conn,
            &scope,
            TENANT,
            &endpoint,
            "resume-key",
            parent_id,
        )
        .await
        .expect("stamp the parent handle as the crashed transaction did");

        let conn2 = harness
            .db
            .conn()
            .expect("checkout the pinned production connection");
        let already_cloned = Uuid::now_v7();
        repo::insert_sku(
            &conn2,
            &scope,
            NewSku {
                sku_id: already_cloned,
                tenant_id: TENANT,
                product_id: parent_id,
                sku_code: "RES-A-copy-1".to_owned(),
                region_scope: "eu".to_owned(),
                brand_scope: String::new(),
                created_by: "principal:author-1".to_owned(),
                created_at: now,
                cloned_from: Some(child_a),
                cloned_from_version: None,
            },
        )
        .await
        .expect("seed the child the crashed attempt already cloned");

        // The same-key retry re-enters.
        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[("Idempotency-Key", "resume-key")],
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::CREATED,
            "the resume answers the act, never IDEMPOTENCY_KEY_IN_FLIGHT"
        );
        let view = view_of(response).await;
        assert_eq!(
            view["product_id"],
            serde_json::json!(parent_id),
            "the resume finished the crashed act's own parent, not a second one"
        );
        let receipt = view["children"].as_array().expect("the receipt");
        assert_eq!(receipt.len(), 2, "the receipt covers the whole family");
        let entry_a = receipt
            .iter()
            .find(|entry| entry["source_sku_id"] == serde_json::json!(child_a))
            .expect("child A is receipted");
        assert_eq!(
            entry_a["new_sku_id"],
            serde_json::json!(already_cloned),
            "the already-cloned source is skipped and reported with its existing id"
        );
        let entry_b = receipt
            .iter()
            .find(|entry| entry["source_sku_id"] == serde_json::json!(child_b))
            .expect("child B is receipted");
        assert_eq!(entry_b["disposition"], serde_json::json!("created"));

        // The claim is answered at completion: the same key now replays.
        let replay = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            serde_json::json!({}),
            &[("Idempotency-Key", "resume-key")],
        )
        .await;
        assert_eq!(replay.status(), StatusCode::CREATED);
        let replay_view = view_of(replay).await;
        assert_eq!(
            replay_view, view,
            "the stored receipt is the answer every later retry gets"
        );
    }
}
