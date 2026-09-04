//! `10-retention-erasure`'s two doors, exercised through the router.
//!
//! # The subject principal is seeded through the repository, not a door
//!
//! No door mints a ref for an arbitrary principal: `resolve_creator_actor_ref`
//! mints for the **caller**, and `authed_ctx` hands every call a fresh
//! `subject_id`, so two requests never share a principal. The subject of an
//! erasure therefore has to be put there directly, through the same
//! `resolve_actor_ref` the shared actor context uses.
//!
//! # Every audit assertion counts rows rather than reading one
//!
//! `raw_string_opt` panics when its query matches no row, so a probe built on
//! it can only ever answer true; a count answers zero, which is what the
//! negative half of *"the access was audited"* needs.
#![allow(clippy::expect_used)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use sea_orm::{ColumnTrait, EntityTrait};
use serde_json::{Value as JsonValue, json};
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::{AccessScope, SecureEntityExt};
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use tower::ServiceExt;
use uuid::Uuid;

use sea_orm_migration::MigratorTrait;

use super::router;
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::infra::events;
use crate::infra::storage::entity::entity_version;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo;
use crate::test_support::{authed_ctx, flat_in_enforcer, raw_i64};

const TENANT: Uuid = Uuid::from_u128(0x7e_43);
const ALICE: &str = "principal:alice";

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
    let path = std::env::temp_dir().join(format!(
        "bss-products-retention-tests-{}.sqlite3",
        Uuid::new_v4()
    ));
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

