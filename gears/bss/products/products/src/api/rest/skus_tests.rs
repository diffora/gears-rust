//! Tests for the SKU create door (`POST /bss-products/v1/skus`).
//!
//! The read door (`GET /bss-products/v1/skus/{id}`) is proven by this file's
//! sibling responsibility split — `products_tests.rs` proves the read-door
//! shape once, generically, since `get_product` and `get_sku` are
//! structural twins; nothing here re-proves the `ETag` round-trip or the
//! miss/hit split, only the create door and the containment rule this
//! module's own [`crate::api::rest::skus`] doc says the Product door does
//! not have.
//!
//! # Harness shape, and why it repeats `products_tests`'s own
//!
//! [`harness`] and its helpers (`raw_i64`, `raw_string_opt`, `drop_table`,
//! `walk_parent_to`) are this file's own copies of
//! `products_tests`'s identically named functions, for the reason
//! `crate::api::rest::skus`'s own module doc gives for
//! [`super::insert_sku_with_event`]: `products_tests.rs` is outside this
//! slice's `target_paths`, so nothing in it can be imported.
//! `walk_parent_to` is this file's one addition beyond
//! `products_tests`'s own set — the parent-terminal tests need to move a
//! seeded parent Product to `retired`/`discarded` after insertion, and
//! `infra::storage::repo` has no lifecycle-transition writer yet (that is
//! the publish/discard doors', not this slice's), so the only lever
//! available here is a raw `UPDATE` through the same auxiliary connection
//! `drop_table` already uses.
//!
//! Every create-door case seeds its parent Product directly through
//! [`repo::insert_product`] rather than through the Product door's own HTTP
//! surface — this file mounts only [`super::router`] (the SKU router), and
//! reaching across to `products::router` for a setup step would couple this
//! suite to a door it is not testing.

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
use axum::response::IntoResponse as _;
use chrono::{TimeZone, Utc};
use sea_orm::sea_query::Expr;
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
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{self, NewProduct};

const TENANT: Uuid = Uuid::from_u128(0xd1_01);
const OTHER_TENANT: Uuid = Uuid::from_u128(0xd1_02);
const BRAND: Uuid = Uuid::from_u128(0xd1_03);

/// Everything a test in this file needs. [`crate::api::rest::products_tests
/// ::TestHarness`]'s own doc explains the file-backed `SQLite` mirror, the
/// production [`events::PendingBrokerProducer`] handler and the `?mode=rwc`
/// DSN this type repeats verbatim.
struct TestHarness {
    dsn: String,
    db: DBProvider<DbError>,
    outbox: Arc<Outbox>,
    #[allow(dead_code)]
    _outbox_handle: OutboxHandle,
}

impl Drop for TestHarness {
    fn drop(&mut self) {
        if let Some(rest) = self.dsn.strip_prefix("sqlite://") {
            let path = rest.split('?').next().unwrap_or(rest);
            std::fs::remove_file(path).ok();
        }
    }
}

fn unique_sqlite_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "bss-products-sku-tests-{label}-{}.sqlite3",
        Uuid::new_v4()
    ))
}

async fn harness() -> TestHarness {
    let path = unique_sqlite_path("db");
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

/// [`raw_i64`]'s definition — see `products_tests`'s own doc for why this
/// goes around `DBRunner` rather than through it.
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

/// Drop `table` via its own auxiliary connection — `products_tests
/// ::drop_table`'s own copy, for `an_unwritable_refusal_audit_answers_audit_unavailable_not_the_domain_refusal`'s
/// own use below, F-4's mirror of that door's own test.
async fn drop_table(dsn: &str, table: &str) {
    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection to drop a table");
    conn.execute_unprepared(&format!("DROP TABLE {table};"))
        .await
        .expect("drop the table this seam needs gone");
    conn.close().await.ok();
}

/// Walk a seeded parent to `retired` or `discarded` along **admitted edges**,
/// bumping `internal_revision` on every step.
///
/// It cannot simply write the target state. Phase 5's head-row guard admits
/// only the edges `draft -> published`, `draft -> discarded`,
/// `published -> deprecated`, `deprecated -> published` and
/// `deprecated -> retired`, and it requires `internal_revision` to move by
/// exactly one on **every** admitted update without exception. An earlier
/// version of this helper wrote `draft -> retired` in one statement with no
/// revision bump; the guard refused it on both counts, correctly, and these
/// two tests went red against a door that was fine. Walking the real path is
/// also a positive control: it proves the guard admits the transitions the
/// lifecycle actually uses, not merely that it refuses everything.
async fn walk_parent_to(
    provider: &DBProvider<DbError>,
    scope: &toolkit_db::secure::AccessScope,
    product_id: Uuid,
    target: &str,
) {
    let path: &[&str] = match target {
        "discarded" => &["discarded"],
        "retired" => &["published", "deprecated", "retired"],
        other => panic!("no admitted path to `{other}` from `draft`"),
    };
    for step in path {
        step_parent_state(provider, scope, product_id, step).await;
    }
}

/// Freeze the version a publish step is about to move the head to.
///
/// The head-row guard admits a `published_version` bump **only where the
/// matching `products_entity_version` row already exists**, so a publish that
/// skips this is refused by that clause before it reaches the edge this
/// helper means to walk. The frozen row is minimal: no publish act exists yet
/// to compute a real canonical rendering, and no assertion in this file reads
/// one back.
async fn freeze_parent_version(
    provider: &DBProvider<DbError>,
    scope: &toolkit_db::secure::AccessScope,
    product_id: Uuid,
    version: i64,
) {
    use crate::infra::storage::entity::entity_version;
    use chrono::Utc;
    use sea_orm::ActiveValue::Set;
    use sea_orm::EntityTrait;
    use toolkit_db::secure::SecureInsertExt as _;

    let conn = provider.conn().expect("scoped connection");
    let model = entity_version::ActiveModel {
        tenant_id: Set(TENANT),
        entity_kind: Set("product".to_owned()),
        entity_id: Set(product_id),
        published_version: Set(version),
        content: Set("{}".to_owned()),
        content_digest: Set(vec![0_u8; 32]),
        digest_version: Set(1),
        approval_ref: Set(None),
        actor_ref: Set(Uuid::from_u128(0xd1_09)),
        published_at: Set(Utc::now()),
    };
    entity_version::Entity::insert(model.clone())
        .secure()
        .scope_with_model(scope, &model)
        .expect("scope insert")
        .exec_with_returning(&conn)
        .await
        .expect("freeze the version the publish step moves to");
}

/// One admitted edge, with the revision bump the guard requires and, on the
/// publish step, the `published_version` bump that goes with it — and the
/// frozen version row that bump now requires.
async fn step_parent_state(
    provider: &DBProvider<DbError>,
    scope: &toolkit_db::secure::AccessScope,
    product_id: Uuid,
    state: &str,
) {
    use crate::infra::storage::entity::product;
    use sea_orm::sea_query::ExprTrait as _;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use toolkit_db::secure::SecureUpdateExt as _;

    let conn = provider.conn().expect("scoped connection");
    let mut update = product::Entity::update_many()
        .col_expr(product::Column::LifecycleState, Expr::value(state))
        .col_expr(
            product::Column::InternalRevision,
            Expr::col(product::Column::InternalRevision).add(1_i64),
        );
    if state == "published" {
        let current = {
            use toolkit_db::secure::SecureEntityExt as _;
            product::Entity::find()
                .secure()
                .scope_with(scope)
                .and_id(product_id)
                .expect("resource-scoped find")
                .one(&conn)
                .await
                .expect("read the parent head")
                .expect("the parent head exists")
        };
        freeze_parent_version(
            provider,
            scope,
            product_id,
            current.published_version.saturating_add(1),
        )
        .await;
        update = update.col_expr(
            product::Column::PublishedVersion,
            Expr::col(product::Column::PublishedVersion).add(1_i64),
        );
    }
    let result = update
        .filter(product::Column::ProductId.eq(product_id))
        .secure()
        .scope_with(scope)
        .exec(&conn)
        .await
        .unwrap_or_else(|e| panic!("move the parent to `{state}`: {e}"));
    assert!(
        result.rows_affected > 0,
        "the parent was never moved to `{state}`, so this test's premise never held"
    );
}

/// Degraded flat-`In` PDP fake — `products_tests::FlatInResolver`'s own
/// twin, duplicated for the same reason.
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

/// A parent Product, unrestricted on both scope dimensions by default —
/// every test that needs a *restricted* parent overrides `region_scope`
/// and/or `brand_scope` explicitly, so a reader never has to guess which
/// scope a given parent carries.
fn new_parent_product(product_id: Uuid, tenant_id: Uuid) -> NewProduct {
    NewProduct {
        product_id,
        tenant_id,
        brand_id: BRAND,
        name: "Fibre Line".to_owned(),
        name_normalized: "fibre line".to_owned(),
        product_code: None,
        region_scope: String::new(),
        brand_scope: String::new(),
        created_by: "principal:author-1".to_owned(),
        created_at: Utc.with_ymd_and_hms(2026, 8, 29, 9, 0, 0).unwrap(),
    }
}

/// Build the router under test, layered with a `flat_in_enforcer` allowed
/// for `tenant`.
fn app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    // The state a boot would carry: these tests configure nothing, so
    // `ProductsConfig`'s typed default is what `gear.rs` would resolve.
    let state = Arc::new(api_state(harness));
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// `POST /bss-products/v1/skus` with `body`, authenticated as `tenant`.
async fn post_create_sku(
    app: Router,
    tenant: Uuid,
    body: &serde_json::Value,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/bss-products/v1/skus")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(tenant))
            .body(Body::from(body.to_string()))
            .expect("build the create request"),
    )
    .await
    .expect("the router answers")
}

