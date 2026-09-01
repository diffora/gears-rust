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

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse as _;
use chrono::{TimeZone, Utc};
use sea_orm::sea_query::Expr;
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
use crate::domain::concurrency::InternalRevision;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{self, NewProduct};
use crate::test_support::{
    audit_action, audit_error_code, authed_ctx, drop_table, enqueued_event_count,
    enqueued_event_envelope, flat_in_enforcer, id_matches, idempotency_rows_for, raw_i64,
    raw_string_opt, table_columns,
};

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
        cloned_from: None,
        cloned_from_version: None,
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
/// `GateMode::Gate` — its own doc says so, and says why: it holds no
/// approval-record store, so it cannot evaluate the ceremony
/// `inst-fd-gate-mode-gate` requires and authorizes without one. That is a
/// recorded deviation, **not** a finding that the act needed no approval;
/// an earlier revision of this doc made the second claim and `05-governance`
/// `inst-gv-materiality` contradicts it, materiality deciding the quorum
/// *count* and never whether a record exists. Either way the
/// `APPROVAL_REQUIRED` path is unreachable through the router, and a path
/// with no test is one nothing pins. This double is why
/// `super::publish_sku_gated` and `super::discard_sku_gated` take the host
/// as an argument at all. The *mode* is a separate argument, for
/// `dod-publish-door`'s own reason; [`RecordingGate`] is what reaches
/// `GateMode::PreAuthorized`.
struct RefusingGate;