fn app_for(harness: &TestHarness, tenant: Uuid) -> Router {
    let state = Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// Mint a live map entry for a principal, the way the shared actor context
/// does. See this module's own doc for why a door cannot do it.
async fn seed_principal(harness: &TestHarness, principal_ref: &str) -> Uuid {
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    repo::resolve_actor_ref(
        &conn,
        &scope,
        TENANT,
        principal_ref,
        crate::domain::canonical::write_instant(chrono::Utc::now()),
    )
    .await
    .expect("mint the subject's ref")
}

async fn erase(app: Router, principal_ref: &str, reason: &str) -> axum::http::Response<Body> {
    let body = json!({ "principal_ref": principal_ref, "reason": reason });
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/bss-products/v1/erasure-requests")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

/// The justification every export carries unless a case is about its absence.
const WHY: &str = "DSAR ticket 4471";

async fn export(app: Router, principal_ref: &str) -> axum::http::Response<Body> {
    export_with(app, principal_ref, WHY).await
}

async fn export_with(
    app: Router,
    principal_ref: &str,
    justification: &str,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(format!(
                "/bss-products/v1/compliance/identity-export?principalRef={principal_ref}\
                 &justification={}",
                justification.replace(' ', "%20")
            ))
            .extension(authed_ctx(TENANT))
            .body(Body::empty())
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

/// Offer an allow-list entry, defaulting every field a case does not pin.
/// One satisfied record for the allow-list's live-op subject — every
/// allow-list act runs the stored host (P-D-144) and spends one.
async fn seed_allowlist_record(harness: &TestHarness) {
    crate::test_support::seed_satisfied_approval(
        &harness.db,
        TENANT,
        crate::domain::governance::GateSubject::governed_live_op(
            TENANT,
            "pii_allowlist",
            crate::domain::governance::SubjectPin::Unpinned,
        ),
        0,
    )
    .await;
}

async fn sign_off(
    harness: &TestHarness,
    value: &str,
    signed_off_by: &str,
) -> axum::http::Response<Body> {
    seed_allowlist_record(harness).await;
    sign_off_via(app_for(harness, TENANT), value, signed_off_by).await
}

async fn sign_off_via(app: Router, value: &str, signed_off_by: &str) -> axum::http::Response<Body> {
    let body = json!({
        "value": value,
        "justification": "product line named for its founder",
        "signed_off_by": signed_off_by,
        "signed_off_at": "2026-09-01T00:00:00Z",
    });
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri("/bss-products/v1/pii-allowlist-entries")
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn revoke(harness: &TestHarness, entry_id: Uuid) -> axum::http::Response<Body> {
    seed_allowlist_record(harness).await;
    revoke_via(app_for(harness, TENANT), entry_id).await
}

async fn revoke_via(app: Router, entry_id: Uuid) -> axum::http::Response<Body> {
    let body = json!({ "op": "revoke", "reason": "the sign-off lapsed" });
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(format!(
                "/bss-products/v1/pii-allowlist-entries/{entry_id}/operations"
            ))
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(authed_ctx(TENANT))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn allowlist_review(app: Router) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri("/bss-products/v1/compliance/pii-allowlist")
            .extension(authed_ctx(TENANT))
            .body(Body::empty())
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn body_json(response: axum::http::Response<Body>) -> JsonValue {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read the body"),
    )
    .expect("the body is JSON")
}

/// The wire code of a refusal, read off the **violation** rather than off
/// `context.reason`.
///
/// The sibling door suites read `context.reason`, which is where a
/// `resource_error` builder's denial puts it. This feature's codes reach the
/// wire through `error_mapping`'s `precondition(...)`, which renders each
/// violation's own `code` as its `type` -- a different place in the same
/// envelope, and reading the sibling's path here answered an empty string on
/// a body that carried the code correctly.
async fn error_code(response: axum::http::Response<Body>) -> String {
    let body = body_json(response).await;
    body["context"]["violations"][0]["type"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

async fn audit_rows(dsn: &str, action: &str) -> i64 {
    raw_i64(
        dsn,
        &format!("SELECT COUNT(*) AS v FROM products_audit_log WHERE action = '{action}'"),
    )
    .await
}

/// **The erasure retires the ref, answers it, and writes its evidential row
/// in the same transaction.**
#[tokio::test]
async fn an_erasure_retires_the_ref_and_records_it() {
    let harness = harness().await;
    let seeded = seed_principal(&harness, ALICE).await;

    let response = erase(app_for(&harness, TENANT), ALICE, "dsar-2026-114").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(
        body["actor_ref"].as_str().expect("the retired ref"),
        seeded.to_string(),
        "the door answers the ref it retired"
    );
    assert!(body["tombstoned_at"].is_string());
    assert_eq!(
        audit_rows(&harness.dsn, "erasure.execute").await,
        1,
        "the evidential row committed with the tombstone"
    );
}

/// **An unknown principal is refused `ERASURE_UNKNOWN_ACTOR` and mints
/// nothing.**
///
/// The mint half is the one that matters: the shared actor context would have
/// created a live row for this principal, and a door built on it would report
/// a successful erasure of a principal it had just invented. The export is
/// used as the read-back so the assertion goes through a door rather than
/// around one.
#[tokio::test]
async fn an_unknown_principal_is_refused_and_nothing_is_minted() {
    let harness = harness().await;

    let response = erase(app_for(&harness, TENANT), "principal:nobody", "dsar-x").await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(error_code(response).await, "ERASURE_UNKNOWN_ACTOR");

    let seen = export(app_for(&harness, TENANT), "principal:nobody").await;
    assert_eq!(seen.status(), axum::http::StatusCode::OK);
    let body = body_json(seen).await;
    assert_eq!(
        body["entries"].as_array().expect("entries").len(),
        0,
        "the refusal minted no row: {body}"
    );
}

/// **A blank reason is refused**, because the evidential row is the point of
/// the act and a row with no reason is not evidence. The positive control is
/// every other case here, which supplies one and succeeds.
#[tokio::test]
async fn a_blank_reason_is_refused() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;

    let response = erase(app_for(&harness, TENANT), ALICE, "   ").await;

    assert_eq!(response.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(
        audit_rows(&harness.dsn, "erasure.execute").await,
        0,
        "and nothing was erased"
    );
}

/// **The export returns the tombstoned entry and the audit references, and
/// audits the access.**
#[tokio::test]
async fn the_export_returns_the_tombstone_and_audits_the_access() {
    let harness = harness().await;
    let seeded = seed_principal(&harness, ALICE).await;
    let erased = erase(app_for(&harness, TENANT), ALICE, "dsar-2026-114").await;
    assert_eq!(erased.status(), axum::http::StatusCode::OK);

    let response = export(app_for(&harness, TENANT), ALICE).await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = body_json(response).await;
    let entries = body["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "{body}");
    assert_eq!(entries[0]["actor_ref"], seeded.to_string());
    assert!(
        entries[0]["tombstoned_at"].is_string(),
        "a DSAR after an erasure must be able to see that the erasure happened: {body}"
    );
    assert_eq!(
        body["audit_references"]
            .as_array()
            .expect("audit references")
            .len(),
        1,
        "the erasure's own row carries the retired ref: {body}"
    );
    assert_eq!(
        audit_rows(&harness.dsn, "compliance.export").await,
        1,
        "and the access itself is audited individually"
    );
}

/// **An export that returns nothing is audited too.**
///
/// The access is the audited event, not the answer. A door that audited only
/// non-empty exports would leave every probe of a principal's presence
/// unrecorded, which is the reconnaissance an individually audited surface
/// exists to catch.
#[tokio::test]
async fn an_empty_export_is_audited_all_the_same() {
    let harness = harness().await;

    let response = export(app_for(&harness, TENANT), "principal:nobody").await;

    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["entries"].as_array().expect("entries").len(), 0);
    assert_eq!(
        audit_rows(&harness.dsn, "compliance.export").await,
        1,
        "the access is the audited event, not the answer"
    );
}

/// **§6's positive-control line for `ERASURE_UNKNOWN_ACTOR`: both arms, one
/// case.**
///
/// *"A principal with no `actor_ref` in this tenant is refused **naming the
/// principal**; the same request for a principal that has one succeeds."* The
/// two arms are asserted together because a refusal-only case passes on a door
/// that refuses everything, and a success-only case passes on one that refuses
/// nothing.
///
/// **"Naming the principal" is asserted on the wire, not on the internal
/// detail.** It is the clause row 24 turned on: the refusal may name the
/// principal precisely *because* the request carries a `principal_ref` and not
/// a real-world identity, so a refusal that echoes it writes a pseudonym into
/// its own audit row and nothing more. If the door ever took an identity
/// string, this assertion is the one that would have to be deleted -- which is
/// what makes it worth making.
#[tokio::test]
async fn the_unknown_actor_refusal_names_the_principal_and_its_control_succeeds() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;

    let refused = erase(app_for(&harness, TENANT), "principal:nobody", "dsar-x").await;
    assert_eq!(refused.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = body_json(refused).await;
    assert_eq!(
        body["context"]["violations"][0]["type"],
        "ERASURE_UNKNOWN_ACTOR"
    );
    let described = body["context"]["violations"][0]["description"]
        .as_str()
        .expect("the violation carries a description");
    assert!(
        described.contains("principal:nobody"),
        "the refusal names the principal it could not resolve: {described}"
    );

    let succeeded = erase(app_for(&harness, TENANT), ALICE, "dsar-2026-114").await;
    assert_eq!(
        succeeded.status(),
        axum::http::StatusCode::OK,
        "the control: the same request for a principal that has a ref succeeds"
    );
}

/// **C1, the flagship: the erasure moves nothing inside a frozen record.**
///
/// §6 asks for both halves in one probe, *"either half alone passes on a build
/// that got the other wrong"*: the frozen row's digest is byte-identical after
/// the erasure, **and** the map shows the tombstone. The frozen row is stamped
/// with the erased principal's own ref, which is the case that matters -- a
/// version published by the very actor being erased. Erasure is a map-only
/// tombstone precisely so this holds, and nothing else in the crate asserts
/// it.
#[tokio::test]
async fn an_erasure_leaves_a_frozen_record_byte_identical() {
    let harness = harness().await;
    let seeded = seed_principal(&harness, ALICE).await;
    let entity_id = Uuid::from_u128(0xf0_99);
    let digest = vec![0x11_u8, 0x22, 0x33, 0x44];

    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    repo::insert_entity_version(
        &conn,
        &scope,
        repo::NewEntityVersion {
            tenant_id: TENANT,
            entity_kind: repo::VersionedEntityKind::Product,
            entity_id,
            published_version: 1,
            content: "{\"name\":\"Fibre 500\"}".to_owned(),
            content_digest: digest.clone(),
            digest_version: 1,
            approval_ref: None,
            // The frozen row is stamped with the ref about to be erased.
            actor_ref: seeded,
            published_at: crate::domain::canonical::write_instant(chrono::Utc::now()),
        },
    )
    .await
    .expect("freeze a version");

    let erased = erase(app_for(&harness, TENANT), ALICE, "dsar-2026-114").await;
    assert_eq!(erased.status(), axum::http::StatusCode::OK);

    // Half one: the immutable record did not move.
    // Read the row back through the entity, not through raw SQL: `SQLite`
    // stores a uuid as a blob, so `WHERE entity_id = '<hyphenated>'` matches
    // nothing at all -- an equality that answers zero for a reason that has
    // nothing to do with the claim under test.
    let frozen = entity_version::Entity::find()
        .secure()
        .scope_with(&scope)
        .filter(
            sea_orm::Condition::all()
                .add(entity_version::Column::TenantId.eq(TENANT))
                .add(entity_version::Column::EntityId.eq(entity_id)),
        )
        .one(&conn)
        .await
        .expect("read the frozen row")
        .expect("it is still there");

    assert_eq!(
        frozen.content_digest, digest,
        "the digest is byte-identical after the erasure"
    );
    assert_eq!(
        frozen.actor_ref, seeded,
        "and the frozen row still carries the erased principal's pseudonym, \
         which is what makes the record readable without re-identifying anyone"
    );

    // Half two: and the map shows the tombstone, so the probe cannot pass by
    // the erasure simply not having happened.
    let export = export(app_for(&harness, TENANT), ALICE).await;
    let body = body_json(export).await;
    assert!(
        body["entries"][0]["tombstoned_at"].is_string(),
        "the erasure did happen: {body}"
    );
}

/// **Two exports are two audit rows** -- *"every access individually"*, which
/// a per-principal or a per-day row would not satisfy.
#[tokio::test]
async fn every_access_is_its_own_audit_row() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;

    for _ in 0..2 {
        let response = export(app_for(&harness, TENANT), ALICE).await;
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    assert_eq!(audit_rows(&harness.dsn, "compliance.export").await, 2);
}

// -- The allow-list doors (`dod-pii-allowlist`) and the export's
//    justification (`dod-compliance-export`) --

/// How many outbox rows carry `payload_type`.
///
/// `COUNT(*)` always answers a row, so the zero case is a real read rather
/// than `raw_string_opt`'s missing-row panic — this module's own doc rule.
async fn outbox_rows(dsn: &str, payload_type: &str) -> i64 {
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    raw_i64(
        dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table} WHERE payload_type = '{payload_type}'"),
    )
    .await
}

/// One string column, where the case has already established the row exists.
async fn raw_string(dsn: &str, sql: &str) -> String {
    crate::test_support::raw_string_opt(dsn, sql)
        .await
        .expect("the case seeded the row this reads")
}

/// Read an entry id off a sign-off receipt.
async fn signed_entry_id(response: axum::http::Response<Body>) -> Uuid {
    let body = body_json(response).await;
    body["entry_id"]
        .as_str()
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .unwrap_or_else(|| panic!("the receipt carries an entryId: {body}"))
}

/// **An entry offered without its Legal sign-off reference is refused riding
/// `VALIDATION` and naming the field — and the same entry WITH one is
/// admitted.**
///
/// §6's own words: *"asserted with its positive control — a mandatory-field
/// rule proven only by its refusal is a rule that may never admit
/// anything"*. The two halves are one case so the control cannot be deleted
/// separately from the rule it guards.
///
/// **P-D-64** is what makes the code `VALIDATION` rather than a minted one:
/// a missing mandatory member of the offered entry is a shape-class refusal,
/// the caller's discriminator is the violation's **field**, and this
/// feature's owned roster stays at one code.
#[tokio::test]
async fn a_missing_sign_off_reference_is_refused_by_field_and_a_complete_entry_is_admitted() {
    let harness = harness().await;

    let refused = sign_off(&harness, "Ann Fritz", "   ").await;
    assert_eq!(refused.status(), axum::http::StatusCode::BAD_REQUEST);
    let body = body_json(refused).await;
    assert_eq!(
        body["context"]["violations"][0]["type"]
            .as_str()
            .unwrap_or_default(),
        "VALIDATION",
        "the missing sign-off rides 01's VALIDATION (P-D-64), never a minted code"
    );
    assert_eq!(
        body["context"]["violations"][0]["subject"]
            .as_str()
            .unwrap_or_default(),
        "signedOffBy",
        "the caller's discriminator is the field, so the field is what the violation must name"
    );

    let admitted = sign_off(&harness, "Ann Fritz", "legal-2026-114").await;
    assert_eq!(
        admitted.status(),
        axum::http::StatusCode::OK,
        "the positive control: the same entry with a reference is admitted"
    );
}

/// **The stored value is the normalized one, and the receipt echoes it.**
///
/// The normalization is the whole of the match rule, so an operator has to be
/// able to see what it made of their input — a rule the caller cannot observe
/// is one they cannot satisfy on the second attempt.
#[tokio::test]
async fn the_receipt_echoes_the_normalized_value_the_detector_will_match() {
    let harness = harness().await;
    let receipt = sign_off(&harness, "  Ann   FRITZ  ", "legal-1").await;
    assert_eq!(receipt.status(), axum::http::StatusCode::OK);
    let body = body_json(receipt).await;
    assert_eq!(body["value_normalized"], "ann fritz");
    assert_eq!(body["state"], "active");
}

/// **A second ACTIVE entry for one normalized value is refused, and the same
/// value is admitted again once the first is revoked.**
///
/// Both arms, because the **partial** predicate is the whole mechanism: a
/// total `UNIQUE` would pass the first half and fail the second, and a table
/// with no index would pass the second and fail the first.
#[tokio::test]
async fn the_active_uniqueness_is_partial_and_a_revoked_value_may_be_signed_off_again() {
    let harness = harness().await;

    let first = sign_off(&harness, "Ann Fritz", "legal-1").await;
    assert_eq!(first.status(), axum::http::StatusCode::OK);
    let entry_id = signed_entry_id(first).await;

    let duplicate = sign_off(&harness, "ANN  fritz", "legal-2").await;
    assert_ne!(
        duplicate.status(),
        axum::http::StatusCode::OK,
        "a second ACTIVE entry for the same normalized value is refused by \
         uq_products_pii_allowlist_active - and note the input differs only in case and \
         spacing, so this also proves the index sees the normalized form"
    );

    let revoked = revoke(&harness, entry_id).await;
    assert_eq!(revoked.status(), axum::http::StatusCode::OK);

    let again = sign_off(&harness, "Ann Fritz", "legal-3").await;
    assert_eq!(
        again.status(),
        axum::http::StatusCode::OK,
        "once the first is revoked the value is signable again - the partial predicate's \
         other half"
    );
}

/// **A revocation is a state flip and the row survives with its sign-off.**
///
/// P-D-47's reasoning is only true if the row is still there: the control the
/// allow-list is *is* the paper sign-off plus the export, and a `DELETE`
/// would take the sign-off out of both.
#[tokio::test]
async fn a_revocation_keeps_the_row_and_its_sign_off_in_the_review() {
    let harness = harness().await;
    let entry_id = signed_entry_id(sign_off(&harness, "Ann Fritz", "legal-1").await).await;
    revoke(&harness, entry_id).await;

    let review = allowlist_review(app_for(&harness, TENANT)).await;
    assert_eq!(review.status(), axum::http::StatusCode::OK);
    let body = body_json(review).await;
    let entries = body["entries"].as_array().expect("the review is a list");
    assert_eq!(
        entries.len(),
        1,
        "the revoked row is IN the review, not gone"
    );
    assert_eq!(entries[0]["state"], "revoked");
    assert_eq!(
        entries[0]["signed_off_by"], "legal-1",
        "the sign-off that admitted it is what the revocation must not destroy"
    );
}

/// **Revoking an entry that is not active answers 404, and answers it the
/// same way for one that never existed.**
#[tokio::test]
async fn revoking_a_missing_or_already_revoked_entry_is_a_404() {
    let harness = harness().await;
    let unknown = revoke(&harness, Uuid::now_v7()).await;
    assert_eq!(unknown.status(), axum::http::StatusCode::NOT_FOUND);

    let entry_id = signed_entry_id(sign_off(&harness, "Ann Fritz", "legal-1").await).await;
    revoke(&harness, entry_id).await;
    let twice = revoke(&harness, entry_id).await;
    assert_eq!(
        twice.status(),
        axum::http::StatusCode::NOT_FOUND,
        "already-revoked and never-existed are the same fact from the caller's side"
    );
}

/// **Each allow-list act writes exactly one audit row and enqueues exactly
/// one `PiiAllowlistChanged`.**
///
/// Counted, not sampled: a probe that reads *a* row cannot tell one write
/// from three, and the event is what a cache-busting consumer subscribes to.
#[tokio::test]
async fn each_allowlist_act_writes_one_audit_row_and_one_event() {
    let harness = harness().await;
    let entry_id = signed_entry_id(sign_off(&harness, "Ann Fritz", "legal-1").await).await;
    assert_eq!(audit_rows(&harness.dsn, "pii_allowlist.sign_off").await, 1);
    assert_eq!(outbox_rows(&harness.dsn, "PiiAllowlistChanged").await, 1);

    revoke(&harness, entry_id).await;
    assert_eq!(audit_rows(&harness.dsn, "pii_allowlist.revoke").await, 1);
    assert_eq!(
        outbox_rows(&harness.dsn, "PiiAllowlistChanged").await,
        2,
        "both acts announce; a revocation a consumer never hears leaves a stale cache admitting \
         a name Legal has withdrawn"
    );
}

/// **The erasure act enqueues its `ActorErased`, and a refused erasure
/// enqueues none.**
///
/// The negative half is the one that matters: the event rides the act's
/// transaction, so an unknown-principal refusal that still announced an
/// erasure would tell every cache to drop a ref that was never retired.
#[tokio::test]
async fn an_erasure_announces_itself_and_a_refused_one_does_not() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;

    erase(app_for(&harness, TENANT), ALICE, "DSAR 1").await;
    assert_eq!(outbox_rows(&harness.dsn, "ActorErased").await, 1);

    let refused = erase(app_for(&harness, TENANT), "principal:nobody", "DSAR 2").await;
    assert_eq!(error_code(refused).await, "ERASURE_UNKNOWN_ACTOR");
    assert_eq!(
        outbox_rows(&harness.dsn, "ActorErased").await,
        1,
        "the refusal announced nothing"
    );
}

/// **The compliance export requires a justification, and the justification it
/// requires lands on the access's own audit row.**
///
/// P-D-133: the one surface that returns real identities is not served
/// unreasoned. Both halves, because a door that demanded the field and then
/// dropped it would pass a refusal-only probe.
#[tokio::test]
async fn the_export_requires_a_justification_and_records_it_on_the_access_row() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;

    let unreasoned = export_with(app_for(&harness, TENANT), ALICE, "").await;
    assert_eq!(unreasoned.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(
        audit_rows(&harness.dsn, "compliance.export").await,
        0,
        "a refused export is not an access and writes no access row"
    );

    let served = export_with(app_for(&harness, TENANT), ALICE, "DSAR ticket 4471").await;
    assert_eq!(served.status(), axum::http::StatusCode::OK);
    assert_eq!(audit_rows(&harness.dsn, "compliance.export").await, 1);
    assert_eq!(
        raw_string(
            &harness.dsn,
            "SELECT reason AS v FROM products_audit_log WHERE action = 'compliance.export'",
        )
        .await,
        "DSAR ticket 4471",
        "the reason column is where the justification lands; a door that demanded it and \
         dropped it would satisfy the refusal half alone"
    );
}

/// **Every access is audited individually, counted rather than sampled.**
#[tokio::test]
async fn three_exports_write_three_access_rows() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;
    for _ in 0..3 {
        let served = export(app_for(&harness, TENANT), ALICE).await;
        assert_eq!(served.status(), axum::http::StatusCode::OK);
    }
    assert_eq!(
        audit_rows(&harness.dsn, "compliance.export").await,
        3,
        "individually audited means one row per access, which only a count can assert"
    );
}

/// **The detector the doors run is this feature's own, over this tenant's
/// list — and the allow-list arm is reachable through the wire.**
///
/// The erasure reason is one of the enumerated free-text fields. An unlisted
/// person-shaped reason is refused `CONTENT_PII_BLOCKED`; signing that same
/// name onto the list makes the same reason pass. That second half is what
/// proves the door reads the tenant's list rather than a compiled-in one.
#[tokio::test]
async fn a_doors_free_text_is_judged_against_this_tenants_allow_list() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;

    let blocked = erase(app_for(&harness, TENANT), ALICE, "requested by Ann Fritz").await;
    assert_eq!(
        error_code(blocked).await,
        "CONTENT_PII_BLOCKED",
        "an unlisted person-shaped run is undecidable, and the hook fails closed on that"
    );

    sign_off(&harness, "Ann Fritz", "legal-1").await;
    let admitted = erase(app_for(&harness, TENANT), ALICE, "requested by Ann Fritz").await;
    assert_eq!(
        admitted.status(),
        axum::http::StatusCode::OK,
        "the allow-by-list arm, reached through the wire: the same text passes once the name \
         is on this tenant's list"
    );
}

/// **A block names the field and never the detected value.**
///
/// The `DoD`'s own clause, and the reason it gives: a refusal that echoed the
/// match would write the personal data into the refusal's own audit row,
/// which is a record erasure cannot reach. Asserted on the **audit row**, not
/// only on the response, because the row is the record that outlives the
/// request.
#[tokio::test]
async fn a_pii_refusal_names_the_field_and_its_audit_row_carries_no_detected_value() {
    let harness = harness().await;
    seed_principal(&harness, ALICE).await;

    let blocked = erase(
        app_for(&harness, TENANT),
        ALICE,
        "requested by Ann Fritz of Acme",
    )
    .await;
    let body = body_json(blocked).await;
    let rendered = body.to_string();
    assert!(
        rendered.contains("reason"),
        "the refusal names the field: {rendered}"
    );
    assert!(
        !rendered.contains("Ann Fritz"),
        "the refusal must not echo the detected value: {rendered}"
    );
    assert_eq!(
        raw_i64(
            &harness.dsn,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE reason LIKE '%Ann Fritz%'",
        )
        .await,
        0,
        "and no audit row carries it either - the row is the record erasure cannot rewrite"
    );
}

/// **Every door that runs the content-PII write block builds its detector
/// from this feature's own host, and no production path constructs the
/// permissive one.**
///
/// A source census rather than N behavioural probes, because the claim is
/// about the *set*: a door added next month with
/// `Arc::new(NoPiiPolicyDetector)` in it would pass every behavioural probe
/// written today, and it is exactly the shape this change had to repair —
/// the permissive host was constructed at **six** production sites, each its
/// own literal, so *"the registered detector"* named a phrase and not a
/// registry.
///
/// # The file list is DISCOVERED, and that is a correction
///
/// The first version of this census named five files. A seventh door then
/// landed in a **sixth** file — `approvals.rs`, the approval-rejection reason
/// — building the permissive host, and this test stayed green because the
/// file was not on its list. A census with a hard-coded population cannot see
/// the member that arrives outside it, which is the same defect one level up
/// from the one it exists to catch. So the population is now every crate
/// source that calls the hook.
///
/// Scoped to the production half of each file: the permissive host is a
/// legitimate **test** double and several suites drive it deliberately, so a
/// whole-file scan would forbid the thing it is right to keep.
#[test]
fn no_production_door_builds_the_permissive_pii_host() {
    let mut checked = 0_usize;
    for path in crate::lib_tests::crate_sources() {
        let name = path.display().to_string();
        if name.ends_with("_tests.rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("a readable crate source");
        let production = source.split("#[cfg(test)]").next().unwrap_or(&source);
        if !production.contains("content_pii_block(") {
            continue;
        }
        // The hook's own module declares it and is not a door.
        if name.ends_with("domain/taxonomy.rs") {
            continue;
        }
        checked += 1;
        assert!(
            !production.contains("Arc::new(NoPiiPolicyDetector)")
                && !production.contains("Arc::new(crate::domain::taxonomy::NoPiiPolicyDetector)"),
            "{name} runs the write block on the permissive host: `dod-pii-detector` obliges the \
             whole door set, and a door left on it admits every string while its neighbours \
             refuse"
        );
        assert!(
            production.contains("tenant_pii_detector"),
            "{name} runs the write block without building this feature's detector"
        );
    }
    assert!(
        checked >= 6,
        "the census found only {checked} doors running the hook; it discovers its own \
         population, so a number this low means the discovery broke rather than that the \
         doors went away"
    );
}

/// **The census can fail.** The perturbation the case above needs: the same
/// scan over a string that *does* carry the literal must trip, so a green
/// census is evidence rather than a scan that matches nothing.
#[test]
fn the_permissive_host_census_can_fail() {
    let poisoned = "fn door() { let d = Arc::new(NoPiiPolicyDetector); }\n#[cfg(test)]\nmod t {}";
    let production = poisoned.split("#[cfg(test)]").next().unwrap_or(poisoned);
    assert!(
        production.contains("Arc::new(NoPiiPolicyDetector)"),
        "the census's own predicate must see the construction it forbids"
    );
}

/// **Both allow-list doors submit their act to `05`'s live-op gate.**
///
/// Call sites, not a verdict: the registered host authorizes everything, so a
/// green verdict assertion would prove nothing about whether the ceremony was
/// asked. `02`'s `every_op_door_submits_its_envelope_to_the_gate` is the
/// shape, and the same reasoning — the day a store-backed host lands, a door
/// that never submitted keeps writing.
#[test]
fn both_allowlist_doors_submit_their_act_to_the_gate() {
    let source = include_str!("retention.rs");
    let production = source.split("#[cfg(test)]").next().unwrap_or(source);
    assert_eq!(
        production
            .matches("crate::api::rest::authorize_live_op(")
            .count(),
        2,
        "one call in each of the two mutating doors, and no more: the read door submits nothing \
         because it changes nothing. The definition line does not match this pattern - it reads \
         `(tenant_id: Uuid)` - so this counts call sites and not the function's own existence, \
         which is what the assertion is for"
    );
}

/// A tenant the harness's enforcer does **not** admit, for the denial half.
const OTHER_TENANT: Uuid = Uuid::from_u128(0x7e_99);

/// **Each of the three grants is spent by a door that exists, and a caller
/// without it is refused — with the refusal audited.**
///
/// §6's criterion for `dod-retention-authz`. Its `DoD`'s own body asserts the
/// four roster tests, which are the **declaration** half; this is the
/// spending half, and the `DoD` had only the first. The audited refusal is the
/// part a status-code assertion alone would miss: `PERMISSION_DENIED` is one
/// of P-D-21's three audit classes, and a door that answered 403 without
/// writing the row would satisfy every wire probe.
#[tokio::test]
async fn each_grant_is_spent_by_a_door_and_a_denial_is_audited() {
    let harness = harness().await;
    // The enforcer admits OTHER_TENANT; every call below authenticates as
    // TENANT, so each door's own `access_scope` refuses.
    let denied = |h: &TestHarness| app_for(h, OTHER_TENANT);

    let erasure = erase(denied(&harness), ALICE, "why").await;
    assert_eq!(erasure.status(), axum::http::StatusCode::FORBIDDEN);

    let export = export(denied(&harness), ALICE).await;
    assert_eq!(export.status(), axum::http::StatusCode::FORBIDDEN);

    let allowlist = sign_off_via(denied(&harness), "Ann Fritz", "legal-1").await;
    assert_eq!(allowlist.status(), axum::http::StatusCode::FORBIDDEN);

    assert_eq!(
        audit_rows(&harness.dsn, "").await,
        0,
        "the empty action matches nothing - the control for the count below"
    );
    assert_eq!(
        raw_i64(
            &harness.dsn,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE error_code = 'PERMISSION_DENIED'",
        )
        .await,
        3,
        "one audited refusal per grant: three doors, three rows"
    );
}

/// **The compliance surface spends its own grant, and neither of its two
/// doors is served under `audit × export`.**
///
/// §6's criterion. `design/10` §4 excludes the map from `audit × export`'s
/// output, and this is the one surface that returns real identities —
/// folding either door into the audit grant would hand every auditor the
/// identities the pseudonymisation scheme exists to withhold. A source census
/// because the claim is about which constant a route reaches for, and a wire
/// probe against a permissive enforcer cannot tell two grants apart.
#[test]
fn the_compliance_doors_spend_their_own_grant_and_never_the_audit_one() {
    let source = include_str!("retention.rs");
    let production = source.split("#[cfg(test)]").next().unwrap_or(source);
    let code: String = production
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !code.contains("resource_types::AUDIT"),
        "no door in this feature may reach for the audit resource"
    );
    assert_eq!(
        code.matches("Gate::Compliance").count(),
        2,
        "exactly two doors pass it - the identity export and the allow-list review. The enum's \
         own arms spell it `Self::Compliance` and are asserted separately below, so this counts \
         SPENDERS and not the mapping's existence"
    );
    assert!(
        code.contains("Self::Compliance => crate::authz::resource_types::COMPLIANCE")
            && code.contains("Self::Compliance => crate::authz::actions::EXPORT"),
        "and the pair it maps to is `compliance x export`"
    );
}

/// **Neither retention payload has a field an identity could reach.**
///
/// §6's criterion for `dod-retention-events`. `ActorErased` is a *defensive
/// cache-buster* whose whole point is that it carries none, and a field added
/// later is exactly how it would stop being one — so the assertion is over
/// the payload's **shape**, not over one serialized instance.
///
/// The roster is every field name the struct declares; each is a pseudonym, a
/// tenant, an act token or a rendered aggregate. `identity_payload` — the one
/// column in the gear that may hold a real identity — must not appear.
#[test]
fn neither_retention_payload_has_a_field_an_identity_could_reach() {
    let source = include_str!("../../infra/broker.rs");
    let body = source
        .split("pub(crate) struct RetentionEventPayload {")
        .nth(1)
        .expect("the payload is declared")
        .split("\n}")
        .next()
        .expect("its body ends");
    let fields: Vec<&str> = body
        .lines()
        .filter_map(|line| line.trim().strip_prefix("pub "))
        .filter_map(|line| line.split(':').next())
        .collect();
    assert_eq!(
        fields,
        vec![
            "tenant_id",
            "subject_ref",
            "act",
            "erased_actor_ref",
            "actor_ref"
        ],
        "the payload's whole field set, transcribed: a pseudonym, a tenant, an act token and a \
         rendered aggregate. A sixth field is the review this assertion exists to force"
    );
    assert!(
        !body.contains("identity_payload") && !body.contains("principal_identity"),
        "the one column that may hold a real identity has no route into an event"
    );
}