/// [`post_create_sku`] carrying `key` as its `Idempotency-Key` — the one
/// dial this file's idempotency cases turn, kept beside the keyless helper
/// rather than replacing it so every pre-idempotency case above keeps
/// exercising the keyless **skip**.
async fn post_create_sku_with_key(
    app: Router,
    tenant: Uuid,
    body: &serde_json::Value,
    key: &str,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/bss-products/v1/skus")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .header("Idempotency-Key", key)
            .extension(authed_ctx(tenant))
            .body(Body::from(body.to_string()))
            .expect("build the create request"),
    )
    .await
    .expect("the router answers")
}

/// How many `products_idempotency` rows exist for `client_key`, in whatever
/// state.
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
///
/// The Product door's twin (`products_tests::digest_of`) is the same three
/// lines against that door's own `DTO`.
fn digest_of(body: &serde_json::Value) -> Vec<u8> {
    let request: super::CreateSkuRequest =
        serde_json::from_value(body.clone()).expect("the case's own body parses as the create DTO");
    super::payload_digest(&request)
}

/// Seed a **live, unanswered** claim under this door's own endpoint,
/// recorded against `payload_hash`: the in-flight state, and the only way to
/// reach it now that a committed create answers its own claim.
///
/// `payload_hash` is a parameter rather than a fixed literal because the
/// digest decides which refusal the seeded state produces: a matching
/// duplicate is refused `IDEMPOTENCY_KEY_IN_FLIGHT` and a differing one
/// `IDEMPOTENCY_CONFLICT`, since a payload mismatch "stays
/// `IDEMPOTENCY_CONFLICT` in either state"
/// (`design/01-foundation.md` §3.2 `inst-fd-idem-claim-inflight`). The
/// Product door's twin (`products_tests::seed_live_claim`) carries the same
/// parameter for the same reason.
///
/// The connection is checked back in when this function returns, because the
/// door's own transaction needs the file-backed pool's single connection.
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
        "/bss-products/v1/skus",
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

/// Insert a parent Product directly through the repository (not through the
/// Product door's own HTTP surface — this file's module doc explains why)
/// and return its id.
async fn seed_parent(harness: &TestHarness, new: NewProduct) -> Uuid {
    let conn = harness
        .db
        .conn()
        .expect("checkout the pinned production connection");
    let scope = toolkit_db::secure::AccessScope::for_tenant(new.tenant_id);
    let product_id = new.product_id;
    repo::insert_product(&conn, &scope, new)
        .await
        .expect("seed the parent Product");
    product_id
}

/// Read a response body as generic JSON. Every DTO on this surface derives
/// `Serialize`/`Deserialize` on at most one side
/// (`products_tests`'s own comment on `ProductView`), so a generic read is
/// this file's own idiom too.
async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read the response body");
    serde_json::from_slice(&bytes).expect("the response body is JSON")
}

/// A well-formed create under a live, unrestricted parent persists a `draft`
/// SKU with `published_version = 0` and `internal_revision = 1` —
/// `dod-create-doors`' own baseline for this door, and the case every other
/// test below is a variation on.
#[tokio::test]
async fn a_well_formed_create_under_a_live_parent_persists_a_draft_sku() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let app = app_for(&harness, TENANT);

    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "a well-formed create under a live parent must be admitted"
    );
    let view = body_json(response).await;
    assert_eq!(view["product_id"], json!(parent_id));
    assert_eq!(view["sku_code"], json!("SKU-500"));
    assert_eq!(
        view["lifecycle_state"],
        json!("draft"),
        "a freshly created head is always draft"
    );
    assert_eq!(
        view["published_version"],
        json!(0),
        "a draft has never been published"
    );
    assert_eq!(
        view["internal_revision"],
        json!(1),
        "the first admitted write starts the revision counter at 1"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(
        persisted, 1,
        "the row this door reports created must actually be the one row now in storage"
    );
}

/// Exactly one `SkuCreated` row is enqueued, on the create door's own
/// mutation transaction, and a successful create writes no audit row — the
/// SKU door's own version of `products_tests
/// ::exactly_one_outbox_row_is_enqueued_and_no_content_row_is_written`.
#[tokio::test]
async fn exactly_one_sku_created_row_is_enqueued_and_no_audit_row_is_written() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let app = app_for(&harness, TENANT);

    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let enqueued = raw_i64(
        &harness.dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table}"),
    )
    .await;
    assert_eq!(
        enqueued, 1,
        "exactly one SkuCreated row must be enqueued for one create"
    );
    let payload_type = raw_string_opt(
        &harness.dsn,
        &format!("SELECT payload_type AS v FROM {body_table}"),
    )
    .await;
    assert_eq!(
        payload_type.as_deref(),
        Some(events::SKU_CREATED_PAYLOAD_TYPE),
        "the enqueued row must carry the SkuCreated payload type, not some other event's"
    );

    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(
        audit_rows, 0,
        "a successful create writes no audit row: its event is its record (P-D-21)"
    );
}

/// A `product_id` that resolves to no row at all in the caller's tenant is
/// refused `VALIDATION` (`dod-containment`'s first clause), and no SKU row
/// is left behind.
#[tokio::test]
async fn an_unresolvable_parent_is_refused_validation() {
    let harness = harness().await;
    let app = app_for(&harness, TENANT);
    let nonexistent_parent = Uuid::now_v7();

    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": nonexistent_parent, "sku_code": "SKU-500" }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "VALIDATION renders as an architectural 422, wire 400 (no transport override)"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("VALIDATION"),
        "the refusal code must be VALIDATION, not one of the other two containment refusals"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(
        persisted, 0,
        "an unresolvable parent must not admit the SKU"
    );
}

/// A parent belonging to another tenant reads exactly like a parent that
/// does not exist at all: `VALIDATION`, never a distinguishable answer that
/// would confirm the row's existence to a caller who cannot see it — the
/// cross-tenant boundary [`repo::find_product`]'s own doc names for the
/// read door, reached here from the write side.
#[tokio::test]
async fn a_parent_belonging_to_another_tenant_is_not_resolvable() {
    let harness = harness().await;
    let foreign_parent =
        seed_parent(&harness, new_parent_product(Uuid::now_v7(), OTHER_TENANT)).await;
    let app = app_for(&harness, TENANT);

    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": foreign_parent, "sku_code": "SKU-500" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let view = body_json(response).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("VALIDATION"),
        "a parent that exists but lies outside the caller's tenant must answer exactly like an \
         absent one"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(
        persisted, 0,
        "a cross-tenant parent must not let the SKU through"
    );
}

/// A `retired` parent is refused `PARENT_TERMINAL` (`dod-containment`'s
/// second clause), distinct from the `VALIDATION` an unresolvable parent
/// gets and from the `SCOPE_NOT_CONTAINED` a scope mismatch gets — three
/// different codes for three different failures.
#[tokio::test]
async fn a_retired_parent_is_refused_parent_terminal() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    walk_parent_to(
        &harness.db,
        &toolkit_db::secure::AccessScope::for_tenant(TENANT),
        parent_id,
        "retired",
    )
    .await;
    let app = app_for(&harness, TENANT);

    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "PARENT_TERMINAL is a 409, distinct from VALIDATION's 400"
    );
    let view = body_json(response).await;
    assert_eq!(view["context"]["reason"], json!("PARENT_TERMINAL"));

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(persisted, 0, "a retired parent must not admit the SKU");
}

/// A `discarded` parent is refused `PARENT_TERMINAL` exactly like a
/// `retired` one — `LifecycleState::is_terminal`'s own two-member roster,
/// both reached from this one door.
#[tokio::test]
async fn a_discarded_parent_is_refused_parent_terminal() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    walk_parent_to(
        &harness.db,
        &toolkit_db::secure::AccessScope::for_tenant(TENANT),
        parent_id,
        "discarded",
    )
    .await;
    let app = app_for(&harness, TENANT);

    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let view = body_json(response).await;
    assert_eq!(view["context"]["reason"], json!("PARENT_TERMINAL"));

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(persisted, 0, "a discarded parent must not admit the SKU");
}

/// A scope the payload names explicitly, that is not a subset of the
/// parent's own restricted set, is refused `SCOPE_NOT_CONTAINED`
/// (`dod-containment`'s third clause, ordinary-subset case).
#[tokio::test]
async fn a_scope_not_contained_in_a_restricted_parent_is_refused() {
    let harness = harness().await;
    let mut parent = new_parent_product(Uuid::now_v7(), TENANT);
    parent.region_scope = "eu".to_owned();
    let parent_id = seed_parent(&harness, parent).await;
    let app = app_for(&harness, TENANT);

    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500", "region_scope": "eu,us" }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "SCOPE_NOT_CONTAINED renders as an architectural 422, wire 400"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("SCOPE_NOT_CONTAINED"),
        "a scope claiming a value the parent does not carry must be refused this code, not \
         VALIDATION or PARENT_TERMINAL"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(persisted, 0, "an uncontained scope must not admit the SKU");
}