impl crate::domain::governance::GovernanceGate for RefusingGate {
    fn evaluate(
        &self,
        _subject: crate::domain::governance::GateSubject,
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
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
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
        crate::domain::governance::GateMode::Gate,
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
///
/// `composition_pending` **is** a column and is deliberately **not** here.
/// The four above share one criterion — they move on writes that produce no
/// version row — and that column is its inverse: its trigger clause admits a
/// change only in the same statement as a `published_version` bump, so it
/// moves only where a version row is written. `inst-fd-publish-freeze` names
/// it in the frozen content outright. Adding it to this array would make the
/// case below assert the opposite of the design set.
/// The columns a frozen row's content leaves out.
///
/// **`design/01` §4.3 names four and this list holds 6, and the difference is
/// stated rather than silent.** §4.3's four are `lifecycle_state`,
/// `deprecation_provenance`, `replaced_by_sku_id` and `internal_revision`
/// (**P-D-24** as **P-D-35** extended it: *"those four move on transitions,
/// which write no version row, so freezing them would need the digest to
/// change on a write that produces no row to digest"*). `published_version`
/// and `updated_at` are excluded on the same criterion by the roster's own
/// argument — the first IS the row's key and the second moves on every write
/// — and §4.3 does not name them because it enumerates the columns whose
/// exclusion was contested, not every column outside the content.
const EXCLUDED_FROM_FROZEN_CONTENT: [&str; 6] = [
    "internal_revision",
    "lifecycle_state",
    "published_version",
    "updated_at",
    "deprecation_provenance",
    "replaced_by_sku_id",
];

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
         slice that adds a content column to products_sku adds it here too. It owes \
         canonical::DIGEST_VERSION a bump as well once any row has been stored under the \
         current value; while none has -- the gear is undeployed and products_entity_version \
         is an unreleased migration -- a bump would mint a version no row ever used and no \
         restore drill could encounter. See that constant's own doc"
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
///
/// `composition_pending` is seeded `true`, against the column's own default,
/// on that same reasoning read for a boolean: a `bool` has only two values
/// and one of them is what every row carries today, so a fixture holding the
/// default is the one fixture that cannot distinguish a builder writing the
/// field from a builder writing nothing at all once this case grows a value
/// assertion.
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
        composition_pending: true,
        region_scope: "eu".to_owned(),
        brand_scope: "acme".to_owned(),
        created_by: "principal:author-1".to_owned(),
        created_at: Utc.with_ymd_and_hms(2026, 8, 29, 9, 0, 0).unwrap(),
        updated_at: Utc.with_ymd_and_hms(2026, 8, 29, 9, 0, 0).unwrap(),
        // Populated on the roster test's own premise (P-D-76 added the pair):
        // the builder legitimately omits an absent optional, so against a
        // bare fixture the equality would hold for a builder that dropped
        // both names.
        cloned_from: Some(Uuid::from_u128(0xd1_13)),
        cloned_from_version: Some(2),
        // Populated for the same reason, and it arms a different clause:
        // `deprecation_provenance` is one of the four columns §4.3
        // **excludes** from frozen version content, and an exclusion a bare
        // `None` fixture cannot test — the builder would pass by omitting an
        // absent optional. The fixture is deliberately over-populated rather
        // than state-consistent: `lifecycle_state` is excluded from the
        // content too, so a provenance beside a `draft` state costs the
        // roster assertion nothing and buys the exclusion its probe.
        deprecation_provenance: Some(crate::domain::deprecation::Provenance::Cascaded),
        // Populated to arm their INCLUSION: the meter pair is version
        // content (03's declaration freezes at publish), so a `None` here
        // would let a builder that dropped both names pass the roster
        // equality — the same premise as `cloned_from` above.
        metering_unit: Some("gib_month".to_owned()),
        usage_type_ref: Some("usage:storage".to_owned()),
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
    let envelope: serde_json::Value =
        serde_json::from_str(&payload).expect("the door enqueues a JSON envelope");
    // See `products_tests::enqueued_event_body`: the door wraps every body in
    // `events::EventEnvelope` and §4.5's five fields live under `data`.
    envelope["data"].clone()
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
    // Read inline rather than through `enqueued_event_body` because this case
    // wants the *newest* row of two; the body still sits under the envelope's
    // `data`, exactly as that helper unwraps it.
    let after: serde_json::Value =
        serde_json::from_str(&after).expect("the door enqueues a JSON envelope");
    assert_eq!(
        after["data"]["publishedVersion"],
        json!(head_version(&harness.dsn, sku_id).await),
        "the re-publish announces 2, the version it produced, not the 1 the head carried \
         when it began"
    );
    // The same number again, against a literal rather than against a second
    // database read: if `head_version` and the event were both wrong in the
    // same direction, the assertion above would still pass.
    assert_eq!(after["data"]["publishedVersion"], json!(2));
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

/// A gate host that records the mode it was asked in and names a record
/// either way — `products_tests::RecordingGate`'s twin, and deliberately the
/// same shape, since the two doors must behave identically under the same
/// mode.
///
/// The two arms differ the way `inst-fd-gate-mode-gate` and
/// `inst-fd-gate-mode-preauthorized` say a real host's must: under `Gate` it
/// names a record **to consume**, under `PreAuthorized` one already
/// **verified**. `NoMaterialityPolicyGate` can do neither — it holds no
/// record store, so it names no record under `Gate` and refuses outright
/// under `PreAuthorized` — which is why the seam is unreachable without a
/// double.
struct RecordingGate {
    approval: crate::domain::governance::ApprovalId,
    asked: std::sync::Mutex<Vec<crate::domain::governance::GateMode>>,
}

impl RecordingGate {
    fn new(approval: crate::domain::governance::ApprovalId) -> Self {
        Self {
            approval,
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Every mode this host was asked in, in order.
    fn modes(&self) -> Vec<crate::domain::governance::GateMode> {
        self.asked
            .lock()
            .expect("no case poisons this lock")
            .clone()
    }
}

impl crate::domain::governance::GovernanceGate for RecordingGate {
    fn evaluate(
        &self,
        _subject: crate::domain::governance::GateSubject,
        _expected_revision: crate::domain::concurrency::InternalRevision,
        mode: crate::domain::governance::GateMode,
    ) -> Result<crate::domain::governance::GateVerdict, crate::domain::error::DomainError> {
        use crate::domain::governance::{ApprovalDisposition, GateMode, GateVerdict};

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
/// `products_tests::a_preauthorized_publish_reaches_the_host_in_that_mode_and_consumes_nothing`
/// is the same case on the Product door, and its doc carries the full
/// argument: `dod-publish-door` (**P-D-30**) requires the mode to be an
/// explicit argument so that `04-lifecycle`'s scheduled-publish runner can
/// drive **this** door, and until it became one `GateMode::PreAuthorized`
/// had no call path anywhere in the gear.
///
/// The load-bearing assertion is the recorded mode, not the `200`: this
/// double authorizes under both modes, so a door that substituted `Gate`
/// would still publish. The "consumes nothing" half is asserted as far as
/// this slice reaches — the consume flip is `inst-fd-publish-consume`'s and
/// belongs to slice 05's record store — so what is pinned is the property
/// the flip will be written against: `approval_to_consume()` is `None` while
/// `approval_ref()` names the record, and the frozen row carries that id.
#[tokio::test]
async fn a_preauthorized_publish_reaches_the_host_in_that_mode_and_consumes_nothing() {
    use crate::domain::governance::{GateMode, GateVerdict, GovernanceGate as _};

    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let approval = crate::domain::governance::ApprovalId::new(Uuid::now_v7());
    let recorder = Arc::new(RecordingGate::new(approval));

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        etag.parse().expect("the ETag is a valid header value"),
    );
    let response = super::publish_sku_gated(
        &api_state(&harness),
        &flat_in_enforcer(TENANT),
        &authed_ctx(TENANT),
        &headers,
        sku_id,
        &(Arc::clone(&recorder)
            as Arc<dyn crate::domain::governance::GovernanceGate + Send + Sync>),
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

    let verdict = recorder
        .evaluate(
            crate::domain::governance::GateSubject::entity_publish(
                crate::domain::governance::EntityRef {
                    tenant_id: TENANT,
                    entity_kind: bss_products_sdk::models::EntityKind::Sku,
                    entity_id: sku_id,
                },
            ),
            crate::domain::concurrency::InternalRevision::new(1),
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
/// `inst-fd-pipeline-gate-phase` puts the phase at *every* mutating door,
/// passing trivially where the act is ungated (**P-D-34**); §1.1 makes
/// governance a phase *inside* the pipeline rather than a path around it.
/// Under the gear's own host a discard is ungated and the phase is
/// invisible, so this double is the only way to tell a phase that passes
/// from a phase that was never asked. `products_tests
/// ::a_gate_that_answers_no_refuses_the_discard_and_writes_nothing` is the
/// same case on the Product door.
///
/// The assertions are the problem body's own code and the audit row's
/// `error_code`: a status alone does not separate this refusal from the
/// `ILLEGAL_TRANSITION` and `ENTITY_TERMINAL` ones this door already raises.
#[tokio::test]
async fn a_gate_that_answers_no_refuses_the_discard_and_writes_nothing() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        etag.parse().expect("the ETag is a valid header value"),
    );
    let result = super::discard_sku_gated(
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
        panic!("a refusing gate must refuse the discard, which proves it was asked")
    };
    let response = refusal.into_response();
    assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "APPROVAL_REQUIRED is the gate's own 403"
    );
    let view = body_json(response).await;
    assert_eq!(view["context"]["reason"], json!("APPROVAL_REQUIRED"));

    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("draft"),
        "a rejection flips no state (inst-fd-gate-rejection), on this door as on publish"
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        1,
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
/// succeeds and audits nothing.
///
/// The other half of the pair. `inst-fd-pipeline-gate-phase` asks for both
/// halves at once — the phase runs *and* it costs an ungated act nothing —
/// and a case proving only the refusal would leave a door that refuses every
/// discard indistinguishable from a correct one.
#[tokio::test]
async fn the_discard_doors_gate_phase_passes_trivially_under_the_default_host() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = post_discard(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;

    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the default host authorizes naming no record, so an ungated discard pays nothing for \
         the phase"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("discarded")
    );
    assert_eq!(
        raw_i64(&harness.dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await,
        0,
        "a success is not a refusal and audits nothing (P-D-21)"
    );
}

/// Narrow a seeded parent's `region_scope` out of band, bumping
/// `internal_revision` the way the head-row guard demands of **every**
/// admitted update.
///
/// Out of band on purpose: no door of this slice edits a Product's scope —
/// the save door is a later slice's — so the only way to reach the state
/// `04-lifecycle` C5 calls a narrowing is to write it. The guard still
/// judges the write, which makes this helper a positive control too: it
/// proves a bucket-iii narrowing **is** admitted on a non-terminal head,
/// which is precisely why a child can be orphaned between create and
/// publish.
async fn narrow_parent_region(dsn: &str, product_id: Uuid, region: &str) {
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

/// **A SKU whose parent narrowed out from under it is refused
/// `SCOPE_NOT_CONTAINED` at publish.**
///
/// §3.3 puts containment in the identity phase *"wherever it runs — create,
/// save, and **the publish re-run**"*, and §4.1 makes `region_scope` a
/// bucket-iii column *"in both directions, widening and narrowing alike, so
/// a narrowing that would orphan a live child meets
/// `fr-parent-child-integrity`'s fail-closed check ... ahead of the
/// governance gate"*. Nothing freezes a parent's scope when a child is
/// minted under it, so this state is reachable and the publish door is the
/// only thing standing between it and a published orphan.
///
/// The child's own two columns are untouched and still parse, so
/// `SkuScopeColumnsStillParse` — the only identity rule the re-run had
/// before `recheck_parent_containment` — passes: the defect this case pins
/// is a door that judged the child alone and never loaded the parent.
///
/// **The code is the assertion, not the status.** `SCOPE_NOT_CONTAINED` and
/// `INCOMPLETE_ENTITY` both render wire `400`, so a status assertion would
/// pass against a door that refused for the wrong reason entirely. The audit
/// row's `error_code` is asserted for the same reason.
#[tokio::test]
async fn a_publish_whose_parent_narrowed_out_of_band_is_refused_scope_not_contained() {
    let harness = harness().await;
    let mut parent = new_parent_product(Uuid::now_v7(), TENANT);
    parent.region_scope = "eu,us".to_owned();
    let parent_id = seed_parent(&harness, parent).await;

    // The child inherits `eu,us` (the omitted-scope arm of P-D-39), so it is
    // contained at create and the create door is not what this case tests.
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    narrow_parent_region(&harness.dsn, parent_id, "eu").await;

    let response = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;

    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "SCOPE_NOT_CONTAINED renders as an architectural 422, wire 400"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("SCOPE_NOT_CONTAINED"),
        "the refusal names containment, not INCOMPLETE_ENTITY: the child's own columns are \
         intact and it is the parent that moved"
    );

    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        0,
        "an orphaned child freezes no version"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("draft"),
        "and the head never leaves draft"
    );
    let error_code = raw_string_opt(
        &harness.dsn,
        "SELECT error_code AS v FROM products_audit_log",
    )
    .await;
    assert_eq!(
        error_code.as_deref(),
        Some("SCOPE_NOT_CONTAINED"),
        "the refusal is audited under the code it raised"
    );
}

/// **A SKU whose parent went terminal after the create is refused
/// `PARENT_TERMINAL` at publish.**
///
/// `create_sku` already refuses a terminal parent; this is the same question
/// asked a second time, of a row that has since moved. Without the publish
/// re-check a SKU minted under a live parent publishes under a `retired`
/// one, which is a live child of a dead parent —
/// `fr-parent-child-integrity`'s own case.
///
/// `PARENT_TERMINAL` rather than `SCOPE_NOT_CONTAINED`: the two are separate
/// codes with separate meanings, and answering the containment code here
/// would tell an operator the scopes disagreed when the scopes are fine.
#[tokio::test]
async fn a_publish_whose_parent_went_terminal_is_refused_parent_terminal() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
    walk_parent_to(&harness.db, &scope, parent_id, "retired").await;

    let response = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;

    assert_eq!(
        response.status(),
        StatusCode::CONFLICT,
        "PARENT_TERMINAL is a 409, which is what tells it from SCOPE_NOT_CONTAINED's wire 400"
    );
    let view = body_json(response).await;
    assert_eq!(
        view["context"]["reason"],
        json!("PARENT_TERMINAL"),
        "a retired parent is a terminal-parent refusal, not a containment one; the create door          words the same refusal the same way"
    );
    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        0,
        "nothing is frozen under a dead parent"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("draft")
    );
}

/// A gate host that authorizes and **raises the uncomposed-bundle override**.
///
/// [`RefusingGate`] and [`RecordingGate`] both leave
/// `GateAuthorization::uncomposed_bundle_override` `false` — `RecordingGate`
/// passes the literal, and `NoMaterialityPolicyGate` cannot raise it at all,
/// holding no record store — so the raised state is unreachable without a
/// double of its own. `inst-fd-publish-freeze` makes that state the operand of
/// `composition_pending`, so this is the only way to exercise the door's write
/// of it before slice 05's real host lands.
struct OverridingGate;

impl crate::domain::governance::GovernanceGate for OverridingGate {
    fn evaluate(
        &self,
        _subject: crate::domain::governance::GateSubject,
        _expected_revision: crate::domain::concurrency::InternalRevision,
        _mode: crate::domain::governance::GateMode,
    ) -> Result<crate::domain::governance::GateVerdict, crate::domain::error::DomainError> {
        Ok(crate::domain::governance::GateVerdict::authorized(
            crate::domain::governance::ApprovalDisposition::NoRecord,
            true,
            "this double authorizes and carries the uncomposed-bundle override".to_owned(),
        ))
    }
}

/// The `content` string of one entity's frozen version row.
async fn frozen_content(dsn: &str, sku_id: Uuid, version: i64) -> String {
    raw_string_opt(
        dsn,
        &format!(
            "SELECT content AS v FROM products_entity_version \
             WHERE {} AND entity_kind = 'sku' AND published_version = {version}",
            id_matches("entity_id", sku_id)
        ),
    )
    .await
    .expect("the publish this case ran froze a row at this version")
}

/// One SKU head's `composition_pending`, as the column stores it.
async fn head_composition_pending(dsn: &str, sku_id: Uuid) -> i64 {
    raw_i64(
        dsn,
        &format!(
            "SELECT composition_pending AS v FROM products_sku WHERE {}",
            id_matches("sku_id", sku_id)
        ),
    )
    .await
}

/// Drive [`super::publish_sku_gated`] against `gate` in `GateMode::Gate`.
///
/// The routed handler wires `NoMaterialityPolicyGate` as a literal — that is
/// `inst-fd-gate-mode`'s wire-invisibility holding — so a case that needs a
/// different host has to enter through the in-process seam, exactly as
/// `a_preauthorized_publish_reaches_the_host_in_that_mode_and_consumes_nothing`
/// does. Everything else is what a routed request would produce.
async fn publish_under_gate(
    harness: &TestHarness,
    sku_id: Uuid,
    etag: &str,
    gate: Arc<dyn crate::domain::governance::GovernanceGate + Send + Sync>,
) -> axum::http::Response<Body> {
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::IF_MATCH,
        etag.parse().expect("the ETag is a valid header value"),
    );
    super::publish_sku_gated(
        &api_state(harness),
        &flat_in_enforcer(TENANT),
        &authed_ctx(TENANT),
        &headers,
        sku_id,
        &gate,
        crate::domain::governance::GateMode::Gate,
    )
    .await
    .expect("this case's gate authorizes, so the publish must land")
    .into_response()
}

/// **A publish carrying the uncomposed-bundle override raises
/// `composition_pending` on the head *and* freezes the raised flag under the
/// version that publish produced.**
///
/// `inst-fd-publish-freeze` (§4.2, **P-D-32**): *"On a `bundle` SKU that same
/// `UPDATE` also carries `composition_pending` — set where this publish
/// carried the uncomposed-bundle override, cleared where it did not"*. The
/// operand is `GateAuthorization::uncomposed_bundle_override`, which had no
/// reader in the gear at all until this wave.
///
/// # The frozen row is the load-bearing assertion, not the head
///
/// The head column alone would pass against the defect this case exists to
/// catch. `composition_pending` is on `super::SKU_VERSION_CONTENT_ROSTER`, and
/// the freeze is taken over `super::post_publish_image`; a door that carried
/// the flag into `repo::publish_sku_head`'s `UPDATE` but not into that image
/// would write a correct head **and** freeze the **pre-act** flag under the
/// **post-act** version's key. The digest over that content would be perfectly
/// valid — the row would agree with itself and lie only about the act that
/// produced it — so no digest check, no restore drill and no other case in
/// this file would notice. Only reading the stored `content` back does.
///
/// It is read out of `products_entity_version` rather than off the door's own
/// in-memory image for the same reason: an assertion against the value the
/// door computed cannot tell a value that reached storage from one that did
/// not.
#[tokio::test]
async fn a_publish_carrying_the_uncomposed_bundle_override_freezes_the_raised_flag() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    assert_eq!(
        head_composition_pending(&harness.dsn, sku_id).await,
        0,
        "this case's own premise: the column's default is the unraised state, so a door that \
         wrote nothing would leave it here"
    );

    let response = publish_under_gate(&harness, sku_id, &etag, Arc::new(OverridingGate)).await;
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        head_composition_pending(&harness.dsn, sku_id).await,
        1,
        "the head-row UPDATE carries the flag the verdict raised; the migration's own guard \
         admits it in that statement and in no other"
    );

