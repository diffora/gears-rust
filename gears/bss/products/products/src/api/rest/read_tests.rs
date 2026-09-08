//! Probes of `08`'s read surface (P-D-150): the browse door, the limiter,
//! the timelines, the dashboards.

use std::sync::Arc;

use chrono::Utc;
use sea_orm_migration::MigratorTrait as _;
use serde_json::json;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use uuid::Uuid;

use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::domain::approval::StoredApprovalGate;
use crate::domain::governance::GateMode;
use crate::infra::events;
use crate::infra::projector::{
    PassOutcome, ProjectorContext, ReadKnobs, poll_dashboards, project_tenant,
};
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo::{self, NewProduct};

pub(super) const TENANT: Uuid = Uuid::from_u128(0x08_01);
pub(super) const BRAND: Uuid = Uuid::from_u128(0x08_02);
pub(super) const ACTOR: Uuid = Uuid::from_u128(0x08_03);
pub(super) const CATEGORY: Uuid = Uuid::from_u128(0x08_0c);

pub(super) struct Harness {
    pub(super) dsn: String,
    pub(super) state: Arc<ApiState>,
    #[allow(dead_code)]
    outbox_handle: OutboxHandle,
}

impl Drop for Harness {
    fn drop(&mut self) {
        if let Some(rest) = self.dsn.strip_prefix("sqlite://") {
            let path = rest.split('?').next().unwrap_or(rest);
            std::fs::remove_file(path).ok();
        }
    }
}

pub(super) async fn harness() -> Harness {
    let path = std::env::temp_dir().join(format!("bss-products-read-{}.sqlite3", Uuid::new_v4()));
    let dsn = format!("sqlite://{}?mode=rwc", path.display());
    let db = connect_db(
        &dsn,
        ConnectOpts {
            max_conns: Some(1),
            min_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .expect("connect");
    toolkit_db::migration_runner::run_migrations_for_testing(&db, Migrator::migrations())
        .await
        .expect("migrate");
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        outbox_migrations_with_prefix(events::OUTBOX_TABLE_PREFIX).expect("prefix"),
    )
    .await
    .expect("outbox migrate");
    let outbox_handle = Outbox::builder(db.clone())
        .table_prefix(events::OUTBOX_TABLE_PREFIX)
        .expect("prefix")
        .queue(events::QUEUE_NAME, Partitions::of(events::PARTITIONS))
        .leased(events::PendingBrokerProducer)
        .start()
        .await
        .expect("start the outbox");
    let defaults = ProductsConfig::default();
    let state = Arc::new(ApiState {
        db: DBProvider::<DbError>::new(db),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(outbox_handle.outbox())),
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: defaults.idempotency_retention_hours,
        bulk_max_rows_per_batch: defaults.bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: defaults.bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: defaults.watermark_skew_tolerance(),
        reference: crate::api::rest::ReferenceKnobs::from(&defaults),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        eol_enabled: false,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
    });
    Harness {
        dsn,
        state,
        outbox_handle,
    }
}

pub(super) fn scope() -> AccessScope {
    AccessScope::for_tenant(TENANT)
}

pub(super) fn ctx(harness: &Harness) -> ProjectorContext {
    ProjectorContext {
        db: harness.state.db.clone(),
        knobs: ReadKnobs {
            poison_retry_ceiling: 2,
            ..ReadKnobs::from(&ProductsConfig::default())
        },
    }
}

#[allow(clippy::unnecessary_wraps)]
fn render_nothing(_record: repo::ProductRecord) -> Result<serde_json::Value, serde_json::Error> {
    Ok(serde_json::Value::Null)
}