/// A payload that omits `region_scope` inherits the parent's own resolved
/// value, and the persisted row carries **that** value, not an empty one —
/// half of the pair that proves an omitted scope and an explicitly
/// unrestricted one are not conflated (`crate::domain::containment`'s own
/// doc; the other half is
/// `an_explicit_unrestricted_scope_against_a_restricted_parent_is_refused`,
/// below).
#[tokio::test]
async fn an_omitted_scope_inherits_the_parents_value() {
    let harness = harness().await;
    let mut parent = new_parent_product(Uuid::now_v7(), TENANT);
    parent.region_scope = "eu".to_owned();
    let parent_id = seed_parent(&harness, parent).await;
    let app = app_for(&harness, TENANT);

    // No "region_scope" key at all — this is the case a bare `String` field
    // could not have told apart from an explicit empty one.
    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "an omitted scope must inherit the parent's, never be refused as unrestricted-against-\
         restricted"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["region_scope"],
        json!("eu"),
        "the persisted row must carry the parent's own resolved region_scope, not an empty one"
    );
}

/// A payload that explicitly claims an unrestricted `region_scope` (`""`)
/// against a restricted parent is refused `SCOPE_NOT_CONTAINED` — clause 2
/// of `crate::domain::containment`'s rule: an unrestricted child is
/// contained only by an unrestricted parent. Paired with the previous test:
/// the same parent, the same door, and the only difference is whether the
/// payload sent the key at all — proof the two payload shapes reach
/// different outcomes rather than being read as the same "empty" value.
#[tokio::test]
async fn an_explicit_unrestricted_scope_against_a_restricted_parent_is_refused() {
    let harness = harness().await;
    let mut parent = new_parent_product(Uuid::now_v7(), TENANT);
    parent.region_scope = "eu".to_owned();
    let parent_id = seed_parent(&harness, parent).await;
    let app = app_for(&harness, TENANT);

    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500", "region_scope": "" }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an explicit unrestricted claim against a restricted parent must be refused, not \
         admitted as if the key had been omitted"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("SCOPE_NOT_CONTAINED")
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(
        persisted, 0,
        "the refused create must not have left a row behind"
    );
}

/// A second create colliding on `sku_code` within the same tenant is
/// refused `DUPLICATE_CODE`, an audit row records it, and the entity row is
/// not persisted (`dod-code-reservation`).
#[tokio::test]
async fn a_duplicate_sku_code_is_refused_and_audited() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;

    let first = post_create_sku(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::CREATED,
        "the first create must succeed"
    );

    let second = post_create_sku(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;
    assert_eq!(
        second.status(),
        StatusCode::CONFLICT,
        "a sku_code collision within the same tenant is a 409"
    );
    let view = body_json(second).await;
    assert_eq!(view["context"]["reason"], json!("DUPLICATE_CODE"));

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
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

/// F-4: the SKU door's own `AUDIT_UNAVAILABLE` seam — `products_tests
/// ::an_unwritable_refusal_audit_answers_audit_unavailable_not_the_domain_refusal`'s
/// mirror. This door's copy of the audit-then-report discipline is
/// independent code (its own `refuse_sku_insert_conflict`, wired through the
/// shared `crate::api::rest::audit_refusal_and_report`), so this door's own
/// suite needs its own proof that an unwritable refusal audit row answers
/// `AUDIT_UNAVAILABLE`, not the `DUPLICATE_CODE` the mutation actually
/// reached.
#[tokio::test]
async fn an_unwritable_refusal_audit_answers_audit_unavailable_not_the_domain_refusal() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;

    let first = post_create_sku(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::CREATED,
        "the setup create must succeed"
    );

    drop_table(&harness.dsn, "products_audit_log").await;

    let second = post_create_sku(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;

    assert_eq!(
        second.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "an unwritable refusal audit must answer 503 AUDIT_UNAVAILABLE, not the domain \
         refusal (DUPLICATE_CODE) the mutation actually reached"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(
        persisted, 1,
        "the losing create's row still must not have been persisted"
    );
}

/// F-6: a caller-supplied `id` is refused `VALIDATION` naming `id`, the SKU
/// door's own mirror of `products_tests::a_caller_supplied_id_is_refused_validation`.
#[tokio::test]
async fn a_caller_supplied_id_is_refused_validation() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let app = app_for(&harness, TENANT);
    let caller_supplied_id = Uuid::now_v7();

    let response = post_create_sku(
        app,
        TENANT,
        &json!({
            "id": caller_supplied_id,
            "product_id": parent_id,
            "sku_code": "SKU-500",
        }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "VALIDATION renders as an architectural 422, wire 400 (no transport override)"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("VALIDATION")
    );
    assert_eq!(
        view["context"]["violations"][0]["subject"],
        json!("id"),
        "the refusal must name the offending field"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(
        persisted, 0,
        "a refused create must not have left a row behind"
    );
}

/// F-5: a `region_scope` containing an empty token (a bare `","`) is refused
/// `VALIDATION`, not silently filtered down to an unrestricted scope, even
/// under a parent that would have admitted an actually-unrestricted claim.
#[tokio::test]
async fn a_scope_with_an_empty_token_is_refused_validation() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let app = app_for(&harness, TENANT);

    let response = post_create_sku(
        app,
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500", "region_scope": "," }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an empty scope token must be refused, not silently admitted"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("VALIDATION"),
        "a malformed scope token is refused VALIDATION, not persisted as a malformed value"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(
        persisted, 0,
        "a refused create must not have left a row behind"
    );
}

/// A keyed create persists the SKU **and** an `answered` row under **this
/// door's own** concrete endpoint.
///
/// The Product door's twin of this case
/// (`products_tests::a_create_with_an_idempotency_key_persists_the_entity_and_an_answered_row`)
/// carries the full reasoning. What is this door's own is the `endpoint`
/// value asserted below: two creates under one client key, one of a Product
/// and one of a SKU, are different acts, and the key component that keeps
/// them apart is exactly this one (P-D-42).
#[tokio::test]
async fn a_keyed_create_persists_the_sku_and_an_answered_row_under_this_doors_endpoint() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;

    let response = post_create_sku_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
        "author-retry-1",
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(persisted, 1, "the entity row is written");
    assert_eq!(
        idempotency_rows_for(&harness.dsn, "author-retry-1").await,
        1,
        "the key is claimed exactly once"
    );

    let endpoint = raw_string_opt(
        &harness.dsn,
        "SELECT endpoint AS v FROM products_idempotency WHERE client_key = 'author-retry-1'",
    )
    .await;
    assert_eq!(
        endpoint.as_deref(),
        Some("/bss-products/v1/skus"),
        "this door claims under its own concrete resource path, not the Product door's and not \
         a route template"
    );
    let state = raw_string_opt(
        &harness.dsn,
        "SELECT state AS v FROM products_idempotency WHERE client_key = 'author-retry-1'",
    )
    .await;
    assert_eq!(
        state.as_deref(),
        Some("answered"),
        "the committed create answered its own claim in the transaction that took it"
    );
}

/// A create **without** the header succeeds and claims nothing: the phase is
/// skipped, not failed (P-D-34).
///
/// Stated at this door too rather than left to the Product door's own case,
/// because the skip is a per-door wiring decision — a door that read the
/// header into a mandatory field would fail here and nowhere else.
#[tokio::test]
async fn a_keyless_sku_create_succeeds_and_claims_nothing() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;

    let response = post_create_sku(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CREATED);

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

/// **A rolled-back SKU mutation frees the key** — the load-bearing property,
/// asserted at this door as well as at the Product door.
///
/// The claim and the entity insert share one transaction (P-D-42), so a
/// `sku_code` collision rolls the claim back with the mutation and the key
/// is immediately reusable. Wiring the claim onto a runner of its own would
/// leave it committed and refuse the client's honest retry
/// `IDEMPOTENCY_KEY_IN_FLIGHT` for the whole retention window — and this
/// door has its own copy of the wiring
/// ([`super::insert_sku_with_event`]), so the Product door's own case cannot
/// prove it here.
///
/// Both halves are asserted: no claim survives the refusal, and a later
/// create on the same key succeeds.
#[tokio::test]
async fn a_rolled_back_sku_mutation_frees_the_key_for_a_later_create() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;

    let setup = post_create_sku(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;
    assert_eq!(setup.status(), StatusCode::CREATED);

    let refused = post_create_sku_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
        "author-retry-2",
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::CONFLICT,
        "the colliding create is refused DUPLICATE_CODE"
    );
    assert_eq!(
        idempotency_rows_for(&harness.dsn, "author-retry-2").await,
        0,
        "a refused mutation stores nothing, claim included (P-D-38, P-D-42): the key is free"
    );

    let retry = post_create_sku_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-900" }),
        "author-retry-2",
    )
    .await;
    assert_eq!(
        retry.status(),
        StatusCode::CREATED,
        "the same key claims again after the earlier mutation rolled back"
    );
    assert_eq!(
        idempotency_rows_for(&harness.dsn, "author-retry-2").await,
        1,
        "the retry's own claim is the only row the key ever committed"
    );
    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(
        persisted, 2,
        "the setup SKU and the retry's, and nothing from the refusal"
    );
}