    let content = frozen_content(&harness.dsn, sku_id, 1).await;
    assert!(
        content.contains(r#""composition_pending":true"#),
        "the freeze is taken over the post-act image, so the version this publish produced \
         carries the flag this publish raised; the stored content was {content}"
    );
}

/// **A publish whose verdict carries no override freezes the cleared flag** —
/// the other half of `inst-fd-publish-freeze`'s *"cleared where it did not"*,
/// and the control that keeps the case above from passing against a door that
/// hard-codes `true`.
///
/// It runs through the router, so the host is the gear's own
/// `NoMaterialityPolicyGate`, whose authorization carries
/// `uncomposed_bundle_override: false` by construction.
///
/// The `false` value is also what the column already held, so this case cannot
/// tell a door that wrote `false` from one that wrote nothing — that
/// discrimination is the raised case's, and this one exists to pin that the
/// ordinary path did not become the raised one.
#[tokio::test]
async fn a_publish_without_the_override_freezes_the_cleared_flag() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(response.status(), StatusCode::OK);

    assert_eq!(
        head_composition_pending(&harness.dsn, sku_id).await,
        0,
        "no override was granted, so the flag stays cleared"
    );
    let content = frozen_content(&harness.dsn, sku_id, 1).await;
    assert!(
        content.contains(r#""composition_pending":false"#),
        "the frozen content states the cleared flag rather than omitting it, Absence::Null's \
         roster naming the field either way; the stored content was {content}"
    );
}

/// `PATCH /bss-products/v1/skus/{id}` with `body` and `headers` — the save
/// door's request shape.
///
/// A separate helper from [`post_head_act`] rather than a seventh parameter
/// on it: a save is the only one of the three head acts that carries a
/// request body, and folding a body into the bodiless helper would let a
/// later case send one to `publish` without the compiler minding.
async fn patch_sku(
    app: Router,
    tenant: Uuid,
    sku_id: Uuid,
    body: &serde_json::Value,
    headers: &[(&str, &str)],
) -> axum::http::Response<Body> {
    let mut builder = Request::builder()
        .method("PATCH")
        .uri(format!("/bss-products/v1/skus/{sku_id}"))
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .extension(authed_ctx(tenant));
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    app.oneshot(
        builder
            .body(Body::from(body.to_string()))
            .expect("build the save request"),
    )
    .await
    .expect("the router answers")
}

/// [`patch_sku`] carrying only an `If-Match` — the shape most cases below
/// send.
async fn save_sku_at(
    harness: &TestHarness,
    sku_id: Uuid,
    if_match: &str,
    body: &serde_json::Value,
) -> axum::http::Response<Body> {
    patch_sku(
        app_for(harness, TENANT),
        TENANT,
        sku_id,
        body,
        &[("If-Match", if_match)],
    )
    .await
}

/// The `ETag` a caller holding `revision` would send back, built the way the
/// read door builds the one it hands out.
fn if_match_for(revision: i64) -> String {
    preconditions::etag(InternalRevision::new(revision))
}

/// One SKU head's `sku_code`.
async fn head_sku_code(dsn: &str, sku_id: Uuid) -> Option<String> {
    raw_string_opt(
        dsn,
        &format!(
            "SELECT sku_code AS v FROM products_sku WHERE {}",
            id_matches("sku_id", sku_id)
        ),
    )
    .await
}

/// One SKU head's `region_scope`.
async fn head_region_scope(dsn: &str, sku_id: Uuid) -> Option<String> {
    raw_string_opt(
        dsn,
        &format!(
            "SELECT region_scope AS v FROM products_sku WHERE {}",
            id_matches("sku_id", sku_id)
        ),
    )
    .await
}

/// **A bucket-iii save on a `draft` head is admitted and moves
/// `internal_revision` by exactly one.**
///
/// "By exactly one" is what the head-row guard makes load-bearing: it refuses
/// any `UPDATE` whose `internal_revision` is not `OLD + 1`, so a save split
/// across two statements would move it twice and the `ETag` this door just
/// handed back would skip a value it never returned.
#[tokio::test]
async fn a_bucket_iii_sku_save_on_a_draft_is_admitted_and_bumps_the_revision_once() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = save_sku_at(&harness, sku_id, &etag, &json!({ "region_scope": "eu" })).await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a bucket-iii save on a non-terminal head is an ordinary admitted write"
    );

    assert_eq!(
        head_region_scope(&harness.dsn, sku_id).await.as_deref(),
        Some("eu"),
        "the routed column was written"
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        2,
        "one admitted UPDATE moves the revision by exactly one"
    );
    assert_eq!(
        head_version(&harness.dsn, sku_id).await,
        0,
        "a save moves no version counter"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("draft"),
        "a save takes no edge"
    );
    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        0,
        "a save writes no products_entity_version row"
    );
}

