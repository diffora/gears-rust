//! Tests for the increment-request door and its in-process binding —
//! `dod-request-door`'s criteria (`features/catalog-version.md` §6:
//! `REQUEST_SOURCE_UNKNOWN`'s both-halves probe) and the P-D-81 contract.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use sea_orm::{ConnectionTrait, Database};
use sea_orm_migration::MigratorTrait;
use serde_json::json;
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use tower::ServiceExt as _;
use uuid::Uuid;

use bss_products_sdk::increments::{IncrementLane, IncrementRequest, IncrementRequests as _};

use super::{InProcessIncrementRequests, router};
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::test_support::{authed_ctx, flat_in_enforcer};

fn unique_sqlite_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "bss-products-tests-{label}-{}.sqlite3",
        Uuid::new_v4()
    ))
}

const TENANT: Uuid = Uuid::from_u128(0xca_01);

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

async fn harness() -> TestHarness {
    let path = unique_sqlite_path("cvdb");
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

fn app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    let openapi = OpenApiRegistryImpl::new();
    router(api_state(harness), &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

async fn post_request(
    app: Router,
    tenant: Uuid,
    body: &serde_json::Value,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/bss-products/v1/catalog-version-requests")
            .header("content-type", "application/json")
            .extension(authed_ctx(tenant))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn body_json(response: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
        .await
        .expect("read the response body");
    serde_json::from_slice(&bytes).expect("the response body is JSON")
}

/// Seed one committed version row and flip the named request onto it, the
/// way the increment transaction will (state + FK stamped together — the
/// shape `CHECK` admits nothing else).
async fn satisfy_request(harness: &TestHarness, source: &str, request_key: &str, version: i64) {
    // An auxiliary connection into the identical file: the production
    // provider is pinned to one connection, and contending with it from a
    // seeding statement is the harness's own documented trap.
    let conn = Database::connect(&harness.dsn)
        .await
        .expect("open an auxiliary connection");
    // The version row's tenant must be byte-identical to the queue rows'
    // stored form (the composite FK compares them), so it is written by
    // copying an existing queue row's own column rather than by guessing
    // the driver's uuid encoding — the harness's own documented trap.
    conn.execute_unprepared(&format!(
        "INSERT INTO products_catalog_version (tenant_id, catalog_version_id, checksum, \
         digest_version, published_at, participant_set_snapshot, freeze_state) SELECT \
         tenant_id, {version}, 'aa', 1, '2026-09-01T10:00:00Z', '[]', 'open' FROM \
         products_catalog_version_request WHERE {} AND source = '{source}' AND \
         request_key = '{request_key}'",
        crate::test_support::id_matches("tenant_id", TENANT),
    ))
    .await
    .expect("seed the version row");
    conn.execute_unprepared(&format!(
        "UPDATE products_catalog_version_request SET state = 'coalesced', \
         satisfied_by_version_id = {version} WHERE {} AND source = \
         '{source}' AND request_key = '{request_key}'",
        crate::test_support::id_matches("tenant_id", TENANT),
    ))
    .await
    .expect("flip the request as the increment transaction will");
}

/// A registered source enqueues and is acknowledged 202/pending; the same
/// key replays the stored state rather than enqueueing a second demand, and
/// once the coalescer satisfies the row the replay answers the version.
#[tokio::test]
async fn a_request_enqueues_once_and_replays_its_state() {
    let harness = harness().await;

    let body = json!({
        "source": "pricing", "lane": "interactive", "request_key": "plan-7",
    });
    let first = post_request(app_for(&harness, TENANT), TENANT, &body).await;
    assert_eq!(
        first.status(),
        StatusCode::ACCEPTED,
        "the door acknowledges"
    );
    let first_view = body_json(first).await;
    assert_eq!(first_view["coalesced"], json!(false));
    assert_eq!(first_view["catalog_version_id"], json!(null));

    let replay = post_request(app_for(&harness, TENANT), TENANT, &body).await;
    assert_eq!(replay.status(), StatusCode::ACCEPTED);
    assert_eq!(body_json(replay).await["coalesced"], json!(false));

    satisfy_request(&harness, "pricing", "plan-7", 1).await;
    let after = post_request(app_for(&harness, TENANT), TENANT, &body).await;
    let view = body_json(after).await;
    assert_eq!(
        view["coalesced"],
        json!(true),
        "a replay of a satisfied request answers the committed state"
    );
    assert_eq!(view["catalog_version_id"], json!(1));
}

/// §6's both-halves probe: a source outside the registered set is refused
/// AFTER the grant passes, carrying the `CATALOG_VERSION_REJECTED`
/// precondition violation — and the identical request from a registered
/// source succeeds. A refusal that omitted the violation type would be
/// invisible to the consumer's `Rejected` arm.
#[tokio::test]
async fn an_unregistered_source_is_refused_with_the_discriminator() {
    let harness = harness().await;

    let refused = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "billing", "lane": "interactive", "request_key": "r-1" }),
    )
    .await;
    assert_eq!(
        refused.status(),
        StatusCode::BAD_REQUEST,
        "a FailedPrecondition renders 400 on the wire"
    );
    let view = body_json(refused).await;
    assert_eq!(
        view["context"]["violations"][0]["type"],
        json!("CATALOG_VERSION_REJECTED"),
        "the violation type is the consumer projection's discriminator"
    );

    let admitted = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "pricing", "lane": "interactive", "request_key": "r-1" }),
    )
    .await;
    assert_eq!(
        admitted.status(),
        StatusCode::ACCEPTED,
        "the same request from a registered source succeeds"
    );
}

