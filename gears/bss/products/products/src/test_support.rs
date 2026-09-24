//! What more than one test module in this crate needs, written once.
//!
//! # Why this module exists
//!
//! Three suites had built their own copy of the same flat-`In` PDP fake, and
//! the two door suites had built their own copy of ten database-introspection
//! helpers on top of that — **twelve** byte-identical functions across
//! `api::rest::products_tests` and `api::rest::skus_tests`, plus a third
//! `FlatInResolver` in `authz_tests`. `FlatInResolver`'s own doc named the
//! reason: *"`authz_tests` is a private `#[cfg(test)]` sibling module, not a
//! reusable test-support crate."* That was true, and this module is the thing
//! whose absence it recorded.
//!
//! It matters more here than duplication usually does. This gear's Product and
//! SKU doors have already drifted apart six times, and a helper copied into
//! both suites is one more surface on which a repair can land in one and not
//! the other — a fix to a `SELECT` here, a widened predicate there, and the two
//! halves are silently measuring different things while both stay green.
//!
//! # What belongs here, and what does not
//!
//! Only what is genuinely **suite-agnostic**: reading a value back out of a
//! test database, and standing up a permissive PDP. A seed, a harness or a
//! request builder stays with its own suite, because those encode what a
//! particular door is being asked and moving them would hide the thing a
//! reader of that suite most needs to see.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;

use async_trait::async_trait;
use authz_resolver_sdk::constraints::{Constraint, InPredicate, Predicate};
use authz_resolver_sdk::models::{
    EvaluationRequest, EvaluationResponse, EvaluationResponseContext,
};
use authz_resolver_sdk::{AuthZResolverApi, PolicyEnforcer};
use sea_orm::{ConnectionTrait, Database, DbBackend, FromQueryResult, Statement};
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_gts::gts_id;
use toolkit_security::{PlatformSecurityContext, SecurityContext, pep_properties};
use uuid::Uuid;

use crate::infra::events;
use time::OffsetDateTime;

/// Degraded flat-`In` PDP fake: permits and emits a single flat
/// `In([allowed])` constraint over `OWNER_TENANT_ID` — **the shape the
/// production PDP returns for a PEP that advertises no tenant-subtree
/// capability** (this gear: [`PolicyEnforcer::new`] with no
/// `with_capabilities`). The request is ignored: the fake models a subject
/// authorized only for the single `allowed` tenant.
///
/// That first clause is what makes this a measurement rather than a
/// convenience, and review wave D's extraction dropped it — the wave verified
/// the *bodies* were byte-identical and did not compare the docs.
struct FlatInResolver {
    allowed: Uuid,
}

