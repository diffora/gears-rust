#![allow(clippy::expect_used, clippy::unwrap_used)]
use crate::infra::{
    broker::{ApprovalUnitDecided, ReferenceForceReleased, SkuChanged, SkuPublished},
    storage::repo,
};
use crate::test_support::*;
use axum::{
    Router,
    body::Body,
    http::{Method, Request},
};
use event_broker_sdk::TypedEvent;
use serde_json::{Value, json};
use std::sync::Arc;
use toolkit_gts::gts_id;
use toolkit_security::SecurityContext;
use tower::ServiceExt;
use uuid::Uuid;

fn routes(s: Arc<crate::api::rest::ApiState>, o: &dyn toolkit::api::OpenApiRegistry) -> Router {
    crate::api::rest::categories::router(s.clone(), o)
        .merge(crate::api::rest::skus::router(s.clone(), o))
        .merge(crate::api::rest::sku_governance::router(s.clone(), o))
        .merge(crate::api::rest::approval_units::router(s.clone(), o))
        .merge(crate::api::rest::approval_policy::router(s.clone(), o))
        .merge(crate::api::rest::references::router(s, o))
}
struct Fixture {
    state: Arc<crate::api::rest::ApiState>,
    app: Router,
    dsn: String,
    tenant: Uuid,
    author: SecurityContext,
    reviewer: SecurityContext,
    owner: SecurityContext,
    id: Uuid,
}
async fn call(
    app: &Router,
    ctx: &SecurityContext,
    method: Method,
    path: &str,
    body: Value,
    key: Option<&str>,
) -> (u16, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/bss-products/v1{path}"))
        .extension(ctx.clone())
        .header("Content-Type", "application/json");
    if let Some(key) = key {
        request = request.header("Idempotency-Key", key);
    }
    let r = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = r.status().as_u16();
    (status, body_json(r).await)
}
impl Fixture {
    async fn new(quorum: u32) -> Self {
        let tenant = Uuid::new_v4();
        let author = authed_ctx(tenant);
        let reviewer = authed_ctx(tenant);
        let owner = SecurityContext::builder()
            .subject_id(Uuid::from_u128(42))
            .subject_tenant_id(tenant)
            .subject_type(gts_id!("cf.core.security.subject_user.v1~"))
            .token_scopes(vec!["*".into()])
            .build()
            .unwrap();
        let (db, _, _, dsn) = test_db().await;
        let (app, state) = rest_app_on_db(tenant, routes, resolved_usage_types(), "test", db).await;
        let (status, c) = call(
            &app,
            &author,
            Method::POST,
            "/categories",
            json!({"code":"c","name":"C"}),
            None,
        )
        .await;
        assert_eq!(status, 201, "{c}");
        let (status,s)=call(&app,&author,Method::POST,"/skus",json!({"code":"SKU","name":"SKU","type":"usage","category_id":c["id"],"usage_type_ref":"storage","unit":"GB"}),None).await;
        assert_eq!(status, 201, "{s}");
        let f = Self {
            state,
            app,
            dsn,
            tenant,
            author,
            reviewer,
            owner,
            id: Uuid::parse_str(s["id"].as_str().unwrap()).unwrap(),
        };
        f.policy(quorum).await;
        f
    }
    async fn policy(&self, quorum: u32) {
        let (status, b) = call(
            &self.app,
            &self.author,
            Method::PUT,
            "/approval-policy",
            json!({"quorum":quorum}),
            None,
        )
        .await;
        assert_eq!(status, 200, "{b}");
    }
    async fn post(&self, suffix: &str, body: Value) -> (u16, Value) {
        call(
            &self.app,
            &self.author,
            Method::POST,
            &format!("/skus/{}{suffix}", self.id),
            body,
            None,
        )
        .await
    }
    async fn vote(&self, unit: &Value, action: &str, generation: i32) -> (u16, Value) {
        call(
            &self.app,
            &self.reviewer,
            Method::POST,
            &format!(
                "/approval-units/{}/{action}",
                unit["unit"]["id"].as_str().unwrap()
            ),
            json!({"generation":generation,"note":"reviewed"}),
            None,
        )
        .await
    }
    async fn card(&self) -> Value {
        let (status, b) = call(
            &self.app,
            &self.author,
            Method::GET,
            &format!("/skus/{}", self.id),
            json!({}),
            None,
        )
        .await;
        assert_eq!(status, 200, "{b}");
        b["sku"].clone()
    }
    async fn publish(&self) {
        self.policy(0).await;
        let (status, b) = self.post("/submit", json!({})).await;
        assert_eq!(status, 200, "{b}");
    }
    async fn reserve(&self, ref_id: Uuid) -> (u16, Value) {
        call(
            &self.app,
            &self.owner,
            Method::POST,
            &format!("/skus/{}/references/reserve", self.id),
            json!({"owner":"pricing","kind":"price","ref_id":ref_id}),
            None,
        )
        .await
    }
    async fn release(&self, id: &Value) -> (u16, Value) {
        call(
            &self.app,
            &self.owner,
            Method::DELETE,
            &format!("/references/{}", id.as_str().unwrap()),
            json!({}),
            None,
        )
        .await
    }
}
#[tokio::test]
async fn quorum_zero_publishes_at_submit_and_records_the_unit() {
    let f = Fixture::new(0).await;
    let (status, u) = f.post("/submit", json!({})).await;
    assert_eq!(status, 200, "{u}");
    assert_eq!(u["applied"], true);
    let s = f.card().await;
    assert_eq!(s["lifecycle"], "published");
    assert_eq!(s["published_version"], 1);
    let (status, units) = call(
        &f.app,
        &f.author,
        Method::GET,
        "/approval-units?state=approved",
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(units["items"].as_array().unwrap().len(), 1);
    for (reference, count) in [(f.id, 1), (Uuid::new_v4(), 0)] {
        let (status, body) = call(
            &f.app,
            &f.author,
            Method::GET,
            &format!("/approval-units?state=approved&kind=sku_publish&ref_id={reference}"),
            json!({}),
            None,
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["items"].as_array().unwrap().len(), count);
    }
    assert_eq!(enqueued_event_count(&f.dsn, SkuPublished::TYPE_ID).await, 1);
    assert_eq!(
        enqueued_event_count(&f.dsn, ApprovalUnitDecided::TYPE_ID).await,
        1
    );
}
#[tokio::test]
async fn quorum_one_the_author_may_not_approve_and_an_independent_reviewer_publishes() {
    let f = Fixture::new(1).await;
    let submitter = authed_ctx(f.tenant);
    let (status, u) = call(
        &f.app,
        &submitter,
        Method::POST,
        &format!("/skus/{}/submit", f.id),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_ne!(submitter.subject_id(), f.author.subject_id());
    let (status, b) = call(
        &f.app,
        &f.author,
        Method::POST,
        &format!(
            "/approval-units/{}/approve",
            u["unit"]["id"].as_str().unwrap()
        ),
        json!({"generation":1}),
        None,
    )
    .await;
    assert_eq!(status, 403);
    assert_eq!(problem_code(&b), "SOD_VIOLATION");
    let locked = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::PATCH)
                .uri(format!("/bss-products/v1/skus/{}", f.id))
                .extension(f.author.clone())
                .header("Content-Type", "application/json")
                .header(
                    "If-Match",
                    format!("\"{}\"", u["sku"]["revision"].as_i64().unwrap()),
                )
                .body(Body::from(r#"{"name":"locked"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(locked.status(), 409);
    assert_eq!(problem_code(&body_json(locked).await), "ROW_LOCKED_PENDING");
    let (status, b) = f.vote(&u, "approve", 1).await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["outcome"], "applied");
    assert_eq!(f.card().await["approved_by_unit_id"], u["unit"]["id"]);
}
#[tokio::test]
async fn a_change_unit_applies_from_its_effective_date_and_emits_the_field_list() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let date = time::OffsetDateTime::now_utc().date() + time::Duration::days(7);
    let (status, b) = f
        .post(
            "/changes",
            json!({"gl_code":"4012-STOR","effective_from":date.to_string()}),
        )
        .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(f.card().await["published_version"], 2);
    for (as_of, version) in [(date - time::Duration::days(1), 1), (date, 2)] {
        let (status, body) = call(
            &f.app,
            &f.author,
            Method::GET,
            &format!("/skus/{}/versions?as_of={as_of}", f.id),
            json!({}),
            None,
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body["published_version"], version);
    }
    assert_eq!(
        enqueued_event_envelope(&f.dsn, SkuChanged::TYPE_ID).await["changed"],
        json!(["gl_code"])
    );
}
#[tokio::test]
async fn content_drift_refreshes_the_generation_and_the_first_reviewer_votes_again() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(2).await;
    let (_, u) = f.post("/changes", json!({"gl_code":"4012"})).await;
    let (status, first) = f.vote(&u, "approve", 1).await;
    assert_eq!(status, 200);
    assert_eq!(first["unit"]["decisions"].as_array().unwrap().len(), 1);
    let (_, queue) = call(
        &f.app,
        &f.reviewer,
        Method::GET,
        &format!("/approval-units?ref_id={}", f.id),
        json!({}),
        None,
    )
    .await;
    let pending = queue["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == u["unit"]["id"])
        .unwrap();
    assert_eq!(pending["decisions"].as_array().unwrap().len(), 1);
    let (db, scope) = repo_connection(&f.dsn, f.tenant).await;
    let conn = db.conn().unwrap();
    let mut c = bss_products_sdk::models::SkuContent::from(
        &repo::find_sku(&conn, &scope, f.tenant, f.id)
            .await
            .unwrap()
            .unwrap(),
    );
    c.description = "drift".into();
    repo::write_sku_content(
        &conn,
        &scope,
        f.tenant,
        f.id,
        &c,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    let b = authed_ctx(f.tenant);
    let path = format!(
        "/approval-units/{}/approve",
        u["unit"]["id"].as_str().unwrap()
    );
    let (status, body) = call(
        &f.app,
        &b,
        Method::POST,
        &path,
        json!({"generation":1}),
        None,
    )
    .await;
    assert_eq!(status, 400, "{body}");
    assert_eq!(problem_code(&body), "UNIT_STALE");
    assert_eq!(body["context"]["generation"], 2);
    let (_, card) = call(
        &f.app,
        &f.reviewer,
        Method::GET,
        &format!("/approval-units/{}", u["unit"]["id"].as_str().unwrap()),
        json!({}),
        None,
    )
    .await;
    assert_eq!(card["generation"], 2);
    assert_eq!(card["decisions"][0]["stale"], true);
    assert_eq!(card["state"], "pending");
    let (status, body) = f.vote(&u, "approve", 1).await;
    assert_eq!(status, 400);
    assert_eq!(problem_code(&body), "GENERATION_MISMATCH");
    assert_eq!(f.vote(&u, "approve", 2).await.0, 200);
    assert_eq!(
        call(
            &f.app,
            &b,
            Method::POST,
            &path,
            json!({"generation":2}),
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(f.card().await["description"], "drift");
}
#[tokio::test]
async fn retire_is_refused_while_a_reservation_is_live_and_a_fenced_sku_refuses_new_reservations() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    let (status, r) = f.reserve(Uuid::new_v4()).await;
    assert_eq!(status, 201, "{r}");
    let (status, b) = f.post("/retire", json!({})).await;
    assert_eq!(status, 409);
    assert_eq!(problem_code(&b), "SKU_REFERENCED");
    assert_eq!(
        b["context"]["references"][0]["reservation_id"],
        r["reservation_id"]
    );
    assert!(
        b.to_string()
            .contains(r["reservation_id"].as_str().unwrap())
    );
    assert_eq!(f.release(&r["reservation_id"]).await.0, 200);
    let (status, u) = f.post("/retire", json!({})).await;
    assert_eq!(status, 200, "{u}");
    let (status, b) = f.reserve(Uuid::new_v4()).await;
    assert_eq!(status, 409);
    assert_eq!(problem_code(&b), "SKU_FENCED");
    assert_eq!(f.vote(&u, "approve", 1).await.0, 200);
    assert_eq!(f.card().await["lifecycle"], "retired");
}
#[tokio::test]
async fn a_type_change_on_a_published_sku_is_frozen_by_a_confirmed_reference() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    let (_, r) = f.reserve(Uuid::new_v4()).await;
    assert_eq!(
        call(
            &f.app,
            &f.owner,
            Method::POST,
            &format!(
                "/references/{}/confirm",
                r["reservation_id"].as_str().unwrap()
            ),
            json!({}),
            None
        )
        .await
        .0,
        200
    );
    let (status, b) = f.post("/changes", json!({"type":"recurring"})).await;
    assert_eq!(status, 409);
    assert_eq!(problem_code(&b), "SKU_TYPE_FROZEN");
    assert_eq!(f.card().await["type_change_pending"], false);
    f.release(&r["reservation_id"]).await;
    let (_, u) = f.post("/changes", json!({"type":"recurring"})).await;
    assert_eq!(f.vote(&u, "reject", 1).await.0, 200);
    assert_eq!(f.card().await["type_change_pending"], false);
    assert_eq!(f.post("/changes", json!({"type":"recurring"})).await.0, 200);
}
#[tokio::test]
async fn an_unconfirmed_reservation_keeps_counting_until_released_and_reserve_is_idempotent() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let id = Uuid::new_v4();
    let (status, a) = f.reserve(id).await;
    assert_eq!(status, 201, "{a}");
    let (status, b) = f.reserve(id).await;
    assert_eq!(status, 200);
    assert_eq!(a["reservation_id"], b["reservation_id"]);
    assert_eq!(f.post("/retire", json!({})).await.0, 409);
    f.release(&a["reservation_id"]).await;
    let (status, b) = f.reserve(id).await;
    assert_eq!(status, 201);
    assert_ne!(a["reservation_id"], b["reservation_id"]);
    let confirm = |id: &Value| format!("/references/{}/confirm", id.as_str().unwrap());
    let (status, body) = call(
        &f.app,
        &f.owner,
        Method::POST,
        &confirm(&a["reservation_id"]),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(problem_code(&body), "REFERENCE_RELEASED");
    for _ in 0..2 {
        assert_eq!(
            call(
                &f.app,
                &f.owner,
                Method::POST,
                &confirm(&b["reservation_id"]),
                json!({}),
                None
            )
            .await
            .0,
            200
        );
    }
}
#[tokio::test]
async fn an_operator_release_needs_force_and_a_reason_and_is_evented() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let (_, r) = f.reserve(Uuid::new_v4()).await;
    let path = format!("/references/{}", r["reservation_id"].as_str().unwrap());
    assert_eq!(
        call(&f.app, &f.author, Method::DELETE, &path, json!({}), None)
            .await
            .0,
        400
    );
    let (status, b) = call(
        &f.app,
        &f.author,
        Method::DELETE,
        &path,
        json!({"force":true,"reason":"orphan"}),
        None,
    )
    .await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["forced"], true);
    assert_eq!(
        enqueued_event_count(&f.dsn, ReferenceForceReleased::TYPE_ID).await,
        1
    );
}
#[tokio::test]
async fn a_rejected_or_withdrawn_retire_lifts_the_fence_and_the_lock_together_and_a_new_fence_can_follow()
 {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    for action in ["reject", "withdraw"] {
        let (_, u) = f.post("/retire", json!({})).await;
        let ctx = if action == "withdraw" {
            &f.author
        } else {
            &f.reviewer
        };
        let (status, b) = call(
            &f.app,
            ctx,
            Method::POST,
            &format!(
                "/approval-units/{}/{action}",
                u["unit"]["id"].as_str().unwrap()
            ),
            json!({"generation":1,"note":"no"}),
            None,
        )
        .await;
        assert_eq!(status, 200, "{b}");
        let s = f.card().await;
        assert_eq!(s["lifecycle"], "published");
        assert!(s["pending_unit_id"].is_null());
    }
    assert_eq!(f.reserve(Uuid::new_v4()).await.0, 201);
}
#[tokio::test]
async fn a_pending_deprecation_survives_submission() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    let (_, u) = f.post("/changes", json!({"lifecycle":"deprecated"})).await;
    assert_eq!(
        u["unit"]["snapshot"]["skus"][0]["after"]["lifecycle"],
        "deprecated"
    );
    let after = &u["unit"]["snapshot"]["skus"][0]["after"]["content"];
    for field in [
        "revision",
        "published_version",
        "pending_unit_id",
        "fenced_at",
        "fence_op_id",
        "type_change_pending",
    ] {
        assert!(after.get(field).is_none());
    }
    let (status, b) = f.vote(&u, "approve", 1).await;
    assert_eq!(status, 200, "{b}");
    assert_eq!(f.card().await["lifecycle"], "deprecated");
}
#[tokio::test]
async fn reject_needs_a_note_unlocks_and_withdraw_is_the_submitters() {
    let f = Fixture::new(1).await;
    let (_, u) = f.post("/submit", json!({})).await;
    let path = format!("/approval-units/{}", u["unit"]["id"].as_str().unwrap());
    let (status, b) = call(
        &f.app,
        &f.reviewer,
        Method::POST,
        &format!("{path}/reject"),
        json!({"generation":1}),
        None,
    )
    .await;
    assert_eq!(status, 400);
    assert_eq!(problem_code(&b), "NOTE_REQUIRED");
    assert_eq!(f.vote(&u, "withdraw", 1).await.0, 403);
    assert_eq!(f.vote(&u, "reject", 1).await.0, 200);
    assert!(f.card().await["pending_unit_id"].is_null());
}
#[tokio::test]
async fn an_idempotent_replay_returns_the_stored_receipt_and_a_different_body_conflicts() {
    let f = Fixture::new(0).await;
    let path = format!("/skus/{}/submit", f.id);
    let a = call(
        &f.app,
        &f.author,
        Method::POST,
        &path,
        json!({}),
        Some("publish"),
    )
    .await;
    let b = call(
        &f.app,
        &f.author,
        Method::POST,
        &path,
        json!({}),
        Some("publish"),
    )
    .await;
    assert_eq!(a, b);
    assert_eq!(a.0, 200);
    let path = format!("/skus/{}/changes", f.id);
    assert_eq!(
        call(
            &f.app,
            &f.author,
            Method::POST,
            &path,
            json!({"gl_code":"a"}),
            Some("change")
        )
        .await
        .0,
        200
    );
    let (status, b) = call(
        &f.app,
        &f.author,
        Method::POST,
        &path,
        json!({"gl_code":"b"}),
        Some("change"),
    )
    .await;
    assert_eq!(status, 409);
    assert_eq!(problem_code(&b), "IDEMPOTENCY_CONFLICT");
}