/// A second keyed create while the first claim is live is refused
/// `IDEMPOTENCY_KEY_IN_FLIGHT` and audited, writing no second SKU.
///
/// The duplicate deliberately sends a `sku_code` no row holds, so the `409`
/// cannot be this door's `DUPLICATE_CODE`: the audited `error_code` is what
/// tells the two apart, and it also proves the refusal took the shared
/// `audit_sku_refusal` path rather than one of its own.
///
/// **The live claim is seeded, not produced by a first create.** A create
/// that commits answers its own claim in the same transaction, so no
/// committed act leaves a `claimed` row behind; `claimed` is the state of an
/// act still in flight, which here is a claim taken on another connection.
/// The Product door's twin
/// (`products_tests::a_second_create_on_a_live_key_is_refused_in_flight_and_audited`)
/// carries the same note.
///
/// **The seeded claim is recorded against this very body's digest.** The
/// in-flight refusal is reserved for "a duplicate whose payload hash matches
/// the claimed key's" (§3.2 `inst-fd-idem-claim-inflight`); a mismatch is
/// `IDEMPOTENCY_CONFLICT` in either state, and is the case below.
#[tokio::test]
async fn a_second_keyed_sku_create_on_a_live_key_is_refused_in_flight_and_audited() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let body = json!({ "product_id": parent_id, "sku_code": "SKU-900" });
    seed_live_claim(&harness, "author-retry-3", &digest_of(&body)).await;

    let second =
        post_create_sku_with_key(app_for(&harness, TENANT), TENANT, &body, "author-retry-3").await;
    assert_eq!(second.status(), StatusCode::CONFLICT);

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(persisted, 0, "the refused duplicate wrote no SKU");
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("IDEMPOTENCY_KEY_IN_FLIGHT"),
        "the audited code names the idempotency refusal, not the 409 this door also raises for a \
         duplicate code"
    );
}

/// A second keyed create while the first claim is live, **carrying a
/// different payload**, is refused `IDEMPOTENCY_CONFLICT` rather than in
/// flight.
///
/// The SKU end of the property the Product door's twin
/// (`products_tests::a_second_create_on_a_live_key_under_a_different_payload_is_refused_conflict`)
/// states in full: a payload mismatch "stays `IDEMPOTENCY_CONFLICT` in
/// either state" (§3.2 `inst-fd-idem-claim-inflight`), so a live claim is
/// compared against just as a stored answer is. Both doors call the same
/// `crate::api::rest::claim_idempotency`, and this case is what proves the
/// SKU door reaches it with its own digest rather than only the Product door
/// doing so.
///
/// The `sku_code` differs from the held claim's, so the `409` is not this
/// door's `DUPLICATE_CODE` either — the audited `error_code` is the
/// assertion.
#[tokio::test]
async fn a_second_keyed_sku_create_on_a_live_key_under_a_different_payload_is_refused_conflict() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let held = json!({ "product_id": parent_id, "sku_code": "SKU-900" });
    seed_live_claim(&harness, "author-retry-3b", &digest_of(&held)).await;

    let second = post_create_sku_with_key(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-901" }),
        "author-retry-3b",
    )
    .await;
    assert_eq!(second.status(), StatusCode::CONFLICT);

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(persisted, 0, "the refused duplicate wrote no SKU");
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("IDEMPOTENCY_CONFLICT"),
        "a mismatching payload under a live claim is a conflict, not an in-flight refusal"
    );
}

/// A retry after a committed SKU create replays the original response and
/// executes nothing — this door's own end of `dod-idempotency-store`.
///
/// The Product door's twin
/// (`products_tests::a_retry_after_a_committed_create_replays_the_original_response`)
/// carries the full reasoning for why this case, and not the claim cases
/// above, is what the store exists for. It is stated at this door too
/// because the answer write is per-door wiring inside
/// `insert_sku_with_event`: a door that took the claim and never answered it
/// would pass every other idempotency case in this file and fail only here,
/// refusing the client's honest retry `IDEMPOTENCY_KEY_IN_FLIGHT`.
///
/// Both "executes nothing" halves are asserted on storage: no second SKU row
/// and no second `SkuCreated` outbox row.
#[tokio::test]
async fn a_retry_after_a_committed_sku_create_replays_the_original_response() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let body = json!({ "product_id": parent_id, "sku_code": "SKU-500" });

    let first =
        post_create_sku_with_key(app_for(&harness, TENANT), TENANT, &body, "author-retry-4").await;
    assert_eq!(first.status(), StatusCode::CREATED);
    let original = body_json(first).await;

    let state = raw_string_opt(
        &harness.dsn,
        "SELECT state AS v FROM products_idempotency WHERE client_key = 'author-retry-4'",
    )
    .await;
    assert_eq!(
        state.as_deref(),
        Some("answered"),
        "the committed create answered its own claim inside the transaction that took it"
    );

    let retry =
        post_create_sku_with_key(app_for(&harness, TENANT), TENANT, &body, "author-retry-4").await;
    assert_eq!(
        retry.status(),
        StatusCode::CREATED,
        "the retry replays the original status rather than being refused in flight"
    );
    assert_eq!(
        body_json(retry).await,
        original,
        "the replay reproduces the original body, not a second SKU's"
    );

    let persisted = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await;
    assert_eq!(persisted, 1, "the retry wrote no second SKU row");
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let enqueued = raw_i64(
        &harness.dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table}"),
    )
    .await;
    assert_eq!(enqueued, 1, "the retry enqueued no second SkuCreated row");
    let audit_rows = raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(audit_rows, 0, "a replay is neither an act nor a refusal");
}

// ---------------------------------------------------------------------------
// The publish and discard doors
// ---------------------------------------------------------------------------

/// A governance host that refuses everything.
///
/// The production default, `NoMaterialityPolicyGate`, **never** refuses under
/// `GateMode::Gate` — its own doc says so, and says why: no materiality
/// policy is registered, so the act needs no ceremony. That makes the
/// `APPROVAL_REQUIRED` path unreachable through the router, and a path with
/// no test is one nothing pins. This double is why `super::publish_sku_gated`
/// takes the host as an argument at all; the *mode* is still a literal
/// inside it, so nothing here reaches `GateMode::PreAuthorized` either.
struct RefusingGate;

impl crate::domain::governance::GovernanceGate for RefusingGate {
    fn evaluate(
        &self,
        _subject: crate::domain::governance::EntityRef,
        _expected_revision: crate::domain::concurrency::InternalRevision,
        _mode: crate::domain::governance::GateMode,
    ) -> Result<crate::domain::governance::GateVerdict, crate::domain::error::DomainError> {
        Ok(crate::domain::governance::GateVerdict::Refused {
            reason: "this double refuses every act, so the door's APPROVAL_REQUIRED arm runs"
                .to_owned(),
        })
    }
}

/// The `ApiState` `app_for` layers, on its own so a case that calls a door's
/// inner function directly (rather than through the router) builds the same
/// state a mounted router would.
fn api_state(harness: &TestHarness) -> ApiState {
    ApiState {
        db: harness.db.clone(),
        outbox: Arc::clone(&harness.outbox),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
    }
}

/// Create a `draft` SKU through this module's own create door and return its
/// id and the `ETag` a publish or a discard sends back as `If-Match`.
///
/// Through the door rather than through `repo::insert_sku` on purpose: the
/// `ETag` these cases pin is the one a real client would have read off the
/// create response, so the round trip the precondition contract promises is
/// exercised rather than assumed.
async fn seed_draft_sku(harness: &TestHarness, parent_id: Uuid, sku_code: &str) -> (Uuid, String) {
    let response = post_create_sku(
        app_for(harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": sku_code }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "this case's own premise: the draft it publishes was created"
    );
    let etag = response
        .headers()
        .get(axum::http::header::ETAG)
        .expect("a create answers an ETag a publish can send back")
        .to_str()
        .expect("the ETag is ASCII")
        .to_owned();
    let view = body_json(response).await;
    let sku_id = Uuid::parse_str(view["sku_id"].as_str().expect("sku_id is a string"))
        .expect("sku_id is a uuid");
    (sku_id, etag)
}

/// `POST /bss-products/v1/skus/{id}/publish`, with `if_match` when supplied.
///
/// `if_match` is an `Option` rather than a `&str` because the **absent**
/// header is itself a case (`VALIDATION`), and a helper that always sent one
/// could not reach it.
async fn post_publish(
    app: Router,
    tenant: Uuid,
    sku_id: Uuid,
    if_match: Option<&str>,
) -> axum::http::Response<Body> {
    post_head_act(app, tenant, sku_id, "publish", if_match, None).await
}

/// [`post_publish`]'s discard twin.
async fn post_discard(
    app: Router,
    tenant: Uuid,
    sku_id: Uuid,
    if_match: Option<&str>,
) -> axum::http::Response<Body> {
    post_head_act(app, tenant, sku_id, "discard", if_match, None).await
}

/// One head act, over the same builder both doors' helpers use.
async fn post_head_act(
    app: Router,
    tenant: Uuid,
    sku_id: Uuid,
    act: &str,
    if_match: Option<&str>,
    key: Option<&str>,
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("/bss-products/v1/skus/{sku_id}/{act}"))
        .extension(authed_ctx(tenant));
    if let Some(tag) = if_match {
        builder = builder.header(axum::http::header::IF_MATCH, tag);
    }
    if let Some(client_key) = key {
        builder = builder.header("Idempotency-Key", client_key);
    }
    app.oneshot(
        builder
            .body(Body::empty())
            .expect("build the head-act request"),
    )
    .await
    .expect("the router answers")
}