#[async_trait]
impl AuthZResolverApi for FlatInResolver {
    async fn evaluate(
        &self,
        _ctx: PlatformSecurityContext,
        _req: EvaluationRequest,
    ) -> Result<EvaluationResponse, CanonicalError> {
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

/// A fixture instant: `2026-09-02` at `hour`, UTC.
///
/// # Four copies, three epochs, and `at(9)` meant three different things
///
/// **P-D-110** arm 1 hoisted this. `repo_tests` had it on `2026-08-29`,
/// `repo/governance_tests` and `repo/taxonomy_tests` on `2026-09-02`, and
/// `repo/retention_tests` arrived on `2026-09-03` — a new module bringing a
/// new epoch, which is how the drift was accelerating. Two other modules had
/// no `at()` at all.
///
/// **The count was never the trigger.** `harness()` is copied five times too
/// and stays copied: its forms differ only in an `.expect()` message, so
/// unifying it would edit five files to agree on a panic string. This one
/// differed in **meaning**, and that is what a hoist is for.
///
/// `.single()` rather than `.unwrap()`, which is the form one of the four
/// already used and the only one that says what it is asserting: that the
/// One UTC instant from its civil components — the fixture spelling that
/// replaced `chrono`'s `crate::test_support::utc(..)` when the gear
/// moved to `time` (P-D-167).
///
/// A helper rather than seventy-two inline conversions: `time` builds an
/// instant through a `Date` and a civil time, so the inline form is four
/// calls where chrono's was one, and the suite would have carried the
/// arithmetic in seventy-two places.
///
/// # Panics
///
/// On components that name no real instant, which is a fixture typo rather
/// than a runtime case.
#[must_use]
pub fn utc(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> OffsetDateTime {
    time::Date::from_calendar_date(
        year,
        time::Month::try_from(month).expect("a month of the year"),
        day,
    )
    .expect("a real date")
    .with_hms(hour, minute, second)
    .expect("a real civil time")
    .assume_utc()
}

/// civil time names exactly one instant.
#[must_use]
///
/// # Panics
/// Panics if the hour is outside the fixture date range.
pub fn at(hour: u32) -> OffsetDateTime {
    utc(
        2026,
        9,
        2,
        u8::try_from(hour).expect("an hour of the day"),
        0,
        0,
    )
}

/// A [`PolicyEnforcer`] over [`FlatInResolver`], scoped to one tenant.
#[must_use]
pub fn flat_in_enforcer(allowed: Uuid) -> PolicyEnforcer {
    PolicyEnforcer::new(Arc::new(FlatInResolver { allowed }))
}

/// An authenticated [`SecurityContext`] for `tenant`, with a fresh subject.
#[must_use]
///
/// # Panics
/// Panics if the fixed fixture identity cannot build a security context.
pub fn authed_ctx(tenant: Uuid) -> SecurityContext {
    SecurityContext::builder()
        .subject_id(Uuid::now_v7())
        .subject_tenant_id(tenant)
        .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
        .token_scopes(vec!["*".to_owned()])
        .build()
        .expect("authed SecurityContext must build")
}

/// Run `sql` (a `SELECT ... AS v FROM ...`) on its own auxiliary connection
/// into `dsn` and return the single integer column it names `v`.
///
/// Its own connection, deliberately: the door harnesses pin `max_conns: 1` on
/// the production provider, so introspecting through it would contend with the
/// very statement under test.
///
/// # Panics
/// Panics if the fixture connection, query, or required result fails.
pub async fn raw_i64(dsn: &str, sql: &str) -> i64 {
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

/// [`raw_i64`] for a single nullable text column named `v`.
///
/// # Panics
/// Panics if the fixture connection or query fails.
pub async fn raw_string_opt(dsn: &str, sql: &str) -> Option<String> {
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

/// Drop `table` from the database at `dsn`, for the seams that need one gone.
///
/// # Panics
/// Panics if the fixture connection or table deletion fails.
pub async fn drop_table(dsn: &str, table: &str) {
    let conn = Database::connect(dsn)
        .await
        .expect("open an auxiliary connection to drop a table");
    conn.execute_unprepared(&format!("DROP TABLE {table};"))
        .await
        .expect("drop the table this seam needs gone");
    conn.close().await.ok();
}

/// The column names `table` declares, as the executed schema holds them.
///
/// # Panics
/// Panics if the fixture table does not exist or its columns cannot be read.
pub async fn table_columns(dsn: &str, table: &str) -> Vec<String> {
    let joined = raw_string_opt(
        dsn,
        &format!("SELECT group_concat(name, ',') AS v FROM pragma_table_info('{table}')"),
    )
    .await
    .expect("the migration chain created this table, so the pragma answers a non-empty list");
    joined.split(',').map(ToOwned::to_owned).collect()
}

/// How many SDK-envelope outbox rows carry this event type.
///
/// Counted on `_body` rather than `_incoming`: `_incoming` is a staging table
/// the running sequencer drains, so a count taken after the response has raced
/// the pipeline.
pub async fn enqueued_event_count(dsn: &str, payload_type: &str) -> i64 {
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    raw_i64(
        dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table} WHERE json_extract(CAST(payload AS TEXT), '$.type') = '{payload_type}'"),
    )
    .await
}

/// The business data of the newest SDK envelope carrying this event type.
///
/// `ORDER BY id DESC LIMIT 1` rather than a bare filter, so a case that
/// enqueued the same token twice reads the one it just wrote. The `payload`
/// column is a `BLOB`; `CAST(.. AS TEXT)` is what lets [`raw_string_opt`]'s
/// single-text-column shape read it.
///
/// # Panics
/// Panics if no matching event exists or its payload is not JSON.
pub async fn enqueued_event_envelope(dsn: &str, payload_type: &str) -> serde_json::Value {
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let payload = raw_string_opt(
        dsn,
        &format!(
            "SELECT CAST(payload AS TEXT) AS v FROM {body_table} \
             WHERE json_extract(CAST(payload AS TEXT), '$.type') = '{payload_type}' ORDER BY id DESC LIMIT 1"
        ),
    )
    .await
    .expect("the enqueued row carries a payload");
    serde_json::from_str::<serde_json::Value>(&payload).expect("the door enqueues a JSON envelope")
        ["data"]
        .clone()
}

/// How many idempotency rows carry `client_key`.
pub async fn idempotency_rows_for(dsn: &str, client_key: &str) -> i64 {
    raw_i64(
        dsn,
        &format!(
            "SELECT COUNT(*) AS v FROM products_idempotency WHERE client_key = '{client_key}'"
        ),
    )
    .await
}

/// A predicate matching `column` against `id` under **either** rendering.
///
/// `SQLite` stores a `UUID` as a 16-byte `BLOB`, so a bare `= '<hyphenated>'`
/// misses rows the driver wrote as bytes; `hex()` is the other side of that.
#[must_use]
pub fn id_matches(column: &str, id: Uuid) -> String {
    let hex = id.simple().to_string().to_uppercase();
    format!("({column} = '{id}' OR hex({column}) = '{hex}')")
}

/// One column of **the** audit row, and a proof that there is exactly one.
///
/// Both readers below carried the precondition "where exactly one was written"
/// in their docs and nothing enforced it: an unqualified `SELECT` over the
/// table hands `raw_string_opt`'s `.one()` an arbitrary row, so a case that
/// wrote a second audit row would read whichever sorted first and keep passing.
/// That is the same defect review wave D fixed for the `hex(actor_ref)` read —
/// and the class sweep that wave declared clean did not catch these, because
/// the detector was keyed to `LIMIT 1` without a `WHERE` and these carry no
/// `LIMIT` at all.
async fn the_one_audit_row(dsn: &str, column: &str) -> Option<String> {
    let rows = raw_i64(dsn, "SELECT COUNT(*) AS v FROM products_audit_log").await;
    assert_eq!(
        rows, 1,
        "these readers name **the** audit row; {rows} were written, so the value read would be \
         whichever the engine returned first"
    );
    raw_string_opt(
        dsn,
        &format!("SELECT {column} AS v FROM products_audit_log"),
    )
    .await
}

/// The `action` of the audit row, where exactly one was written.
pub async fn audit_action(dsn: &str) -> Option<String> {
    the_one_audit_row(dsn, "action").await
}

/// The `error_code` of the audit row, where exactly one was written.
pub async fn audit_error_code(dsn: &str) -> Option<String> {
    the_one_audit_row(dsn, "error_code").await
}

// **Owed, and measured rather than guessed**: 24 sites in the door suites still
// spell `SELECT error_code AS v FROM products_audit_log` inline against 6 that
// call the reader above, and 4 against 4 for `action`. Two spellings of one read
// is the drift surface this module exists to remove — but the swap is not
// mechanical, because the reader now asserts the table holds exactly one row and
// some of those sites may legitimately have written more. Each has to be looked
// at, which is why they are recorded here rather than converted blind.

/// A usage-type resolver that answers `Resolved` for every ref — what a test
/// `ApiState` carries unless a probe injects [`StubUsageTypes`] to script the
/// other two answers. Production never sees it: `gear.rs` installs the
/// collector's client or `NoCollector` (P-D-141).
#[must_use]
pub fn resolved_usage_types() -> Arc<dyn bss_products_sdk::usage_types::UsageTypeCatalog> {
    Arc::new(StubUsageTypes::always(
        crate::domain::recognized::UsageTypeAnswer::Resolved(probe_binding()),
    ))
}

/// The binding every `Resolved` stub answers with — what a probe expects to
/// find frozen in `binding_snapshot` after a publish (`dod-binding-snapshot`).
/// The metadata keys are deliberately unsorted here: the stored form sorts.
#[must_use]
pub fn probe_binding() -> crate::domain::recognized::UsageTypeBinding {
    crate::domain::recognized::UsageTypeBinding {
        gts_id: "usage:storage".to_owned(),
        kind: "counter".to_owned(),
        metadata_fields: vec!["zone".to_owned(), "region".to_owned()],
    }
}

/// A scripted collector: answers in order, then repeats the last one.
pub struct StubUsageTypes {
    answers:
        std::sync::Mutex<std::collections::VecDeque<crate::domain::recognized::UsageTypeAnswer>>,
    last: crate::domain::recognized::UsageTypeAnswer,
    /// How many times the door asked — the *once per publish* clause's operand.
    pub asked: std::sync::atomic::AtomicUsize,
}

impl StubUsageTypes {
    /// One answer, forever.
    #[must_use]
    pub fn always(answer: crate::domain::recognized::UsageTypeAnswer) -> Self {
        Self::scripted([answer])
    }

    /// `answers` in the order the door will receive them; the last repeats.
    #[must_use]
    ///
    /// # Panics
    /// Panics when the answer sequence is empty.
    pub fn scripted(
        answers: impl IntoIterator<Item = crate::domain::recognized::UsageTypeAnswer>,
    ) -> Self {
        let mut queue: std::collections::VecDeque<_> = answers.into_iter().collect();
        let last = queue.pop_back().expect("a stub needs at least one answer");
        Self {
            answers: std::sync::Mutex::new(queue),
            last,
            asked: std::sync::atomic::AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl bss_products_sdk::usage_types::UsageTypeCatalog for StubUsageTypes {
    async fn resolve(
        &self,
        _ctx: &SecurityContext,
        _usage_type_ref: &str,
    ) -> crate::domain::recognized::UsageTypeAnswer {
        self.asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let next = self.answers.lock().expect("stub lock").pop_front();
        next.unwrap_or_else(|| self.last.clone())
    }

    async fn list(
        &self,
        _ctx: &toolkit_security::SecurityContext,
        _q: Option<&str>,
        _kind: Option<&str>,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<
        bss_products_sdk::usage_types::UsageTypePage,
        toolkit_canonical_errors::CanonicalError,
    > {
        // The stub exists for the publish gate; a case that needs the
        // pick-list builds its own catalog and says so, rather than inheriting
        // an answer this one never meant.
        Ok(bss_products_sdk::usage_types::UsageTypePage::default())
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct EmptyUsageTypes;

#[async_trait::async_trait]
impl bss_products_sdk::usage_types::UsageTypeCatalog for EmptyUsageTypes {
    async fn resolve(
        &self,
        _ctx: &SecurityContext,
        _usage_type_ref: &str,
    ) -> crate::domain::recognized::UsageTypeAnswer {
        crate::domain::recognized::UsageTypeAnswer::Unresolved
    }

    async fn list(
        &self,
        _ctx: &SecurityContext,
        _q: Option<&str>,
        _kind: Option<&str>,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<
        bss_products_sdk::usage_types::UsageTypePage,
        toolkit_canonical_errors::CanonicalError,
    > {
        Ok(bss_products_sdk::usage_types::UsageTypePage::default())
    }
}

/// A configured catalog that cannot be reached — the 503 leg, which no other
/// stub here can produce because both answer `Ok`.
///
/// Without it the door's `.error_503` and the `USAGE_TYPE_CATALOG_UNAVAILABLE`
/// finding are asserted nowhere, and a regression collapsing either into an
/// empty 200 — the exact failure the surface exists to prevent — would stay
/// green.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnreachableUsageTypes;

#[async_trait::async_trait]
impl bss_products_sdk::usage_types::UsageTypeCatalog for UnreachableUsageTypes {
    async fn resolve(
        &self,
        _ctx: &SecurityContext,
        _usage_type_ref: &str,
    ) -> crate::domain::recognized::UsageTypeAnswer {
        crate::domain::recognized::UsageTypeAnswer::Unavailable
    }

    async fn list(
        &self,
        _ctx: &SecurityContext,
        _q: Option<&str>,
        _kind: Option<&str>,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<
        bss_products_sdk::usage_types::UsageTypePage,
        toolkit_canonical_errors::CanonicalError,
    > {
        Err(
            bss_products_sdk::usage_types::usage_type_catalog_unreachable(
                "the probe's catalog is unreachable by construction",
            ),
        )
    }
}

/// File-backed database with the production migration chains and a PDP-derived scope.
///
/// # Panics
/// Panics if fixture initialization fails.
pub async fn test_db() -> (
    toolkit_db::DBProvider<toolkit_db::DbError>,
    toolkit_db::secure::AccessScope,
    Uuid,
    String,
) {
    use sea_orm_migration::MigratorTrait;
    let path = std::env::temp_dir().join(format!("products-repos-{}.sqlite3", Uuid::new_v4()));
    let dsn = format!("sqlite://{}?mode=rwc", path.display());
    let db = toolkit_db::connect_db(
        &dsn,
        toolkit_db::ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        crate::infra::storage::migrations::Migrator::migrations(),
    )
    .await
    .unwrap();
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        toolkit_db::outbox::outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX).unwrap(),
    )
    .await
    .unwrap();
    let tenant = Uuid::new_v4();
    let scope = crate::authz::access_scope(
        &flat_in_enforcer(tenant),
        &authed_ctx(tenant),
        &crate::authz::resource_types::SKU,
        crate::authz::actions::READ,
        Some(tenant),
        None,
        true,
    )
    .await
    .unwrap();
    (toolkit_db::DBProvider::new(db), scope, tenant, dsn)
}

/// Running outbox lifetime retained by every clone of a REST test router.
struct RestOutbox {
    _handle: toolkit_db::outbox::OutboxHandle,
}

/// Build a door with the production database/outbox migrations and a resolved catalog.
pub async fn rest_app(
    tenant: Uuid,
    build: fn(Arc<crate::api::rest::ApiState>, &dyn toolkit::api::OpenApiRegistry) -> axum::Router,
) -> (axum::Router, String) {
    rest_app_with_catalog(tenant, build, resolved_usage_types(), "test").await
}

/// The same REST fixture with an explicitly selected catalog answer and provenance.
/// # Panics
/// Panics if fixture setup or the asserted operation fails.
pub async fn rest_app_with_catalog(
    tenant: Uuid,
    build: fn(Arc<crate::api::rest::ApiState>, &dyn toolkit::api::OpenApiRegistry) -> axum::Router,
    catalog: Arc<dyn bss_products_sdk::usage_types::UsageTypeCatalog>,
    source: &'static str,
) -> (axum::Router, String) {
    let (db, _, _, dsn) = test_db().await;
    let (app, _) = rest_app_on_db(tenant, build, catalog, source, db).await;
    (app, dsn)
}

/// Build a router on a supplied provider so race tests use independent connections.
/// # Panics
/// Panics if outbox initialization fails.
pub async fn rest_app_on_db(
    tenant: Uuid,
    build: fn(Arc<crate::api::rest::ApiState>, &dyn toolkit::api::OpenApiRegistry) -> axum::Router,
    catalog: Arc<dyn bss_products_sdk::usage_types::UsageTypeCatalog>,
    source: &'static str,
    db: toolkit_db::DBProvider<toolkit_db::DbError>,
) -> (axum::Router, Arc<crate::api::rest::ApiState>) {
    let handle = toolkit_db::outbox::Outbox::builder(db.db().clone())
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .unwrap()
        .queue(
            events::QUEUE_NAME,
            toolkit_db::outbox::Partitions::of(events::PARTITIONS),
        )
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .unwrap();
    let state = Arc::new(crate::api::rest::ApiState {
        db,
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(handle.outbox())),
        usage_type_catalog: catalog,
        usage_type_catalog_source: source,
        idempotency_retention_hours: 24,
        fence_ttl_minutes: 30,
        reference_principals: std::collections::BTreeMap::from([(
            Uuid::from_u128(42),
            "pricing".into(),
        )]),
    });
    let app = build(state.clone(), &toolkit::api::OpenApiRegistryImpl::new())
        .layer(axum::Extension(flat_in_enforcer(tenant)))
        .layer(axum::Extension(Arc::new(RestOutbox { _handle: handle })));
    (app, state)
}

/// Open an auxiliary scoped provider to seed the REST fixture through repositories.
/// # Panics
/// Panics if fixture setup or the asserted operation fails.
pub async fn repo_connection(
    dsn: &str,
    tenant: Uuid,
) -> (
    toolkit_db::DBProvider<toolkit_db::DbError>,
    toolkit_db::secure::AccessScope,
) {
    let db = toolkit_db::connect_db(
        dsn,
        toolkit_db::ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let scope = crate::authz::access_scope(
        &flat_in_enforcer(tenant),
        &authed_ctx(tenant),
        &crate::authz::resource_types::SKU,
        crate::authz::actions::AUTHOR,
        Some(tenant),
        None,
        true,
    )
    .await
    .unwrap();
    (toolkit_db::DBProvider::new(db), scope)
}

/// Seed an unmetered draft for category and SKU door tests.
/// # Panics
/// Panics if fixture setup or the asserted operation fails.
pub async fn seed_rest_sku(
    runner: &impl toolkit_db::secure::DBRunner,
    scope: &toolkit_db::secure::AccessScope,
    tenant: Uuid,
    category_id: Uuid,
    code: &str,
) -> bss_products_sdk::models::Sku {
    crate::infra::storage::repo::insert_sku(
        runner,
        scope,
        tenant,
        crate::domain::sku::NewSku {
            code: code.to_owned(),
            name: code.to_owned(),
            r#type: bss_products_sdk::models::SkuType::Usage,
            category_id,
            description: String::new(),
            sellable: true,
            gl_code: None,
            tax_category: None,
            invoice_line_template: None,
            billing_timing: None,
            usage_type_ref: None,
            unit: None,
        },
        authed_ctx(tenant).subject_id(),
        OffsetDateTime::now_utc(),
    )
    .await
    .unwrap()
}

/// Exercise the router with a request-scoped authenticated principal.
/// # Panics
/// Panics if fixture setup or the asserted operation fails.
pub async fn request(
    app: &axum::Router,
    tenant: Uuid,
    method: axum::http::Method,
    uri: &str,
    body: Option<serde_json::Value>,
    etag: Option<&str>,
) -> axum::response::Response {
    use tower::ServiceExt;
    let mut builder = axum::http::Request::builder()
        .method(method)
        .uri(uri)
        .extension(authed_ctx(tenant));
    if let Some(etag) = etag {
        builder = builder.header("If-Match", etag);
    }
    let body = body.map_or_else(axum::body::Body::empty, |b| {
        axum::body::Body::from(b.to_string())
    });
    app.clone()
        .oneshot(
            builder
                .header("Content-Type", "application/json")
                .body(body)
                .unwrap(),
        )
        .await
        .unwrap()
}
/// POST a JSON request.
pub async fn post(
    app: &axum::Router,
    tenant: Uuid,
    uri: &str,
    body: serde_json::Value,
) -> axum::response::Response {
    request(app, tenant, axum::http::Method::POST, uri, Some(body), None).await
}
/// PATCH under the supplied revision precondition.
pub async fn patch(
    app: &axum::Router,
    tenant: Uuid,
    uri: &str,
    body: serde_json::Value,
    etag: Option<&str>,
) -> axum::response::Response {
    request(
        app,
        tenant,
        axum::http::Method::PATCH,
        uri,
        Some(body),
        etag,
    )
    .await
}
/// GET with the request principal.
pub async fn get(app: &axum::Router, tenant: Uuid, uri: &str) -> axum::response::Response {
    request(app, tenant, axum::http::Method::GET, uri, None, None).await
}
/// Decode an HTTP response body.
/// # Panics
/// Panics if fixture setup or the asserted operation fails.
pub async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// Read a machine code from a canonical reason or precondition violation.
/// # Panics
/// Panics if the response has no machine-readable error code.
#[must_use]
pub fn problem_code(body: &serde_json::Value) -> String {
    find_code(body).expect("problem contains a machine-readable code")
}
fn find_code(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            for key in ["reason", "type", "code"] {
                if let Some(serde_json::Value::String(found)) = map.get(key)
                    && found.chars().all(|c| c.is_ascii_uppercase() || c == '_')
                    && found.len() > 3
                {
                    return Some(found.clone());
                }
            }
            map.values().find_map(find_code)
        }
        serde_json::Value::Array(items) => items.iter().find_map(find_code),
        _ => None,
    }
}

/// Read the violation for a wire field.
pub fn violation_for(body: &serde_json::Value, subject: &str) -> Option<String> {
    fn violations(value: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
        match value {
            serde_json::Value::Object(map) => map
                .get("violations")
                .and_then(serde_json::Value::as_array)
                .or_else(|| map.values().find_map(violations)),
            serde_json::Value::Array(items) => items.iter().find_map(violations),
            _ => None,
        }
    }
    violations(body)?
        .iter()
        .find(|violation| violation["subject"] == serde_json::json!(subject))
        .and_then(|violation| violation["description"].as_str())
        .map(ToOwned::to_owned)
}