async fn second_app(
    f: &Fixture,
    catalog: Arc<dyn bss_products_sdk::usage_types::UsageTypeCatalog>,
) -> Router {
    let (db, _) = repo_connection(&f.dsn, f.tenant).await;
    let state = Arc::new(crate::api::rest::ApiState {
        db,
        sink: f.state.sink.clone(),
        usage_type_catalog: catalog,
        usage_type_catalog_source: "test",
        idempotency_retention_hours: 24,
        fence_ttl_minutes: 30,
        reference_principals: f.state.reference_principals.clone(),
    });
    routes(state, &toolkit::api::OpenApiRegistryImpl::new())
        .layer(axum::Extension(flat_in_enforcer(f.tenant)))
}
#[tokio::test]
async fn a_usage_sku_whose_ref_the_catalog_does_not_know_cannot_be_submitted() {
    let f = Fixture::new(0).await;
    for (answer, expected, code) in [
        (
            crate::domain::recognized::UsageTypeAnswer::Unresolved,
            400,
            "USAGE_TYPE_UNRESOLVED",
        ),
        (
            crate::domain::recognized::UsageTypeAnswer::Unavailable,
            503,
            "USAGE_TYPE_UNAVAILABLE",
        ),
    ] {
        let app = second_app(&f, Arc::new(StubUsageTypes::always(answer))).await;
        let (status, b) = call(
            &app,
            &f.author,
            Method::POST,
            &format!("/skus/{}/submit", f.id),
            json!({}),
            Some("retry-after-catalog"),
        )
        .await;
        assert_eq!(status, expected, "{b}");
        assert!(b.to_string().contains(code), "{b}");
        assert_eq!(
            raw_i64(&f.dsn, "SELECT count(*) AS v FROM products_approval_unit").await,
            0
        );
        assert_eq!(idempotency_rows_for(&f.dsn, "retry-after-catalog").await, 0);
    }
    assert_eq!(f.post("/submit", json!({})).await.0, 200);
}
#[tokio::test]
async fn a_fence_left_behind_is_resumed_by_the_next_retire_and_expired_ones_are_lifted() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    let (db, scope) = repo_connection(&f.dsn, f.tenant).await;
    let conn = db.conn().unwrap();
    let op = Uuid::new_v4();
    let now = time::OffsetDateTime::now_utc();
    repo::fence_sku(&conn, &scope, f.tenant, f.id, repo::Fence::Retire, op, now)
        .await
        .unwrap();
    let (status, u) = f.post("/retire", json!({})).await;
    assert_eq!(status, 200, "{u}");
    assert_eq!(
        repo::find_sku_fence(&conn, &scope, f.tenant, f.id)
            .await
            .unwrap()
            .unwrap()
            .fence_op_id,
        Some(op)
    );
    assert_eq!(f.post("/unfence", json!({})).await.0, 409);
    assert_eq!(f.vote(&u, "reject", 1).await.0, 200);
    let expired = Uuid::new_v4();
    repo::fence_sku(
        &conn,
        &scope,
        f.tenant,
        f.id,
        repo::Fence::Retire,
        expired,
        now - time::Duration::hours(2),
    )
    .await
    .unwrap();
    assert_eq!(f.card().await["lifecycle"], "published");
    repo::fence_sku(
        &conn,
        &scope,
        f.tenant,
        f.id,
        repo::Fence::TypeChange,
        Uuid::new_v4(),
        now,
    )
    .await
    .unwrap();
    assert_eq!(f.post("/unfence", json!({})).await.0, 200);
    assert_eq!(f.card().await["type_change_pending"], false);
}
#[tokio::test]
async fn a_reserve_and_a_fence_racing_end_consistent() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    let second = second_app(&f, resolved_usage_types()).await;
    let path = format!("/skus/{}/retire", f.id);
    let (reserve, retire) = tokio::join!(
        f.reserve(Uuid::new_v4()),
        call(&second, &f.author, Method::POST, &path, json!({}), None)
    );
    match (reserve.0, retire.0) {
        (201, 409) => {
            assert_eq!(problem_code(&retire.1), "SKU_REFERENCED");
            assert_eq!(f.card().await["lifecycle"], "published");
        }
        (409, 200) => {
            assert_eq!(problem_code(&reserve.1), "SKU_FENCED");
            assert_eq!(f.card().await["lifecycle"], "retiring");
        }
        other => panic!("inconsistent race {other:?}: {reserve:?}, {retire:?}"),
    }
}
#[tokio::test]
async fn two_reviewers_approving_at_once_apply_exactly_once() {
    let f = Fixture::new(1).await;
    let (_, u) = f.post("/submit", json!({})).await;
    let second = second_app(&f, resolved_usage_types()).await;
    let b = authed_ctx(f.tenant);
    let path = format!(
        "/approval-units/{}/approve",
        u["unit"]["id"].as_str().unwrap()
    );
    let (a, b) = tokio::join!(
        f.vote(&u, "approve", 1),
        call(
            &second,
            &b,
            Method::POST,
            &path,
            json!({"generation":1}),
            None
        )
    );
    let (winner, loser) = if a.0 == 200 { (a, b) } else { (b, a) };
    assert_eq!(winner.0, 200, "{winner:?}");
    assert_eq!(winner.1["outcome"], "applied");
    assert_eq!(loser.0, 409, "{loser:?}");
    assert!(matches!(
        problem_code(&loser.1).as_str(),
        "UNIT_ALREADY_DECIDED" | "UNIT_CONTENDED"
    ));
    assert_eq!(f.card().await["published_version"], 1);
    assert_eq!(enqueued_event_count(&f.dsn, SkuPublished::TYPE_ID).await, 1);
}
#[tokio::test]
async fn a_vote_must_name_the_generation_it_saw() {
    let f = Fixture::new(2).await;
    let (_, u) = f.post("/submit", json!({})).await;
    let path = format!(
        "/approval-units/{}/approve",
        u["unit"]["id"].as_str().unwrap()
    );
    assert_eq!(
        call(&f.app, &f.reviewer, Method::POST, &path, json!({}), None)
            .await
            .0,
        400
    );
    let (status, b) = f.vote(&u, "approve", 0).await;
    assert_eq!(status, 400);
    assert_eq!(problem_code(&b), "GENERATION_MISMATCH");
    assert_eq!(b["context"]["generation"], 1);
    let (status, b) = f.vote(&u, "reject", 0).await;
    assert_eq!(status, 400);
    assert_eq!(b["context"]["generation"], 1);
    assert_eq!(f.vote(&u, "approve", 1).await.0, 200);
}
#[tokio::test]
async fn reference_owner_is_bound_to_the_principal_and_cannot_be_spoofed() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let path = format!("/skus/{}/references/reserve", f.id);
    assert_eq!(
        call(
            &f.app,
            &f.author,
            Method::POST,
            &path,
            json!({"owner":"pricing","kind":"price","ref_id":Uuid::new_v4()}),
            None
        )
        .await
        .0,
        403
    );
    assert_eq!(
        call(
            &f.app,
            &f.owner,
            Method::POST,
            &path,
            json!({"owner":"other","kind":"price","ref_id":Uuid::new_v4()}),
            None
        )
        .await
        .0,
        403
    );
    let (_, r) = f.reserve(Uuid::new_v4()).await;
    assert_eq!(
        call(
            &f.app,
            &f.author,
            Method::POST,
            &format!(
                "/references/{}/confirm",
                r["reservation_id"].as_str().unwrap()
            ),
            json!({}),
            None
        )
        .await
        .0,
        403
    );
}

