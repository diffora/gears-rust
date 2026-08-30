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
use crate::domain::concurrency::InternalRevision;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{self, NewProduct};

const TENANT: Uuid = Uuid::from_u128(0xd0_01);
const BRAND: Uuid = Uuid::from_u128(0xd0_02);

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
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// `POST /bss-products/v1/products` with `body`, authenticated as `tenant`.
async fn post_create_product(
    app: Router,
    tenant: Uuid,
    body: &serde_json::Value,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/bss-products/v1/products")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(tenant))
            .body(Body::from(body.to_string()))
            .expect("build the create request"),
    )
    .await
    .expect("the router answers")
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