/// **A bucket-iii save on a `published` head is admitted too, writes no
/// version row and does not move `published_version`.**
///
/// §4.1 admits a bucket-iii write on any non-terminal head, published or not.
/// The two negative assertions are what separate this door from the publish
/// door: the head is the authoring surface in every non-terminal state
/// (`inst-fd-transition-guard`), and the version row is the publish act's
/// alone.
#[tokio::test]
async fn a_bucket_iii_sku_save_on_a_published_head_writes_no_version_row() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let published = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(
        published.status(),
        StatusCode::OK,
        "this case's own premise: the draft publishes"
    );
    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        1,
        "this case's own premise: the publish froze exactly one version"
    );

    let response = save_sku_at(
        &harness,
        sku_id,
        &if_match_for(2),
        &json!({ "region_scope": "eu" }),
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a published SKU's scope columns are still bucket iii (section 4.1)"
    );

    assert_eq!(
        head_region_scope(&harness.dsn, sku_id).await.as_deref(),
        Some("eu")
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        3,
        "the save bumped the revision the publish left at 2"
    );
    assert_eq!(
        head_version(&harness.dsn, sku_id).await,
        1,
        "a save does not move published_version"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("published"),
        "a save takes no edge, so the state the publish set stands"
    );
    assert_eq!(
        frozen_versions_for(&harness.dsn, sku_id).await,
        1,
        "still the publish's one row: a save freezes nothing"
    );
}

/// **A bucket-i save before first publish is admitted; after first publish
/// the same field is `ILLEGAL_FIELD_MUTATION`.**
///
/// One case for both halves: the refusal alone passes against a door that
/// refuses every bucket-i write, and the admission alone against one that
/// admits every bucket-i write. Only the pair pins the rule, which is keyed
/// to `published_version`.
///
/// The assertion is the **problem body's own code**, not the status: §3.3
/// renders `ILLEGAL_FIELD_MUTATION`, `STALE_REVISION` and `ENTITY_TERMINAL`
/// all as `409`.
#[tokio::test]
async fn a_bucket_i_sku_save_is_admitted_before_first_publish_and_refused_after_it() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let admitted = save_sku_at(&harness, sku_id, &etag, &json!({ "sku_code": "SKU-900" })).await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "published_version is 0, so identity is still writable"
    );
    assert_eq!(
        head_sku_code(&harness.dsn, sku_id).await.as_deref(),
        Some("SKU-900"),
        "the bucket-i column was written"
    );

    let published = post_publish(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        Some(&if_match_for(2)),
    )
    .await;
    assert_eq!(
        published.status(),
        StatusCode::OK,
        "this case's own premise: the draft publishes"
    );

    let refused = save_sku_at(
        &harness,
        sku_id,
        &if_match_for(3),
        &json!({ "sku_code": "SKU-000" }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(refused).await["context"]["reason"],
        json!("ILLEGAL_FIELD_MUTATION"),
        "a bucket-i write after first publish is refused by its own rule, not by a stale \
         precondition or a terminal state"
    );

    assert_eq!(
        head_sku_code(&harness.dsn, sku_id).await.as_deref(),
        Some("SKU-900"),
        "the refused save wrote nothing"
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        3,
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
/// `name` is the sharpest case this door has: it is a **Product** bucket-iii
/// column and `products_sku` has none, so a door that keyed the registry on
/// the column name alone would route it to the Product's tag and admit a
/// write to a column that does not exist. `bucket::classify` is keyed by
/// entity *and* column and answers a miss here.
///
/// The positive control is the point of the second half: a door that refused
/// every field would pass the first assertion alone.
#[tokio::test]
async fn a_field_the_sku_registry_does_not_name_is_refused_by_the_fail_closed_miss() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let refused = save_sku_at(&harness, sku_id, &etag, &json!({ "name": "Fibre 900" })).await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(refused).await["context"]["reason"],
        json!("ILLEGAL_FIELD_MUTATION"),
        "a miss is a refusal, never a default bucket and never the Product's tag"
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        1,
        "the refused save wrote nothing at all"
    );

    let admitted = save_sku_at(&harness, sku_id, &etag, &json!({ "region_scope": "eu" })).await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "the positive control: this door does not refuse everything"
    );
}

/// **A save whose second field is refused applies neither.**
///
/// Routing runs over the whole request before any column is written, so a
/// `PATCH` naming one admitted field and one refused one is refused whole. A
/// door that routed and wrote field by field would leave the head carrying
/// half a request the caller was told had failed.
#[tokio::test]
async fn a_sku_save_with_one_refused_field_applies_none_of_the_others() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let published = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(published.status(), StatusCode::OK);

    let refused = save_sku_at(
        &harness,
        sku_id,
        &if_match_for(2),
        &json!({ "region_scope": "eu", "sku_code": "SKU-000" }),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(refused).await["context"]["reason"],
        json!("ILLEGAL_FIELD_MUTATION")
    );

    assert_eq!(
        head_region_scope(&harness.dsn, sku_id).await.as_deref(),
        Some(""),
        "the admitted field in the same request was not applied either"
    );
    assert_eq!(
        head_sku_code(&harness.dsn, sku_id).await.as_deref(),
        Some("SKU-500"),
        "nor, obviously, the refused one"
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        2,
        "and no counter moved"
    );
}