/// A `WHERE` fragment matching `column` against `id`, whichever way the
/// driver stored it.
///
/// The `SQLite` mirror declares these columns `text`, but `sqlx` binds a
/// `Uuid` as a **blob**, and `SQLite`'s type affinity leaves a blob a blob.
/// So `WHERE sku_id = '<uuid>'` matches nothing at all — silently, as a
/// zero-row result rather than an error — which is exactly the trap the
/// `PostgreSQL` reading of this schema sets for a reader of these tests.
/// Both spellings are compared so the helper is right on either engine and
/// on either binding.
fn id_matches(column: &str, id: Uuid) -> String {
    let hex = id.simple().to_string().to_uppercase();
    format!("({column} = '{id}' OR hex({column}) = '{hex}')")
}

/// How many `products_entity_version` rows this entity has frozen.
async fn frozen_versions_for(dsn: &str, sku_id: Uuid) -> i64 {
    raw_i64(
        dsn,
        &format!(
            "SELECT COUNT(*) AS v FROM products_entity_version \
             WHERE {} AND entity_kind = 'sku'",
            id_matches("entity_id", sku_id)
        ),
    )
    .await
}

/// One SKU head's `internal_revision`.
async fn head_revision(dsn: &str, sku_id: Uuid) -> i64 {
    raw_i64(
        dsn,
        &format!(
            "SELECT internal_revision AS v FROM products_sku WHERE {}",
            id_matches("sku_id", sku_id)
        ),
    )
    .await
}

/// One SKU head's `published_version`.
async fn head_version(dsn: &str, sku_id: Uuid) -> i64 {
    raw_i64(
        dsn,
        &format!(
            "SELECT published_version AS v FROM products_sku WHERE {}",
            id_matches("sku_id", sku_id)
        ),
    )
    .await
}

/// One SKU head's `lifecycle_state`.
async fn head_state(dsn: &str, sku_id: Uuid) -> Option<String> {
    raw_string_opt(
        dsn,
        &format!(
            "SELECT lifecycle_state AS v FROM products_sku WHERE {}",
            id_matches("sku_id", sku_id)
        ),
    )
    .await
}

/// The lower-case hex of `bytes`, for comparing a `BLOB` column read back as
/// `hex(...)` against a digest recomputed in this process.
///
/// `SQLite`'s own `hex()` answers upper case, so the comparison below
/// lower-cases both sides rather than assuming either.
fn hex_of(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(DIGITS[usize::from(byte >> 4)]));
        out.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    out
}

/// A first publish of a `draft` SKU freezes exactly one version row at
/// `published_version = 1`, moves **both** counters by exactly one, and takes
/// the `draft -> published` edge.
///
/// Both counters are asserted explicitly, because they move for different
/// reasons and a door that moved only one would still look plausible from the
/// response body: `published_version` is `inst-fd-publish-freeze`'s and
/// `internal_revision` is `inst-fd-publish-bump`'s "**once**", which is the
/// property the `ETag` contract depends on.
#[tokio::test]
async fn a_first_publish_freezes_one_version_and_moves_both_counters_by_exactly_one() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a first publish of a live draft must be admitted"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["lifecycle_state"],
        json!("published"),
        "the draft -> published edge is the publish door's own"
    );
    assert_eq!(view["published_version"], json!(1));
    assert_eq!(view["internal_revision"], json!(2));

    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        1,
        "a publish freezes exactly one version row"
    );
    assert_eq!(
        raw_i64(
            &harness.dsn,
            &format!(
                "SELECT COUNT(*) AS v FROM products_entity_version \
                 WHERE {} AND published_version = 1",
                id_matches("entity_id", sku_id)
            )
        )
        .await,
        1,
        "the frozen row is keyed at published_version + 1, which is 1 for a first publish"
    );
    assert_eq!(
        head_version(&harness.dsn, sku_id).await,
        1,
        "published_version moves to 1"
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        2,
        "internal_revision moves by exactly one: the create left it at 1"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("published")
    );
}

/// The frozen row's `content_digest` is the digest **of the rendering the row
/// stores**, recomputed here from the bytes read back out of the table.
///
/// The recomputation reads `content` out of the database and hashes that,
/// rather than re-rendering the door's own in-memory image and hashing the
/// result: hashing the same value with the same helper the door just called
/// would assert the door against itself and would pass even if the door had
/// stored a rendering of some *other* value beside a digest of this one. What
/// slice 10's restore drill re-verifies is exactly this pair, so this is the
/// pair the test compares.
#[tokio::test]
async fn the_frozen_rows_digest_is_the_digest_of_the_stored_rendering() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(response.status(), StatusCode::OK, "the publish must land");

    let stored_content = raw_string_opt(
        &harness.dsn,
        &format!(
            "SELECT content AS v FROM products_entity_version \
             WHERE {} AND published_version = 1",
            id_matches("entity_id", sku_id)
        ),
    )
    .await
    .expect("the frozen row carries its rendering");
    let stored_digest = raw_string_opt(
        &harness.dsn,
        &format!(
            "SELECT hex(content_digest) AS v FROM products_entity_version \
             WHERE {} AND published_version = 1",
            id_matches("entity_id", sku_id)
        ),
    )
    .await
    .expect("the frozen row carries its digest");

    assert_eq!(
        hex_of(&crate::domain::canonical::content_digest(&stored_content)),
        stored_digest.to_lowercase(),
        "the stored digest must be SHA-256 over the stored rendering, byte for byte"
    );

    // None of the four excluded columns reaches the content, and
    // `published_version` is the one that reads as a surprise: the row is
    // keyed at version 1 (the query above selects on it), and the content
    // does not restate that number, because restating it would put the key
    // inside the payload it keys — `super::SKU_VERSION_CONTENT_ROSTER`'s doc
    // carries the argument for all four.
    //
    // The four together are what makes *the same content produces the same
    // digest* true, and that property is what lets a reader answer `did the
    // content change between N and N+1` by comparing two rows' digests —
    // the question slice 06's CatalogVersion and slice 10's restore drill
    // both ask.
    for excluded in EXCLUDED_FROM_FROZEN_CONTENT {
        assert!(
            !stored_content.contains(excluded),
            "{excluded} is excluded from a frozen row's content; stored rendering was \
             {stored_content}"
        );
    }

    let digest_version = raw_i64(
        &harness.dsn,
        &format!(
            "SELECT digest_version AS v FROM products_entity_version WHERE {}",
            id_matches("entity_id", sku_id)
        ),
    )
    .await;
    assert_eq!(
        digest_version,
        i64::from(crate::domain::canonical::DIGEST_VERSION),
        "the row records the scheme its digest was computed under"
    );
}

/// A re-publish of a `published` head moves the version to 2 and leaves the
/// state `published` — `inst-fd-publish-freeze`'s "a re-publish changes the
/// version, never the state".
#[tokio::test]
async fn a_republish_moves_the_version_to_two_and_leaves_the_state_published() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let first = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(
        first.status(),
        StatusCode::OK,
        "the first publish must land"
    );
    let next_etag = first
        .headers()
        .get(axum::http::header::ETAG)
        .expect("a publish answers the ETag its own UPDATE produced")
        .to_str()
        .expect("the ETag is ASCII")
        .to_owned();

    let second = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&next_etag)).await;

    assert_eq!(
        second.status(),
        StatusCode::OK,
        "a re-publish of a published head is admitted"
    );
    let view = body_json(second).await;
    assert_eq!(view["published_version"], json!(2));
    assert_eq!(view["internal_revision"], json!(3));
    assert_eq!(
        view["lifecycle_state"],
        json!("published"),
        "a re-publish takes no edge"
    );
    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        2,
        "each publish freezes its own version row"
    );
}

/// A publish whose `If-Match` names a revision the head no longer carries is
/// refused `STALE_REVISION`, and **nothing is written** — no frozen version
/// row and no counter movement.
///
/// The freeze runs before the head-row `UPDATE` on the same transaction, so
/// "nothing is written" is the property the rollback has to provide: a
/// committed freeze beside an unmoved head would be a version row for a
/// publish that never happened, which is exactly what the head-row guard
/// would later read as permission to bump.
#[tokio::test]
async fn a_publish_with_a_stale_if_match_is_refused_and_writes_nothing() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, _etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let stale = super::preconditions::etag(crate::domain::concurrency::InternalRevision::new(99));
    let response = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&stale)).await;

    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "a stale precondition is a 409"
    );
    let view = body_json(response).await;
    assert_eq!(view["context"]["reason"], json!("STALE_REVISION"));

    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        0,
        "the freeze must roll back with the head write that matched no row"
    );
    assert_eq!(head_version(&harness.dsn, sku_id).await, 0);
    assert_eq!(head_revision(&harness.dsn, sku_id).await, 1);

    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("STALE_REVISION"),
        "every refusal audits, this one included"
    );
    let action = raw_string_opt(&harness.dsn, "SELECT action AS v FROM products_audit_log").await;
    assert_eq!(
        action.as_deref(),
        Some("publish"),
        "the row records the act that was refused. The shared \
         `audit_refusal_and_report` delegates with the literal `create`, so a publish door \
         calling it files its refusals under the create door's token and an operator reading \
         products_audit_log is told the wrong thing about a row whose error_code and subject \
         are both right"
    );
}

