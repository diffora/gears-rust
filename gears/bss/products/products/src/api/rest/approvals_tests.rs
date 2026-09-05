//! The ceremony's three doors, probed on the clauses whose absence would ship
//! the defect each rule names (**P-D-119**, **P-D-120**, **P-D-133**).
//!
//! # What the role harness has to do that no other door's does
//!
//! `authed_ctx` builds a context whose `token_scopes` is `["*"]` — a
//! **permission** wildcard, not a role. The decide door reads C1's base role
//! set from that same claim (**P-D-134** row 25), so the ordinary harness
//! context holds no role and is refused. That is not a harness defect: it is
//! the production posture, and [`ctx_with_roles`] exists so the positive
//! cases can state which role they are asserting under rather than inheriting
//! one silently.

#![allow(clippy::expect_used)]

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::Request;
use serde_json::{Value as JsonValue, json};
use toolkit::api::OpenApiRegistryImpl;
use toolkit_db::outbox::{Outbox, OutboxHandle, Partitions, outbox_migrations_with_prefix};
use toolkit_db::secure::AccessScope;
use toolkit_db::{ConnectOpts, DBProvider, DbError, connect_db};
use toolkit_gts::gts_id;
use toolkit_security::SecurityContext;
use tower::ServiceExt;
use uuid::Uuid;

use sea_orm_migration::MigratorTrait;

use super::router;
use crate::api::rest::ApiState;
use crate::config::ProductsConfig;
use crate::domain::approval::ApproverRole;
use crate::infra::events;
use crate::infra::storage::migrations::Migrator;
use crate::infra::storage::repo;
use crate::test_support::flat_in_enforcer;

const TENANT: Uuid = Uuid::from_u128(0x5a_11);
const BRAND: Uuid = Uuid::from_u128(0x5a_b1);
const PRODUCT: Uuid = Uuid::from_u128(0x5a_f0);
const TARGET_TENANT: Uuid = Uuid::from_u128(0x5a_77);
const PLATFORM_A: Uuid = Uuid::from_u128(0x5a_c1);
const PLATFORM_B: Uuid = Uuid::from_u128(0x5a_c2);

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
        "bss-products-approvals-tests-{}.sqlite3",
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
        reference: crate::api::rest::ReferenceKnobs::from(&ProductsConfig::default()),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        eol_enabled: false,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
    });
    let openapi = OpenApiRegistryImpl::new();
    router(state, &openapi).layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// An authenticated context carrying the permission wildcard **and** the
/// named roles, for one identified subject.
///
/// The subject id is an argument because C1's other half — *"each distinct
/// from the author"* — is by **principal**, so a case that needs two
/// approvers needs two subjects, and `authed_ctx`'s fresh-`Uuid` shape cannot
/// express a caller acting twice.
fn ctx_with_roles(subject: Uuid, roles: &[ApproverRole]) -> SecurityContext {
    ctx_with_claims(subject, roles, &[])
}

/// A context carrying the wildcard, the named roles **and** scope claims
/// (`region:<code>` / `brand:<code>`, P-D-155) — no claim on a dimension is
/// the unrestricted claim set.
fn ctx_with_claims(subject: Uuid, roles: &[ApproverRole], claims: &[&str]) -> SecurityContext {
    let mut scopes = vec!["*".to_owned()];
    scopes.extend(roles.iter().map(|r| (*r).as_str().to_owned()));
    scopes.extend(claims.iter().map(|c| (*c).to_owned()));
    SecurityContext::builder()
        .subject_id(subject)
        .subject_tenant_id(TENANT)
        .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
        .token_scopes(scopes)
        .build()
        .expect("authed SecurityContext must build")
}

/// A context with the wildcard and **no** role claim — the production posture
/// until the platform's PDP encodes one.
fn ctx_without_roles(subject: Uuid) -> SecurityContext {
    ctx_with_roles(subject, &[])
}

async fn post(
    app: Router,
    uri: &str,
    ctx: SecurityContext,
    body: JsonValue,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .extension(ctx)
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn body_of(response: axum::http::Response<Body>) -> JsonValue {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("read the body");
    serde_json::from_slice(&bytes).expect("the body is JSON")
}

/// Seed one `draft` Product head, so an `entity_publish` submission has a
/// head to read its snapshot from.
async fn seed_head(harness: &TestHarness) {
    seed_scoped_head(harness, PRODUCT, "Fibre 500", "eu", "").await;
}

/// Seed a `draft` Product head with the two scope columns a case names.
async fn seed_scoped_head(
    harness: &TestHarness,
    product_id: Uuid,
    name: &str,
    region_scope: &str,
    brand_scope: &str,
) {
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    repo::insert_product(
        &conn,
        &scope,
        repo::NewProduct {
            product_id,
            tenant_id: TENANT,
            brand_id: BRAND,
            name: name.to_owned(),
            name_normalized: name.to_lowercase(),
            product_code: Some(name.to_uppercase().replace(' ', "-")),
            region_scope: region_scope.to_owned(),
            brand_scope: brand_scope.to_owned(),
            created_by: "principal:author-1".to_owned(),
            created_at: crate::test_support::at(9),
            cloned_from: None,
            cloned_from_version: None,
        },
    )
    .await
    .expect("seed the head");
}

/// Set the tenant's `N`, so a case can name the quorum it is asserting under.
async fn set_quorum(harness: &TestHarness, n: u32) {
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    repo::write_materiality_policy(
        &conn,
        &scope,
        TENANT,
        &crate::domain::materiality::MaterialityPolicy::new(Vec::new(), 10, n),
        Uuid::from_u128(0x5a_ad),
        crate::test_support::at(9),
    )
    .await
    .expect("write the policy");
}

fn submission_body() -> JsonValue {
    submission_body_for(PRODUCT)
}

fn submission_body_for(product_id: Uuid) -> JsonValue {
    json!({
        "subject_kind": "entity_publish",
        "subject_ref": format!("product/{product_id}"),
        "finance_material": false,
    })
}

// ---------------------------------------------------------------------------
// Submit
// ---------------------------------------------------------------------------

/// **A submission stores the head's own content as the snapshot** — read from
/// the head, never taken from the request.
///
/// The refutation is the third assertion: the body a caller *could* have sent
/// is explicitly refused, so a door that accepted one and stored it would
/// fail here rather than silently letting an approver sign bytes the publish
/// will not write (`dod-stored-snapshot`).
#[tokio::test]
async fn a_submission_reads_its_snapshot_from_the_head_and_refuses_a_supplied_one() {
    let harness = harness().await;
    seed_head(&harness).await;

    let response = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/approvals",
        ctx_without_roles(Uuid::from_u128(0x5a_a0)),
        submission_body(),
    )
    .await;
    assert_eq!(response.status(), 201, "the submission is admitted");
    let body = body_of(response).await;
    let approval_id: Uuid = body["approval_id"]
        .as_str()
        .expect("the receipt names the record")
        .parse()
        .expect("a uuid");

    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let stored = repo::read_approval(
        &conn,
        &scope,
        TENANT,
        crate::domain::governance::ApprovalId::new(approval_id),
    )
    .await
    .expect("the read runs")
    .expect("the record exists");
    assert!(
        stored.content_snapshot.contains("Fibre 500"),
        "the snapshot carries the head's own name, not a caller's string: {}",
        stored.content_snapshot
    );

    let mut supplied = submission_body();
    supplied["content_snapshot"] = json!(r#"{"name":"not what the head says"}"#);
    let refused = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/approvals",
        ctx_without_roles(Uuid::from_u128(0x5a_a0)),
        supplied,
    )
    .await;
    assert_eq!(
        refused.status(),
        400,
        "an entity submission may not carry its own snapshot"
    );
}