/// The lane's batching operand is judged both ways: a bulk request must
/// name its `operation_key`, an interactive one must not.
#[tokio::test]
async fn the_operation_key_belongs_to_the_bulk_lane() {
    let harness = harness().await;

    let bulk_without = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "pricing", "lane": "bulk", "request_key": "b-1" }),
    )
    .await;
    assert_eq!(bulk_without.status(), StatusCode::BAD_REQUEST);

    let interactive_with = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({
            "source": "pricing", "lane": "interactive", "request_key": "i-1",
            "operation_key": "op-1",
        }),
    )
    .await;
    assert_eq!(interactive_with.status(), StatusCode::BAD_REQUEST);

    let bulk_with = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({
            "source": "pricing", "lane": "bulk", "request_key": "b-2",
            "operation_key": "op-1",
        }),
    )
    .await;
    assert_eq!(bulk_with.status(), StatusCode::ACCEPTED);
}

/// The in-process binding runs the identical gate and core (P-D-15): a
/// request through the SDK trait lands on the same queue the wire door
/// serves, and the poll answers `None` until the coalescer satisfies the
/// row, then the committed version.
#[tokio::test]
async fn the_in_process_binding_shares_the_queue_and_the_poll() {
    let harness = harness().await;
    let binding = InProcessIncrementRequests {
        state: api_state(&harness),
        enforcer: flat_in_enforcer(TENANT),
    };
    let ctx = authed_ctx(TENANT);

    let ack = binding
        .request(
            &ctx,
            TENANT,
            IncrementRequest {
                source: "pricing".to_owned(),
                lane: IncrementLane::Interactive,
                request_key: "sdk-1".to_owned(),
                operation_key: None,
            },
        )
        .await
        .expect("the binding acknowledges");
    assert!(!ack.coalesced);

    let pending = binding
        .committed(&ctx, TENANT, "pricing", "sdk-1")
        .await
        .expect("the poll answers");
    assert_eq!(pending, None, "None while the batch has not committed");

    satisfy_request(&harness, "pricing", "sdk-1", 3).await;
    let committed = binding
        .committed(&ctx, TENANT, "pricing", "sdk-1")
        .await
        .expect("the poll answers")
        .expect("the row is satisfied");
    assert_eq!(committed.catalog_version_id, 3);

    // The wire door replays the SDK-enqueued key: one contract, one queue.
    let via_wire = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "pricing", "lane": "interactive", "request_key": "sdk-1" }),
    )
    .await;
    assert_eq!(body_json(via_wire).await["catalog_version_id"], json!(3));
}

/// P-D-82: every stored instant is truncated to microseconds at the write —
/// the queue's `requested_at` here, asserted at the driver level so neither
/// engine ever holds a digit the other could round.
#[tokio::test]
async fn stored_instants_carry_no_sub_microsecond_digits() {
    let harness = harness().await;
    let response = post_request(
        app_for(&harness, TENANT),
        TENANT,
        &json!({ "source": "pricing", "lane": "interactive", "request_key": "t-1" }),
    )
    .await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);

    let stored = crate::test_support::raw_string_opt(
        &harness.dsn,
        &format!(
            "SELECT requested_at AS v FROM products_catalog_version_request WHERE \
             {} AND request_key = 't-1'",
            crate::test_support::id_matches("tenant_id", TENANT),
        ),
    )
    .await
    .expect("the row exists");
    let fraction = stored.split('.').nth(1).unwrap_or("");
    let digits: String = fraction.chars().take_while(char::is_ascii_digit).collect();
    assert!(
        digits.len() <= 6 || digits[6..].chars().all(|c| c == '0'),
        "no sub-microsecond digit survives the write: {stored}"
    );
}