/// A product created through the Foundation's own insert path (one inbox
/// row: `ProductCreated`) and given its primary category.
pub(super) async fn draft_product(harness: &Harness, name: &str, region: &str) -> Uuid {
    let product_id = Uuid::new_v4();
    let now = crate::domain::canonical::write_instant(Utc::now());
    let new = NewProduct {
        product_id,
        tenant_id: TENANT,
        brand_id: BRAND,
        name: name.to_owned(),
        name_normalized: crate::domain::name::normalize(name),
        product_code: Some(format!("{}-CODE", name.replace(' ', "-").to_uppercase())),
        region_scope: region.to_owned(),
        brand_scope: String::new(),
        created_by: ACTOR.to_string(),
        created_at: now,
        cloned_from: None,
        cloned_from_version: None,
    };
    crate::infra::create::insert_product_with_event(
        &harness.state.db,
        &harness.state.sink,
        scope(),
        new,
        crate::infra::create::JoinedRecords {
            claim: None,
            stamp: None,
            content: None,
        },
        ACTOR,
        render_nothing,
    )
    .await
    .expect("insert the product");
    let conn = harness.state.db.conn().expect("conn");
    let _existing = repo::insert_category(
        &conn,
        &scope(),
        repo::NewCategory {
            tenant_id: TENANT,
            category_id: CATEGORY,
            parent_id: None,
            name: "Fixture",
            name_normalized: "fixture",
        },
        now,
    )
    .await
    .expect("the category insert runs");
    repo::replace_category_assignments(
        &conn,
        &scope(),
        TENANT,
        product_id,
        &[(CATEGORY, crate::domain::taxonomy::AssignmentRole::Primary)],
        now,
    )
    .await
    .expect("assign the primary category");
    product_id
}

/// Publish a product through the Foundation's own door (ungoverned host):
/// one frozen version row and one `ProductPublished` inbox row.
pub(super) async fn publish_product(harness: &Harness, product_id: Uuid) -> i64 {
    use crate::api::rest::products;
    let conn = harness.state.db.conn().expect("conn");
    let head = repo::find_product(&conn, &scope(), TENANT, product_id)
        .await
        .expect("read")
        .expect("the head exists");
    let inputs = products::HeadActInputs {
        scope: scope(),
        tenant_id: TENANT,
        product_id,
        actor_ref: ACTOR,
        expected: head.internal_revision,
        now: crate::domain::canonical::write_instant(Utc::now()),
        claim: None,
    };
    let outcome = products::run_publish(
        &conn,
        &inputs,
        &StoredApprovalGate::ungoverned(),
        GateMode::Gate,
        &harness.state.sink,
    )
    .await;
    assert!(
        matches!(outcome, Ok(products::HeadActOutcome::Applied { .. })),
        "the fixture publish lands"
    );
    repo::find_product(&conn, &scope(), TENANT, product_id)
        .await
        .expect("read")
        .expect("the head exists")
        .published_version
}

pub(super) async fn project(harness: &Harness) -> PassOutcome {
    project_tenant(
        &ctx(harness),
        TENANT,
        crate::domain::canonical::write_instant(Utc::now()),
    )
    .await
    .expect("the pass runs")
}

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt as _;

use super::ReadPathLimiter;

fn app(harness: &Harness, tenant: Uuid) -> Router {
    super::router(
        Arc::clone(&harness.state),
        &toolkit::api::OpenApiRegistryImpl::new(),
    )
    .layer(axum::Extension(crate::test_support::flat_in_enforcer(
        tenant,
    )))
}

async fn get(harness: &Harness, uri: &str, tenant: Uuid) -> axum::http::Response<Body> {
    app(harness, tenant)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .extension(crate::test_support::authed_ctx(tenant))
                .body(Body::empty())
                .expect("build the request"),
        )
        .await
        .expect("the router answers")
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("read the body");
    serde_json::from_slice(&bytes).expect("json")
}

