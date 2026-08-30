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
//! `set_parent_lifecycle_state`) are this file's own copies of
//! `products_tests`'s identically named functions, for the reason
//! `crate::api::rest::skus`'s own module doc gives for
//! [`super::insert_sku_with_event`]: `products_tests.rs` is outside this
//! slice's `target_paths`, so nothing in it can be imported.
//! `set_parent_lifecycle_state` is this file's one addition beyond
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

/// Move a seeded parent to `retired`/`discarded` after insertion — the lever
/// the parent-terminal tests need, since `infra::storage::repo` exposes no
/// lifecycle-transition writer this door's own slice may call.
///
/// Goes through the **secure** update wrapper, not a bare
/// `UpdateMany::exec`: the workspace disallows the bare form precisely so a
/// write cannot reach the database without a scope, and a test helper is not
/// exempt from the rule it exists alongside. It also settles the storage
/// representation of the `Uuid`, which a raw `UPDATE ... WHERE product_id =
/// '<hyphenated>'` did not — the first version of this helper matched no
/// rows for that reason, leaving the parent `draft` and failing two tests
/// against a door that was behaving correctly. The `rows_affected`
/// assertion below is what would catch the next such miss.
async fn set_parent_lifecycle_state(
    provider: &DBProvider<DbError>,
    scope: &toolkit_db::secure::AccessScope,
    product_id: Uuid,
    state: &str,
) {
    use crate::infra::storage::entity::product;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
    use toolkit_db::secure::SecureUpdateExt as _;

    let conn = provider.conn().expect("scoped connection");
    let result = product::Entity::update_many()
        .col_expr(product::Column::LifecycleState, Expr::value(state))
        .filter(product::Column::ProductId.eq(product_id))
        .secure()
        .scope_with(scope)
        .exec(&conn)
        .await
        .expect("move the parent's lifecycle state");
    assert!(
        result.rows_affected > 0,
        "the parent's state was never moved to `{state}`, so this test's premise never held"
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
    let state = Arc::new(ApiState {
        db: harness.db.clone(),
        outbox: Arc::clone(&harness.outbox),
    });
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
    set_parent_lifecycle_state(
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
    set_parent_lifecycle_state(
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