struct ActionResolver {
    id: Option<Uuid>,
    allowed: Option<&'static str>,
    tenant: Uuid,
    seen: Arc<std::sync::atomic::AtomicUsize>,
}
#[async_trait::async_trait]
impl authz_resolver_sdk::AuthZResolverApi for ActionResolver {
    async fn evaluate(
        &self,
        _: toolkit_security::PlatformSecurityContext,
        req: authz_resolver_sdk::models::EvaluationRequest,
    ) -> Result<
        authz_resolver_sdk::models::EvaluationResponse,
        toolkit::api::canonical_prelude::CanonicalError,
    > {
        use authz_resolver_sdk::{
            constraints::{Constraint, InPredicate, Predicate},
            models::{EvaluationResponse, EvaluationResponseContext},
        };
        self.seen.store(
            crate::authz::actions::ALL
                .iter()
                .position(|a| *a == req.action.name)
                .unwrap()
                + 1,
            std::sync::atomic::Ordering::Relaxed,
        );
        Ok(EvaluationResponse {
            decision: self.allowed == Some(req.action.name.as_str()),
            context: EvaluationResponseContext {
                constraints: vec![Constraint {
                    predicates: vec![Predicate::In(InPredicate::new(
                        toolkit_security::pep_properties::OWNER_TENANT_ID,
                        vec![self.tenant],
                    ))]
                    .into_iter()
                    .chain(self.id.map(|id| {
                        Predicate::In(InPredicate::new(
                            toolkit_security::pep_properties::RESOURCE_ID,
                            vec![id],
                        ))
                    }))
                    .collect(),
                }],
                deny_reason: None,
            },
        })
    }
}
#[tokio::test]
async fn every_route_denies_the_wrong_action_and_the_other_tenant() {
    let f = Fixture::new(1).await;
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let enforcer = authz_resolver_sdk::PolicyEnforcer::new(Arc::new(ActionResolver {
        id: None,
        allowed: None,
        tenant: f.tenant,
        seen: seen.clone(),
    }));
    let app = routes(f.state.clone(), &toolkit::api::OpenApiRegistryImpl::new())
        .layer(axum::Extension(enforcer));
    let sku_path = format!("/skus/{}", f.id);
    let category_path = format!("/categories/{}", Uuid::new_v4());
    let unit_path = format!("/approval-units/{}", Uuid::new_v4());
    let reference_path = format!("/references/{}", Uuid::new_v4());
    let cases = vec![
        (Method::POST, "/categories".into(), "author"),
        (Method::GET, "/categories".into(), "read"),
        (Method::PATCH, category_path.clone(), "author"),
        (Method::POST, format!("{category_path}/retire"), "author"),
        (Method::POST, "/skus".into(), "author"),
        (Method::GET, "/skus".into(), "read"),
        (Method::GET, sku_path.clone(), "read"),
        (Method::PATCH, sku_path.clone(), "author"),
        (Method::GET, format!("{sku_path}/versions"), "read"),
        (Method::GET, format!("{sku_path}/references"), "read"),
        (Method::POST, format!("{sku_path}/submit"), "submit"),
        (Method::POST, format!("{sku_path}/changes"), "submit"),
        (Method::POST, format!("{sku_path}/retire"), "submit"),
        (Method::POST, format!("{sku_path}/unfence"), "submit"),
        (Method::GET, "/approval-units".into(), "read"),
        (Method::GET, unit_path.clone(), "read"),
        (Method::POST, format!("{unit_path}/approve"), "approve"),
        (Method::POST, format!("{unit_path}/reject"), "approve"),
        (Method::POST, format!("{unit_path}/withdraw"), "submit"),
        (Method::GET, "/approval-policy".into(), "settings"),
        (Method::PUT, "/approval-policy".into(), "settings"),
        (
            Method::POST,
            format!("{sku_path}/references/reserve"),
            "reference",
        ),
        (
            Method::POST,
            format!("{reference_path}/confirm"),
            "reference",
        ),
        (Method::DELETE, reference_path, "submit"),
    ];
    assert_eq!(cases.len(), 24);
    for (method, path, action) in cases {
        seen.store(0, std::sync::atomic::Ordering::Relaxed);
        let (status, b) = call(&app, &f.author, method.clone(), &path, json!({}), None).await;
        assert_eq!(status, 403, "{method} {path}: {b}");
        assert_eq!(
            seen.load(std::sync::atomic::Ordering::Relaxed),
            crate::authz::actions::ALL
                .iter()
                .position(|a| *a == action)
                .unwrap()
                + 1
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method.clone())
                    .uri(format!("/bss-products/v1{path}"))
                    .header("Content-Type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), 401, "{method} {path}");
    }
    let foreign = authed_ctx(Uuid::new_v4());
    assert_eq!(
        call(&f.app, &foreign, Method::GET, &sku_path, json!({}), None)
            .await
            .0,
        404
    );
    let (_, unit) = f.post("/submit", json!({})).await;
    assert_eq!(
        call(
            &f.app,
            &foreign,
            Method::GET,
            &format!("/approval-units/{}", unit["unit"]["id"].as_str().unwrap()),
            json!({}),
            None
        )
        .await
        .0,
        404
    );
    // A reviewer with APPROVE alone is valid; no hidden SUBMIT check can be introduced.
    let reviewer_app =
        routes(f.state.clone(), &toolkit::api::OpenApiRegistryImpl::new()).layer(axum::Extension(
            authz_resolver_sdk::PolicyEnforcer::new(Arc::new(ActionResolver {
                id: None,
                allowed: Some("approve"),
                tenant: f.tenant,
                seen,
            })),
        ));
    assert_eq!(
        call(
            &reviewer_app,
            &f.reviewer,
            Method::POST,
            &format!(
                "/approval-units/{}/approve",
                unit["unit"]["id"].as_str().unwrap()
            ),
            json!({"generation":1}),
            None
        )
        .await
        .0,
        200
    );
}
#[tokio::test]
async fn reject_refreshes_drift_and_commits_without_counting_the_rejection() {
    let f = Fixture::new(1).await;
    let (_, u) = f.post("/submit", json!({})).await;
    let (db, scope) = repo_connection(&f.dsn, f.tenant).await;
    let conn = db.conn().unwrap();
    let mut c = bss_products_sdk::models::SkuContent::from(
        &repo::find_sku(&conn, &scope, f.tenant, f.id)
            .await
            .unwrap()
            .unwrap(),
    );
    c.description = "changed".into();
    repo::write_sku_content(
        &conn,
        &scope,
        f.tenant,
        f.id,
        &c,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    let (status, b) = f.vote(&u, "reject", 1).await;
    assert_eq!(status, 400, "{b}");
    assert_eq!(problem_code(&b), "UNIT_STALE");
    assert_eq!(b["context"]["generation"], 2);
    let (_, card) = call(
        &f.app,
        &f.reviewer,
        Method::GET,
        &format!("/approval-units/{}", u["unit"]["id"].as_str().unwrap()),
        json!({}),
        None,
    )
    .await;
    assert_eq!(card["state"], "pending");
    assert_eq!(card["generation"], 2);
    assert_eq!(card["decisions"], json!([]));
    assert_eq!(card["impact_live"]["description"], "changed");
    assert_eq!(f.vote(&u, "reject", 2).await.0, 200);
}
#[tokio::test]
async fn replay_precedes_resolution_and_audit_failure_rolls_back_fence_unit_and_events() {
    let f = Fixture::new(0).await;
    let path = format!("/skus/{}/submit", f.id);
    let first = call(
        &f.app,
        &f.author,
        Method::POST,
        &path,
        json!({}),
        Some("replay"),
    )
    .await;
    assert_eq!(first.0, 200);
    let catalog = Arc::new(StubUsageTypes::always(
        crate::domain::recognized::UsageTypeAnswer::Unavailable,
    ));
    let unavailable = second_app(&f, catalog.clone()).await;
    assert_eq!(
        call(
            &unavailable,
            &f.author,
            Method::POST,
            &path,
            json!({}),
            Some("replay")
        )
        .await,
        first
    );
    assert_eq!(catalog.asked.load(std::sync::atomic::Ordering::Relaxed), 0);
    let before = raw_i64(&f.dsn, "SELECT count(*) AS v FROM products_approval_unit").await;
    drop_table(&f.dsn, "products_audit_log").await;
    assert_eq!(f.post("/retire", json!({})).await.0, 500);
    assert_eq!(f.card().await["lifecycle"], "published");
    assert_eq!(
        raw_i64(&f.dsn, "SELECT count(*) AS v FROM products_approval_unit").await,
        before
    );
    assert_eq!(
        enqueued_event_count(&f.dsn, crate::infra::broker::SkuRetired::TYPE_ID).await,
        0
    );
}

#[tokio::test]
async fn list_recovers_an_expired_orphan_before_filtering_and_submit_accepts_no_body() {
    let f = Fixture::new(0).await;
    let response = f
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri(format!("/bss-products/v1/skus/{}/submit", f.id))
                .extension(f.author.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let (db, scope) = repo_connection(&f.dsn, f.tenant).await;
    repo::fence_sku(
        &db.conn().unwrap(),
        &scope,
        f.tenant,
        f.id,
        repo::Fence::Retire,
        Uuid::new_v4(),
        time::OffsetDateTime::now_utc() - time::Duration::hours(2),
    )
    .await
    .unwrap();
    let (status, body) = call(
        &f.app,
        &f.author,
        Method::GET,
        "/skus?lifecycle=published",
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert!(
        repo::find_sku_fence(&db.conn().unwrap(), &scope, f.tenant, f.id)
            .await
            .unwrap()
            .unwrap()
            .fenced_at
            .is_none()
    );
}
struct ProposedCatalog;
#[async_trait::async_trait]
impl bss_products_sdk::usage_types::UsageTypeCatalog for ProposedCatalog {
    async fn resolve(
        &self,
        _: &SecurityContext,
        reference: &str,
    ) -> crate::domain::recognized::UsageTypeAnswer {
        assert_eq!(
            reference, "new-meter",
            "both submit and approve resolve proposed content"
        );
        crate::domain::recognized::UsageTypeAnswer::Resolved(probe_binding())
    }
    async fn list(
        &self,
        _: &SecurityContext,
        _: Option<&str>,
        _: Option<&str>,
        _: u32,
        _: Option<&str>,
    ) -> Result<
        bss_products_sdk::usage_types::UsageTypePage,
        toolkit::api::canonical_prelude::CanonicalError,
    > {
        Ok(bss_products_sdk::usage_types::UsageTypePage::default())
    }
}
#[tokio::test]
async fn changes_resolve_the_proposed_meter_and_apply_revalidates_catalog_answers() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    let proposed = second_app(&f, Arc::new(ProposedCatalog)).await;
    let (status, u) = call(
        &proposed,
        &f.author,
        Method::POST,
        &format!("/skus/{}/changes", f.id),
        json!({"usage_type_ref":"new-meter"}),
        None,
    )
    .await;
    assert_eq!(status, 200, "{u}");
    let path = format!(
        "/approval-units/{}/approve",
        u["unit"]["id"].as_str().unwrap()
    );
    let unavailable = second_app(
        &f,
        Arc::new(StubUsageTypes::always(
            crate::domain::recognized::UsageTypeAnswer::Unresolved,
        )),
    )
    .await;
    let (status, b) = call(
        &unavailable,
        &f.reviewer,
        Method::POST,
        &path,
        json!({"generation":1}),
        None,
    )
    .await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "USAGE_TYPE_UNRESOLVED");
    assert_eq!(f.card().await["usage_type_ref"], "storage");
    assert_eq!(
        call(
            &proposed,
            &f.reviewer,
            Method::POST,
            &path,
            json!({"generation":1}),
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(f.card().await["usage_type_ref"], "new-meter");
}
#[tokio::test]
async fn equal_version_dates_work_and_backwards_changes_roll_back_the_head() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let date = time::OffsetDateTime::now_utc().date() + time::Duration::days(7);
    for value in ["one", "two"] {
        let (status, b) = f
            .post(
                "/changes",
                json!({"gl_code":value,"effective_from":date.to_string()}),
            )
            .await;
        assert_eq!(status, 200, "{b}");
    }
    let (status,b)=f.post("/changes",json!({"gl_code":"backwards","effective_from":(date-time::Duration::days(1)).to_string()})).await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "VERSION_ORDER");
    assert_eq!(f.card().await["gl_code"], "two");
    assert_eq!(f.card().await["published_version"], 3);
    let (_, version) = call(
        &f.app,
        &f.author,
        Method::GET,
        &format!("/skus/{}/versions?as_of={date}", f.id),
        json!({}),
        None,
    )
    .await;
    assert_eq!(version["published_version"], 3);
}
#[tokio::test]
async fn approved_type_change_clears_its_fence_and_retire_replays_before_fence_work() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    let (_, u) = f.post("/changes", json!({"type":"recurring"})).await;
    let (status, b) = f.vote(&u, "approve", 1).await;
    assert_eq!(status, 200, "{b}");
    let s = f.card().await;
    assert_eq!(s["type"], "recurring");
    assert_eq!(s["type_change_pending"], false);
    assert_eq!(s["approved_by_unit_id"], u["unit"]["id"]);
    let (db, scope) = repo_connection(&f.dsn, f.tenant).await;
    let row = repo::find_sku_fence(&db.conn().unwrap(), &scope, f.tenant, f.id)
        .await
        .unwrap()
        .unwrap();
    assert!(row.fenced_at.is_none());
    assert!(row.fence_op_id.is_none());
    let (_, reference) = f.reserve(Uuid::new_v4()).await;
    f.release(&reference["reservation_id"]).await;
    let path = format!("/skus/{}/retire", f.id);
    let first = call(
        &f.app,
        &f.author,
        Method::POST,
        &path,
        json!({}),
        Some("retire"),
    )
    .await;
    assert_eq!(first.0, 200);
    assert_eq!(
        call(
            &f.app,
            &f.author,
            Method::POST,
            &path,
            json!({}),
            Some("retire")
        )
        .await,
        first
    );
}

#[tokio::test]
async fn unchanged_type_with_live_reference_does_not_fence() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    assert_eq!(f.reserve(Uuid::new_v4()).await.0, 201);
    let (status, unit) = f
        .post("/changes", json!({"type":"usage","gl_code":"4012"}))
        .await;
    assert_eq!(status, 200, "{unit}");
    assert_eq!(f.card().await["type_change_pending"], false);
    assert_eq!(f.vote(&unit, "approve", 1).await.0, 200);
    assert_eq!(f.card().await["gl_code"], "4012");
}
#[tokio::test]
async fn reserve_receipt_uses_literal_reservation_id() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let (status, row) = f.reserve(Uuid::new_v4()).await;
    assert_eq!(status, 201);
    assert!(row.get("id").is_none(), "{row}");
    assert!(Uuid::parse_str(row["reservation_id"].as_str().unwrap()).is_ok());
}