/// `dod-browse-door`, `inst-rb-stamp`: an empty projection answers with the
/// anchorless stamp; a projected published row appears; a deprecated row
/// carries its flag and `excludeDeprecated=true` drops it; a draft never
/// shows; facets count every assigned category.
#[tokio::test]
async fn browse_serves_the_projection_under_the_visibility_contract_with_the_stamp() {
    use crate::api::rest::products;
    let harness = harness().await;
    let empty = get(&harness, "/bss-products/v1/browse", TENANT).await;
    assert_eq!(empty.status(), StatusCode::OK);
    let view = body_json(empty).await;
    assert_eq!(view["rows"], json!([]));
    assert_eq!(
        view["stamp"]["as_of_catalog_version"],
        json!(null),
        "the anchorless arm"
    );
    assert!(
        view["stamp"]["projected_at"].is_string(),
        "the stamp is never omitted"
    );

    let published = draft_product(&harness, "Alpha Line", "eu").await;
    let deprecated = draft_product(&harness, "Beta Line", "eu").await;
    let _draft = draft_product(&harness, "Draft Line", "eu").await;
    publish_product(&harness, published).await;
    publish_product(&harness, deprecated).await;
    {
        let conn = harness.state.db.conn().expect("conn");
        let head = repo::find_product(&conn, &scope(), TENANT, deprecated)
            .await
            .expect("read")
            .expect("head");
        let inputs = products::HeadActInputs {
            scope: scope(),
            tenant_id: TENANT,
            product_id: deprecated,
            actor_ref: ACTOR,
            expected: head.internal_revision,
            now: crate::domain::canonical::write_instant(Utc::now()),
            claim: None,
        };
        let outcome = products::run_deprecate(
            &conn,
            &inputs,
            &scope(),
            &StoredApprovalGate::ungoverned(),
            GateMode::Gate,
            &harness.state.sink,
        )
        .await;
        assert!(matches!(
            outcome,
            Ok(products::HeadActOutcome::Applied { .. })
        ));
    }
    project(&harness).await;

    let all = body_json(
        get(
            &harness,
            "/bss-products/v1/browse?includeFacets=true",
            TENANT,
        )
        .await,
    )
    .await;
    let rows = all["rows"].as_array().expect("rows");
    assert_eq!(
        rows.len(),
        2,
        "published and deprecated, never the draft: {all}"
    );
    let flagged = rows
        .iter()
        .find(|r| r["entity_id"] == json!(deprecated))
        .expect("the deprecated row");
    assert_eq!(flagged["deprecated"], json!(true));
    assert_eq!(
        all["facets"]["categories"],
        json!([{ "value": "Fixture", "count": 2 }])
    );

    let filtered = body_json(
        get(
            &harness,
            "/bss-products/v1/browse?excludeDeprecated=true",
            TENANT,
        )
        .await,
    )
    .await;
    let rows = filtered["rows"].as_array().expect("rows");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["entity_id"], json!(published));

    // The prefix search the door used to spell `?q=`. `startswith` is the
    // platform's own lowering, which escapes the LIKE metacharacters the
    // hand-rolled prefix used to delete.
    let by_prefix = body_json(
        get(
            &harness,
            &browse_url(&[("$filter", "startswith(name,'Alp')")]),
            TENANT,
        )
        .await,
    )
    .await;
    assert_eq!(
        by_prefix["rows"].as_array().expect("rows").len(),
        1,
        "{by_prefix}"
    );
    let bad_kind = get(&harness, "/bss-products/v1/browse?kind=widget", TENANT).await;
    assert_eq!(bad_kind.status(), StatusCode::BAD_REQUEST);
}

/// The defect the query seam exists to stop (**P-D-165**): a parameter this
/// door does not declare used to be **dropped**, so a caller asking for a
/// filter received `200` and the whole unfiltered set. Every spelling the
/// door retired is now a refusal that names the key.
///
/// @cpt-dod:cpt-cf-bss-products-dod-browse-door:p2
#[tokio::test]
async fn a_retired_or_invented_query_key_is_refused_and_not_dropped() {
    let harness = harness().await;
    let published = draft_product(&harness, "Alpha Line", "eu").await;
    publish_product(&harness, published).await;
    project(&harness).await;

    // Every key the hand-rolled surface used to bind and no longer does,
    // plus one a caller might invent from a generic-REST habit.
    for (key, value) in [
        ("q", "Alp"),
        ("category", "Fixture"),
        ("skuType", "plan"),
        ("tier", "gold"),
        ("sellable", "true"),
        ("unit", "GB"),
        ("status", "published"),
        ("excludedeprecated", "true"),
    ] {
        let response = get(&harness, &browse_url(&[(key, value)]), TENANT).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`?{key}=` must be refused, not silently ignored"
        );
        let body = body_json(response).await;
        assert_eq!(
            body["context"]["violations"][0]["subject"],
            json!(key),
            "the refusal must name `{key}`: {body}"
        );
    }

    // The four the door still owns, and the OData family, are admitted.
    for (key, value) in [
        ("kind", "product"),
        ("excludeDeprecated", "true"),
        ("brand", "acme"),
        ("region", "eu"),
        ("includeFacets", "true"),
        ("limit", "1"),
        ("$filter", "deprecated eq false"),
        ("$orderby", "name desc"),
        ("$top", "1"),
    ] {
        let response = get(&harness, &browse_url(&[(key, value)]), TENANT).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "`?{key}={value}` is part of this door's contract"
        );
    }
}