/// A publish carrying no `If-Match` at all is refused `VALIDATION` — the save
/// flow's own Acceptance Criteria, read through the shared `preconditions`
/// module, since an unconditional publish is exactly the write that rule
/// exists to make unreachable.
#[tokio::test]
async fn a_publish_with_no_if_match_is_refused_validation() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, _etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = post_publish(app_for(&harness, TENANT), TENANT, sku_id, None).await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "VALIDATION renders as an architectural 422, wire 400"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("VALIDATION")
    );
    assert_eq!(
        view["context"]["violations"][0]["subject"],
        json!("If-Match")
    );
    assert_eq!(head_version(&harness.dsn, sku_id).await, 0);
}

/// A publish on a terminal head is refused `ENTITY_TERMINAL`, and the refusal
/// is audited.
///
/// The head is walked to `discarded` through this module's own discard door
/// rather than by a raw `UPDATE`: the terminal state a real caller can reach
/// is the one the doors produce, and driving it through the door also proves
/// the two acts compose.
#[tokio::test]
async fn a_publish_on_a_terminal_head_is_refused_entity_terminal_and_audited() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let discarded = post_discard(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(
        discarded.status(),
        StatusCode::OK,
        "this case's premise: the head is terminal because the discard landed"
    );
    let terminal_etag = discarded
        .headers()
        .get(axum::http::header::ETAG)
        .expect("a discard answers its own ETag")
        .to_str()
        .expect("the ETag is ASCII")
        .to_owned();

    let response = post_publish(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        Some(&terminal_etag),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let view = body_json(response).await;
    assert_eq!(view["context"]["reason"], json!("ENTITY_TERMINAL"));

    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        0,
        "a refused publish freezes nothing"
    );
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("ENTITY_TERMINAL"),
        "the second audited refusal this suite pins"
    );
}

/// A publish a governance host refuses is `APPROVAL_REQUIRED`, and nothing is
/// written: no frozen row, no counter movement, no state flip
/// (`inst-fd-gate-rejection`).
///
/// Driven through `super::publish_sku_gated` with [`RefusingGate`] rather
/// than through the router, because the router's host is
/// `NoMaterialityPolicyGate` and it never refuses — see [`RefusingGate`]'s
/// own doc.
#[tokio::test]
async fn a_publish_a_gate_refuses_is_approval_required_and_writes_nothing() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        etag.parse().expect("the ETag is a valid header value"),
    );
    let result = super::publish_sku_gated(
        &api_state(&harness),
        &flat_in_enforcer(TENANT),
        &authed_ctx(TENANT),
        &headers,
        sku_id,
        &(Arc::new(RefusingGate)
            as Arc<dyn crate::domain::governance::GovernanceGate + Send + Sync>),
    )
    .await;

    let Err(refusal) = result else {
        panic!("a refusing gate must refuse the act")
    };
    let response = refusal.into_response();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "APPROVAL_REQUIRED is the governance gate's own 403"
    );
    let view = body_json(response).await;
    assert_eq!(view["context"]["reason"], json!("APPROVAL_REQUIRED"));

    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        0,
        "a rejection flips no state and writes nothing"
    );
    assert_eq!(head_version(&harness.dsn, sku_id).await, 0);
    assert_eq!(head_revision(&harness.dsn, sku_id).await, 1);
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("draft"),
        "a first-publish entity refused by the gate stays draft"
    );
}

/// A discard of a `draft` at `published_version = 0` succeeds: the head goes
/// terminal, `internal_revision` moves by exactly one and nothing is frozen.
#[tokio::test]
async fn a_discard_of_a_never_published_draft_succeeds() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = post_discard(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;

    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(view["lifecycle_state"], json!("discarded"));
    assert_eq!(view["internal_revision"], json!(2));
    assert_eq!(
        view["published_version"],
        json!(0),
        "a discard publishes nothing"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("discarded")
    );
    assert_eq!(head_revision(&harness.dsn, sku_id).await, 2);
    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        0,
        "a discard freezes no version"
    );
}

/// A discard of a `published` head is refused `ILLEGAL_TRANSITION`:
/// `published -> discarded` is not in `transition::ADMITTED_EDGES`, and
/// `inst-fd-discard` admits the act only from a never-published `draft`.
#[tokio::test]
async fn a_discard_of_a_published_head_is_refused_illegal_transition() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let published = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(
        published.status(),
        StatusCode::OK,
        "this case's premise: the head is published"
    );
    let published_etag = published
        .headers()
        .get(axum::http::header::ETAG)
        .expect("a publish answers its own ETag")
        .to_str()
        .expect("the ETag is ASCII")
        .to_owned();

    let response = post_discard(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        Some(&published_etag),
    )
    .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let view = body_json(response).await;
    assert_eq!(view["context"]["reason"], json!("ILLEGAL_TRANSITION"));
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("published"),
        "a refused discard leaves the head where it was"
    );

    // The publish above succeeded, and a successful act writes no audit row
    // (P-D-21: its event is its record), so the single row in the table is
    // this discard's refusal and nothing else.
    assert_eq!(
        raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await,
        1,
        "only the refused discard audited: the publish that set this case up succeeded"
    );
    let action = raw_string_opt(&harness.dsn, "SELECT action AS v FROM products_audit_log").await;
    assert_eq!(
        action.as_deref(),
        Some("discard"),
        "the discard door records `discard`, not the `create` the shared helper delegates \
         with, and not the `write` it gates on: the authorization vocabulary and the audit \
         vocabulary are two different sets"
    );
}

/// After a discard, a new SKU may take the discarded one's `sku_code`.
///
/// `uq_products_sku_code` is partial on `lifecycle_state <> 'discarded'`, so
/// the reservation releases by the discard's own `UPDATE` and by no second
/// statement — the property `discard_sku_head`'s doc states and this case
/// measures from the outside.
#[tokio::test]
async fn a_discarded_skus_code_is_free_for_the_next_holder() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let discarded = post_discard(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(discarded.status(), StatusCode::OK, "the discard must land");

    let response = post_create_sku(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "product_id": parent_id, "sku_code": "SKU-500" }),
    )
    .await;

    assert_eq!(
        response.status(),
        StatusCode::CREATED,
        "the discarded row left the partial unique index, so its skuCode is free"
    );
    assert_eq!(
        raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_sku").await,
        2,
        "the discarded row is still there; it simply no longer reserves its code"
    );
}

/// A replayed publish under the same `Idempotency-Key` returns the stored
/// answer and does **not** publish twice.
///
/// The retry sends the same `If-Match` as the original, which by then names a
/// revision the head no longer carries. That is deliberate: the claim runs
/// first, inside the mutation's own transaction (P-D-42), so a replay
/// short-circuits before the head write is ever attempted. A door that
/// replayed *after* its write would have frozen a second version row here.
#[tokio::test]
async fn a_replayed_publish_returns_the_stored_answer_and_does_not_publish_twice() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let first = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        "publish",
        Some(&etag),
        Some("key-publish-1"),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::OK,
        "the first publish must land"
    );
    let first_body = body_json(first).await;

    let replay = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        "publish",
        Some(&etag),
        Some("key-publish-1"),
    )
    .await;

    assert_eq!(
        replay.status(),
        StatusCode::OK,
        "a replay reproduces the original status"
    );
    assert_eq!(
        body_json(replay).await,
        first_body,
        "a replay reproduces the original body, byte for byte"
    );

    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        1,
        "the replay must not have frozen a second version"
    );
    assert_eq!(head_version(&harness.dsn, sku_id).await, 1);
    assert_eq!(head_revision(&harness.dsn, sku_id).await, 2);
    assert_eq!(
        idempotency_rows_for(&harness.dsn, "key-publish-1").await,
        1,
        "one key, one row"
    );
    let endpoint = raw_string_opt(
        &harness.dsn,
        "SELECT endpoint AS v FROM products_idempotency WHERE client_key = 'key-publish-1'",
    )
    .await;
    assert_eq!(
        endpoint.as_deref(),
        Some(format!("/bss-products/v1/skus/{sku_id}/publish").as_str()),
        "P-D-42: the key's endpoint is the concrete resource path, so it carries this SKU's \
         own id rather than a route template's placeholder"
    );
}

/// The four head columns §4.3 keeps **out** of a frozen row's content, as
/// this file states them independently of the door.
///
/// Spelled here rather than imported from
/// [`super::SKU_VERSION_CONTENT_ROSTER`]'s own module because that is what
/// makes the case below a **drift** test: the roster is a literal list, and
/// the only way to catch a literal list that forgot a column is to derive
/// the answer from something the roster does not control — the executed
/// schema, plus this short, independently authored exclusion set.
///
/// Two of the four are §4.3's own words (`lifecycle_state`,
/// `internal_revision`; P-D-24 and P-D-35). The other two are readings the
/// door states and argues, because §4.3 enumerates its exclusions as a
/// closed list of four columns plus the metadata map and names neither:
/// `updated_at` is P-D-35's own stated criterion applied to a column the
/// enumeration does not list, and `published_version` is the version row's
/// own key column, which the content would otherwise restate inside the
/// payload the key keys. See [`super::SKU_VERSION_CONTENT_ROSTER`]'s doc for
/// both arguments and for the additions the design set is owed. §4.3's other two exclusions,
/// `deprecation_provenance` and `replaced_by_sku_id`, are deliberately
/// absent from this array: they are not columns of `products_sku` at this
/// commit, and naming a column the table does not have would make the
/// `is a real column` assertion below fail for the wrong reason.
const EXCLUDED_FROM_FROZEN_CONTENT: [&str; 4] = [
    "internal_revision",
    "lifecycle_state",
    "published_version",
    "updated_at",
];