/// **At `N = 0` the record is born `satisfied` through the door**
/// (**P-D-119** row 31), and at the default it is born `pending`.
///
/// The pair is one case because the negative control is the whole assertion:
/// a door that answered `satisfied` unconditionally would authorize every
/// governed act in the gear on submission alone.
#[tokio::test]
async fn the_born_state_follows_the_tenants_configured_quorum() {
    let harness = harness().await;
    seed_head(&harness).await;

    let pending = body_of(
        post(
            app_for(&harness, TENANT),
            "/bss-products/v1/approvals",
            ctx_without_roles(Uuid::from_u128(0x5a_a0)),
            submission_body(),
        )
        .await,
    )
    .await;
    assert_eq!(pending["state"], "pending", "the default N is 2");
    assert_eq!(pending["required"], 2);
    assert_eq!(pending["configured_quorum"], 2);

    set_quorum(&harness, 0).await;
    let satisfied = body_of(
        post(
            app_for(&harness, TENANT),
            "/bss-products/v1/approvals",
            ctx_without_roles(Uuid::from_u128(0x5a_a0)),
            submission_body(),
        )
        .await,
    )
    .await;
    assert_eq!(
        satisfied["state"], "satisfied",
        "a tenant at N = 0 publishes approver-less by policy, and the record says so"
    );
    assert_eq!(satisfied["required"], 0);
    assert_eq!(
        satisfied["quorum_reduced"], true,
        "an effective count below the retained default of two is a reduced ceremony"
    );
}

/// **A subject kind outside the `CHECK`'s roster is refused at the door**,
/// not at the constraint.
#[tokio::test]
async fn an_unknown_subject_kind_is_refused_before_the_write() {
    let harness = harness().await;
    let mut body = submission_body();
    body["subject_kind"] = json!("not_a_kind");
    let response = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/approvals",
        ctx_without_roles(Uuid::from_u128(0x5a_a0)),
        body,
    )
    .await;
    assert_eq!(response.status(), 400);
}

/// **`materiality_policy` is a subject kind the store accepts** (**P-D-120**
/// row 38), end to end: through the door, past the `CHECK`, and back out of
/// `subject_kind_from_stored`.
///
/// The last clause is the one that matters. The reader was a hand-written
/// array of five, so a record could be written with the sixth kind and be
/// invisible to `gate_candidates` — green schema, green door, silent gate.
#[tokio::test]
async fn a_materiality_policy_subject_round_trips_through_the_store() {
    let harness = harness().await;
    let response = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/approvals",
        ctx_without_roles(Uuid::from_u128(0x5a_a0)),
        json!({
            "subject_kind": "materiality_policy",
            "subject_ref": "materiality_policy",
            "finance_material": false,
            "content_snapshot": r#"{"approverCount":3}"#,
        }),
    )
    .await;
    assert_eq!(response.status(), 201, "the CHECK admits the sixth kind");

    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TENANT);
    let candidates = repo::gate_candidates(
        &conn,
        &scope,
        &crate::domain::governance::GateSubject::materiality_policy(TENANT, 0),
    )
    .await
    .expect("the gate read runs");
    assert_eq!(
        candidates.len(),
        1,
        "the stored-kind reader parses the sixth kind: a record the gate cannot see is a record \
         that authorizes nothing"
    );
}

// ---------------------------------------------------------------------------
// Decide
// ---------------------------------------------------------------------------

/// Submit and return the record's id.
async fn get(app: Router, uri: &str, ctx: SecurityContext) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method("GET")
            .uri(uri)
            .extension(ctx)
            .body(Body::empty())
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