/// `$filter` and `$orderby` are validated against the door's declared
/// vocabulary, so an unknown field, an unorderable one and an operator the
/// field's kind does not admit are each a `400` rather than a query that
/// quietly matches everything.
#[tokio::test]
async fn the_filter_vocabulary_is_closed() {
    let harness = harness().await;
    for (key, value) in [
        // Not a field this door exposes.
        ("$filter", "tenant_id eq 'x'"),
        ("$filter", "brand_scope eq 'acme'"),
        ("$filter", "entity_kind eq 'sku'"),
        ("$orderby", "region_scope asc"),
        // A nullable column cannot be an order key: SQLite sorts NULLs
        // first and Postgres sorts them last, so the keyset walk would not
        // be the same walk on the two engines.
        ("$orderby", "sku_type asc"),
        ("$orderby", "category_paths desc"),
        // Not a system query option this platform binds.
        ("$skip", "10"),
        ("$filtre", "name eq 'x'"),
    ] {
        let response = get(&harness, &browse_url(&[(key, value)]), TENANT).await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`?{key}={value}` must be refused"
        );
    }
    // The orderable columns are the NOT NULL ones, and they work.
    for (key, value) in [
        ("$orderby", "name desc"),
        ("$orderby", "published_version asc"),
        ("$orderby", "lifecycle_state asc"),
        ("$orderby", "entity_id asc"),
        ("$filter", "published_version ge 1"),
        ("$filter", "contains(category_paths,'Fix')"),
    ] {
        let response = get(&harness, &browse_url(&[(key, value)]), TENANT).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "`?{key}={value}` is in the declared vocabulary"
        );
    }
}

/// The page is a window on the set and the set is reachable through it: the
/// walk visits every row exactly once and stops by saying so.
///
/// This is the capability the door did not have — `limit` was a ceiling with
/// nothing past it, and a tenant with more matching rows than the ceiling
/// could not read them at all.
#[tokio::test]
async fn the_walk_visits_every_row_once_and_then_says_it_is_done() {
    let harness = harness().await;
    let mut expected = Vec::new();
    for name in ["Alpha", "Bravo", "Charlie", "Delta", "Echo"] {
        let id = draft_product(&harness, name, "eu").await;
        publish_product(&harness, id).await;
        expected.push(id);
    }
    project(&harness).await;

    let mut seen: Vec<serde_json::Value> = Vec::new();
    let mut url = browse_url(&[("limit", "2")]);
    let mut pages = 0_u32;
    loop {
        let body = body_json(get(&harness, &url, TENANT).await).await;
        let rows = body["rows"].as_array().expect("rows").clone();
        assert!(rows.len() <= 2, "the page honours `limit`: {body}");
        seen.extend(rows.iter().map(|r| r["entity_id"].clone()));
        assert_eq!(body["page_info"]["limit"], json!(2));
        pages += 1;
        assert!(pages <= 5, "a five-row set cannot need six pages of two");
        match body["page_info"]["next_cursor"].as_str() {
            Some(cursor) => url = browse_url(&[("limit", "2"), ("cursor", cursor)]),
            None => break,
        }
    }
    assert_eq!(pages, 3, "five rows at two per page: 2 + 2 + 1");
    assert_eq!(seen.len(), 5, "every row once, no duplicates: {seen:?}");
    let mut unique = seen.clone();
    unique.sort_by_key(std::string::ToString::to_string);
    unique.dedup();
    assert_eq!(unique.len(), 5);
    // Default order is `name ASC`, which is what the door always served.
    let names: Vec<&str> = seen
        .iter()
        .map(|id| {
            let idx = expected
                .iter()
                .position(|e| json!(e) == *id)
                .expect("a known id");
            ["Alpha", "Bravo", "Charlie", "Delta", "Echo"][idx]
        })
        .collect();
    assert_eq!(names, ["Alpha", "Bravo", "Charlie", "Delta", "Echo"]);
}