// ---------------------------------------------------------------------------
// The freeze doors and the resolver (P-D-84; dod-ack-door,
// dod-liveness-and-release, dod-intentful-resolver, dod-version-binding)

mod freeze_and_resolve_tests {
    use chrono::Duration as ChronoDuration;
    use sea_orm::ActiveValue::Set;
    use sea_orm::EntityTrait as _;
    use toolkit_db::secure::SecureInsertExt as _;

    use super::*;
    use crate::infra::increment::{DrainOutcome, drain_tenant};
    use crate::infra::storage::entity::freeze_participant;
    use crate::infra::storage::repo::{self, NewIncrementRequest};

    fn scope() -> toolkit_db::secure::AccessScope {
        toolkit_db::secure::AccessScope::for_tenant(TENANT)
    }

    /// Return the pinned production connection before the drain checks one
    /// out again (`DbConn` is not `Drop`; the named fn keeps clippy's
    /// mem-drop lint quiet without a scope pyramid).
    fn drop_pinned<T>(conn: T) {
        let _returned = conn;
    }

    /// Register participants, enqueue aged demand and drain: one committed
    /// `open` version with one `pending` ledger row per participant.
    async fn seed_open_version(harness: &TestHarness, participants: &[&str]) -> i64 {
        let conn = harness.db.conn().expect("conn");
        for participant in participants {
            let model = freeze_participant::ActiveModel {
                tenant_id: Set(TENANT),
                participant: Set((*participant).to_owned()),
                registered_at: Set(chrono::Utc::now() - ChronoDuration::hours(1)),
            };
            freeze_participant::Entity::insert(model.clone())
                .secure()
                .scope_with_model(&scope(), &model)
                .expect("scope")
                .exec(&conn)
                .await
                .expect("register the participant");
        }
        repo::enqueue_increment_request(
            &conn,
            &scope(),
            TENANT,
            NewIncrementRequest {
                source: "pricing",
                request_key: "fz-seed",
                lane: "interactive",
                operation_key: None,
                requested_at: chrono::Utc::now() - ChronoDuration::seconds(10),
            },
        )
        .await
        .expect("enqueue");
        drop_pinned(conn);
        let outcome = drain_tenant(&harness.db, TENANT, chrono::Utc::now())
            .await
            .expect("drain");
        match outcome {
            DrainOutcome::Committed {
                catalog_version_id, ..
            } => catalog_version_id,
            other => panic!("the seed drain must commit, got {other:?}"),
        }
    }