/// **`dod-inbox-envelope`**: the inbox lists the pending records with the
/// record's **effective** count as `required`, the raw `N` beside it, and the
/// distinct approving principals as `satisfied`; any other `state` is refused.
#[tokio::test]
async fn the_inbox_lists_pending_records_with_effective_counts_and_progress() {
    let harness = harness().await;
    seed_head(&harness).await;
    set_quorum(&harness, 2).await;
    let approval = submit(&harness, Uuid::from_u128(0x5a_a0)).await;

    let response = get(
        app_for(&harness, TENANT),
        "/bss-products/v1/approvals?state=pending",
        ctx_without_roles(Uuid::from_u128(0x5a_b0)),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_of(response).await;
    let items = body["items"].as_array().expect("an items array");
    assert_eq!(items.len(), 1, "one pending record");
    let card = &items[0];
    assert_eq!(card["approval_id"], json!(approval.to_string()));
    assert_eq!(card["subject_kind"], "entity_publish");
    assert_eq!(card["state"], "pending");
    assert_eq!(card["quorum"]["required"], 2, "the effective count");
    assert_eq!(
        card["quorum"]["configured_quorum"], 2,
        "the raw N beside it"
    );
    assert_eq!(card["quorum"]["satisfied"], 0);
    assert_eq!(card["quorum"]["quorum_reduced"], false);
    assert!(
        card["quorum"]["predicate_unsatisfiable"].is_null(),
        "no finance lens was demanded of this change"
    );

    let decided = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_with_roles(Uuid::from_u128(0x5a_a1), &[ApproverRole::CatalogAdmin]),
        json!({ "verdict": "approved" }),
    )
    .await;
    assert_eq!(decided.status(), 200);
    let body = body_of(
        get(
            app_for(&harness, TENANT),
            "/bss-products/v1/approvals?state=pending",
            ctx_without_roles(Uuid::from_u128(0x5a_b0)),
        )
        .await,
    )
    .await;
    assert_eq!(
        body["items"][0]["quorum"]["satisfied"], 1,
        "one distinct approving principal so far; the record is still pending at N = 2"
    );

    let refused = get(
        app_for(&harness, TENANT),
        "/bss-products/v1/approvals?state=satisfied",
        ctx_without_roles(Uuid::from_u128(0x5a_b0)),
    )
    .await;
    assert_eq!(refused.status(), 400, "the inbox is the open queue only");
}

/// **A `system_signal` cannot be submitted over REST** (`dod-system-signal`):
/// the record is the signal consumer's to write, born satisfied with the
/// signal as its principal; a caller minting one would be minting a directly
/// consumable record with no human behind it.
#[tokio::test]
async fn a_system_signal_cannot_be_submitted_over_rest() {
    let harness = harness().await;
    let mut body = submission_body();
    body["subject_kind"] = json!("system_signal");
    body["subject_ref"] = json!(Uuid::from_u128(0x5a_51).to_string());
    body["content_snapshot"] = json!("{}");
    let response = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/approvals",
        ctx_without_roles(Uuid::from_u128(0x5a_a0)),
        body,
    )
    .await;
    assert_eq!(response.status(), 400);
}

async fn submit(harness: &TestHarness, author: Uuid) -> Uuid {
    submit_for(harness, author, PRODUCT).await
}

async fn submit_for(harness: &TestHarness, author: Uuid, product_id: Uuid) -> Uuid {
    let body = body_of(
        post(
            app_for(harness, TENANT),
            "/bss-products/v1/approvals",
            ctx_without_roles(author),
            submission_body_for(product_id),
        )
        .await,
    )
    .await;
    body["approval_id"]
        .as_str()
        .expect("the receipt names the record")
        .parse()
        .expect("a uuid")
}

/// **A principal with no role claim is refused `APPROVER_ROLE_REQUIRED`
/// (403)** — never told the role was held (**P-D-119** rows 13 and 30,
/// **P-D-134** row 25).
#[tokio::test]
async fn a_decider_with_no_role_claim_is_refused_403() {
    let harness = harness().await;
    seed_head(&harness).await;
    let approval = submit(&harness, Uuid::from_u128(0x5a_a0)).await;

    let response = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_without_roles(Uuid::from_u128(0x5a_a1)),
        json!({ "verdict": "approved" }),
    )
    .await;
    assert_eq!(response.status(), 403);
    let body = body_of(response).await;
    assert_eq!(
        body["context"]["reason"], "APPROVER_ROLE_REQUIRED",
        "the code is the ceremony's own, not the platform's permission denial"
    );
}

/// **A `CatalogAdmin` decides, and the record moves.** The positive control
/// the refusal above needs: without it that probe would pass against a door
/// that refused every decision.
#[tokio::test]
async fn a_catalog_admin_decides_and_the_count_moves() {
    let harness = harness().await;
    seed_head(&harness).await;
    set_quorum(&harness, 1).await;
    let approval = submit(&harness, Uuid::from_u128(0x5a_a0)).await;

    let response = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_with_roles(Uuid::from_u128(0x5a_a1), &[ApproverRole::CatalogAdmin]),
        json!({ "verdict": "approved" }),
    )
    .await;
    assert_eq!(response.status(), 200);
    let body = body_of(response).await;
    assert_eq!(
        body["state"], "satisfied",
        "at required = 1 one eligible approver meets the descriptor, and the decide \
         transaction is what writes the flip (P-D-120 row 11)"
    );
    assert_eq!(body["counted"], 1);
    assert_eq!(body["required"], 1);
}