/// The continuation token describes one walk, and changing the walk under it
/// is refused rather than answered from the wrong set.
#[tokio::test]
async fn a_cursor_from_another_walk_is_refused() {
    let harness = harness().await;
    for name in ["Alpha", "Bravo", "Charlie"] {
        let id = draft_product(&harness, name, "eu").await;
        publish_product(&harness, id).await;
    }
    project(&harness).await;

    let first = body_json(get(&harness, &browse_url(&[("limit", "1")]), TENANT).await).await;
    let cursor = first["page_info"]["next_cursor"]
        .as_str()
        .expect("more rows remain")
        .to_owned();

    // Same token, a different `$orderby` — the walk it describes is not the
    // walk being asked for.
    let mismatched = get(
        &harness,
        &browse_url(&[
            ("limit", "1"),
            ("cursor", &cursor),
            ("$orderby", "published_version desc"),
        ]),
        TENANT,
    )
    .await;
    assert_eq!(mismatched.status(), StatusCode::BAD_REQUEST);

    // Same token, a different `$filter` — the set has changed underneath it.
    // The walk began unfiltered, which the platform stamps as *no* hash at
    // all; without the door's own "no filter" stamp this request was served
    // a keyset predicate over a different set, with a 200.
    let refiltered = get(
        &harness,
        &browse_url(&[
            ("limit", "1"),
            ("cursor", &cursor),
            ("$filter", "published_version ge 1"),
        ]),
        TENANT,
    )
    .await;
    assert_eq!(refiltered.status(), StatusCode::BAD_REQUEST);

    // And the other direction: a walk begun *with* a filter cannot drop it.
    let filtered_first = body_json(
        get(
            &harness,
            &browse_url(&[("limit", "1"), ("$filter", "published_version ge 1")]),
            TENANT,
        )
        .await,
    )
    .await;
    let filtered_cursor = filtered_first["page_info"]["next_cursor"]
        .as_str()
        .expect("more rows remain")
        .to_owned();
    let unfiltered = get(
        &harness,
        &browse_url(&[("limit", "1"), ("cursor", &filtered_cursor)]),
        TENANT,
    )
    .await;
    assert_eq!(unfiltered.status(), StatusCode::BAD_REQUEST);

    let garbage = get(
        &harness,
        &browse_url(&[("limit", "1"), ("cursor", "not-a-token")]),
        TENANT,
    )
    .await;
    assert_eq!(garbage.status(), StatusCode::BAD_REQUEST);
}

/// The walk is bidirectional: the platform's `page_info` carries a
/// `prev_cursor` and it goes back to the page it came from.
///
/// Written because the door's own author assumed the opposite — the sibling
/// pricing gear serves `prev_cursor: null` by its own decision (D-125), and
/// reading that as the platform's behaviour would have shipped a documented
/// `null` over a token that works.
#[tokio::test]
async fn the_walk_goes_back_the_way_it_came() {
    let harness = harness().await;
    for name in ["Alpha", "Bravo", "Charlie"] {
        let id = draft_product(&harness, name, "eu").await;
        publish_product(&harness, id).await;
    }
    project(&harness).await;

    let first = body_json(get(&harness, &browse_url(&[("limit", "1")]), TENANT).await).await;
    let first_row = first["rows"][0]["entity_id"].clone();
    let forward = first["page_info"]["next_cursor"]
        .as_str()
        .expect("more rows remain")
        .to_owned();

    let second = body_json(
        get(
            &harness,
            &browse_url(&[("limit", "1"), ("cursor", &forward)]),
            TENANT,
        )
        .await,
    )
    .await;
    assert_ne!(second["rows"][0]["entity_id"], first_row);
    let back = second["page_info"]["prev_cursor"]
        .as_str()
        .expect("the second page knows where it came from")
        .to_owned();

    let again = body_json(
        get(
            &harness,
            &browse_url(&[("limit", "1"), ("cursor", &back)]),
            TENANT,
        )
        .await,
    )
    .await;
    assert_eq!(
        again["rows"][0]["entity_id"], first_row,
        "walking back lands on the page the forward token left: {again}"
    );
}