struct PausedCatalog(Arc<tokio::sync::Notify>);
#[async_trait::async_trait]
impl bss_products_sdk::usage_types::UsageTypeCatalog for PausedCatalog {
    async fn resolve(
        &self,
        _: &SecurityContext,
        _: &str,
    ) -> crate::domain::recognized::UsageTypeAnswer {
        self.0.notify_one();
        std::future::pending().await
    }
    async fn list(
        &self,
        _: &SecurityContext,
        _: Option<&str>,
        _: Option<&str>,
        _: u32,
        _: Option<&str>,
    ) -> Result<
        bss_products_sdk::usage_types::UsageTypePage,
        toolkit::api::canonical_prelude::CanonicalError,
    > {
        Ok(bss_products_sdk::usage_types::UsageTypePage::default())
    }
}
#[tokio::test]
async fn cancelled_submission_does_not_strand_an_idempotency_claim() {
    let f = Fixture::new(0).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let paused = second_app(&f, Arc::new(PausedCatalog(entered.clone()))).await;
    let path = format!("/skus/{}/submit", f.id);
    let mut request = Box::pin(call(
        &paused,
        &f.author,
        Method::POST,
        &path,
        json!({}),
        Some("crash"),
    ));
    tokio::select! {
        () = entered.notified() => {},
        result = &mut request => panic!("request should be paused: {result:?}"),
    }
    drop(request);
    assert_eq!(idempotency_rows_for(&f.dsn, "crash").await, 0);
    let retry = call(
        &f.app,
        &f.author,
        Method::POST,
        &path,
        json!({}),
        Some("crash"),
    )
    .await;
    assert_eq!(retry.0, 200, "{retry:?}");
}
#[tokio::test]
async fn keyed_approve_replays_after_success() {
    let f = Fixture::new(1).await;
    let (_, unit) = f.post("/submit", json!({})).await;
    let path = format!(
        "/approval-units/{}/approve",
        unit["unit"]["id"].as_str().unwrap()
    );
    let body = json!({"generation":1});
    let first = call(
        &f.app,
        &f.reviewer,
        Method::POST,
        &path,
        body.clone(),
        Some("approve"),
    )
    .await;
    assert_eq!(first.0, 200, "{first:?}");
    assert_eq!(
        call(
            &f.app,
            &f.reviewer,
            Method::POST,
            &path,
            body,
            Some("approve")
        )
        .await,
        first
    );
    let conflict = call(
        &f.app,
        &f.reviewer,
        Method::POST,
        &path,
        json!({"generation":2}),
        Some("approve"),
    )
    .await;
    assert_eq!(problem_code(&conflict.1), "IDEMPOTENCY_CONFLICT");
}
#[tokio::test]
async fn keyed_reserve_and_confirm_replay_the_original_attempt_after_release() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let path = format!("/skus/{}/references/reserve", f.id);
    let body = json!({"owner":"pricing","kind":"price","ref_id":Uuid::new_v4()});
    let first = call(
        &f.app,
        &f.owner,
        Method::POST,
        &path,
        body.clone(),
        Some("attempt"),
    )
    .await;
    assert_eq!(first.0, 201);
    let confirm = format!(
        "/references/{}/confirm",
        first.1["reservation_id"].as_str().unwrap()
    );
    let confirmed = call(
        &f.app,
        &f.owner,
        Method::POST,
        &confirm,
        json!({}),
        Some("attempt"),
    )
    .await;
    assert_eq!(confirmed.0, 200);
    assert_eq!(f.release(&first.1["reservation_id"]).await.0, 200);
    assert_eq!(
        call(&f.app, &f.owner, Method::POST, &path, body, Some("attempt")).await,
        first
    );
    assert_eq!(
        call(
            &f.app,
            &f.owner,
            Method::POST,
            &confirm,
            json!({}),
            Some("attempt")
        )
        .await,
        confirmed
    );
    assert_eq!(
        raw_i64(&f.dsn, "SELECT count(*) AS v FROM products_sku_reference").await,
        1
    );
}
#[tokio::test]
async fn all_other_posts_replay_with_endpoint_scoped_keys() {
    let f = Fixture::new(1).await;
    for (path, body) in [
        ("/categories".to_owned(), json!({"code":"new","name":"New"})),
        (
            "/skus".to_owned(),
            json!({"code":"new","name":"New","type":"recurring","category_id":f.card().await["category_id"]}),
        ),
        (format!("/skus/{}/unfence", f.id), json!({})),
    ] {
        let before = idempotency_rows_for(&f.dsn, "shared").await;
        let first = call(
            &f.app,
            &f.author,
            Method::POST,
            &path,
            body.clone(),
            Some("shared"),
        )
        .await;
        assert!([200, 201].contains(&first.0), "{first:?}");
        assert_eq!(
            call(&f.app, &f.author, Method::POST, &path, body, Some("shared")).await,
            first,
            "{path}"
        );
        assert_eq!(idempotency_rows_for(&f.dsn, "shared").await, before + 1);
        if path == "/categories" {
            let retire = format!("/categories/{}/retire", first.1["id"].as_str().unwrap());
            let first = call(
                &f.app,
                &f.author,
                Method::POST,
                &retire,
                json!({}),
                Some("shared"),
            )
            .await;
            assert_eq!(first.0, 200);
            assert_eq!(
                call(
                    &f.app,
                    &f.author,
                    Method::POST,
                    &retire,
                    json!({}),
                    Some("shared")
                )
                .await,
                first
            );
        }
    }
    for action in ["reject", "withdraw"] {
        let (_, unit) = f.post("/submit", json!({})).await;
        let path = format!(
            "/approval-units/{}/{action}",
            unit["unit"]["id"].as_str().unwrap()
        );
        let actor = if action == "reject" {
            &f.reviewer
        } else {
            &f.author
        };
        let body = json!({"generation":1,"note":"reviewed"});
        let first = call(
            &f.app,
            actor,
            Method::POST,
            &path,
            body.clone(),
            Some("shared"),
        )
        .await;
        assert_eq!(first.0, 200, "{first:?}");
        assert_eq!(
            call(&f.app, actor, Method::POST, &path, body, Some("shared")).await,
            first
        );
    }
}