async fn decide_as(
    harness: &TestHarness,
    approval: Uuid,
    ctx: SecurityContext,
) -> axum::http::Response<Body> {
    post(
        app_for(harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx,
        json!({ "verdict": "approved" }),
    )
    .await
}

async fn count(harness: &TestHarness, sql: &str) -> i64 {
    crate::test_support::raw_i64(&harness.dsn, sql).await
}

/// **An out-of-scope approver is refused `APPROVER_SCOPE_EXCEEDED` and
/// audited; an in-scope one decides** (`dod-approver-scope`; P-D-155). The
/// head is `eu`-scoped: a `region:us` approver meets 403, one audit row and
/// no decision row; a `region:eu` approver is counted. The 403 carries the
/// code and no detail — the ladder's denial shape — so the audit row and the
/// absent decision row are what prove the refusal was the scope rule's.
#[tokio::test]
async fn an_out_of_scope_approver_is_refused_and_audited_and_an_in_scope_one_decides() {
    let harness = harness().await;
    seed_head(&harness).await;
    set_quorum(&harness, 1).await;
    let approval = submit(&harness, Uuid::from_u128(0x5a_a0)).await;
    let refused = decide_as(
        &harness,
        approval,
        ctx_with_claims(
            Uuid::from_u128(0x5a_b1),
            &[ApproverRole::CatalogAdmin],
            &["region:us"],
        ),
    )
    .await;
    assert_eq!(refused.status(), 403);
    let body = body_of(refused).await;
    assert_eq!(
        body["context"]["reason"], "APPROVER_SCOPE_EXCEEDED",
        "the code rides the denial; the ladder's 403 carries no detail by design, and the \
         dimension is the domain verdict's to name (`both_dimensions_failing_reports_the_region`)"
    );
    assert_eq!(
        count(
            &harness,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE error_code = 'APPROVER_SCOPE_EXCEEDED'"
        )
        .await,
        1,
        "audited like any scope violation"
    );
    assert_eq!(
        count(
            &harness,
            "SELECT COUNT(*) AS v FROM products_approval_decision"
        )
        .await,
        0,
        "no verdict was recorded"
    );

    let admitted = decide_as(
        &harness,
        approval,
        ctx_with_claims(
            Uuid::from_u128(0x5a_b2),
            &[ApproverRole::CatalogAdmin],
            &["region:eu"],
        ),
    )
    .await;
    assert_eq!(admitted.status(), 200, "the in-scope control");
    assert_eq!(body_of(admitted).await["counted"], 1);
}

/// **The brand dimension is judged on its own at the door**: the head is
/// `eu` / `acme,globex`; a `region:eu` approver claiming `brand:acme` alone
/// is refused naming `brand_scope`, and one claiming both brands is counted.
/// Without this case a check that read region twice passes every other.
#[tokio::test]
async fn the_brand_dimension_is_judged_on_its_own_at_the_door() {
    let harness = harness().await;
    let product = Uuid::from_u128(0x5a_c0);
    seed_scoped_head(&harness, product, "Fibre Duo", "eu", "acme,globex").await;
    set_quorum(&harness, 1).await;
    let approval = submit_for(&harness, Uuid::from_u128(0x5a_a0), product).await;
    let refused = decide_as(
        &harness,
        approval,
        ctx_with_claims(
            Uuid::from_u128(0x5a_b3),
            &[ApproverRole::CatalogAdmin],
            &["region:eu", "brand:acme"],
        ),
    )
    .await;
    assert_eq!(refused.status(), 403);
    assert_eq!(
        body_of(refused).await["context"]["reason"],
        "APPROVER_SCOPE_EXCEEDED",
        "region covers, brand does not: the pair with the control below isolates the dimension"
    );
    let admitted = decide_as(
        &harness,
        approval,
        ctx_with_claims(
            Uuid::from_u128(0x5a_b4),
            &[ApproverRole::CatalogAdmin],
            &["region:eu", "brand:acme", "brand:globex"],
        ),
    )
    .await;
    assert_eq!(
        admitted.status(),
        200,
        "subset on both dimensions is covered"
    );
}

/// **Clause 2 at the door**: a tenant-wide subject (both scope columns empty)
/// is covered only by an unrestricted claim set — a `region:eu` approver is
/// refused, an approver with no scope claim at all is counted. The
/// transposed mapping would admit the first and is the scope rule deleted.
#[tokio::test]
async fn a_restricted_approver_does_not_cover_a_tenant_wide_subject_and_an_unrestricted_one_does() {
    let harness = harness().await;
    let product = Uuid::from_u128(0x5a_c1);
    seed_scoped_head(&harness, product, "Fibre Wide", "", "").await;
    set_quorum(&harness, 1).await;
    let approval = submit_for(&harness, Uuid::from_u128(0x5a_a0), product).await;
    let refused = decide_as(
        &harness,
        approval,
        ctx_with_claims(
            Uuid::from_u128(0x5a_b5),
            &[ApproverRole::CatalogAdmin],
            &["region:eu"],
        ),
    )
    .await;
    assert_eq!(refused.status(), 403);
    assert_eq!(
        body_of(refused).await["context"]["reason"],
        "APPROVER_SCOPE_EXCEEDED"
    );
    let admitted = decide_as(
        &harness,
        approval,
        ctx_with_roles(Uuid::from_u128(0x5a_b6), &[ApproverRole::CatalogAdmin]),
    )
    .await;
    assert_eq!(
        admitted.status(),
        200,
        "the unrestricted claim set covers every subject"
    );
}

/// **Materiality and `N` are read once** (§6): a record submitted at `N = 2`
/// keeps `required = 2`, `configuredQuorum = 2` and its `pending` state after
/// the tenant's `N` moves to 0 — the change neither re-judges nor voids it.
/// **A second submission on the same subject supersedes rather than doubling**
/// — the first record reads `superseded`, one record is open, and the new one
/// is born under the new `N`. **And none of it emits a broker event**: the
/// outbox is empty after both submissions, asserted as the whole set.
#[tokio::test]
async fn a_pending_record_keeps_its_descriptor_and_a_resubmission_supersedes_it_silently() {
    let harness = harness().await;
    seed_head(&harness).await;
    set_quorum(&harness, 2).await;
    let first = submit(&harness, Uuid::from_u128(0x5a_a0)).await;
    set_quorum(&harness, 0).await;

    let scope = AccessScope::for_tenant(TENANT);
    {
        // Handed back before the next door call: the harness pins one connection.
        let conn = harness.db.conn().expect("scoped connection");
        let stored = repo::read_approval(
            &conn,
            &scope,
            TENANT,
            crate::domain::governance::ApprovalId::new(first),
        )
        .await
        .expect("read")
        .expect("the record");
        let descriptor = crate::domain::approval::descriptor_from_stored(&stored.quorum_descriptor)
            .expect("a stored descriptor decodes");
        assert_eq!(stored.state, "pending", "the policy change voids nothing");
        assert_eq!(descriptor.required(), 2, "not re-judged under the new N");
        assert_eq!(
            descriptor.configured_quorum(),
            2,
            "the raw N is the one read at submission"
        );
    }

    let second = submit(&harness, Uuid::from_u128(0x5a_a0)).await;
    assert_ne!(second, first);
    {
        let conn = harness.db.conn().expect("scoped connection");
        let first_now = repo::read_approval(
            &conn,
            &scope,
            TENANT,
            crate::domain::governance::ApprovalId::new(first),
        )
        .await
        .expect("read")
        .expect("the record");
        assert_eq!(first_now.state, "superseded", "the resubmission supersedes");
        let second_row = repo::read_approval(
            &conn,
            &scope,
            TENANT,
            crate::domain::governance::ApprovalId::new(second),
        )
        .await
        .expect("read")
        .expect("the record");
        let second_descriptor =
            crate::domain::approval::descriptor_from_stored(&second_row.quorum_descriptor)
                .expect("decodes");
        assert_eq!(
            second_descriptor.configured_quorum(),
            0,
            "born under the new N"
        );
        assert_eq!(second_row.state, "satisfied", "N = 0 is born satisfied");
    }
    assert_eq!(
        count(
            &harness,
            &format!(
                "SELECT COUNT(*) AS v FROM products_approval WHERE subject_ref = 'product/{PRODUCT}' \
                 AND state IN ('pending', 'satisfied')"
            )
        )
        .await,
        1,
        "one open record per subject - `uq_products_approval_open`'s shape"
    );
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    assert_eq!(
        count(&harness, &format!("SELECT COUNT(*) AS v FROM {body_table}")).await,
        0,
        "submissions and supersessions emit nothing: the emitted set is empty"
    );
}

/// **The record's own author cannot decide it** — by principal, never by
/// role (`design/05` C2).
#[tokio::test]
async fn the_author_cannot_decide_their_own_submission() {
    let harness = harness().await;
    seed_head(&harness).await;
    set_quorum(&harness, 1).await;
    let author = Uuid::from_u128(0x5a_a0);
    let approval = submit(&harness, author).await;

    let response = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_with_roles(author, &[ApproverRole::CatalogAdmin]),
        json!({ "verdict": "approved" }),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        body_of(response).await["context"]["reason"],
        "SELF_APPROVAL_FORBIDDEN"
    );
}

