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

use async_trait::async_trait;
use authz_resolver_sdk::constraints::{Constraint, InPredicate, Predicate};
use authz_resolver_sdk::models::{
    EvaluationRequest, EvaluationResponse, EvaluationResponseContext,
};
use authz_resolver_sdk::{AuthZResolverClient, AuthZResolverError, PolicyEnforcer};
use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{TimeZone, Utc};
use sea_orm::{ConnectionTrait, Database, DbBackend, FromQueryResult, Statement};
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use toolkit_gts::gts_id;
use toolkit_security::{SecurityContext, pep_properties};
use tower::ServiceExt as _;
use uuid::Uuid;

use super::router;
use crate::api::rest::ApiState;
use crate::api::rest::preconditions;
use crate::config::ProductsConfig;
use crate::domain::concurrency::InternalRevision;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{self, NewProduct};

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
/// own running pipeline with (see `api/rest.rs`'s own doc on the wiring gap
/// this leaves there) — with [`events::QUEUE_NAME`] registered and the
/// **same** [`events::PendingBrokerProducer`] the running gear declares.
///
/// The handler is not an incidental choice. An earlier version of this
/// harness registered a stub answering `Success`, whose doc claimed
/// "nothing in this file drains the queue" — untrue, because `.start()`
/// runs the processor, and a `Success` marks the message delivered and
/// hands it to the vacuum. The enqueue assertion below then read zero rows
/// for a create that had enqueued one. Registering the production handler
/// both fixes the count and makes these tests exercise the configuration
/// the gear actually boots with rather than a friendlier one.
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

/// Run `sql` (a `SELECT ... AS v FROM ...`) on its own auxiliary connection
/// into `dsn` and return the single integer column it names `v`. See this
/// module's doc for why this goes around `DBRunner` rather than through it.
async fn raw_i64(dsn: &str, sql: &str) -> i64 {
    #[derive(Debug, FromQueryResult)]
    struct Row {
        v: i64,
    }

    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection for test introspection");
    let row = Row::find_by_statement(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .one(&conn)
        .await
        .expect("the introspection query runs")
        .expect("an aggregate SELECT always returns exactly one row");
    conn.close().await.ok();
    row.v
}

/// [`raw_i64`]'s twin for a single, possibly-`NULL`, text column named `v`.
async fn raw_string_opt(dsn: &str, sql: &str) -> Option<String> {
    #[derive(Debug, FromQueryResult)]
    struct Row {
        v: Option<String>,
    }

    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection for test introspection");
    let row = Row::find_by_statement(Statement::from_string(DbBackend::Sqlite, sql.to_owned()))
        .one(&conn)
        .await
        .expect("the introspection query runs")
        .expect("the row this test just wrote must exist");
    conn.close().await.ok();
    row.v
}

/// Drop `table` via its own auxiliary connection — the `AUDIT_UNAVAILABLE`
/// seam's only lever: making the refusal audit row's own insert fail without
/// touching anything the mutation transaction itself reads or writes. See
/// this module's doc, "The create-door harness", for why this goes around
/// `DBRunner` rather than through it, and
/// `an_unwritable_refusal_audit_answers_audit_unavailable_not_the_domain_refusal`
/// for the one place it is used.
async fn drop_table(dsn: &str, table: &str) {
    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection to drop a table");
    conn.execute_unprepared(&format!("DROP TABLE {table};"))
        .await
        .expect("drop the table this seam needs gone");
    conn.close().await.ok();
}

/// Degraded flat-`In` PDP fake, `crate::authz::authz_tests::FlatInResolver`'s
/// twin: permits and emits a single flat `In([allowed])` constraint over
/// `OWNER_TENANT_ID`, ignoring the request. Duplicated rather than imported —
/// `authz_tests` is a private `#[cfg(test)]` sibling module, not a reusable
/// test-support crate.
struct FlatInResolver {
    allowed: Uuid,
}

#[async_trait]
impl AuthZResolverClient for FlatInResolver {
    async fn evaluate(
        &self,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, AuthZResolverError> {
        Ok(EvaluationResponse {
            decision: true,
            context: EvaluationResponseContext {
                constraints: vec![Constraint {
                    predicates: vec![Predicate::In(InPredicate::new(
                        pep_properties::OWNER_TENANT_ID,
                        vec![self.allowed],
                    ))],
                }],
                deny_reason: None,
            },
        })
    }
}

fn flat_in_enforcer(allowed: Uuid) -> PolicyEnforcer {
    PolicyEnforcer::new(Arc::new(FlatInResolver { allowed }))
}

fn authed_ctx(tenant: Uuid) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(Uuid::now_v7())
        .subject_tenant_id(tenant)
        .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
        .token_scopes(vec!["*".to_owned()])
        .build()
        .expect("authed SecurityContext must build")
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
    }
}

/// Build the router under test, layered with a `flat_in_enforcer` allowed
/// for `tenant` — the shape every test below shares.
fn app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    let state = Arc::new(ApiState {
        db: harness.db.clone(),
        outbox: Arc::clone(&harness.outbox),
        // The value `gear.rs` resolves from the operator's file; the tests
        // configure nothing, so the typed default is what an unconfigured
        // boot would carry here.
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
    });
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

/// How many `products_idempotency` rows exist for `client_key`, in whatever
/// state — the count every case below asserts the claim, or its absence, on.
async fn idempotency_rows_for(dsn: &str, client_key: &str) -> i64 {
    raw_i64(
        dsn,
        &format!(
            "SELECT COUNT(*) AS v FROM products_idempotency WHERE client_key = '{client_key}'"
        ),
    )
    .await
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