#[tokio::test]
async fn unit_only_reviewer_can_approve() {
    let f = Fixture::new(1).await;
    let (_, unit) = f.post("/submit", json!({})).await;
    let unit_id = Uuid::parse_str(unit["unit"]["id"].as_str().unwrap()).unwrap();
    let restricted = |action, id| {
        routes(f.state.clone(), &toolkit::api::OpenApiRegistryImpl::new()).layer(axum::Extension(
            authz_resolver_sdk::PolicyEnforcer::new(Arc::new(ActionResolver {
                id: Some(id),
                allowed: Some(action),
                tenant: f.tenant,
                seen: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            })),
        ))
    };
    let result = call(
        &restricted("approve", unit_id),
        &f.reviewer,
        Method::POST,
        &format!("/approval-units/{unit_id}/approve"),
        json!({"generation":1}),
        None,
    )
    .await;
    assert_eq!(result.0, 200, "{result:?}");
}

#[tokio::test]
async fn sku_only_reader_sees_reference_counts() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let restricted = |action, id| {
        routes(f.state.clone(), &toolkit::api::OpenApiRegistryImpl::new()).layer(axum::Extension(
            authz_resolver_sdk::PolicyEnforcer::new(Arc::new(ActionResolver {
                id: Some(id),
                allowed: Some(action),
                tenant: f.tenant,
                seen: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            })),
        ))
    };
    assert_eq!(f.reserve(Uuid::new_v4()).await.0, 201);
    let app = restricted("read", f.id);
    let (status, card) = call(
        &app,
        &f.author,
        Method::GET,
        &format!("/skus/{}", f.id),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(card["references"]["reserved"], 1, "{card}");
    let (status, refs) = call(
        &app,
        &f.author,
        Method::GET,
        &format!("/skus/{}/references", f.id),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(refs["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        call(
            &app,
            &f.author,
            Method::GET,
            &format!("/skus/{}", Uuid::new_v4()),
            json!({}),
            None
        )
        .await
        .0,
        404
    );
}

// Phase 2c: exercise the bound transport against the very same REST fixture.
fn local(f: &Fixture, owner: &str) -> crate::infra::reference_registry::LocalReferenceRegistry {
    crate::infra::reference_registry::LocalReferenceRegistry::for_owner(owner)
        .with_runtime(f.state.clone(), Arc::new(flat_in_enforcer(f.tenant)))
}
fn canonical_code(error: toolkit_canonical_errors::CanonicalError) -> String {
    let problem = toolkit::api::canonical_prelude::Problem::from(error);
    problem_code(&serde_json::to_value(problem).unwrap())
}
#[tokio::test]
async fn bound_registry_reserve_confirm_release_states_and_owner_isolation() {
    use bss_products_sdk::{ReferenceKind, ReferenceRegistryV1, ReferenceState};
    let f = Fixture::new(0).await;
    f.publish().await;
    let registry = local(&f, "pricing");
    let ref_id = Uuid::new_v4();
    let r = registry
        .reserve(&f.author, f.tenant, f.id, ReferenceKind::Price, ref_id)
        .await
        .unwrap();
    assert_eq!(r.state, ReferenceState::Reserved);
    assert_eq!(
        registry
            .reserve(&f.author, f.tenant, f.id, ReferenceKind::Price, ref_id)
            .await
            .unwrap(),
        r
    );
    registry
        .confirm(&f.author, f.tenant, r.reservation_id)
        .await
        .unwrap();
    registry
        .confirm(&f.author, f.tenant, r.reservation_id)
        .await
        .unwrap();
    let r2 = registry
        .reserve(
            &f.author,
            f.tenant,
            f.id,
            ReferenceKind::Price,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    registry
        .release(&f.author, f.tenant, r2.reservation_id)
        .await
        .unwrap();
    registry
        .release(&f.author, f.tenant, r2.reservation_id)
        .await
        .unwrap();
    assert_eq!(
        registry
            .states(&f.author, f.tenant, &[r2.reservation_id, r.reservation_id])
            .await
            .unwrap(),
        vec![
            (r2.reservation_id, ReferenceState::Released),
            (r.reservation_id, ReferenceState::Confirmed)
        ]
    );
    let foreign = local(&f, "subscriptions")
        .reserve(
            &f.author,
            f.tenant,
            f.id,
            ReferenceKind::PlanItem,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    for error in [
        registry
            .confirm(&f.author, f.tenant, foreign.reservation_id)
            .await
            .unwrap_err(),
        registry
            .release(&f.author, f.tenant, foreign.reservation_id)
            .await
            .unwrap_err(),
        registry
            .states(&f.author, f.tenant, &[foreign.reservation_id])
            .await
            .unwrap_err(),
    ] {
        assert_eq!(canonical_code(error), "REFERENCE_OWNER_MISMATCH");
    }
    assert_eq!(
        local(&f, "subscriptions")
            .states(&f.author, f.tenant, &[foreign.reservation_id])
            .await
            .unwrap()[0]
            .1,
        ReferenceState::Reserved
    );
}
#[tokio::test]
async fn bound_registry_refusals_match_rest_codes() {
    use bss_products_sdk::{ReferenceKind, ReferenceRegistryV1};
    let f = Fixture::new(0).await;
    let registry = local(&f, "pricing");
    let (_, rest) = f.reserve(Uuid::new_v4()).await;
    let error = registry
        .reserve(
            &f.author,
            f.tenant,
            f.id,
            ReferenceKind::Price,
            Uuid::new_v4(),
        )
        .await
        .unwrap_err();
    assert_eq!(canonical_code(error), problem_code(&rest));
    f.publish().await;
    let r = registry
        .reserve(
            &f.author,
            f.tenant,
            f.id,
            ReferenceKind::Price,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    registry
        .release(&f.author, f.tenant, r.reservation_id)
        .await
        .unwrap();
    let (_, rest) = call(
        &f.app,
        &f.owner,
        Method::POST,
        &format!("/references/{}/confirm", r.reservation_id),
        json!({}),
        None,
    )
    .await;
    assert_eq!(
        canonical_code(
            registry
                .confirm(&f.author, f.tenant, r.reservation_id)
                .await
                .unwrap_err()
        ),
        problem_code(&rest)
    );
    let missing = Uuid::new_v4();
    let (_, rest) = call(
        &f.app,
        &f.owner,
        Method::POST,
        &format!("/references/{missing}/confirm"),
        json!({}),
        None,
    )
    .await;
    let local = serde_json::to_value(toolkit::api::canonical_prelude::Problem::from(
        registry
            .confirm(&f.author, f.tenant, missing)
            .await
            .unwrap_err(),
    ))
    .unwrap();
    assert_eq!(local["type"], rest["type"]);
    assert_eq!(local["status"], 404);
    f.policy(1).await;
    assert_eq!(f.post("/retire", json!({})).await.0, 200);
    let (_, rest) = f.reserve(Uuid::new_v4()).await;
    assert_eq!(
        canonical_code(
            registry
                .reserve(
                    &f.author,
                    f.tenant,
                    f.id,
                    ReferenceKind::Price,
                    Uuid::new_v4()
                )
                .await
                .unwrap_err()
        ),
        problem_code(&rest)
    );
    assert_eq!(problem_code(&rest), "SKU_FENCED");
    // SKU_RETIRING is the existing domain refusal for an unfenced retiring head.
    assert_eq!(
        canonical_code(
            crate::domain::references::reservation_allowed(
                bss_products_sdk::Lifecycle::Retiring,
                false
            )
            .unwrap_err()
            .into()
        ),
        "SKU_RETIRING"
    );
}
#[tokio::test]
async fn bound_registry_fresh_head_and_dated_versions() {
    use bss_products_sdk::ReferenceRegistryV1;
    let f = Fixture::new(0).await;
    let registry = local(&f, "pricing");
    assert_eq!(
        registry
            .sku_for_write(&f.author, f.tenant, f.id)
            .await
            .unwrap()
            .lifecycle,
        bss_products_sdk::Lifecycle::Draft
    );
    let today = time::OffsetDateTime::now_utc().date();
    assert!(
        registry
            .sku_version_as_of(&f.author, f.tenant, f.id, today)
            .await
            .unwrap()
            .is_none()
    );
    f.publish().await;
    let date = today + time::Duration::days(7);
    assert_eq!(
        f.post(
            "/changes",
            json!({"gl_code":"new","effective_from":date.to_string()})
        )
        .await
        .0,
        200
    );
    for (day, version) in [
        (today - time::Duration::days(1), None),
        (date - time::Duration::days(1), Some(1)),
        (date, Some(2)),
        (date + time::Duration::days(1), Some(2)),
    ] {
        assert_eq!(
            registry
                .sku_version_as_of(&f.author, f.tenant, f.id, day)
                .await
                .unwrap()
                .map(|v| v.published_version),
            version
        );
    }
    assert_eq!(
        registry
            .sku_for_write(&f.author, f.tenant, f.id)
            .await
            .unwrap()
            .gl_code
            .as_deref(),
        Some("new")
    );
}
#[tokio::test]
async fn bound_registry_system_identity_and_tenant_are_checked() {
    use bss_products_sdk::{PRICING_SYSTEM_ACTOR, ReferenceKind, ReferenceRegistryV1};
    let f = Fixture::new(0).await;
    f.publish().await;
    let registry = local(&f, "pricing");
    let system = SecurityContext::builder()
        .subject_id(PRICING_SYSTEM_ACTOR)
        .subject_type("bss-pricing.system")
        .subject_tenant_id(f.tenant)
        .build()
        .unwrap();
    registry
        .reserve(
            &system,
            f.tenant,
            f.id,
            ReferenceKind::Price,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    let other = SecurityContext::builder()
        .subject_id(PRICING_SYSTEM_ACTOR)
        .subject_type("bss-products.system")
        .subject_tenant_id(f.tenant)
        .build()
        .unwrap();
    assert_eq!(
        canonical_code(
            registry
                .reserve(&other, f.tenant, f.id, ReferenceKind::Price, Uuid::new_v4())
                .await
                .unwrap_err()
        ),
        "REFERENCE_OWNER_MISMATCH"
    );
    assert!(
        registry
            .sku_for_write(&system, Uuid::new_v4(), f.id)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn bound_registry_unfenced_retiring_head_matches_rest_refusal() {
    use bss_products_sdk::{Lifecycle, ReferenceKind, ReferenceRegistryV1};
    let f = Fixture::new(0).await;
    f.publish().await;
    let db = f.state.db.db();
    let conn = db.conn().unwrap();
    let scope = toolkit_db::secure::AccessScope::for_tenant(f.tenant);
    repo::set_lifecycle(
        &conn,
        &scope,
        f.tenant,
        f.id,
        &[Lifecycle::Published],
        Lifecycle::Retiring,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    let (_, rest) = f.reserve(Uuid::new_v4()).await;
    assert_eq!(problem_code(&rest), "SKU_RETIRING");
    let error = local(&f, "pricing")
        .reserve(
            &f.author,
            f.tenant,
            f.id,
            ReferenceKind::Price,
            Uuid::new_v4(),
        )
        .await
        .unwrap_err();
    assert_eq!(canonical_code(error), problem_code(&rest));
}

/// One pricing request through the real pricing router, as the cross-gear test sends it.
async fn price_call(
    app: &Router,
    ctx: &SecurityContext,
    method: Method,
    path: &str,
    body: Value,
) -> (u16, Value) {
    let req = Request::builder()
        .method(method)
        .uri(format!("/bss-pricing/v1{path}"))
        .extension(ctx.clone())
        .header("content-type", "application/json")
        .header("idempotency-key", "cross-gear")
        .body(Body::from(body.to_string()))
        .unwrap();
    let response = app.clone().oneshot(req).await.unwrap();
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}
#[tokio::test]
async fn real_pricing_price_blocks_retirement_until_delete_and_ticker_pass() {
    use toolkit::contracts::DatabaseCapability;
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    let dsn = format!(
        "sqlite://{}?mode=rwc",
        std::env::temp_dir()
            .join(format!("pricing-cross-gear-{}.sqlite3", Uuid::new_v4()))
            .display()
    );
    let db = toolkit_db::connect_db(
        &dsn,
        toolkit_db::ConnectOpts {
            max_conns: Some(1),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    toolkit_db::migration_runner::run_migrations_for_testing(
        &db,
        bss_pricing::module::BssPricingGear::default().migrations(),
    )
    .await
    .unwrap();
    let hub = Arc::new(toolkit::ClientHub::default());
    let registry = crate::infra::reference_registry::LocalReferenceRegistry::for_owner("pricing")
        .with_runtime(f.state.clone(), Arc::new(flat_in_enforcer(f.tenant)));
    hub.register::<bss_products_sdk::PricingReferenceRegistry>(Arc::new(
        bss_products_sdk::PricingReferenceRegistry(Arc::new(registry)),
    ));
    let state = Arc::new(
        bss_pricing::api::rest::authoring::AuthoringState::new(
            toolkit_db::DBProvider::new(db),
            hub,
        )
        .await
        .unwrap(),
    );
    let pricing = bss_pricing::api::rest::authoring::router(
        state.clone(),
        &toolkit::api::OpenApiRegistryImpl::new(),
    )
    .layer(axum::Extension(flat_in_enforcer(f.tenant)));
    let (status, book) = price_call(
        &pricing,
        &f.author,
        Method::POST,
        "/price-books",
        json!({"code":"standard","name":"Standard","currency":"EUR"}),
    )
    .await;
    assert_eq!(status, 201, "{book}");
    let (status, price) = price_call(
        &pricing,
        &f.author,
        Method::POST,
        &format!("/price-books/{}/prices", book["id"].as_str().unwrap()),
        json!({"sku_id":f.id}),
    )
    .await;
    assert_eq!(status, 201, "{price}");
    assert_eq!(price["reference_state"], "confirmed");
    let (status, references) = call(
        &f.app,
        &f.author,
        Method::GET,
        &format!("/skus/{}/references", f.id),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200, "{references}");
    assert!(
        references
            .to_string()
            .contains(price["reservation_id"].as_str().unwrap())
    );
    let (status, refusal) = f.post("/retire", json!({})).await;
    assert_eq!(status, 409);
    assert_eq!(problem_code(&refusal), "SKU_REFERENCED");
    let (status, _) = price_call(
        &pricing,
        &f.author,
        Method::DELETE,
        &format!("/prices/{}", price["id"].as_str().unwrap()),
        json!({}),
    )
    .await;
    assert_eq!(status, 204);
    bss_pricing::infra::reference_ticker::Ticker::new(
        state,
        Arc::new(bss_pricing::infra::reference_work::WallClock),
        100,
        1,
    )
    .tick()
    .await
    .unwrap();
    let (status, unit) = f.post("/retire", json!({})).await;
    assert_eq!(status, 200, "{unit}");
    assert_eq!(f.vote(&unit, "approve", 1).await.0, 200);
    assert_eq!(f.card().await["lifecycle"], "retired");
}