/// **A second verdict from one principal is `DECISION_ALREADY_RECORDED`
/// (409), not a 500** (**P-D-119** row 37).
///
/// This is the defect the code was minted for: C2's `UNIQUE` reached the wire
/// through `RepoError::Db`, so a double-clicked approve told the operator the
/// database had broken.
#[tokio::test]
async fn a_second_verdict_from_one_principal_is_409_not_500() {
    let harness = harness().await;
    seed_head(&harness).await;
    set_quorum(&harness, 3).await;
    let approval = submit(&harness, Uuid::from_u128(0x5a_a0)).await;
    let approver = Uuid::from_u128(0x5a_a1);

    let first = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_with_roles(approver, &[ApproverRole::CatalogAdmin]),
        json!({ "verdict": "approved" }),
    )
    .await;
    assert_eq!(first.status(), 200, "the first verdict lands");

    let second = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_with_roles(approver, &[ApproverRole::CatalogAdmin]),
        json!({ "verdict": "approved" }),
    )
    .await;
    assert_eq!(
        second.status(),
        409,
        "one principal, one decision: the record's state refuses the act"
    );
    assert_eq!(
        body_of(second).await["context"]["reason"],
        "DECISION_ALREADY_RECORDED",
        "the seventh code, and the reason it was minted"
    );
}

/// **A rejection finalizes the record and carries a mandatory reason**, and
/// a rejection without one is refused before the append-only row lands.
#[tokio::test]
async fn a_rejection_needs_a_reason_and_finalizes_the_record() {
    let harness = harness().await;
    seed_head(&harness).await;
    set_quorum(&harness, 1).await;
    let approval = submit(&harness, Uuid::from_u128(0x5a_a0)).await;
    let approver = Uuid::from_u128(0x5a_a1);

    let unreasoned = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_with_roles(approver, &[ApproverRole::CatalogAdmin]),
        json!({ "verdict": "rejected" }),
    )
    .await;
    assert_eq!(
        unreasoned.status(),
        500,
        "an unreasoned rejection is refused; design/05 section 3.3 declares no code for it, so it \
         travels on the codeless channel rather than borrowing one"
    );

    let reasoned = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_with_roles(approver, &[ApproverRole::CatalogAdmin]),
        json!({ "verdict": "rejected", "reason": "the scope is wrong" }),
    )
    .await;
    assert_eq!(reasoned.status(), 200);
    assert_eq!(body_of(reasoned).await["state"], "rejected");
}

/// **A clean reason passes the PII block** — `dod-pii-on-reasons`' positive
/// control, without which a stub refusing every string would satisfy the
/// obligation.
#[tokio::test]
async fn a_clean_reason_is_admitted_by_the_write_block() {
    let harness = harness().await;
    seed_head(&harness).await;
    set_quorum(&harness, 1).await;
    let approval = submit(&harness, Uuid::from_u128(0x5a_a0)).await;

    let response = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_with_roles(Uuid::from_u128(0x5a_a1), &[ApproverRole::CatalogAdmin]),
        json!({ "verdict": "approved", "reason": "reviewed against the launch checklist" }),
    )
    .await;
    assert_eq!(
        response.status(),
        200,
        "the registered detector admits everything and says so; a detector that refused every \
         string would make the refusal probe pass and this one fail"
    );
}