    async fn post_edge(
        app: Router,
        version: i64,
        act: &str,
        participant: &str,
    ) -> axum::http::Response<Body> {
        app.oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/bss-products/v1/catalog-versions/{version}/{act}"))
                .header("content-type", "application/json")
                .extension(authed_ctx(TENANT))
                .body(Body::from(
                    json!({ "participant": participant }).to_string(),
                ))
                .expect("build the request"),
        )
        .await
        .expect("the router answers")
    }

    async fn get_resolve(app: Router, version: i64, query: &str) -> axum::http::Response<Body> {
        app.oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/bss-products/v1/catalog-versions/{version}{query}"
                ))
                .extension(authed_ctx(TENANT))
                .body(Body::empty())
                .expect("build the request"),
        )
        .await
        .expect("the router answers")
    }

    /// The ack door flips the seeded row and the LAST member's ack lands
    /// `complete` in the same transaction (P-D-73); each committed act
    /// writes its audit row and no broker event.
    #[tokio::test]
    async fn an_ack_flips_the_row_and_the_last_ack_completes() {
        let harness = harness().await;
        let version = seed_open_version(&harness, &["billing", "pricing"]).await;

        let first = post_edge(app_for(&harness, TENANT), version, "acks", "pricing").await;
        assert_eq!(first.status(), StatusCode::OK);
        let view = body_json(first).await;
        assert_eq!(view["state"], json!("acked"));
        assert_eq!(
            view["freeze_state"],
            json!("open"),
            "one of two members acked: still open"
        );

        let second = post_edge(app_for(&harness, TENANT), version, "acks", "billing").await;
        let view = body_json(second).await;
        assert_eq!(
            view["freeze_state"],
            json!("complete"),
            "the last member's ack lands complete"
        );

        let audits = crate::test_support::raw_i64(
            &harness.dsn,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE action = \
             'catalog_version.freeze.ack'",
        )
        .await;
        assert_eq!(audits, 2, "each committed ack is audit-plane");
    }

    /// A release settles exactly as an ack under P-D-84's predicate — and
    /// the door stamps NOTHING into `released_at`, that column being the
    /// force ceremony's alone (P-D-67).
    #[tokio::test]
    async fn a_release_settles_like_an_ack_and_stamps_nothing() {
        let harness = harness().await;
        let version = seed_open_version(&harness, &["billing", "pricing"]).await;

        post_edge(app_for(&harness, TENANT), version, "acks", "pricing").await;
        let release = post_edge(app_for(&harness, TENANT), version, "releases", "billing").await;
        assert_eq!(release.status(), StatusCode::OK);
        let view = body_json(release).await;
        assert_eq!(view["state"], json!("released"));
        assert_eq!(
            view["freeze_state"],
            json!("complete"),
            "released settles: no pending row remains"
        );

        let released_at = crate::test_support::raw_string_opt(
            &harness.dsn,
            "SELECT released_at AS v FROM products_freeze_ack WHERE participant = 'billing'",
        )
        .await;
        assert_eq!(
            released_at, None,
            "the release door never stamps released_at"
        );
    }

    /// A re-ack replays idempotently; an ack after a release is the state
    /// machine's own refusal.
    #[tokio::test]
    async fn a_re_ack_replays_and_a_released_participant_cannot_ack() {
        let harness = harness().await;
        let version = seed_open_version(&harness, &["pricing"]).await;

        post_edge(app_for(&harness, TENANT), version, "acks", "pricing").await;
        let replay = post_edge(app_for(&harness, TENANT), version, "acks", "pricing").await;
        assert_eq!(replay.status(), StatusCode::OK, "idempotent per the PK");

        post_edge(app_for(&harness, TENANT), version, "releases", "pricing").await;
        let after_release = post_edge(app_for(&harness, TENANT), version, "acks", "pricing").await;
        assert_eq!(
            after_release.status(),
            StatusCode::CONFLICT,
            "released is terminal for the ack edge"
        );
        let view = body_json(after_release).await;
        assert_eq!(view["context"]["reason"], json!("ILLEGAL_TRANSITION"));
    }

    /// A principal outside the version's snapshotted set is refused 403
    /// `PARTICIPANT_UNKNOWN` — membership is the seeded row's existence, and
    /// a 404 would leak whether the version exists.
    #[tokio::test]
    async fn a_non_member_is_refused_participant_unknown() {
        let harness = harness().await;
        let version = seed_open_version(&harness, &["pricing"]).await;

        let refused = post_edge(app_for(&harness, TENANT), version, "acks", "billing").await;
        assert_eq!(refused.status(), StatusCode::FORBIDDEN);
        let view = body_json(refused).await;
        assert_eq!(view["context"]["reason"], json!("PARTICIPANT_UNKNOWN"));

        let bogus = post_edge(app_for(&harness, TENANT), 999, "acks", "pricing").await;
        assert_eq!(
            bogus.status(),
            StatusCode::FORBIDDEN,
            "an unknown version answers the same 403: no existence leak"
        );
    }

    /// The resolver requires intent, serves browse at once, and serves the
    /// same bytes on every re-resolution (inst-rv-bytes) — from the stored
    /// manifest, checksum re-verified before serving.
    #[tokio::test]
    async fn the_resolver_requires_intent_and_serves_stable_bytes() {
        let harness = harness().await;
        let version = seed_open_version(&harness, &["pricing"]).await;

        let missing = get_resolve(app_for(&harness, TENANT), version, "").await;
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
        let view = body_json(missing).await;
        assert_eq!(
            view["context"]["violations"][0]["type"],
            json!("INTENT_REQUIRED")
        );

        let first = get_resolve(app_for(&harness, TENANT), version, "?intent=browse").await;
        assert_eq!(first.status(), StatusCode::OK);
        let first_view = body_json(first).await;
        assert_eq!(first_view["freeze_complete"], json!(false));
        assert!(
            first_view["checksum"]
                .as_str()
                .is_some_and(|c| c.len() == 64),
            "the hex checksum is returned and verifiable"
        );

        let second = get_resolve(app_for(&harness, TENANT), version, "?intent=browse").await;
        assert_eq!(
            body_json(second).await,
            first_view,
            "re-resolution is byte-identical, rendered from the stored manifest"
        );
    }

    /// `posted` fails closed while the ledger holds a pending row and
    /// serves once the last member settles — the strict flag flipping with
    /// it (P-D-84 arm 3).
    #[tokio::test]
    async fn posted_is_fail_closed_until_the_ledger_settles() {
        let harness = harness().await;
        let version = seed_open_version(&harness, &["pricing"]).await;

        let early = get_resolve(app_for(&harness, TENANT), version, "?intent=posted").await;
        assert_eq!(early.status(), StatusCode::CONFLICT);
        assert_eq!(
            body_json(early).await["context"]["reason"],
            json!("FREEZE_INCOMPLETE")
        );

        post_edge(app_for(&harness, TENANT), version, "acks", "pricing").await;
        let served = get_resolve(app_for(&harness, TENANT), version, "?intent=posted").await;
        assert_eq!(served.status(), StatusCode::OK);
        assert_eq!(body_json(served).await["freeze_complete"], json!(true));
    }

    /// A force-completed version refuses `posted` naming each still-forced
    /// participant, and the strict flag stays false (P-D-84 arm 3; the
    /// forced rows seeded the way the ceremony writes them — state, both
    /// stamps and the ceremony ref together, the shape CHECK's own pairing).
    #[tokio::test]
    async fn a_forced_version_refuses_posted_naming_the_silent() {
        let harness = harness().await;
        let version = seed_open_version(&harness, &["pricing"]).await;

        let conn = sea_orm::Database::connect(&harness.dsn)
            .await
            .expect("aux connection");
        // The stamps copy an instant sea-orm itself wrote, so the driver's
        // own text format is preserved (the harness's uuid trap, for dates).
        conn.execute_unprepared(&format!(
            "UPDATE products_freeze_ack SET state = 'not_frozen(forced)', forced_at = \
             (SELECT published_at FROM products_catalog_version WHERE catalog_version_id = \
             {version}), ceremony_ref = X'00000000000000000000000000000001', \
             released_at = (SELECT published_at FROM products_catalog_version WHERE \
             catalog_version_id = {version}) WHERE participant = 'pricing'"
        ))
        .await
        .expect("force the row as the ceremony will");
        conn.execute_unprepared(&format!(
            "UPDATE products_catalog_version SET freeze_state = 'complete(forced)' WHERE \
             catalog_version_id = {version}"
        ))
        .await
        .expect("flip the cache as the ceremony will");

        let refused = get_resolve(app_for(&harness, TENANT), version, "?intent=posted").await;
        assert_eq!(refused.status(), StatusCode::CONFLICT);
        let view = body_json(refused).await;
        assert_eq!(
            view["context"]["reason"],
            json!("VERSION_FORCED_INCOMPLETE")
        );
        assert!(
            view["detail"]
                .as_str()
                .is_some_and(|d| d.contains("pricing")),
            "the refusal names each not_frozen(forced) participant"
        );

        let browse = get_resolve(app_for(&harness, TENANT), version, "?intent=browse").await;
        let view = body_json(browse).await;
        assert_eq!(
            view["freeze_complete"],
            json!(false),
            "the strict flag never reads complete(forced) as complete"
        );
    }

    /// An unknown id is the resolver's own 404 — the single raising door of
    /// `CATALOG_VERSION_UNKNOWN`.
    #[tokio::test]
    async fn an_unknown_version_is_the_resolvers_404() {
        let harness = harness().await;
        let missing = get_resolve(app_for(&harness, TENANT), 99, "?intent=browse").await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    /// A differing `bound_version` surfaces the re-binding triple
    /// (`dod-version-binding`): the diff is handed TO the module.
    #[tokio::test]
    async fn a_differing_bound_version_surfaces_the_rebinding_triple() {
        let harness = harness().await;
        let version = seed_open_version(&harness, &["pricing"]).await;

        let same = get_resolve(
            app_for(&harness, TENANT),
            version,
            &format!("?intent=browse&bound_version={version}"),
        )
        .await;
        assert_eq!(body_json(same).await["diff_ref"], json!(null));

        let moved = get_resolve(
            app_for(&harness, TENANT),
            version,
            "?intent=browse&bound_version=7",
        )
        .await;
        let view = body_json(moved).await;
        assert_eq!(view["bound_version"], json!(7));
        assert_eq!(view["resolved_version"], json!(version));
        assert_eq!(view["diff_ref"], json!(format!("7..{version}")));
    }
}