/// Every column of `table`, read out of the **executed** `SQLite` schema.
///
/// `pragma_table_info` rather than a hand-written list, and rather than the
/// migration's own source text: the property the case below needs is that
/// the roster matches the table the chain actually created, which is the
/// only artifact that can disagree with the roster at run time.
/// `group_concat` collapses the pragma's rows into the single named column
/// [`raw_string_opt`] reads, so no second row-shape helper is needed here.
async fn table_columns(dsn: &str, table: &str) -> Vec<String> {
    let joined = raw_string_opt(
        dsn,
        &format!("SELECT group_concat(name, ',') AS v FROM pragma_table_info('{table}')"),
    )
    .await
    .expect("the migration chain created this table, so the pragma answers a non-empty list");
    joined.split(',').map(ToOwned::to_owned).collect()
}

/// **[`super::SKU_VERSION_CONTENT_ROSTER`] is `products_sku`'s own columns
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
async fn the_sku_content_roster_is_the_head_table_minus_the_excluded_columns() {
    let harness = harness().await;
    let columns = table_columns(&harness.dsn, "products_sku").await;

    for excluded in EXCLUDED_FROM_FROZEN_CONTENT {
        assert!(
            columns.contains(&excluded.to_owned()),
            "{excluded} must be a real column of products_sku for its exclusion to subtract \
             anything; the executed schema has {columns:?}"
        );
        assert!(
            !super::SKU_VERSION_CONTENT_ROSTER.contains(&excluded),
            "section 4.3 excludes {excluded} from a frozen row's content, so the roster must \
             not name it"
        );
    }

    let mut expected: Vec<String> = columns
        .into_iter()
        .filter(|column| !EXCLUDED_FROM_FROZEN_CONTENT.contains(&column.as_str()))
        .collect();
    expected.sort();
    let mut roster: Vec<String> = super::SKU_VERSION_CONTENT_ROSTER
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    roster.sort();

    assert_eq!(
        roster, expected,
        "section 4.3 scopes a frozen row's content as the publish-time entity minus its named \
         exclusions, so the roster is the head table's columns minus those and nothing else. A \
         slice that adds a content column to products_sku adds it here too, and bumps \
         canonical::DIGEST_VERSION with it"
    );
}

/// **[`super::sku_version_content`] writes exactly
/// [`super::SKU_VERSION_CONTENT_ROSTER`]'s names** — no extra key, and, the
/// part that matters, no missing one.
///
/// The drift case above compares the *roster* to the executed schema.
/// Nothing compared the *builder* to the roster, and
/// `crate::domain::canonical::Absence::Null` is what makes that gap silent:
/// a roster name the builder forgot is rendered `null` rather than refused,
/// so a builder that dropped `sku_code` would freeze `"sku_code":null`,
/// digest cleanly, and pass every other case in this file — the digest case
/// included, since that one re-hashes whatever was stored rather than
/// judging what it says.
///
/// The record is built here rather than read back through the harness
/// because `products_sku` has **no optional column**: every roster field is
/// `NOT NULL` on the head, so there is no fixture choice that could make
/// this case prove less than it appears to, the way the Product twin's
/// `product_code` can. Every value below is distinct and non-empty for the
/// same reason.
#[test]
fn the_sku_content_builder_writes_exactly_the_roster() {
    let record = repo::SkuRecord {
        sku_id: Uuid::from_u128(0xd1_11),
        tenant_id: TENANT,
        product_id: Uuid::from_u128(0xd1_12),
        sku_code: "SKU-ROSTER".to_owned(),
        lifecycle_state: bss_products_sdk::models::LifecycleState::Draft,
        internal_revision: 1,
        published_version: 0,
        region_scope: "eu".to_owned(),
        brand_scope: "acme".to_owned(),
        created_by: "principal:author-1".to_owned(),
        created_at: Utc.with_ymd_and_hms(2026, 8, 29, 9, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 8, 29, 9, 0, 0).unwrap(),
    };

    let content = super::sku_version_content(&record);
    let mut written: Vec<&str> = content
        .as_object()
        .expect("the builder renders a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    written.sort_unstable();
    let mut roster = super::SKU_VERSION_CONTENT_ROSTER;
    roster.sort_unstable();

    assert_eq!(
        written, roster,
        "the builder and the roster are one field set stated twice, and only this assertion \
         holds them equal: Absence::Null renders a name the builder forgot as null instead of \
         failing, so the omission would reach storage and no other case would notice"
    );
}

/// The JSON body of the one enqueued outbox row carrying `payload_type`.
///
/// The `payload` column is a `BLOB`; `CAST(.. AS TEXT)` is what lets
/// [`raw_string_opt`]'s single-text-column shape read it. Filtering by
/// `payload_type` is what keeps this from picking up the `SkuCreated` row the
/// seed enqueued.
async fn enqueued_event_body(dsn: &str, payload_type: &str) -> serde_json::Value {
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let payload = raw_string_opt(
        dsn,
        &format!(
            "SELECT CAST(payload AS TEXT) AS v FROM {body_table} \
             WHERE payload_type = '{payload_type}'"
        ),
    )
    .await
    .expect("the enqueued row carries a payload");
    serde_json::from_str(&payload).expect("the door enqueues a JSON body")
}

/// **`SkuPublished` carries `publishedVersion`, and it is the version the act
/// produced.**
///
/// §4.5: every one of the eight Foundation events carries the same body core,
/// and `ProductPublished`/`SkuPublished` **additionally** carry
/// `publishedVersion` — which `06` reads as content and `08`'s projector keys
/// on. A body without it is a body those two consumers cannot use.
///
/// The **value** is asserted, not merely the key's presence. A door that
/// hard-coded a zero, or that announced the pre-act `N` the head carried
/// before the publish, would satisfy an existence check and would still point
/// `06` and `08` at the wrong version. So this reads `published_version` off
/// the head after the act and requires the event to agree with it — and, to
/// pin down which of the two candidate numbers that is, the case publishes
/// **twice**: after a re-publish the pre-act value is `1` and the post-act
/// value is `2`, so the two readings are no longer the same number and a door
/// announcing `N` fails.
#[tokio::test]
async fn the_published_event_carries_the_post_act_published_version() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let first = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(first.status(), StatusCode::OK, "this case's premise");
    let published_etag = first
        .headers()
        .get(axum::http::header::ETAG)
        .expect("a publish answers its own ETag")
        .to_str()
        .expect("the ETag is ASCII")
        .to_owned();

    let body = enqueued_event_body(&harness.dsn, "SkuPublished").await;
    assert_eq!(
        body["publishedVersion"],
        json!(head_version(&harness.dsn, sku_id).await),
        "the event announces the version the act wrote, read back off the head"
    );
    assert_eq!(
        body["publishedVersion"],
        json!(1),
        "a first publish produces version 1"
    );
    // The core is still there, unchanged: `publishedVersion` is *additional
    // to* the core (§4.5), not a replacement for any of it.
    assert_eq!(body["entityKind"], json!("sku"));
    assert_eq!(body["entityId"], json!(sku_id.to_string()));
    assert_eq!(body["tenantId"], json!(TENANT.to_string()));
    assert_eq!(body["lifecycleState"], json!("published"));
    assert_eq!(body["internalRevision"], json!(2));

    let second = post_publish(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        Some(&published_etag),
    )
    .await;
    assert_eq!(second.status(), StatusCode::OK, "a re-publish is admitted");

    let after = raw_string_opt(
        &harness.dsn,
        &format!(
            "SELECT CAST(payload AS TEXT) AS v FROM {}_body \
             WHERE payload_type = 'SkuPublished' ORDER BY id DESC LIMIT 1",
            events::OUTBOX_TABLE_PREFIX
        ),
    )
    .await
    .expect("the re-publish enqueued its own row");
    let after: serde_json::Value =
        serde_json::from_str(&after).expect("the door enqueues a JSON body");
    assert_eq!(
        after["publishedVersion"],
        json!(head_version(&harness.dsn, sku_id).await),
        "the re-publish announces 2, the version it produced, not the 1 the head carried \
         when it began"
    );
    assert_eq!(after["publishedVersion"], json!(2));
}

/// **A `SkuDiscarded` body carries no `publishedVersion` at all.**
///
/// §4.5 puts the field on the two `*Published` events and on no other, which
/// is the whole reason it lives on `events::PublishedEventBody` rather than
/// becoming a sixth field of `events::EventBodyCore`. A discard writes no
/// version row and moves no version counter, so any number it announced
/// would be one nothing produced.
#[tokio::test]
async fn a_discarded_event_carries_no_published_version() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = post_discard(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(response.status(), StatusCode::OK, "this case's premise");

    let body = enqueued_event_body(&harness.dsn, "SkuDiscarded").await;
    assert_eq!(
        body.get("publishedVersion"),
        None,
        "section 4.5 names publishedVersion on the two *Published events and on no other"
    );
    assert_eq!(body["lifecycleState"], json!("discarded"));
    assert_eq!(body["internalRevision"], json!(2));
}