/// The code a `CONTENT_PII_BLOCKED` refusal carries, on either channel the
/// ladder renders it through.
async fn pii_code_of(response: axum::http::Response<Body>) -> String {
    let body = body_of(response).await;
    body["context"]["reason"]
        .as_str()
        .or_else(|| body["context"]["violations"][0]["type"].as_str())
        .unwrap_or_else(|| panic!("a coded refusal: {body}"))
        .to_owned()
}

/// **The approval-rejection reason runs `10`'s hook at the door** (`10` §6's
/// every-door criterion; P-D-158): a person-shaped reason is refused
/// `CONTENT_PII_BLOCKED` before any decision row is written, and the clean
/// reason above is the control.
#[tokio::test]
async fn a_rejection_with_a_person_shaped_reason_is_refused_content_pii_blocked() {
    let harness = harness().await;
    seed_head(&harness).await;
    set_quorum(&harness, 1).await;
    let approval = submit(&harness, Uuid::from_u128(0x5a_a0)).await;
    let refused = post(
        app_for(&harness, TENANT),
        &format!("/bss-products/v1/approvals/{approval}/decisions"),
        ctx_with_roles(Uuid::from_u128(0x5a_a1), &[ApproverRole::CatalogAdmin]),
        json!({ "verdict": "rejected", "reason": "requested by Ann Fritz" }),
    )
    .await;
    assert_eq!(refused.status(), 400);
    assert_eq!(pii_code_of(refused).await, "CONTENT_PII_BLOCKED");
    assert_eq!(
        crate::test_support::raw_i64(
            &harness.dsn,
            "SELECT COUNT(*) AS v FROM products_approval_decision"
        )
        .await,
        0,
        "refused before the row is written"
    );
}

/// **The break-glass session reason runs the same hook** (P-D-158): a
/// person-shaped reason on the elevation door is `CONTENT_PII_BLOCKED`, and
/// no session row is written.
#[tokio::test]
async fn an_elevation_with_a_person_shaped_reason_is_refused_content_pii_blocked() {
    let harness = harness().await;
    let refused = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/breakglass-sessions",
        ctx_without_roles(Uuid::from_u128(0x5a_b0)),
        json!({ "target_tenant_id": TARGET_TENANT, "reason": "requested by Ann Fritz" }),
    )
    .await;
    assert_eq!(refused.status(), 400);
    assert_eq!(pii_code_of(refused).await, "CONTENT_PII_BLOCKED");
    assert_eq!(
        crate::test_support::raw_i64(
            &harness.dsn,
            "SELECT COUNT(*) AS v FROM products_breakglass_session"
        )
        .await,
        0,
        "no session opens on a refused reason"
    );
}

// ---------------------------------------------------------------------------
// Break-glass
// ---------------------------------------------------------------------------

/// **An elevation opens with two distinct platform approvers on the session
/// row** (**P-D-133** row 9), and the window comes from configuration.
#[tokio::test]
async fn an_elevation_stores_its_two_platform_approvers_and_a_configured_window() {
    let harness = harness().await;
    let response = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/breakglass-sessions",
        ctx_without_roles(Uuid::from_u128(0x5a_b0)),
        json!({
            "target_tenant_id": TARGET_TENANT,
            "reason": "incident 4471: the tenant cannot read its own catalog",
            "two_person_approval_ref": Uuid::from_u128(0x5a_c0),
            "approver_a": PLATFORM_A,
            "approver_b": PLATFORM_B,
        }),
    )
    .await;
    let status = response.status();
    let body = body_of(response).await;
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["path"], "two_person");

    let from: chrono::DateTime<chrono::Utc> = body["valid_from"]
        .as_str()
        .expect("the receipt carries the window")
        .parse()
        .expect("an instant");
    let until: chrono::DateTime<chrono::Utc> = body["valid_until"]
        .as_str()
        .expect("the receipt carries the window")
        .parse()
        .expect("an instant");
    assert_eq!(
        (until - from).num_hours(),
        i64::from(crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT),
        "the window is `breakglass_window_hours`, read and never inlined"
    );
}

/// **One human named twice is refused**, before the `CHECK` sees it: the
/// two-person floor is two distinct principals.
#[tokio::test]
async fn one_principal_named_as_both_approvers_is_refused() {
    let harness = harness().await;
    let response = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/breakglass-sessions",
        ctx_without_roles(Uuid::from_u128(0x5a_b0)),
        json!({
            "target_tenant_id": TARGET_TENANT,
            "reason": "incident 4471",
            "two_person_approval_ref": Uuid::from_u128(0x5a_c0),
            "approver_a": PLATFORM_A,
            "approver_b": PLATFORM_A,
        }),
    )
    .await;
    assert_eq!(response.status(), 500, "the rule refuses before the CHECK");
}

/// **A half-named two-person path is refused**: the exclusivity
/// `chk_products_breakglass_path` enforces is stated at the door, so a caller
/// gets a rule rather than a constraint name.
#[tokio::test]
async fn a_partial_two_person_path_is_refused() {
    let harness = harness().await;
    let response = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/breakglass-sessions",
        ctx_without_roles(Uuid::from_u128(0x5a_b0)),
        json!({
            "target_tenant_id": TARGET_TENANT,
            "reason": "incident 4471",
            "approver_a": PLATFORM_A,
        }),
    )
    .await;
    assert_eq!(response.status(), 400);
}

/// **The post-hoc path opens with no approvers and records its obligation.**
#[tokio::test]
async fn the_post_hoc_path_opens_with_a_pending_review_obligation() {
    let harness = harness().await;
    let response = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/breakglass-sessions",
        ctx_without_roles(Uuid::from_u128(0x5a_b0)),
        json!({
            "target_tenant_id": TARGET_TENANT,
            "reason": "incident 4471, out of hours",
        }),
    )
    .await;
    assert_eq!(response.status(), 201);
    assert_eq!(body_of(response).await["path"], "post_hoc");
}