/// **A stale `If-Match` is `STALE_REVISION` and writes nothing**, with its
/// own audit row under this act's token.
#[tokio::test]
async fn a_sku_save_with_a_stale_if_match_is_refused_and_writes_nothing() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, _etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = save_sku_at(
        &harness,
        sku_id,
        &if_match_for(7),
        &json!({ "region_scope": "eu" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(response).await["context"]["reason"],
        json!("STALE_REVISION"),
        "the body's code is what tells this refusal from the two other 409s"
    );

    assert_eq!(
        head_region_scope(&harness.dsn, sku_id).await.as_deref(),
        Some(""),
        "nothing was written"
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        1,
        "and no counter moved"
    );
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
/// there is nothing to be stale.
#[tokio::test]
async fn a_sku_save_without_if_match_is_refused_validation() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, _etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = patch_sku(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        &json!({ "region_scope": "eu" }),
        &[],
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "an absent precondition rides VALIDATION, which renders 400"
    );
    assert_eq!(
        head_region_scope(&harness.dsn, sku_id).await.as_deref(),
        Some(""),
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
/// The terminal head is produced by this module's own discard door rather
/// than written by hand, so the case also proves the doors compose.
#[tokio::test]
async fn a_sku_save_on_a_terminal_head_is_refused_entity_terminal() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let discarded = post_discard(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
    assert_eq!(
        discarded.status(),
        StatusCode::OK,
        "this case's own premise: the draft discards cleanly"
    );

    let response = save_sku_at(
        &harness,
        sku_id,
        &if_match_for(2),
        &json!({ "region_scope": "eu" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert_eq!(
        body_json(response).await["context"]["reason"],
        json!("ENTITY_TERMINAL"),
        "no head write is admitted on a discarded entity"
    );
    assert_eq!(
        head_region_scope(&harness.dsn, sku_id).await.as_deref(),
        Some(""),
        "and the terminal row is unchanged"
    );
}

/// **A replayed save under the same key returns the stored answer and does
/// not save twice.**
///
/// The case the store exists for, on this door: a client whose save committed
/// and whose response was lost retries under the key it still holds and with
/// the `If-Match` it still holds — a precondition stale **by construction**,
/// since the act it never learned about moved the revision. A door that
/// judged the precondition before the claim would refuse this retry
/// `STALE_REVISION` and never reach the stored answer.
///
/// "Does not save twice" is asserted on the revision rather than the status:
/// a door that re-ran the mutation and happened to answer `200` would pass a
/// status-only assertion while bumping the revision a second time.
#[tokio::test]
async fn a_replayed_sku_save_serves_the_stored_answer_and_does_not_save_twice() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let body = json!({ "region_scope": "eu" });
    let headers = [
        ("If-Match", etag.as_str()),
        ("Idempotency-Key", "sku-save-key-1"),
    ];

    let first = patch_sku(app_for(&harness, TENANT), TENANT, sku_id, &body, &headers).await;
    assert_eq!(first.status(), StatusCode::OK);
    let original = body_json(first).await;

    let retry = patch_sku(app_for(&harness, TENANT), TENANT, sku_id, &body, &headers).await;
    assert_eq!(
        retry.status(),
        StatusCode::OK,
        "the retry replays the stored answer rather than being refused for its now-stale \
         precondition"
    );
    assert_eq!(
        body_json(retry).await,
        original,
        "byte for byte the first answer, not a re-render"
    );

    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        2,
        "one save, one bump: the replay executed nothing"
    );
    assert_eq!(
        idempotency_rows_for(&harness.dsn, "sku-save-key-1").await,
        1,
        "one claim, answered in the mutation's own transaction"
    );
}

/// **A save naming no field at all is refused `VALIDATION`.**
///
/// A bare `internal_revision` bump is a write with no content that still
/// invalidates every `ETag` a client holds, so it is refused at the door
/// rather than admitted as a no-op.
#[tokio::test]
async fn a_sku_save_naming_no_field_is_refused_validation() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let response = save_sku_at(&harness, sku_id, &etag, &json!({})).await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        1,
        "no bump for a request that named nothing"
    );
}

/// **A save that widens a child's scope out of its parent's is refused
/// `SCOPE_NOT_CONTAINED`, and a contained one beside it saves.**
///
/// §3.3 puts containment in the identity phase *"wherever it runs — create,
/// **save**, and the publish re-run"*, and §4.1 puts the two scope columns in
/// bucket iii *"in both directions, widening and narrowing alike"*. A save is
/// therefore the one door that can widen a child out of its parent, and the
/// re-check is over the image the save **would** store rather than the one it
/// replaces — a check against the stored value would pass every widening by
/// construction.
///
/// The Product save door has no analogue and correctly asks nothing: a
/// Product has no parent, so containment has no second operand there. The
/// asymmetry is the schema's, the same one the publish doors already carry.
#[tokio::test]
async fn a_sku_save_widening_out_of_the_parents_scope_is_refused() {
    let harness = harness().await;
    let mut parent = new_parent_product(Uuid::now_v7(), TENANT);
    parent.region_scope = "eu".to_owned();
    let parent_id = seed_parent(&harness, parent).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let refused = save_sku_at(&harness, sku_id, &etag, &json!({ "region_scope": "us" })).await;
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "SCOPE_NOT_CONTAINED is one of the taxonomy's architectural 422s, rendered 400"
    );
    assert_eq!(
        body_json(refused).await["context"]["violations"][0]["type"],
        json!("SCOPE_NOT_CONTAINED"),
        "SCOPE_NOT_CONTAINED is one of the taxonomy's architectural 422s and renders as a \
         violation entry rather than a `reason`, exactly as it does at the create door"
    );
    assert_eq!(
        head_region_scope(&harness.dsn, sku_id).await.as_deref(),
        Some("eu"),
        "the child kept the scope it inherited at create; nothing was written"
    );

    let admitted = save_sku_at(&harness, sku_id, &etag, &json!({ "brand_scope": "acme" })).await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "the positive control: a save whose image is still contained is admitted"
    );
}

/// **A save storing a scope column with an empty token is refused
/// `VALIDATION`, and a well-formed list beside it saves.**
///
/// `crate::domain::containment::ResolvedScope::parse`'s own rule, which the
/// create door already applies to a payload and which this door applies to
/// the other way a stored scope can change. `products_tests::
/// a_save_storing_a_scope_with_an_empty_token_is_refused_validation` is the
/// Product twin, and it records why an unparseable *stored* scope is worse
/// than a bad request: the parent's copy is read on every child publish.
#[tokio::test]
async fn a_sku_save_storing_a_scope_with_an_empty_token_is_refused_validation() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let refused = save_sku_at(
        &harness,
        sku_id,
        &etag,
        &json!({ "brand_scope": "acme,,globex" }),
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "an empty token between separators is refused rather than silently filtered"
    );
    assert_eq!(
        head_revision(&harness.dsn, sku_id).await,
        1,
        "nothing was written and no counter moved"
    );

    let admitted = save_sku_at(
        &harness,
        sku_id,
        &etag,
        &json!({ "brand_scope": "acme,globex" }),
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "the positive control: a well-formed list is an ordinary bucket-iii save"
    );
}

/// **A save whose `sku_code` is padded stores the trimmed value, so the
/// reservation another live SKU holds still collides.**
///
/// `uq_products_sku_code` is a byte-comparing partial unique index over
/// `(tenant_id, sku_code)`, so `" SKU-1 "` and `"SKU-1"` are two different
/// reservations to the database. A save that stored the caller's padding
/// would therefore be admitted beside a live holder of the code, leaving two
/// rows holding what an operator reads as one `skuCode` — and one of them a
/// value no create door could produce, since `create_sku` trims. The
/// assertion is the refusal *plus* the stored value of the row that did save,
/// because a door that trimmed only for the collision check and stored the
/// padding would pass a status-only assertion.
#[tokio::test]
async fn a_save_of_a_padded_sku_code_collides_with_the_held_reservation() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (_holder_id, _holder_etag) = seed_draft_sku(&harness, parent_id, "SKU-1").await;
    let (mover_id, mover_etag) = seed_draft_sku(&harness, parent_id, "SKU-2").await;

    let refused = save_sku_at(
        &harness,
        mover_id,
        &mover_etag,
        &json!({ "sku_code": " SKU-1 " }),
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::CONFLICT,
        "the padded code is the held code, so the save loses the reservation race"
    );
    assert_eq!(
        body_json(refused).await["context"]["reason"],
        json!("DUPLICATE_CODE"),
        "and it loses it as a code collision, not as some other 409"
    );
    assert_eq!(
        head_sku_code(&harness.dsn, mover_id).await.as_deref(),
        Some("SKU-2"),
        "the refused save wrote nothing"
    );

    let admitted = save_sku_at(
        &harness,
        mover_id,
        &mover_etag,
        &json!({ "sku_code": "  SKU-3  " }),
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::OK,
        "the positive control: a padded code nobody holds is an ordinary save"
    );
    assert_eq!(
        head_sku_code(&harness.dsn, mover_id).await.as_deref(),
        Some("SKU-3"),
        "and it is stored trimmed, exactly as the create door stores one"
    );
}