// ---------------------------------------------------------------------------
// The idempotency store, at the head doors
// ---------------------------------------------------------------------------

/// Walk a published SKU head to `retired` **out of band**, along the admitted
/// edges, bumping `internal_revision` on every step.
///
/// It cannot simply write the target state: `trg_products_sku_lifecycle_edge`
/// admits `published -> deprecated` and `deprecated -> retired` and nothing
/// that skips the middle, so the walk is two statements. Neither touches
/// `published_version`, so neither needs a frozen row to satisfy
/// `trg_products_sku_published_version_row`.
///
/// This stands for the neighbour the retry cases below are about — someone
/// else moving the head between a client's lost response and its retry. It
/// goes around the doors deliberately: no door in this slice retires a SKU
/// (that is `04-lifecycle`'s), and the case needs the *state*, not the path.
async fn retire_sku_out_of_band(
    provider: &DBProvider<DbError>,
    scope: &toolkit_db::secure::AccessScope,
    sku_id: Uuid,
) {
    use crate::infra::storage::entity::sku;
    use sea_orm::sea_query::ExprTrait as _;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use toolkit_db::secure::SecureUpdateExt as _;

    let conn = provider.conn().expect("scoped connection");
    for state in ["deprecated", "retired"] {
        let result = sku::Entity::update_many()
            .col_expr(sku::Column::LifecycleState, Expr::value(state))
            .col_expr(
                sku::Column::InternalRevision,
                Expr::col(sku::Column::InternalRevision).add(1_i64),
            )
            .filter(sku::Column::SkuId.eq(sku_id))
            .secure()
            .scope_with(scope)
            .exec(&conn)
            .await
            .unwrap_or_else(|e| panic!("move the SKU head to `{state}`: {e}"));
        assert!(
            result.rows_affected > 0,
            "the head was never moved to `{state}`, so this case's premise never held"
        );
    }
}

/// A retry under the **same** `Idempotency-Key` and a **different**
/// `If-Match` replays the stored answer.
///
/// This is the case `inst-fd-idem-hash` (P-D-34) names: *"The hash covers the
/// body's present fields and not the precondition ... hashing the precondition
/// in would answer that retry `IDEMPOTENCY_CONFLICT` instead of running it."*
/// The client here does exactly what a client recovering a lost response
/// does — it re-reads the head, so the tag it holds on the retry is the
/// **post**-act one, not the one it originally sent. A door whose payload
/// digest folded the revision in would compute two different digests for one
/// act and answer `409` for the whole retention window.
///
/// It is deliberately distinct from
/// [`a_replayed_publish_returns_the_stored_answer_and_does_not_publish_twice`],
/// which re-sends the *stale* tag: that case passes under either digest, so
/// it cannot see this defect.
#[tokio::test]
async fn a_retry_under_a_different_if_match_replays_the_stored_answer() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let first = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        "publish",
        Some(&etag),
        Some("key-publish-digest"),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::OK,
        "this case's premise: the first publish landed"
    );
    // The tag the act committed — the very one a client re-reading the head to
    // recover its lost response would hold.
    let fresh_etag = first
        .headers()
        .get(axum::http::header::ETAG)
        .expect("a publish answers the ETag of the revision it committed")
        .to_str()
        .expect("the ETag is ASCII")
        .to_owned();
    assert_ne!(
        fresh_etag, etag,
        "this case's premise: the act moved the revision, so the two tags differ"
    );
    let first_body = body_json(first).await;

    let retry = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        "publish",
        Some(&fresh_etag),
        Some("key-publish-digest"),
    )
    .await;

    assert_eq!(
        retry.status(),
        StatusCode::OK,
        "the precondition is not an operand of the payload digest, so the fresher tag reaches \
         the stored answer rather than IDEMPOTENCY_CONFLICT"
    );
    assert_eq!(
        body_json(retry).await,
        first_body,
        "a replay reproduces the original body, byte for byte"
    );
    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        1,
        "the replay executed nothing"
    );
    assert_eq!(head_version(&harness.dsn, sku_id).await, 1);
    assert_eq!(head_revision(&harness.dsn, sku_id).await, 2);
}

/// A retry whose head has gone **terminal** since the answered act still
/// replays the stored answer.
///
/// §3.1 runs `Phase::Idempotency` first and P-D-42 puts the claim `INSERT`
/// inside the mutation, which together put terminality — and the precondition,
/// and the gate — **after** the claim. The sequence here is the one the store
/// exists for: the publish commits, the response is lost, a neighbour
/// deprecates and retires the head, and the client retries the key it still
/// holds. A door that asked `transition::check_head_write` before consulting
/// the claim answers `409 ENTITY_TERMINAL` and never reaches the stored `200`,
/// leaving the store inert at exactly this door.
#[tokio::test]
async fn a_retried_publish_replays_after_the_head_has_gone_terminal() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let first = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        "publish",
        Some(&etag),
        Some("key-publish-terminal"),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::OK,
        "this case's premise: the first publish landed"
    );
    let first_body = body_json(first).await;

    retire_sku_out_of_band(
        &harness.db,
        &toolkit_db::secure::AccessScope::for_tenant(TENANT),
        sku_id,
    )
    .await;
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("retired"),
        "this case's premise: the head is terminal when the retry arrives"
    );

    let retry = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        "publish",
        Some(&etag),
        Some("key-publish-terminal"),
    )
    .await;

    assert_eq!(
        retry.status(),
        StatusCode::OK,
        "the claim is the transaction's first statement, so the stored answer is served before \
         terminality is judged"
    );
    assert_eq!(
        body_json(retry).await,
        first_body,
        "the replay reproduces the answer the act committed, not the head as it now stands"
    );
    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        1,
        "the replay executed nothing"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("retired"),
        "and moved nothing"
    );
}

/// The discard door's twin of the case above: a discard leaves its own head
/// terminal, so **every** retry of an answered discard key arrives at a
/// terminal row.
///
/// This one needs no neighbour at all, which is what makes it the sharper
/// case: the act's own success is what would refuse its own retry under an
/// order that judged the edge before the claim.
#[tokio::test]
async fn a_retried_discard_replays_rather_than_refusing_its_own_terminal_head() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let first = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        "discard",
        Some(&etag),
        Some("key-discard-replay"),
    )
    .await;
    assert_eq!(
        first.status(),
        StatusCode::OK,
        "this case's premise: the discard landed"
    );
    let first_body = body_json(first).await;
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("discarded"),
        "a discard leaves its own head terminal"
    );

    let retry = post_head_act(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        "discard",
        Some(&etag),
        Some("key-discard-replay"),
    )
    .await;

    assert_eq!(
        retry.status(),
        StatusCode::OK,
        "the stored answer is served before the edge is judged"
    );
    assert_eq!(
        body_json(retry).await,
        first_body,
        "a replay reproduces the original body, byte for byte"
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        2,
        "the replay wrote nothing, so the discard's single bump still stands"
    );
    assert_eq!(
        idempotency_rows_for(&harness.dsn, "key-discard-replay").await,
        1,
        "one key, one row"
    );
}

/// A head act whose **target tenant is not a member of the compiled scope**
/// is refused an audited `403 PERMISSION_DENIED`, not a bare `404`.
///
/// `crate::authz::access_scope` splits reads from writes on exactly this: a
/// read passes `owner_tenant_id = None` and uses the scope as its SQL filter,
/// while a write passes the tenant the row is written to and the function
/// then asserts that tenant is a member of the compiled scope — an assertion
/// gated on the argument being `Some`, and so simply absent for a write that
/// passes `None`.
///
/// The caller here authenticates as `TENANT` while the `PDP` compiles a scope
/// naming `OTHER_TENANT`, which is the degraded flat-`In` decision the
/// assertion exists for: the PDP does not re-check `owner_tenant_id`, so
/// nothing but this assertion tells the two apart. The `.secure()
/// .scope_with(scope)` filter would still have kept the row out — that is why
/// this is defence in depth rather than an escalation — but the answer would
/// have degraded to an unaudited `404`, and the audit row is the whole point:
/// an operator cannot see a cross-tenant write attempt that was reported as a
/// missing row.
#[tokio::test]
async fn a_publish_whose_target_tenant_is_outside_the_compiled_scope_is_denied_and_audited() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    // `TENANT` is the caller; `OTHER_TENANT` is all the compiled scope names.
    let response = post_publish(app_for(&harness, OTHER_TENANT), TENANT, sku_id, Some(&etag)).await;

    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "a write whose target tenant is not in the compiled scope is denied, not silently \
         filtered into a 404"
    );
    let view = body_json(response).await;
    let reason = view["context"]["reason"]
        .as_str()
        .expect("a denial carries the PDP's own reason")
        .to_owned();
    assert!(
        reason.contains("not authorized to write resources owned by tenant"),
        "the reason names the cross-tenant target the membership assertion refused, not some          other denial: {reason}"
    );

    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("PERMISSION_DENIED"),
        "the denial is audited; a bare 404 would have recorded nothing"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("draft"),
        "and nothing moved"
    );
}