/// **An elevation with a blank reason is refused**: the reason is mandatory
/// and it is the only record of why the tenant's data was reached.
#[tokio::test]
async fn an_elevation_without_a_reason_is_refused() {
    let harness = harness().await;
    let response = post(
        app_for(&harness, TENANT),
        "/bss-products/v1/breakglass-sessions",
        ctx_without_roles(Uuid::from_u128(0x5a_b0)),
        json!({ "target_tenant_id": TARGET_TENANT, "reason": "   " }),
    )
    .await;
    assert_eq!(response.status(), 400);
}

// ---------------------------------------------------------------------------
// The pre-pipeline elevation gate (`api::rest::elevation_gate`)
// ---------------------------------------------------------------------------

/// The three doors **plus the elevation gate and a read probe**.
///
/// The gate is a layer in production (`gear.rs`'s one call site), so a
/// harness that merged the routes without it would test the doors and not the
/// gate. The probe route exists because this module's three doors are all
/// `POST`, and v1's whole posture is that a `GET` is admitted and everything
/// else is not — an admitted arm needs a `GET` to be admitted on.
fn elevated_app(harness: &TestHarness, tenant: Uuid) -> Router {
    let state = Arc::new(ApiState {
        db: harness.db.clone(),
        sink: crate::infra::broker::EventSink::Interim(Arc::clone(&harness.outbox)),
        taxonomy_caps: crate::api::rest::TaxonomyCaps::from(&ProductsConfig::default()),
        idempotency_retention_hours: ProductsConfig::default().idempotency_retention_hours,
        bulk_max_rows_per_batch: ProductsConfig::default().bulk_max_rows_per_batch,
        bulk_max_concurrent_batches_per_tenant: ProductsConfig::default()
            .bulk_max_concurrent_batches_per_tenant,
        watermark_skew_tolerance: ProductsConfig::default().watermark_skew_tolerance(),
        reference: crate::api::rest::ReferenceKnobs::from(&ProductsConfig::default()),
        breakglass_window_hours: crate::config::BREAKGLASS_WINDOW_HOURS_DEFAULT,
        breakglass_review_sla_hours: crate::config::BREAKGLASS_REVIEW_SLA_HOURS_DEFAULT,
        eol_enabled: false,
        usage_type_resolver: crate::test_support::resolved_usage_types(),
    });
    let openapi = OpenApiRegistryImpl::new();
    router(Arc::clone(&state), &openapi)
        .route(
            "/bss-products/v1/_probe/tenant",
            axum::routing::get(probe_tenant),
        )
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::api::rest::elevation_gate,
        ))
        .layer(axum::Extension(flat_in_enforcer(tenant)))
}

/// Answer the tenant the request's `SecurityContext` names — which is exactly
/// what the gate substitutes.
async fn probe_tenant(
    ctx: Option<axum::Extension<SecurityContext>>,
) -> axum::Json<serde_json::Value> {
    let tenant = ctx.map_or_else(Uuid::nil, |axum::Extension(c)| c.subject_tenant_id());
    axum::Json(json!({ "tenant": tenant }))
}

/// Open a session directly in the store, so a case can choose its window.
async fn open_session(harness: &TestHarness, opener: Uuid, from_h: i64, until_h: i64) -> Uuid {
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TARGET_TENANT);
    let session_id = Uuid::now_v7();
    repo::open_breakglass_session(
        &conn,
        &scope,
        repo::NewElevation {
            session_id,
            principal: opener,
            target_tenant: TARGET_TENANT,
            valid_from: chrono::Utc::now() + chrono::TimeDelta::try_hours(from_h).expect("hours"),
            valid_until: chrono::Utc::now() + chrono::TimeDelta::try_hours(until_h).expect("hours"),
            path: repo::ApprovalPath::PostHoc,
            opened_at: chrono::Utc::now(),
        },
        "incident 4471",
    )
    .await
    .expect("the session opens");
    session_id
}

/// The `actor_ref` the gate resolves for `subject` in the target tenant —
/// the value it compares against the session's `principal`.
async fn actor_ref_of(harness: &TestHarness, subject: Uuid) -> Uuid {
    let conn = harness.db.conn().expect("scoped connection");
    let scope = AccessScope::for_tenant(TARGET_TENANT);
    repo::resolve_actor_ref(
        &conn,
        &scope,
        TARGET_TENANT,
        &subject.to_string(),
        chrono::Utc::now(),
    )
    .await
    .expect("resolve the pseudonym")
}

async fn elevated(
    app: Router,
    method: &str,
    uri: &str,
    subject: Uuid,
    session: Uuid,
    body: JsonValue,
) -> axum::http::Response<Body> {
    app.oneshot(
        Request::builder()
            .method(method)
            .uri(uri)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .header(
                crate::api::rest::BREAK_GLASS_SESSION_HEADER,
                session.to_string(),
            )
            .extension(ctx_without_roles(subject))
            .body(Body::from(body.to_string()))
            .expect("build the request"),
    )
    .await
    .expect("the router answers")
}

async fn audit_rows(harness: &TestHarness) -> i64 {
    crate::test_support::raw_i64(
        &harness.dsn,
        "SELECT COUNT(*) AS v FROM products_audit_log WHERE subject_kind = 'breakglass'",
    )
    .await
}