/// [`enqueued_event_body`]'s reader for the case where a case enqueued the
/// same `payload_type` more than once: the **newest** row rather than the one
/// row that helper's doc assumes.
///
/// The whole of it is the shared envelope reader plus §4.5's `data` unwrap —
/// which is what it always was, spelled out rather than re-implemented.
async fn newest_enqueued_event_body(dsn: &str, payload_type: &str) -> serde_json::Value {
    enqueued_event_envelope(dsn, payload_type).await["data"].clone()
}

/// **A save enqueues exactly one `SkuHeadSaved` row, and its body carries the
/// revision the act committed and the state the head is actually in.**
///
/// [`crate::api::rest::products_tests
/// ::a_save_enqueues_one_product_head_saved_carrying_the_committed_revision_and_state`]'s
/// twin, and the argument is that case's: `inst-fd-save-txn` makes the outbox
/// row a clause of the save, §4.5 puts `SkuHeadSaved` in the roster of eight,
/// and no other case in this file reads the outbox after a save, so deleting
/// the enqueue leaves every one of them green.
///
/// The literal `"SkuHeadSaved"` is asserted rather than the constant, because
/// the token is what a consumer subscribes on and a test written against the
/// constant renames with it. The revision is asserted as the **post**-bump
/// value and against the head read back (P-D-29's *"the value as committed by
/// the act"*), and the case saves twice — once on a `draft` head and once
/// after it has published — so that a `lifecycleState` that was hard-coded
/// rather than read off the head fails the second half.
#[tokio::test]
async fn a_save_enqueues_one_sku_head_saved_carrying_the_committed_revision_and_state() {
    let harness = harness().await;
    let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
    let (sku_id, etag) = seed_draft_sku(&harness, parent_id, "SKU-500").await;

    let first = save_sku_at(&harness, sku_id, &etag, &json!({ "region_scope": "eu" })).await;
    assert_eq!(
        first.status(),
        StatusCode::OK,
        "this case's own premise: the bucket-iii save on a draft is admitted"
    );

    assert_eq!(
        enqueued_event_count(&harness.dsn, "SkuHeadSaved").await,
        1,
        "one admitted save enqueues exactly one SkuHeadSaved row, no more and no fewer"
    );
    assert_eq!(
        enqueued_event_count(&harness.dsn, "SkuCreated").await,
        1,
        "the control on the filter: the seed's own create row is there under its own token"
    );

    let body = enqueued_event_body(&harness.dsn, "SkuHeadSaved").await;
    assert_eq!(body["entityKind"], json!("sku"));
    assert_eq!(body["entityId"], json!(sku_id.to_string()));
    assert_eq!(body["tenantId"], json!(TENANT.to_string()));
    assert_eq!(
        body["internalRevision"],
        json!(head_revision(&harness.dsn, sku_id).await),
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

    let published = post_publish(
        app_for(&harness, TENANT),
        TENANT,
        sku_id,
        Some(&if_match_for(2)),
    )
    .await;
    assert_eq!(
        published.status(),
        StatusCode::OK,
        "this case's own premise: the draft publishes"
    );

    let second = save_sku_at(
        &harness,
        sku_id,
        &if_match_for(3),
        &json!({ "region_scope": "apac" }),
    )
    .await;
    assert_eq!(
        second.status(),
        StatusCode::OK,
        "section 4.1 admits a bucket-iii save on a published head"
    );

    assert_eq!(
        enqueued_event_count(&harness.dsn, "SkuHeadSaved").await,
        2,
        "the second save announced once too, and the publish's own row is not one of these"
    );

    let body = newest_enqueued_event_body(&harness.dsn, "SkuHeadSaved").await;
    assert_eq!(
        body["internalRevision"],
        json!(head_revision(&harness.dsn, sku_id).await),
        "the newest row is the second save's, and it announces its own committed revision"
    );
    assert_eq!(body["internalRevision"], json!(4));
    assert_eq!(
        body["lifecycleState"],
        json!("published"),
        "the discriminator is read off the head, so a save on a published head says so"
    );
    assert_eq!(
        head_state(&harness.dsn, sku_id).await.as_deref(),
        Some("published"),
        "and that reading agrees with the head itself"
    );
}

/// **The envelope P-D-01 requires comes out of this door too** — the twin of
/// `products_tests::a_created_events_envelope_carries_the_four_obligations_from_the_door`.
///
/// The two doors were built by parallel slices and diverged six times in
/// Phase 6, every divergence defended by its own prose. This pair is the
/// guard against a seventh: whatever the Product door's envelope carries,
/// this one carries, under the same names, with only the schema reference
/// differing.
#[tokio::test]
async fn a_created_events_envelope_carries_the_four_obligations_from_the_door() {
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

    let envelope = enqueued_event_envelope(&harness.dsn, "SkuCreated").await;

    assert_eq!(
        envelope["schemaRef"], "bss-products.SkuCreated.v1.0.0",
        "the SKU door announces its own event's schema, not the Product door's"
    );
    let event_id = envelope["eventId"]
        .as_str()
        .expect("the interim envelope must carry its own event id");
    assert!(
        Uuid::parse_str(event_id).is_ok(),
        "the event id must be a UUID, not a placeholder: {event_id}"
    );

    // The ref the identity map minted for the acting principal, not a fresh
    // one. See the Product twin for why presence alone would not catch a door
    // that invented it.
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

    assert!(
        envelope.get("causationId").is_none(),
        "an operator-caused event must name no causing event"
    );
    assert!(
        envelope.get("correlationId").is_none(),
        "an untraced request must leave the correlation id off the wire"
    );

    assert_eq!(envelope["data"]["entityKind"], "sku");
    assert_eq!(envelope["data"]["internalRevision"], 1);
    assert_eq!(envelope["data"]["lifecycleState"], "draft");
}

// ---------------------------------------------------------------------------
// The lone-SKU clone door (`inst-cn-door`, P-D-75, P-D-62, P-D-76)

mod clone_door_tests {
    use super::*;

    /// `POST /bss-products/v1/skus/{id}/clone` with `body`.
    async fn post_clone(
        app: Router,
        tenant: Uuid,
        sku_id: Uuid,
        body: &serde_json::Value,
        headers: &[(&str, &str)],
    ) -> axum::http::Response<Body> {
        let mut request = Request::builder()
            .method("POST")
            .uri(format!("/bss-products/v1/skus/{sku_id}/clone"))
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

    /// The stored head of one SKU, read back for lineage assertions.
    async fn sku_head_of(harness: &TestHarness, sku_id: Uuid) -> repo::SkuRecord {
        let conn = harness
            .db
            .conn()
            .expect("checkout the pinned production connection");
        let scope = toolkit_db::secure::AccessScope::for_tenant(TENANT);
        repo::find_sku(&conn, &scope, TENANT, sku_id)
            .await
            .expect("read the SKU head")
            .expect("the SKU exists")
    }

    /// A draft source clones with the first-free code suggestion
    /// (`{source}-copy-1`), the parent link copied, and the lineage pair
    /// written with the head-read sentinel (P-D-76).
    #[tokio::test]
    async fn a_draft_source_clones_with_the_first_free_code() {
        let harness = harness().await;
        let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
        let (source_id, _etag) = seed_draft_sku(&harness, parent_id, "LINE-1").await;

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            &json!({}),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED, "a clone is a 201");
        let view = body_json(response).await;
        assert_eq!(
            view["sku_code"],
            json!("LINE-1-copy-1"),
            "the suggested code is the first free of the -copy family"
        );
        assert_eq!(
            view["product_id"],
            json!(parent_id),
            "a lone clone copies the parent link"
        );
        assert_eq!(view["lifecycle_state"], json!("draft"));

        let clone_id = Uuid::parse_str(view["sku_id"].as_str().expect("sku_id"))
            .expect("the clone's id is a uuid");
        let head = sku_head_of(&harness, clone_id).await;
        assert_eq!(head.cloned_from, Some(source_id), "the immediate source");
        assert_eq!(
            head.cloned_from_version, None,
            "a draft source is read at its head, and NULL is that sentinel"
        );
    }

    /// A second clone of the same source walks to `-copy-2`: the first free
    /// integer is decided by the reservation, not by a counter (P-D-62).
    #[tokio::test]
    async fn a_second_clone_walks_to_copy_two() {
        let harness = harness().await;
        let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
        let (source_id, _etag) = seed_draft_sku(&harness, parent_id, "LINE-2").await;

        let first = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            &json!({}),
            &[],
        )
        .await;
        assert_eq!(first.status(), StatusCode::CREATED);
        let second = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            &json!({}),
            &[],
        )
        .await;
        assert_eq!(second.status(), StatusCode::CREATED);
        let view = body_json(second).await;
        assert_eq!(
            view["sku_code"],
            json!("LINE-2-copy-2"),
            "the walk moved to the next free integer under the reservation"
        );
    }

    /// A published source is read at its last frozen version: the clone pins
    /// `cloned_from_version = 1` and suggests from the frozen code — the
    /// read went through the version row and the canonical decoder
    /// (P-D-77), not the head.
    #[tokio::test]
    async fn a_published_source_clones_at_its_frozen_version() {
        let harness = harness().await;
        let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
        let (source_id, etag) = seed_draft_sku(&harness, parent_id, "LINE-3").await;

        let publish = post_head_act(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            "publish",
            Some(&etag),
            None,
        )
        .await;
        assert_eq!(
            publish.status(),
            StatusCode::OK,
            "premise: the source publishes"
        );

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            &json!({}),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let view = body_json(response).await;
        assert_eq!(
            view["sku_code"],
            json!("LINE-3-copy-1"),
            "the suggestion derives from the frozen code"
        );
        let clone_id = Uuid::parse_str(view["sku_id"].as_str().expect("sku_id"))
            .expect("the clone's id is a uuid");
        let head = sku_head_of(&harness, clone_id).await;
        assert_eq!(
            head.cloned_from_version,
            Some(1),
            "the lineage pins exactly the frozen version the content was read at"
        );
    }

    /// A `discarded` source is refused with the minted code (P-D-75): 409,
    /// `CLONE_SOURCE_DISCARDED` in the canonical context, nothing created.
    #[tokio::test]
    async fn a_discarded_source_is_refused_with_the_minted_code() {
        let harness = harness().await;
        let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
        let (source_id, etag) = seed_draft_sku(&harness, parent_id, "LINE-4").await;

        let discard = post_head_act(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            "discard",
            Some(&etag),
            None,
        )
        .await;
        assert_eq!(
            discard.status(),
            StatusCode::OK,
            "premise: the source discards"
        );

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            &json!({}),
            &[],
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::CONFLICT,
            "the state refuses the act, not the address"
        );
        let body = body_json(response).await;
        assert_eq!(
            body["context"]["reason"],
            json!("CLONE_SOURCE_DISCARDED"),
            "the caller learns exactly what stands in the way"
        );
    }

    /// A collision on an operator-supplied code is the ordinary
    /// `DUPLICATE_CODE` — only the *suggested* code walks first-free.
    #[tokio::test]
    async fn an_overridden_code_collision_is_the_ordinary_refusal() {
        let harness = harness().await;
        let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
        let (source_id, _etag) = seed_draft_sku(&harness, parent_id, "LINE-5").await;
        let (_holder, _etag2) = seed_draft_sku(&harness, parent_id, "TAKEN-5").await;

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            &json!({ "code": "TAKEN-5" }),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = body_json(response).await;
        assert_eq!(
            body["context"]["reason"],
            json!("DUPLICATE_CODE"),
            "the operator's own collision is never walked past"
        );
    }

    /// A keyed retry replays the first clone rather than minting a second
    /// (P-D-75 arm 5): same id, same code, one row.
    #[tokio::test]
    async fn a_keyed_retry_replays_the_first_clone() {
        let harness = harness().await;
        let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
        let (source_id, _etag) = seed_draft_sku(&harness, parent_id, "LINE-6").await;

        let first = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            &json!({}),
            &[("Idempotency-Key", "clone-once")],
        )
        .await;
        assert_eq!(first.status(), StatusCode::CREATED);
        let first_view = body_json(first).await;

        let retry = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            &json!({}),
            &[("Idempotency-Key", "clone-once")],
        )
        .await;
        assert_eq!(
            retry.status(),
            StatusCode::CREATED,
            "a replay is the original answer"
        );
        let retry_view = body_json(retry).await;
        assert_eq!(
            retry_view["sku_id"], first_view["sku_id"],
            "the retry replays the first clone, never a second mint"
        );
    }

    /// `new_parent_id` remaps the parent link (§3.1's lone-SKU carve-out),
    /// judged by the ordinary create-door checks against the *new* parent.
    #[tokio::test]
    async fn a_new_parent_override_remaps_the_link() {
        let harness = harness().await;
        let parent_id = seed_parent(&harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
        let mut other = new_parent_product(Uuid::now_v7(), TENANT);
        other.name = "Fibre Line Second".to_owned();
        other.name_normalized = "fibre line second".to_owned();
        let other_parent_id = seed_parent(&harness, other).await;
        let (source_id, _etag) = seed_draft_sku(&harness, parent_id, "LINE-7").await;

        let response = post_clone(
            app_for(&harness, TENANT),
            TENANT,
            source_id,
            &json!({ "new_parent_id": other_parent_id }),
            &[],
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        let view = body_json(response).await;
        assert_eq!(
            view["product_id"],
            json!(other_parent_id),
            "the override replaces the copied link"
        );
        let clone_id = Uuid::parse_str(view["sku_id"].as_str().expect("sku_id"))
            .expect("the clone's id is a uuid");
        let head = sku_head_of(&harness, clone_id).await;
        assert_eq!(
            head.cloned_from,
            Some(source_id),
            "lineage still names the source SKU, not the parent act"
        );
    }
}

/// The meter declaration at the doors — `dod-meter-atomic` and
/// `dod-unit-recognition`, both halves of each: the pair rule at the save
/// door with the `CHECK` behind it, and the recognition rules with the
/// clean-path positive control the `DoD` demands (a stub refusing every
/// string would otherwise satisfy the refusal cases).
mod meter_declaration_tests {
    use sea_orm::{ConnectionTrait, Database};

    use super::*;

    /// Put `code` into the metering-unit set in `state`, over SQL — the set
    /// doors live in their own router, and what is under test here is the
    /// SKU door's read of the set, not the set's own machine.
    async fn seed_unit(harness: &TestHarness, code: &str, state: &str) {
        let conn = Database::connect(&harness.dsn)
            .await
            .expect("open an auxiliary connection");
        conn.execute_unprepared(&format!(
            "INSERT INTO products_recognized_set (tenant_id, set_kind, member_code, \
             display_label, state, seeded_by, created_at, updated_at) VALUES (X'{tenant}', \
             'metering_unit', '{code}', NULL, '{state}', NULL, \
             '2026-08-29 09:00:00.000000 +00:00', '2026-08-29 09:00:00.000000 +00:00')",
            tenant = TENANT.simple(),
        ))
        .await
        .expect("seed the member");
    }

    async fn draft_with_etag(harness: &TestHarness) -> (Uuid, String) {
        let parent_id = seed_parent(harness, new_parent_product(Uuid::now_v7(), TENANT)).await;
        seed_draft_sku(
            harness,
            parent_id,
            &format!("SKU-M{}", Uuid::now_v7().simple()),
        )
        .await
    }

    /// **The atomic pair**: half a declaration is refused with the code the
    /// taxonomy names, and the paired `CHECK` refuses the same shape at the
    /// physical layer — probed on the resulting ROW, so a save completing a
    /// standing half is admitted.
    #[tokio::test]
    async fn half_a_declaration_is_refused_and_the_whole_pair_lands() {
        let harness = harness().await;
        seed_unit(&harness, "gib_month", "active").await;
        let (sku_id, etag) = draft_with_etag(&harness).await;

        let half = patch_sku(
            app_for(&harness, TENANT),
            TENANT,
            sku_id,
            &json!({ "metering_unit": "gib_month" }),
            &[("If-Match", &etag)],
        )
        .await;
        assert_eq!(half.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(half).await["context"]["violations"][0]["type"],
            json!("METER_DECLARATION_INCOMPLETE")
        );

        let whole = patch_sku(
            app_for(&harness, TENANT),
            TENANT,
            sku_id,
            &json!({ "metering_unit": "gib_month", "usage_type_ref": "usage:storage" }),
            &[("If-Match", &etag)],
        )
        .await;
        assert_eq!(
            whole.status(),
            StatusCode::OK,
            "the clean-path positive control"
        );
        let view = body_json(whole).await;
        assert_eq!(view["metering_unit"], json!("gib_month"));
        assert_eq!(view["usage_type_ref"], json!("usage:storage"));

        // The physical floor, probed directly: the CHECK refuses the same
        // half-pair shape this door just refused.
        let conn = Database::connect(&harness.dsn)
            .await
            .expect("open an auxiliary connection");
        let poisoned = conn
            .execute_unprepared(&format!(
                "UPDATE products_sku SET usage_type_ref = NULL, \
                 internal_revision = internal_revision + 1 WHERE sku_id = X'{}'",
                sku_id.simple()
            ))
            .await;
        assert!(
            poisoned.is_err(),
            "chk_products_sku_meter_pair must refuse a half-pair whatever door writes it"
        );
    }

    /// **Recognition, all three verdicts** — unknown and `removed` are one
    /// refusal (outside the set), `deprecated` its own, `active` admitted.
    #[tokio::test]
    async fn a_new_declaration_is_judged_against_the_set() {
        let harness = harness().await;
        seed_unit(&harness, "old_unit", "deprecated").await;
        seed_unit(&harness, "gone_unit", "removed").await;
        let (sku_id, etag) = draft_with_etag(&harness).await;

        let unknown = patch_sku(
            app_for(&harness, TENANT),
            TENANT,
            sku_id,
            &json!({ "metering_unit": "never_seen", "usage_type_ref": "usage:x" }),
            &[("If-Match", &etag)],
        )
        .await;
        assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(unknown).await["context"]["violations"][0]["type"],
            json!("UNRECOGNIZED_UNIT")
        );

        let tombstone = patch_sku(
            app_for(&harness, TENANT),
            TENANT,
            sku_id,
            &json!({ "metering_unit": "gone_unit", "usage_type_ref": "usage:x" }),
            &[("If-Match", &etag)],
        )
        .await;
        assert_eq!(
            body_json(tombstone).await["context"]["violations"][0]["type"],
            json!("UNRECOGNIZED_UNIT"),
            "a removed member is outside the set exactly like a code that never existed"
        );

        let deprecated = patch_sku(
            app_for(&harness, TENANT),
            TENANT,
            sku_id,
            &json!({ "metering_unit": "old_unit", "usage_type_ref": "usage:x" }),
            &[("If-Match", &etag)],
        )
        .await;
        assert_eq!(deprecated.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(deprecated).await["context"]["violations"][0]["type"],
            json!("UNIT_DEPRECATED")
        );
    }

    /// **A draft whose unit was deprecated after authoring is refused at its
    /// first publish** — the PRD treats it as a new declaration, and the
    /// publish door is where it bites.
    #[tokio::test]
    async fn a_first_publish_rejudges_the_drafts_unit() {
        let harness = harness().await;
        seed_unit(&harness, "gib_month", "active").await;
        let (sku_id, etag) = draft_with_etag(&harness).await;

        let declared = patch_sku(
            app_for(&harness, TENANT),
            TENANT,
            sku_id,
            &json!({ "metering_unit": "gib_month", "usage_type_ref": "usage:storage" }),
            &[("If-Match", &etag)],
        )
        .await;
        assert_eq!(declared.status(), StatusCode::OK);
        let after = body_json(declared).await;
        let fresh = format!(
            "\"{}\"",
            after["internal_revision"].as_i64().expect("a revision")
        );

        // The unit deprecates AFTER the declaration and BEFORE the publish —
        // one statement, the member guard's admitted pair.
        let conn = Database::connect(&harness.dsn)
            .await
            .expect("open an auxiliary connection");
        conn.execute_unprepared(&format!(
            "UPDATE products_recognized_set SET state = 'deprecated' WHERE member_code = \
             'gib_month' AND tenant_id = X'{}'",
            TENANT.simple()
        ))
        .await
        .expect("the member guard admits a state flip");

        let refused = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&fresh)).await;
        assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            body_json(refused).await["context"]["violations"][0]["type"],
            json!("UNIT_DEPRECATED"),
            "a first publish re-judges the draft's declaration as new"
        );
    }

    /// **A draft carrying a standing deprecated unit still edits freely.**
    ///
    /// The rule bites on a **new** declaration; a save that re-declares
    /// nothing makes none. An earlier revision derived `first_publish` from
    /// `published_version == 0` instead of taking it as a parameter, so every
    /// draft save re-judged the member and a `sku_code` edit came back
    /// `UNIT_DEPRECATED` — naming a declaration the caller never made, and
    /// freezing the draft plane `03` §1.6 says edits freely. Three lenses
    /// found it independently; this is what would have caught it.
    #[tokio::test]
    async fn an_unrelated_draft_save_does_not_rejudge_a_standing_unit() {
        let harness = harness().await;
        seed_unit(&harness, "gib_month", "active").await;
        let (sku_id, etag) = draft_with_etag(&harness).await;

        let declared = patch_sku(
            app_for(&harness, TENANT),
            TENANT,
            sku_id,
            &json!({ "metering_unit": "gib_month", "usage_type_ref": "usage:storage" }),
            &[("If-Match", &etag)],
        )
        .await;
        assert_eq!(declared.status(), StatusCode::OK);
        let fresh = format!(
            "\"{}\"",
            body_json(declared).await["internal_revision"]
                .as_i64()
                .expect("a revision")
        );

        let conn = Database::connect(&harness.dsn)
            .await
            .expect("open an auxiliary connection");
        conn.execute_unprepared(&format!(
            "UPDATE products_recognized_set SET state = 'deprecated' WHERE member_code = \
             'gib_month' AND tenant_id = X'{}'",
            TENANT.simple()
        ))
        .await
        .expect("the member guard admits a state flip");

        let unrelated = patch_sku(
            app_for(&harness, TENANT),
            TENANT,
            sku_id,
            &json!({ "region_scope": "eu" }),
            &[("If-Match", &fresh)],
        )
        .await;
        assert_eq!(
            unrelated.status(),
            StatusCode::OK,
            "a save touching neither meter field re-declares nothing, so the deprecated unit \
             it merely carries must not refuse it"
        );
    }

    /// **After first publish the pair is the correction door's** — the
    /// bucket-ii refusal arm, reachable for the first time now that the
    /// class has members.
    #[tokio::test]
    async fn the_pair_is_refused_at_the_save_door_after_first_publish() {
        let harness = harness().await;
        seed_unit(&harness, "gib_month", "active").await;
        let (sku_id, etag) = draft_with_etag(&harness).await;
        let published = post_publish(app_for(&harness, TENANT), TENANT, sku_id, Some(&etag)).await;
        assert_eq!(published.status(), StatusCode::OK);
        let fresh = format!(
            "\"{}\"",
            body_json(published).await["internal_revision"]
                .as_i64()
                .expect("a revision")
        );

        let refused = patch_sku(
            app_for(&harness, TENANT),
            TENANT,
            sku_id,
            &json!({ "metering_unit": "gib_month", "usage_type_ref": "usage:storage" }),
            &[("If-Match", &fresh)],
        )
        .await;
        assert_eq!(refused.status(), StatusCode::CONFLICT);
        let body = body_json(refused).await;
        assert_eq!(body["context"]["reason"], json!("ILLEGAL_FIELD_MUTATION"));
        assert!(
            body.to_string().contains("correction door"),
            "the refusal names slice 07's correction door rather than forwarding: {body}"
        );
    }
}