/// Percent-encode one query-string **value**.
///
/// `$filter` and `$orderby` carry spaces, quotes and parentheses, and a
/// cursor is base64url with `=` padding — none of which may travel raw in a
/// URI. Encoding only the value, and only the characters that need it, keeps
/// the test URLs readable: a helper that encoded the whole query string
/// would also encode the `?`, `&` and `=` that give it its shape, and one
/// that encoded nothing would be a test that fails on the encoder rather
/// than on the door.
fn qval(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b',' => {
                (b as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// A browse URL from `(key, value)` pairs, each value encoded by [`qval`].
fn browse_url(params: &[(&str, &str)]) -> String {
    let query: Vec<String> = params
        .iter()
        .map(|(k, v)| format!("{}={}", qval(k), qval(v)))
        .collect();
    format!("/bss-products/v1/browse?{}", query.join("&"))
}

/// The timeline is paged, and a page that does not start at version one is
/// still diffed against the version **before** it — not against the
/// previous row the caller happened to be shown.
///
/// This is what the page costs and what pays for it: without the
/// predecessor seed, page two's first entry would report every key as
/// changed, which is the same wrongness as a fresh history.
///
/// @cpt-dod:cpt-cf-bss-products-dod-history-timeline:p2
#[tokio::test]
async fn a_timeline_page_is_diffed_against_the_version_before_it() {
    let harness = harness().await;
    let product = draft_product(&harness, "Zeta Line", "eu").await;
    // A published head is publishable again as version N+1, so three
    // publishes give a history whose middle page neither starts nor ends it.
    for expected in 1..=3 {
        assert_eq!(publish_product(&harness, product).await, expected);
    }

    let url = format!("/bss-products/v1/products/{product}/versions");
    let whole = body_json(get(&harness, &url, TENANT).await).await;
    let versions = whole["versions"].as_array().expect("versions");
    assert_eq!(versions.len(), 3, "three publishes: {whole}");
    assert_eq!(
        whole["page_info"]["next_cursor"],
        json!(null),
        "three versions fit one page"
    );

    // Page two of one-per-page: version two, whose `changedKeys` must be
    // the diff against version one and therefore name `name` and not every
    // key.
    let first = body_json(get(&harness, &format!("{url}?limit=1"), TENANT).await).await;
    assert_eq!(first["versions"][0]["published_version"], json!(1));
    let cursor = first["page_info"]["next_cursor"]
        .as_str()
        .expect("two more versions remain")
        .to_owned();
    let second = body_json(
        get(
            &harness,
            &format!("{url}?limit=1&cursor={}", qval(&cursor)),
            TENANT,
        )
        .await,
    )
    .await;
    assert_eq!(second["versions"][0]["published_version"], json!(2));
    let changed = second["versions"][0]["changed_keys"]
        .as_array()
        .expect("changed keys")
        .clone();
    // Nothing changed between the two publishes, so nothing is reported.
    // Without the predecessor seed this page would have no previous version
    // in hand and would report **every** key — which is exactly the wrong
    // answer this probe is armed against, and it is the assertion that
    // fails if the seed is removed.
    assert!(
        changed.is_empty(),
        "version two republished the same content: {second}"
    );
    assert!(
        !versions[0]["changed_keys"]
            .as_array()
            .expect("the first version's keys")
            .is_empty(),
        "and version one, which has no predecessor, changed everything"
    );

    // Lineage and clones are the entity's, not the page's: every page
    // carries them whole.
    assert_eq!(second["entity_id"], json!(product));
    assert_eq!(second["clones"], json!([]));

    // The three options this door binds nothing to are refused, each naming
    // itself, rather than accepted and ignored.
    for (key, value) in [
        ("$filter", "published_version eq 2"),
        ("$orderby", "published_version desc"),
        ("$select", "published_version"),
    ] {
        let response = get(
            &harness,
            &format!("{url}?{}={}", qval(key), qval(value)),
            TENANT,
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "`?{key}=` must be refused on the timeline"
        );
        let body = body_json(response).await;
        assert_eq!(
            body["context"]["violations"][0]["subject"],
            json!(key),
            "the refusal must name the option: {body}"
        );
    }
}

/// `dod-degradation`: above the tenant's ceiling the door answers `503
/// READ_MODEL_OVERLOADED` with `Retry-After` and no rows; another tenant is
/// unaffected (per-partition shedding).
#[tokio::test]
async fn the_limiter_sheds_one_tenant_with_retry_after_and_spares_another() {
    let harness = harness().await;
    let shed_tenant = Uuid::from_u128(0x08_5e);
    ReadPathLimiter::global().set_ceiling_for(shed_tenant, 1);
    let first = get(&harness, "/bss-products/v1/browse", shed_tenant).await;
    assert_eq!(first.status(), StatusCode::OK, "the one token");
    let second = get(&harness, "/bss-products/v1/browse", shed_tenant).await;
    assert_eq!(second.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        second
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok()),
        Some("1")
    );
    let body = body_json(second).await;
    assert!(
        body.get("rows").is_none(),
        "a shed response leaks neither content nor counts: {body}"
    );
    let other = get(&harness, "/bss-products/v1/read/delivery-state", TENANT).await;
    assert_eq!(
        other.status(),
        StatusCode::OK,
        "another tenant's traffic is not shed"
    );
}

/// `dod-history-timeline`: the frozen versions with their changed keys and
/// pseudonyms, a retired head still reachable, an unknown id the miss.
#[tokio::test]
async fn the_timeline_renders_frozen_versions_and_their_diffs() {
    let harness = harness().await;
    let product = draft_product(&harness, "Zeta Line", "eu").await;
    publish_product(&harness, product).await;
    let response = get(
        &harness,
        &format!("/bss-products/v1/products/{product}/versions"),
        TENANT,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(view["entity_id"], json!(product));
    assert_eq!(view["lifecycle_state"], json!("published"));
    let versions = view["versions"].as_array().expect("versions");
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["published_version"], json!(1));
    assert_eq!(versions[0]["actor_pseudonym"], json!(ACTOR));
    assert!(
        versions[0]["changed_keys"]
            .as_array()
            .is_some_and(|keys| keys.iter().any(|k| k == "name")),
        "the first version changes every key: {view}"
    );
    assert!(view["stamp"]["projected_at"].is_string());

    let missing = get(
        &harness,
        &format!("/bss-products/v1/skus/{}/versions", Uuid::now_v7()),
        TENANT,
    )
    .await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}

/// `dod-dashboards`: the three polled tables answer through their doors with
/// the stamp, and refresh with the projector's consumer never involved.
#[tokio::test]
async fn the_three_dashboards_answer_from_their_polled_tables() {
    let harness = harness().await;
    let before =
        body_json(get(&harness, "/bss-products/v1/read/delivery-state", TENANT).await).await;
    assert_eq!(before["polled_at"], json!(null), "before the first poll");
    let product = draft_product(&harness, "Theta Line", "eu").await;
    publish_product(&harness, product).await;
    poll_dashboards(
        &ctx(&harness),
        crate::domain::canonical::write_instant(Utc::now()),
        &tokio_util::sync::CancellationToken::new(),
    )
    .await
    .expect("poll");
    let delivery =
        body_json(get(&harness, "/bss-products/v1/read/delivery-state", TENANT).await).await;
    assert_eq!(
        delivery["inbox_pending"],
        json!(2),
        "two inbox rows above a checkpoint of zero"
    );
    assert!(delivery["polled_at"].is_string());
    let freeze = get(&harness, "/bss-products/v1/read/freeze-status", TENANT).await;
    assert_eq!(freeze.status(), StatusCode::OK);
    assert_eq!(
        body_json(freeze).await["items"],
        json!([]),
        "no catalog version yet"
    );
    let deferred = get(&harness, "/bss-products/v1/read/deferred-intents", TENANT).await;
    assert_eq!(deferred.status(), StatusCode::OK);
    let view = body_json(deferred).await;
    assert_eq!(view["items"], json!([]));
    assert!(
        view["stamp"]["projected_at"].is_string(),
        "the stamp on a dashboard too"
    );
}

/// **Lineage rides the timeline, both ways** (`dod-clone-lineage`, P-D-152).
///
/// A clone's `cloned_from` is a head column no read model exposed, which left
/// `design/11`'s "queryable" justification for having no clone event unmet.
/// The source's timeline now lists the entities cloned from it — a draft clone
/// included, because a clone is born a draft — and the clone's own timeline,
/// once it publishes, names its source and the version it read.
#[tokio::test]
async fn the_timeline_carries_lineage_forward_and_the_reverse_lookup() {
    let harness = harness().await;
    let source = draft_product(&harness, "Lineage Source", "eu").await;
    let source_version = publish_product(&harness, source).await;
    project(&harness).await;

    // A clone: the create path with the lineage columns set, as the clone
    // door writes them (`cloned_from` = the immediate source, the version read).
    let clone_id = Uuid::new_v4();
    let now = crate::domain::canonical::write_instant(Utc::now());
    crate::infra::create::insert_product_with_event(
        &harness.state.db,
        &harness.state.sink,
        scope(),
        NewProduct {
            product_id: clone_id,
            tenant_id: TENANT,
            brand_id: BRAND,
            name: "Lineage Source (copy)".to_owned(),
            name_normalized: crate::domain::name::normalize("Lineage Source (copy)"),
            product_code: Some("LINEAGE-SOURCE-COPY".to_owned()),
            region_scope: "eu".to_owned(),
            brand_scope: String::new(),
            created_by: ACTOR.to_string(),
            created_at: now,
            cloned_from: Some(source),
            cloned_from_version: Some(source_version),
        },
        crate::infra::create::JoinedRecords {
            claim: None,
            stamp: None,
            content: None,
        },
        ACTOR,
        render_nothing,
    )
    .await
    .expect("insert the clone");

    let response = get(
        &harness,
        &format!("/bss-products/v1/products/{source}/versions"),
        TENANT,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert!(
        view["lineage"].is_null(),
        "the source was not itself cloned"
    );
    let clones = view["clones"].as_array().expect("clones");
    assert_eq!(clones.len(), 1, "the reverse lookup lists the draft clone");
    assert_eq!(clones[0]["entity_id"], json!(clone_id));
    assert_eq!(clones[0]["cloned_from_version"], json!(source_version));

    // The clone's own timeline exists once it publishes, and names its source.
    {
        let conn = harness.state.db.conn().expect("conn");
        repo::replace_category_assignments(
            &conn,
            &scope(),
            TENANT,
            clone_id,
            &[(CATEGORY, crate::domain::taxonomy::AssignmentRole::Primary)],
            now,
        )
        .await
        .expect("assign the primary category");
    }
    publish_product(&harness, clone_id).await;
    let response = get(
        &harness,
        &format!("/bss-products/v1/products/{clone_id}/versions"),
        TENANT,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let view = body_json(response).await;
    assert_eq!(view["lineage"]["cloned_from"], json!(source));
    assert_eq!(
        view["lineage"]["cloned_from_version"],
        json!(source_version)
    );
    assert!(
        view["clones"].as_array().expect("clones").is_empty(),
        "nothing was cloned from the clone"
    );
}

/// **The limiter's bucket map is bounded** (P-D-163). Past the high-water
/// mark an acquire drops every bucket idle for a full second, and the drop is
/// lossless: an idle bucket is back at capacity, which is what an absent one
/// means, so the evicted tenant's next acquire still succeeds.
#[test]
fn idle_limiter_buckets_are_evicted_past_the_high_water_mark() {
    let limiter = ReadPathLimiter::new(200);
    let mark = super::LIMITER_BUCKET_HIGH_WATER;
    for i in 1..=(mark + 8) {
        limiter
            .try_acquire(Uuid::from_u128(u128::try_from(i).expect("small")))
            .expect("a fresh tenant has a full bucket");
    }
    let len = || {
        limiter
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len()
    };
    assert!(
        len() > mark,
        "nothing was idle, so nothing was evicted: {} buckets",
        len()
    );

    // Age every bucket past the idle window, then one more acquire.
    {
        let mut buckets = limiter
            .buckets
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for bucket in buckets.values_mut() {
            bucket.refilled_at = bucket
                .refilled_at
                .checked_sub(std::time::Duration::from_secs(2))
                .expect("the clock has been up for two seconds");
        }
    }
    let newcomer = Uuid::from_u128(0xffff_ffff);
    limiter.try_acquire(newcomer).expect("admitted");
    assert_eq!(len(), 1, "only the newcomer's bucket survives the sweep");
    limiter
        .try_acquire(Uuid::from_u128(1))
        .expect("an evicted tenant is back at capacity, exactly as if never seen");
}