/// **Every write under an elevation is refused `BREAKGLASS_WRITE_FORBIDDEN`,
/// with no exception in v1** — and a `GET` through the same session is
/// admitted, under the **target** tenant.
///
/// The two halves are one case because either alone is satisfiable by a
/// defect: a gate that refused everything would pass the first, and one that
/// refused nothing would pass the second.
#[tokio::test]
async fn an_elevation_refuses_every_write_and_admits_a_read_under_the_target() {
    let harness = harness().await;
    let subject = Uuid::from_u128(0x5a_b0);
    let opener = actor_ref_of(&harness, subject).await;
    let session = open_session(&harness, opener, -1, 3).await;

    let write = elevated(
        elevated_app(&harness, TENANT),
        "POST",
        "/bss-products/v1/approvals",
        subject,
        session,
        submission_body(),
    )
    .await;
    let status = write.status();
    let body = body_of(write).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(body["context"]["reason"], "BREAKGLASS_WRITE_FORBIDDEN");

    let read = elevated(
        elevated_app(&harness, TENANT),
        "GET",
        "/bss-products/v1/_probe/tenant",
        subject,
        session,
        json!({}),
    )
    .await;
    assert_eq!(read.status(), 200, "a read inside the window is admitted");
    assert_eq!(
        body_of(read).await["tenant"],
        json!(TARGET_TENANT),
        "the gate substitutes the session's target tenant for the caller's own, which is the \
         whole of what an elevation changes (P-D-133 row 18)"
    );
}

/// **A session that is not the caller's is refused**, and refused the same
/// way a session that does not exist is.
///
/// The gate reads the row unconstrained — it is looking the target tenant up
/// — so this check is the only thing standing between a session id and
/// another principal's elevation.
#[tokio::test]
async fn a_session_opened_by_someone_else_is_refused() {
    let harness = harness().await;
    let opener = actor_ref_of(&harness, Uuid::from_u128(0x5a_b0)).await;
    let session = open_session(&harness, opener, -1, 3).await;

    let response = elevated(
        elevated_app(&harness, TENANT),
        "GET",
        "/bss-products/v1/_probe/tenant",
        Uuid::from_u128(0x5a_be),
        session,
        json!({}),
    )
    .await;
    assert_eq!(response.status(), 403);
    assert_eq!(
        body_of(response).await["context"]["reason"],
        "BREAK_GLASS_SESSION_UNKNOWN",
        "one answer for unknown and not-yours alike, so a caller cannot enumerate elevations by id"
    );

    let unknown = elevated(
        elevated_app(&harness, TENANT),
        "GET",
        "/bss-products/v1/_probe/tenant",
        Uuid::from_u128(0x5a_b0),
        Uuid::now_v7(),
        json!({}),
    )
    .await;
    assert_eq!(unknown.status(), 403);
}

/// **Past the window every call refuses `BREAKGLASS_EXPIRED`, and
/// `expired_emitted` flips for exactly one of them** (**P-D-68** arm 2).
///
/// Three calls, one stamp. The count is what the CAS buys and a
/// read-then-write would give three.
#[tokio::test]
async fn past_the_window_every_call_refuses_and_exactly_one_emits() {
    let harness = harness().await;
    let subject = Uuid::from_u128(0x5a_b0);
    let opener = actor_ref_of(&harness, subject).await;
    let session = open_session(&harness, opener, -5, -1).await;

    for _ in 0..3 {
        let response = elevated(
            elevated_app(&harness, TENANT),
            "GET",
            "/bss-products/v1/_probe/tenant",
            subject,
            session,
            json!({}),
        )
        .await;
        assert_eq!(response.status(), 403);
        assert_eq!(
            body_of(response).await["context"]["reason"],
            "BREAKGLASS_EXPIRED"
        );
    }

    let emitted = crate::test_support::raw_i64(
        &harness.dsn,
        "SELECT COUNT(*) AS v FROM products_breakglass_session WHERE expired_emitted = 1",
    )
    .await;
    assert_eq!(
        emitted, 1,
        "the CAS on the write gives one emission however many callers arrive; reading the column \
         and then writing it would give three"
    );

    // **And the event actually went out, once.** The stamp and the enqueue
    // commit in one transaction, so a flipped stamp beside no outbox row
    // would be the exactly-once guarantee inverted — announced zero times,
    // with no later caller left to send it.
    let body_table = format!("{}_body", events::OUTBOX_TABLE_PREFIX);
    let announced = crate::test_support::raw_i64(
        &harness.dsn,
        &format!("SELECT COUNT(*) AS v FROM {body_table} WHERE payload_type = 'BreakGlassExpired'"),
    )
    .await;
    assert_eq!(announced, 1, "one flip, one BreakGlassExpired");
}

/// **Every elevated access is audited individually** — the probe asserts the
/// **count**, not a sample (`dod-breakglass-readonly`).
///
/// Four calls, four rows. A gate that audited the session once at open, or
/// that sampled, passes a "there is a row" assertion and fails this one.
#[tokio::test]
async fn every_elevated_access_writes_its_own_audit_row() {
    let harness = harness().await;
    let subject = Uuid::from_u128(0x5a_b0);
    let opener = actor_ref_of(&harness, subject).await;
    let session = open_session(&harness, opener, -1, 3).await;
    let before = audit_rows(&harness).await;

    for _ in 0..4 {
        let response = elevated(
            elevated_app(&harness, TENANT),
            "GET",
            "/bss-products/v1/_probe/tenant",
            subject,
            session,
            json!({}),
        )
        .await;
        assert_eq!(response.status(), 200);
    }

    assert_eq!(
        audit_rows(&harness).await - before,
        4,
        "one row per access, not one per session"
    );
}

/// **A request with no elevation header is untouched** — the arm every other
/// request in the gear takes, and the one a gate that mis-read an absent
/// header would break.
#[tokio::test]
async fn a_request_without_the_header_passes_through_unchanged() {
    let harness = harness().await;
    let response = elevated_app(&harness, TENANT)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/bss-products/v1/_probe/tenant")
                .extension(ctx_without_roles(Uuid::from_u128(0x5a_b0)))
                .body(Body::empty())
                .expect("build the request"),
        )
        .await
        .expect("the router answers");
    assert_eq!(response.status(), 200);
    assert_eq!(
        body_of(response).await["tenant"],
        json!(TENANT),
        "the caller's own tenant, unsubstituted"
    );
    assert_eq!(audit_rows(&harness).await, 0, "and no elevation audit row");
}
