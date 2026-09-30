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
        .merge(crate::api::rest::references::router(s.clone(), o))
        .merge(crate::api::rest::browse::router(s.clone(), o))
        .merge(crate::api::rest::usage_types::router(s, o))
}
struct Fixture {
    state: Arc<crate::api::rest::ApiState>,
    app: Router,
    /// Held for the test's life: its temporary directory holds the database.
    dsn: TestDsn,
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
/// [`call`] with arbitrary request headers, answering the response headers too; an empty body
/// (a 204) reads as `null`.
async fn call_with(
    app: &Router,
    ctx: &SecurityContext,
    method: Method,
    path: &str,
    body: Value,
    headers: &[(&str, String)],
) -> (u16, axum::http::HeaderMap, Value) {
    let mut request = Request::builder()
        .method(method)
        .uri(format!("/bss-products/v1{path}"))
        .extension(ctx.clone())
        .header("Content-Type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, value);
    }
    let r = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = r.status().as_u16();
    let response_headers = r.headers().clone();
    let bytes = axum::body::to_bytes(r.into_body(), usize::MAX)
        .await
        .unwrap();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, response_headers, body)
}
impl Fixture {
    async fn new(quorum: u32) -> Self {
        let (db, _, _, dsn) = test_db().await;
        Self::on(quorum, db, dsn).await
    }
    /// [`Fixture::new`] over a database whose statements `recorder` records; the recorder is
    /// cleared after the fixture's own writes.
    async fn recorded(quorum: u32) -> (Self, toolkit_db::test_support::QueryRecorder) {
        let (db, _, _, dsn, recorder) = recorded_test_db().await;
        let f = Self::on(quorum, db, dsn).await;
        recorder.clear();
        (f, recorder)
    }
    async fn on(
        quorum: u32,
        db: toolkit_db::DBProvider<toolkit_db::DbError>,
        dsn: TestDsn,
    ) -> Self {
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
    /// P-D-205: the policy is written at the tag its read answered.
    async fn policy(&self, quorum: u32) {
        let (status, headers, b) = call_with(
            &self.app,
            &self.author,
            Method::GET,
            "/approval-policy",
            json!({}),
            &[],
        )
        .await;
        assert_eq!(status, 200, "{b}");
        let tag = headers["etag"].to_str().unwrap().to_owned();
        let (status, _, b) = call_with(
            &self.app,
            &self.author,
            Method::PUT,
            "/approval-policy",
            json!({"quorum":quorum}),
            &[("If-Match", tag)],
        )
        .await;
        assert_eq!(status, 200, "{b}");
    }
    /// The SKU's current `ETag`, as its card answers it.
    async fn etag(&self) -> String {
        let (status, headers, b) = call_with(
            &self.app,
            &self.author,
            Method::GET,
            &format!("/skus/{}", self.id),
            json!({}),
            &[],
        )
        .await;
        assert_eq!(status, 200, "{b}");
        headers["etag"].to_str().unwrap().to_owned()
    }
    /// `DELETE /skus/{id}` as `ctx`, with an optional `If-Match`.
    async fn delete(&self, ctx: &SecurityContext, tag: Option<&str>) -> (u16, Value) {
        let headers: Vec<(&str, String)> = tag
            .map(|t| vec![("If-Match", t.to_owned())])
            .unwrap_or_default();
        let (status, _, b) = call_with(
            &self.app,
            ctx,
            Method::DELETE,
            &format!("/skus/{}", self.id),
            json!({}),
            &headers,
        )
        .await;
        (status, b)
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
    /// A second recurring draft of the tenant, by the fixture's author; its id.
    async fn draft(&self, code: &str) -> Uuid {
        let (status, s) = call(
            &self.app,
            &self.author,
            Method::POST,
            "/skus",
            json!({"code":code,"name":code,"type":"recurring"}),
            None,
        )
        .await;
        assert_eq!(status, 201, "{s}");
        Uuid::parse_str(s["id"].as_str().unwrap()).unwrap()
    }
    /// Submit the draft `id` for publication; the unit's id.
    async fn submit(&self, id: Uuid) -> String {
        let (status, u) = call(
            &self.app,
            &self.author,
            Method::POST,
            &format!("/skus/{id}/submit"),
            json!({}),
            None,
        )
        .await;
        assert_eq!(status, 200, "{u}");
        u["unit"]["id"].as_str().unwrap().to_owned()
    }
    /// `GET /approval-units` with `query`, as the reviewer.
    async fn units(&self, query: &str) -> (u16, Value) {
        call(
            &self.app,
            &self.reviewer,
            Method::GET,
            &format!("/approval-units{query}"),
            json!({}),
            None,
        )
        .await
    }
    /// Every unit the list answers under `narrowing` (`&`-joined, may be empty), following
    /// `page_info.next_cursor` to the last page (P-D-224).
    async fn all_units(&self, narrowing: &str) -> Vec<Value> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let mut query = narrowing.to_owned();
            if let Some(c) = &cursor {
                if !query.is_empty() {
                    query.push('&');
                }
                query.push_str("cursor=");
                query.push_str(c);
            }
            let (status, page) = self
                .units(&if query.is_empty() {
                    String::new()
                } else {
                    format!("?{query}")
                })
                .await;
            assert_eq!(status, 200, "{page}");
            items.extend(page["items"].as_array().unwrap().iter().cloned());
            match page["page_info"]["next_cursor"].as_str() {
                Some(next) => cursor = Some(next.to_owned()),
                None => return items,
            }
        }
    }
    async fn reserve(&self, ref_id: Uuid) -> (u16, Value) {
        call(
            &self.app,
            &self.owner,
            Method::POST,
            &format!("/skus/{}/references/reserve", self.id),
            json!({"owner":"pricing","kind":"price_book_entry","ref_id":ref_id}),
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
    assert_eq!(f.all_units("state=approved").await.len(), 1);
    for (reference, count) in [(f.id, 1), (Uuid::new_v4(), 0)] {
        let units = f
            .all_units(&format!(
                "state=approved&kind=sku_publish&ref_id={reference}"
            ))
            .await;
        assert_eq!(units.len(), count);
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
            &format!("/skus/{}/versions/as-of?date={as_of}", f.id),
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
    let queue = f.all_units(&format!("ref_id={}", f.id)).await;
    let pending = queue
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
    // RT-02: each half of the guard refuses on its own: no force, a force with no reason, and a
    // force with a blank reason; none releases.
    for body in [
        json!({}),
        json!({"force":true}),
        json!({"force":true,"reason":"  "}),
    ] {
        let (status, b) = call(&f.app, &f.author, Method::DELETE, &path, body.clone(), None).await;
        assert_eq!(status, 400, "{body}: {b}");
    }
    let (_, refs) = call(
        &f.app,
        &f.author,
        Method::GET,
        &format!("/skus/{}/references", f.id),
        json!({}),
        None,
    )
    .await;
    assert!(
        refs.to_string().contains("\"reserved\""),
        "still live: {refs}"
    );
    assert_eq!(
        enqueued_event_count(&f.dsn, ReferenceForceReleased::TYPE_ID).await,
        0
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
        hub: f.state.hub.clone(),
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
            json!({"owner":"pricing","kind":"price_book_entry","ref_id":Uuid::new_v4()}),
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
            json!({"owner":"other","kind":"price_book_entry","ref_id":Uuid::new_v4()}),
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
/// The action each served operation's door asks the PDP, by `operationId`. The release of a
/// reference is `submit` for a caller that owns no reference (the operator route).
const DOOR_ACTIONS: &[(&str, &str)] = &[
    ("bss_products.create_category", "author"),
    ("bss_products.list_categories", "read"),
    ("bss_products.get_category", "read"),
    ("bss_products.update_category", "author"),
    ("bss_products.retire_category", "author"),
    ("bss_products.create_sku", "author"),
    ("bss_products.list_skus", "read"),
    ("bss_products.count_skus", "read"),
    ("bss_products.get_sku", "read"),
    ("bss_products.update_sku_draft", "author"),
    ("bss_products.delete_sku_draft", "author"),
    ("bss_products.sku_versions", "read"),
    ("bss_products.sku_version_as_of", "read"),
    ("bss_products.sku_references", "read"),
    ("bss_products.sku_history", "read"),
    ("bss_products.submit_sku", "submit"),
    ("bss_products.change_sku", "submit"),
    ("bss_products.retire_sku", "submit"),
    ("bss_products.unfence_sku", "submit"),
    ("bss_products.list_approval_units", "read"),
    ("bss_products.get_approval_unit", "read"),
    ("bss_products.approve_unit", "approve"),
    ("bss_products.reject_unit", "approve"),
    ("bss_products.withdraw_unit", "submit"),
    ("bss_products.get_approval_policy", "settings"),
    ("bss_products.put_approval_policy", "settings"),
    ("bss_products.delete_approval_policy_override", "settings"),
    ("bss_products.reserve_reference", "reference"),
    ("bss_products.confirm_reference", "reference"),
    ("bss_products.release_reference", "submit"),
    ("bss_products.browse", "read"),
    ("bss_products.list_usage_types", "author"),
];

/// Every operation `routes` serves, from the `OpenAPI` it registers, as the method, the concrete
/// path this suite calls and the action its door asks (RT-01): an operation without a row in
/// [`DOOR_ACTIONS`], or a row that names no served operation, fails here. A SKU path names the
/// fixture's SKU; any other id is fresh.
fn served_doors(f: &Fixture) -> Vec<(Method, String, &'static str)> {
    let openapi = toolkit::api::OpenApiRegistryImpl::new();
    // Registering the routes fills the registry; the router itself is not served here.
    drop(routes(f.state.clone(), &openapi));
    let api = serde_json::to_value(
        openapi
            .build_openapi(&toolkit::api::OpenApiInfo::default())
            .unwrap(),
    )
    .unwrap();
    let table: std::collections::BTreeMap<&str, &'static str> =
        DOOR_ACTIONS.iter().copied().collect();
    let mut served = std::collections::BTreeSet::new();
    let mut doors = Vec::new();
    for (template, item) in api["paths"].as_object().unwrap() {
        for (method, operation) in item.as_object().unwrap() {
            let Some(id) = operation["operationId"].as_str() else {
                continue;
            };
            let action = *table
                .get(id)
                .unwrap_or_else(|| panic!("{id} is served and has no row in DOOR_ACTIONS"));
            served.insert(id.to_owned());
            let path = template
                .strip_prefix("/bss-products/v1")
                .unwrap()
                .replace("/skus/{id}", &format!("/skus/{}", f.id))
                .replace("{id}", &Uuid::new_v4().to_string())
                .replace("{kind}", "sku_publish");
            let path = match id {
                "bss_products.sku_version_as_of" => format!("{path}?date=2026-09-27"),
                "bss_products.browse" => format!("{path}?kind=sku"),
                _ => path,
            };
            doors.push((
                Method::from_bytes(method.to_uppercase().as_bytes()).unwrap(),
                path,
                action,
            ));
        }
    }
    let rows: std::collections::BTreeSet<String> =
        table.keys().map(|id| (*id).to_owned()).collect();
    assert_eq!(
        served, rows,
        "a row of DOOR_ACTIONS names no served operation"
    );
    doors
}

/// The position [`ActionResolver`] records for `action`.
fn action_seen(action: &str) -> usize {
    crate::authz::actions::ALL
        .iter()
        .position(|a| *a == action)
        .unwrap()
        + 1
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
    for (method, path, action) in served_doors(&f) {
        seen.store(0, std::sync::atomic::Ordering::Relaxed);
        let (status, b) = call(&app, &f.author, method.clone(), &path, json!({}), None).await;
        assert_eq!(status, 403, "{method} {path}: {b}");
        assert_eq!(
            seen.load(std::sync::atomic::Ordering::Relaxed),
            action_seen(action),
            "{method} {path} asks {action}"
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
/// O1 (P-D-222): a REST caller whose context asserts the pricing system actor (the subject type
/// `bss-pricing.system` and the id `PRICING_SYSTEM_ACTOR`, which a token's claims can carry) is
/// judged by the PDP at every served door, as any caller is. Under a PDP that allows nothing,
/// every door answers 403 after asking its own action: no door honours the registry's in-process
/// trust of that actor.
#[tokio::test]
async fn a_rest_caller_asserting_the_pricing_system_actor_gets_no_bypass() {
    let f = Fixture::new(1).await;
    let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app =
        routes(f.state.clone(), &toolkit::api::OpenApiRegistryImpl::new()).layer(axum::Extension(
            authz_resolver_sdk::PolicyEnforcer::new(Arc::new(ActionResolver {
                id: None,
                allowed: None,
                tenant: f.tenant,
                seen: seen.clone(),
            })),
        ));
    let asserted = SecurityContext::builder()
        .subject_id(bss_products_sdk::PRICING_SYSTEM_ACTOR)
        .subject_tenant_id(f.tenant)
        .subject_type("bss-pricing.system")
        .token_scopes(vec!["*".into()])
        .build()
        .unwrap();
    let doors = served_doors(&f);
    assert_eq!(doors.len(), DOOR_ACTIONS.len());
    for (method, path, action) in doors {
        seen.store(0, std::sync::atomic::Ordering::Relaxed);
        let (status, b) = call(&app, &asserted, method.clone(), &path, json!({}), None).await;
        assert_eq!(status, 403, "{method} {path}: {b}");
        assert_eq!(
            seen.load(std::sync::atomic::Ordering::Relaxed),
            action_seen(action),
            "{method} {path}: the PDP judged {action}"
        );
    }
}
/// O1 (P-D-222): a REST door never records the registry's trust either. A reserve through the
/// door by a principal whose token asserts a `.system` subject type is audited as a subject's
/// act; only the in-process registry records the pricing system actor's act as the system's.
#[tokio::test]
async fn a_rest_reservation_is_a_subjects_act_whatever_the_token_asserts() {
    use bss_products_sdk::{PRICING_SYSTEM_ACTOR, ReferenceKind, ReferenceRegistryV1};
    let f = Fixture::new(0).await;
    f.publish().await;
    let asserting = SecurityContext::builder()
        .subject_id(Uuid::from_u128(42))
        .subject_tenant_id(f.tenant)
        .subject_type("bss-pricing.system")
        .token_scopes(vec!["*".into()])
        .build()
        .unwrap();
    let (status, b) = call(
        &f.app,
        &asserting,
        Method::POST,
        &format!("/skus/{}/references/reserve", f.id),
        json!({"owner":"pricing","kind":"price_book_entry","ref_id":Uuid::new_v4()}),
        None,
    )
    .await;
    assert_eq!(status, 201, "{b}");
    let system = SecurityContext::builder()
        .subject_id(PRICING_SYSTEM_ACTOR)
        .subject_type("bss-pricing.system")
        .subject_tenant_id(f.tenant)
        .build()
        .unwrap();
    local(&f, "pricing")
        .reserve(
            &system,
            f.tenant,
            f.id,
            ReferenceKind::PriceBookEntry,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    for (actor, kind) in [
        (Uuid::from_u128(42), "subject"),
        (PRICING_SYSTEM_ACTOR, "system"),
    ] {
        let reason = raw_string_opt(
            &f.dsn,
            &format!(
                "SELECT reason AS v FROM products_audit_log WHERE action = 'reference.reserve' \
                 AND {}",
                id_matches("actor_ref", actor)
            ),
        )
        .await;
        assert_eq!(
            reason.as_deref(),
            Some(format!("owner=pricing; actor_kind={kind}").as_str())
        );
    }
}
/// RS-03 (O2, P-D-224): the unit list pages in submission order, `limit` 200 by default and
/// clamped at 500, `cursor` from `page_info`. A cursor replayed under another narrowing is 400
/// `FILTER_MISMATCH`, a cursor that does not read and a `limit` that is not a number 400.
#[tokio::test]
async fn the_unit_list_pages_in_submission_order() {
    let f = Fixture::new(2).await;
    let mut units = vec![f.submit(f.id).await];
    for code in ["P1", "P2"] {
        let id = f.draft(code).await;
        units.push(f.submit(id).await);
    }
    let (status, first) = f.units("?limit=2").await;
    assert_eq!(status, 200, "{first}");
    let ids = |page: &Value| -> Vec<String> {
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["id"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(ids(&first), units[..2]);
    assert_eq!(first["page_info"]["limit"], 2);
    let next = first["page_info"]["next_cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let (status, second) = f.units(&format!("?limit=2&cursor={next}")).await;
    assert_eq!(status, 200, "{second}");
    assert_eq!(ids(&second), units[2..]);
    assert!(second["page_info"]["next_cursor"].is_null(), "{second}");
    assert_eq!(f.units("").await.1["page_info"]["limit"], 200);
    assert_eq!(f.units("?limit=9999").await.1["page_info"]["limit"], 500);
    let (status, b) = f.units(&format!("?state=pending&cursor={next}")).await;
    assert_eq!(status, 400, "{b}");
    assert!(b.to_string().contains("FILTER_MISMATCH"), "{b}");
    for bad in ["?cursor=not-a-cursor", "?limit=many"] {
        assert_eq!(f.units(bad).await.0, 400, "{bad}");
    }
    let every: Vec<String> = f
        .all_units("limit=1")
        .await
        .iter()
        .map(|u| u["id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(every, units);
}

/// RS-03 (P-D-224): a page of the unit list reads its units and all their decisions in the same
/// statements for 10 and for 100 units, each with a vote; it read each unit's decisions on its own.
#[tokio::test]
async fn the_unit_list_reads_a_page_in_the_same_statements_for_10_and_100_units() {
    let mut runs = Vec::new();
    for n in [10, 100] {
        let (f, recorder) = Fixture::recorded(2).await;
        for i in 0..n {
            let id = if i == 0 {
                f.id
            } else {
                f.draft(&format!("U{i:03}")).await
            };
            let unit = f.submit(id).await;
            let (status, b) = call(
                &f.app,
                &f.reviewer,
                Method::POST,
                &format!("/approval-units/{unit}/approve"),
                json!({"generation":1}),
                None,
            )
            .await;
            assert_eq!(status, 200, "{b}");
        }
        recorder.clear();
        let (status, page) = f.units("").await;
        assert_eq!(status, 200, "{page}");
        let items = page["items"].as_array().unwrap();
        assert_eq!(items.len(), n);
        assert!(
            items
                .iter()
                .all(|u| u["decisions"].as_array().unwrap().len() == 1),
            "every unit carries its vote"
        );
        runs.push(products_statements(&recorder));
    }
    assert_eq!(runs[0], runs[1]);
}
/// RS-23 (P-D-226): the SDK's `Sku` and `SkuVersion` read the doors' JSON (instants RFC 3339,
/// the effective date `YYYY-MM-DD`, as `SkuDto` and `SkuVersionDto` write them) and write it back
/// unchanged.
#[tokio::test]
async fn the_sdk_sku_types_read_the_doors_json() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let card = f.card().await;
    let sku: bss_products_sdk::models::Sku = serde_json::from_value(card.clone()).unwrap();
    assert_eq!(sku.id, f.id);
    assert_eq!(serde_json::to_value(&sku).unwrap(), card);
    let (status, versions) = call(
        &f.app,
        &f.author,
        Method::GET,
        &format!("/skus/{}/versions", f.id),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200, "{versions}");
    let read: Vec<bss_products_sdk::models::SkuVersion> =
        serde_json::from_value(versions.clone()).unwrap();
    assert_eq!(read.len(), 1);
    assert_eq!(serde_json::to_value(&read).unwrap(), versions);
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
        "/skus?%24filter=lifecycle%20eq%20%27published%27",
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
        &format!("/skus/{}/versions/as-of?date={date}", f.id),
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
    let body = json!({"owner":"pricing","kind":"price_book_entry","ref_id":Uuid::new_v4()});
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
    // RT-03: a second unit of the tenant, outside the grant's RESOURCE_ID, is refused.
    let other = f.submit(f.draft("OTHER").await).await;
    let refused = call(
        &restricted("approve", unit_id),
        &f.reviewer,
        Method::POST,
        &format!("/approval-units/{other}/approve"),
        json!({"generation":1}),
        None,
    )
    .await;
    assert_eq!(refused.0, 404, "{refused:?}");
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
    // RT-03: another SKU of the tenant, outside the grant's RESOURCE_ID, is not read; a fresh id
    // would be 404 whether the constraint applied or not.
    let outside = f.draft("OUTSIDE").await;
    let unrestricted = call(
        &f.app,
        &f.author,
        Method::GET,
        &format!("/skus/{outside}"),
        json!({}),
        None,
    )
    .await;
    assert_eq!(unrestricted.0, 200, "{unrestricted:?}");
    assert_eq!(
        call(
            &app,
            &f.author,
            Method::GET,
            &format!("/skus/{outside}"),
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
        .reserve(
            &f.author,
            f.tenant,
            f.id,
            ReferenceKind::PriceBookEntry,
            ref_id,
        )
        .await
        .unwrap();
    assert_eq!(r.state, ReferenceState::Reserved);
    assert_eq!(
        registry
            .reserve(
                &f.author,
                f.tenant,
                f.id,
                ReferenceKind::PriceBookEntry,
                ref_id
            )
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
            ReferenceKind::PriceBookEntry,
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
/// RS-04 / RS-05: a reservation whose insert loses the live-reference index on both attempts is
/// 409 `REFERENCE_EXISTS`, at the door and through the registry, never a 500. The loss is made
/// lasting by an extra unique index over the logical reference with no `state` condition: the
/// released attempt holds it, and the live read each attempt makes never sees a released row.
#[tokio::test]
async fn a_reservation_that_loses_the_index_twice_is_409_reference_exists() {
    use bss_products_sdk::{ReferenceKind, ReferenceRegistryV1};
    let f = Fixture::new(0).await;
    f.publish().await;
    let ref_id = Uuid::new_v4();
    let (status, r) = f.reserve(ref_id).await;
    assert_eq!(status, 201, "{r}");
    let (status, b) = f.release(&r["reservation_id"]).await;
    assert_eq!(status, 200, "{b}");
    {
        use sea_orm::{ConnectionTrait, Database};
        let raw = Database::connect(&f.dsn).await.unwrap();
        raw.execute_unprepared(
            "CREATE UNIQUE INDEX every_attempt_of_a_reference ON products_sku_reference \
             (tenant_id, owner_gear, ref_kind, ref_id)",
        )
        .await
        .unwrap();
        raw.close().await.ok();
    }
    let (status, b) = f.reserve(ref_id).await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "REFERENCE_EXISTS");
    let error = local(&f, "pricing")
        .reserve(
            &f.author,
            f.tenant,
            f.id,
            ReferenceKind::PriceBookEntry,
            ref_id,
        )
        .await
        .unwrap_err();
    assert_eq!(canonical_code(error), "REFERENCE_EXISTS");
}
/// RS-13: the registry answers the states of a batch of reservations in the same statements for
/// 10 and for 100 ids, one read with the owner checked per row; it read each id on its own. An
/// unknown id is still the batch's 404.
#[tokio::test]
async fn registry_states_read_in_the_same_statements_for_10_and_100_ids() {
    use bss_products_sdk::{ReferenceKind, ReferenceRegistryV1, ReferenceState};
    let mut runs = Vec::new();
    for n in [10, 100] {
        let (f, recorder) = Fixture::recorded(0).await;
        f.publish().await;
        let registry = local(&f, "pricing");
        let mut ids = Vec::with_capacity(n);
        for _ in 0..n {
            ids.push(
                registry
                    .reserve(
                        &f.author,
                        f.tenant,
                        f.id,
                        ReferenceKind::PriceBookEntry,
                        Uuid::new_v4(),
                    )
                    .await
                    .unwrap()
                    .reservation_id,
            );
        }
        recorder.clear();
        let states = registry.states(&f.author, f.tenant, &ids).await.unwrap();
        assert_eq!(
            states,
            ids.iter()
                .map(|id| (*id, ReferenceState::Reserved))
                .collect::<Vec<_>>()
        );
        runs.push(products_statements(&recorder));
        let mut with_unknown = ids.clone();
        with_unknown.insert(1, Uuid::new_v4());
        assert_eq!(
            registry
                .states(&f.author, f.tenant, &with_unknown)
                .await
                .unwrap_err()
                .status_code(),
            404
        );
    }
    assert_eq!(runs[0], runs[1]);
}

/// RS-15: a SKU card counts its live references in one grouped read; it loaded every live row.
/// The counts it answers are unchanged: by owner and kind, and the reserved subset.
#[tokio::test]
async fn a_sku_card_counts_its_references_in_one_grouped_read() {
    let (f, recorder) = Fixture::recorded(0).await;
    f.publish().await;
    for _ in 0..3 {
        assert_eq!(f.reserve(Uuid::new_v4()).await.0, 201);
    }
    recorder.clear();
    let (status, card) = call(
        &f.app,
        &f.author,
        Method::GET,
        &format!("/skus/{}", f.id),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(card["references"]["price_book_entries"], 3, "{card}");
    assert_eq!(card["references"]["reserved"], 3, "{card}");
    let reads: Vec<String> = products_statements(&recorder)
        .into_iter()
        .filter(|sql| sql.contains("products_sku_reference"))
        .collect();
    assert_eq!(reads.len(), 1, "{reads:?}");
    assert!(
        reads[0].contains("GROUP BY") && reads[0].contains("COUNT"),
        "{}",
        reads[0]
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
            ReferenceKind::PriceBookEntry,
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
            ReferenceKind::PriceBookEntry,
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
                    ReferenceKind::PriceBookEntry,
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
            ReferenceKind::PriceBookEntry,
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
                .reserve(
                    &other,
                    f.tenant,
                    f.id,
                    ReferenceKind::PriceBookEntry,
                    Uuid::new_v4()
                )
                .await
                .unwrap_err()
        ),
        "REFERENCE_OWNER_MISMATCH"
    );
    // RT-04: the tenant check's own code, not a data-scope 404 another tenant's scope would give.
    assert_eq!(
        canonical_code(
            registry
                .sku_for_write(&system, Uuid::new_v4(), f.id)
                .await
                .unwrap_err()
        ),
        "REFERENCE_OWNER_MISMATCH"
    );
}

#[tokio::test]
async fn bound_registry_serves_a_principal_without_a_subject_type() {
    // The static-authn default identity and OIDC third-party tokens carry no subject type;
    // neither pricing's nor products' REST door requires one, so the registry must not either.
    // Such a principal is a human caller and goes through the PDP like any other.
    use bss_products_sdk::{ReferenceKind, ReferenceRegistryV1, ReferenceState};
    let f = Fixture::new(0).await;
    f.publish().await;
    let registry = local(&f, "pricing");
    let untyped = SecurityContext::builder()
        .subject_id(Uuid::now_v7())
        .subject_tenant_id(f.tenant)
        .token_scopes(vec!["*".into()])
        .build()
        .unwrap();
    assert!(untyped.subject_type().is_none());
    let receipt = registry
        .reserve(
            &untyped,
            f.tenant,
            f.id,
            ReferenceKind::PriceBookEntry,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    assert_eq!(receipt.state, ReferenceState::Reserved);
    registry
        .release(&untyped, f.tenant, receipt.reservation_id)
        .await
        .unwrap();
    assert_eq!(
        registry
            .sku_for_write(&untyped, f.tenant, f.id)
            .await
            .unwrap()
            .id,
        f.id
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
            ReferenceKind::PriceBookEntry,
            Uuid::new_v4(),
        )
        .await
        .unwrap_err();
    assert_eq!(canonical_code(error), problem_code(&rest));
}

/// One pricing request through the real pricing router, as the cross-gear test sends it.
async fn pricing_call(
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
async fn real_pricing_entry_blocks_retirement_until_delete_and_ticker_pass() {
    use toolkit::contracts::DatabaseCapability;
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;
    // Held for the test's life: pricing's database in its own temporary directory.
    let dsn = TestDsn::new("pricing-cross-gear-");
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
    let (status, book) = pricing_call(
        &pricing,
        &f.author,
        Method::POST,
        "/price-books",
        json!({"code":"standard","name":"Standard","currency":"EUR"}),
    )
    .await;
    assert_eq!(status, 201, "{book}");
    let (status, entry) = pricing_call(
        &pricing,
        &f.author,
        Method::POST,
        &format!("/price-books/{}/entries", book["id"].as_str().unwrap()),
        // Pricing D-427: an entry is created in a model; `per_unit` is one every charge kind allows.
        json!({"sku_id":f.id,"model":"per_unit"}),
    )
    .await;
    assert_eq!(status, 201, "{entry}");
    assert_eq!(entry["reference_state"], "confirmed");
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
            .contains(entry["reservation_id"].as_str().unwrap())
    );
    let (status, refusal) = f.post("/retire", json!({})).await;
    assert_eq!(status, 409);
    assert_eq!(problem_code(&refusal), "SKU_REFERENCED");
    let (status, _) = pricing_call(
        &pricing,
        &f.author,
        Method::DELETE,
        &format!("/price-book-entries/{}", entry["id"].as_str().unwrap()),
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

/// P-D-196: a SKU without a category publishes, and a `sku_change` sets and then clears the
/// category: the unit's item diff, the applied version and `SkuChanged.changed` show it.
#[tokio::test]
async fn a_sku_without_a_category_publishes_and_a_change_sets_and_clears_it() {
    let f = Fixture::new(0).await;
    let cat = f.card().await["category_id"].clone();
    assert!(cat.is_string(), "the fixture's draft has a category");
    let r = request_as(
        &f.app,
        &f.author,
        Method::PATCH,
        &format!("/bss-products/v1/skus/{}", f.id),
        Some(json!({"category_id":null})),
        Some("\"1\""),
    )
    .await;
    assert_eq!(r.status(), 200);
    assert_eq!(f.card().await["category_id"], Value::Null);
    f.publish().await;
    let today = time::OffsetDateTime::now_utc().date();
    let in_force = || async {
        let (status, body) = call(
            &f.app,
            &f.author,
            Method::GET,
            &format!("/skus/{}/versions/as-of?date={today}", f.id),
            json!({}),
            None,
        )
        .await;
        assert_eq!(status, 200, "{body}");
        body
    };
    let v = in_force().await;
    assert_eq!(v["published_version"], 1);
    assert_eq!(v["content"]["category_id"], Value::Null);
    for (version, before, after) in [(2, Value::Null, cat.clone()), (3, cat.clone(), Value::Null)] {
        let (status, u) = f.post("/changes", json!({"category_id":after})).await;
        assert_eq!(status, 200, "{u}");
        assert_eq!(u["applied"], true);
        let item = &u["unit"]["snapshot"]["skus"][0];
        assert_eq!(item["before"]["content"]["category_id"], before, "{item}");
        assert_eq!(item["after"]["content"]["category_id"], after, "{item}");
        assert_eq!(f.card().await["category_id"], after);
        let v = in_force().await;
        assert_eq!(v["published_version"], version);
        assert_eq!(v["content"]["category_id"], after);
        assert_eq!(
            enqueued_event_envelope(&f.dsn, SkuChanged::TYPE_ID).await["changed"],
            json!(["category_id"])
        );
    }
}

/// An active category of the fixture's tenant, created through the door; its id.
async fn new_category(f: &Fixture, code: &str) -> Value {
    let (status, c) = call(
        &f.app,
        &f.author,
        Method::POST,
        "/categories",
        json!({"code":code,"name":code}),
        None,
    )
    .await;
    assert_eq!(status, 201, "{c}");
    c["id"].clone()
}

async fn retire_category(f: &Fixture, id: &Value) {
    let (status, b) = call(
        &f.app,
        &f.author,
        Method::POST,
        &format!("/categories/{}/retire", id.as_str().unwrap()),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200, "{b}");
}

/// The category row gone from the tenant, as for a unit submitted before a `sku_change` resolved
/// its category: nothing else names it.
async fn forget_category(f: &Fixture, id: &Value) {
    use sea_orm::{ConnectionTrait, Database, DbBackend, Statement};
    let id = Uuid::parse_str(id.as_str().unwrap()).unwrap();
    let conn = Database::connect(&f.dsn).await.unwrap();
    let deleted = conn
        .execute_raw(Statement::from_string(
            DbBackend::Sqlite,
            format!(
                "DELETE FROM products_category WHERE lower(hex(id)) = '{}' OR id = '{id}'",
                id.simple()
            ),
        ))
        .await
        .unwrap()
        .rows_affected();
    conn.close().await.ok();
    assert_eq!(deleted, 1);
}

/// P-D-196: a category a `sku_change` sets is resolved in tenant scope and must be active, as the
/// draft doors resolve it. At quorum 0 the submit is also the apply: an unknown category answers
/// 404 and a retired one 409 `CATEGORY_RETIRED`; no unit is recorded and no version is written.
#[tokio::test]
async fn a_change_to_an_unknown_or_retired_category_is_refused_at_quorum_zero() {
    let f = Fixture::new(0).await;
    f.publish().await;
    let retired = new_category(&f, "retired").await;
    retire_category(&f, &retired).await;
    let units = "SELECT count(*) AS v FROM products_approval_unit";
    let before = raw_i64(&f.dsn, units).await;

    let (status, b) = f
        .post("/changes", json!({"category_id":Uuid::new_v4()}))
        .await;
    assert_eq!(status, 404, "an unknown category: {b}");
    let (status, b) = f.post("/changes", json!({"category_id":retired})).await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "CATEGORY_RETIRED");

    assert_eq!(raw_i64(&f.dsn, units).await, before, "no unit recorded");
    let s = f.card().await;
    assert_eq!(s["published_version"], 1);
    assert!(s["pending_unit_id"].is_null());
}

/// At quorum 1 the submit refuses an unknown category with 404 before any unit exists. A unit
/// already pending on a category the tenant no longer holds answers its approve with 404, not a
/// 500, and leaves the SKU at its version until it is withdrawn; a unit pending on a category
/// retired since answers 409 `CATEGORY_RETIRED`.
#[tokio::test]
async fn a_change_to_an_unknown_category_is_refused_at_submit_and_at_approve_at_quorum_one() {
    let f = Fixture::new(0).await;
    f.publish().await;
    f.policy(1).await;

    let (status, b) = f
        .post("/changes", json!({"category_id":Uuid::new_v4()}))
        .await;
    assert_eq!(status, 404, "an unknown category at submit: {b}");
    assert!(
        f.card().await["pending_unit_id"].is_null(),
        "no unit locks the SKU"
    );

    let gone = new_category(&f, "gone").await;
    let (status, unit) = f.post("/changes", json!({"category_id":gone})).await;
    assert_eq!(status, 200, "{unit}");
    assert_eq!(unit["applied"], false, "{unit}");
    forget_category(&f, &gone).await;
    let (status, b) = f.vote(&unit, "approve", 1).await;
    assert_eq!(status, 404, "an unknown category at apply: {b}");
    let s = f.card().await;
    assert_eq!(s["published_version"], 1);
    assert_eq!(s["pending_unit_id"], unit["unit"]["id"], "still pending");
    let (status, b) = call(
        &f.app,
        &f.author,
        Method::POST,
        &format!(
            "/approval-units/{}/withdraw",
            unit["unit"]["id"].as_str().unwrap()
        ),
        json!({"generation":1,"note":"its category is gone"}),
        None,
    )
    .await;
    assert_eq!(status, 200, "{b}");

    let retired = new_category(&f, "retired").await;
    let (status, unit) = f.post("/changes", json!({"category_id":retired})).await;
    assert_eq!(status, 200, "{unit}");
    retire_category(&f, &retired).await;
    let (status, b) = f.vote(&unit, "approve", 1).await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "CATEGORY_RETIRED");
    assert_eq!(f.card().await["published_version"], 1);
}

/// P-D-206: a never-published draft is deleted by its author under `If-Match`: 204, an audit row
/// whose subject is the SKU, the row gone (a second DELETE is 404) and its code free again.
#[tokio::test]
async fn a_never_published_draft_is_deleted_by_its_author() {
    let f = Fixture::new(1).await;
    let tag = f.etag().await;
    let (status, b) = f.delete(&f.author, None).await;
    assert_eq!(status, 400, "{b}");
    assert!(violation_for(&b, "If-Match").is_some(), "{b}");
    let (status, b) = f.delete(&f.author, Some("\"99\"")).await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "STALE_REVISION");
    let (status, b) = f.delete(&f.reviewer, Some(&tag)).await;
    assert_eq!(status, 403, "{b}");
    assert_eq!(problem_code(&b), "NOT_DRAFT_AUTHOR");
    let (status, b) = f.delete(&f.author, Some(&tag)).await;
    assert_eq!(status, 204, "{b}");
    assert_eq!(b, Value::Null, "a 204 carries no body");
    let (status, _) = call(
        &f.app,
        &f.author,
        Method::GET,
        &format!("/skus/{}", f.id),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(f.delete(&f.author, Some(&tag)).await.0, 404);
    assert_eq!(
        raw_i64(
            &f.dsn,
            &format!(
                "SELECT COUNT(*) AS v FROM products_audit_log WHERE action = 'sku.delete' \
                 AND subject_kind = 'sku' AND {} AND {}",
                id_matches("subject_id", f.id),
                id_matches("actor_ref", f.author.subject_id()),
            ),
        )
        .await,
        1
    );
    assert_eq!(
        raw_i64(
            &f.dsn,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE action = 'sku.create'"
        )
        .await,
        1,
        "the draft's earlier audit rows stay"
    );
    let (status, b) = call(
        &f.app,
        &f.author,
        Method::POST,
        "/skus",
        json!({"code":"SKU","name":"SKU","type":"recurring"}),
        None,
    )
    .await;
    assert_eq!(status, 201, "the code and name are free again: {b}");
}

/// P-D-206: a SKU that was ever published, or whose draft a pending unit locks, is not deleted.
#[tokio::test]
async fn a_published_or_locked_sku_is_not_deleted() {
    let f = Fixture::new(1).await;
    let (status, u) = f.post("/submit", json!({})).await;
    assert_eq!(status, 200, "{u}");
    let (status, b) = f.delete(&f.author, Some(&f.etag().await)).await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "ROW_LOCKED_PENDING");
    let (status, b) = f.vote(&u, "approve", 1).await;
    assert_eq!(status, 200, "{b}");
    let (status, b) = f.delete(&f.author, Some(&f.etag().await)).await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "SKU_NOT_DRAFT");
    assert_eq!(f.card().await["lifecycle"], "published");
    // A draft row that says it was published once is not a never-published draft.
    let (status, s) = call(
        &f.app,
        &f.author,
        Method::POST,
        "/skus",
        json!({"code":"ONCE","name":"Once","type":"recurring"}),
        None,
    )
    .await;
    assert_eq!(status, 201, "{s}");
    let once = Uuid::parse_str(s["id"].as_str().unwrap()).unwrap();
    {
        use sea_orm::{ConnectionTrait, Database};
        let raw = Database::connect(&f.dsn).await.unwrap();
        raw.execute_unprepared(&format!(
            "UPDATE products_sku SET published_version = 1 WHERE {}",
            id_matches("id", once)
        ))
        .await
        .unwrap();
        raw.close().await.unwrap();
    }
    let (status, _, b) = call_with(
        &f.app,
        &f.author,
        Method::DELETE,
        &format!("/skus/{once}"),
        json!({}),
        &[(
            "If-Match",
            format!("\"{}\"", s["revision"].as_i64().unwrap()),
        )],
    )
    .await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "SKU_NOT_DRAFT");
}

/// P-D-206: the rejected unit that named a deleted draft stays readable; its card answers
/// `impact_live: null` instead of 404.
#[tokio::test]
async fn the_unit_card_of_a_deleted_draft_answers_impact_live_null() {
    let f = Fixture::new(1).await;
    let (status, u) = f.post("/submit", json!({})).await;
    assert_eq!(status, 200, "{u}");
    let (status, b) = f.vote(&u, "reject", 1).await;
    assert_eq!(status, 200, "{b}");
    let (status, b) = f.delete(&f.author, Some(&f.etag().await)).await;
    assert_eq!(status, 204, "{b}");
    let unit = u["unit"]["id"].as_str().unwrap();
    let (status, card) = call(
        &f.app,
        &f.author,
        Method::GET,
        &format!("/approval-units/{unit}"),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 200, "{card}");
    assert_eq!(card["state"], "rejected");
    assert_eq!(card["impact_live"], Value::Null);
    assert!(card["snapshot"].is_object(), "{card}");
    assert_eq!(f.all_units(&format!("ref_id={}", f.id)).await.len(), 1);
}

/// P-D-206: a draft cannot be reserved, so no registry row can name a never-published draft; were
/// one to exist, the delete is refused `SKU_REFERENCED` rather than dropping it.
#[tokio::test]
async fn a_draft_holding_a_registry_row_is_not_deleted() {
    let f = Fixture::new(1).await;
    let (status, b) = f.reserve(Uuid::new_v4()).await;
    assert_eq!(status, 409, "a draft admits no reservation: {b}");
    let (db, scope) = repo_connection(&f.dsn, f.tenant).await;
    repo::reserve_reference(
        &db.conn().unwrap(),
        &scope,
        f.tenant,
        f.id,
        "pricing",
        crate::domain::references::RefKind::PriceBookEntry,
        Uuid::new_v4(),
        f.tenant,
        at(9),
    )
    .await
    .unwrap();
    let (status, b) = f.delete(&f.author, Some(&f.etag().await)).await;
    assert_eq!(status, 409, "{b}");
    assert_eq!(problem_code(&b), "SKU_REFERENCED");
    assert_eq!(f.card().await["lifecycle"], "draft");
}

/// P-D-206, documented rather than changed: a replay of the create's Idempotency-Key answers the
/// stored 201 of the deleted id.
#[tokio::test]
async fn a_replayed_create_answers_the_deleted_draft() {
    let f = Fixture::new(1).await;
    let body = json!({"code":"KEYED","name":"Keyed","type":"recurring"});
    let (status, s) = call(
        &f.app,
        &f.author,
        Method::POST,
        "/skus",
        body.clone(),
        Some("k1"),
    )
    .await;
    assert_eq!(status, 201, "{s}");
    let id = s["id"].as_str().unwrap().to_owned();
    let (status, _, b) = call_with(
        &f.app,
        &f.author,
        Method::DELETE,
        &format!("/skus/{id}"),
        json!({}),
        &[(
            "If-Match",
            format!("\"{}\"", s["revision"].as_i64().unwrap()),
        )],
    )
    .await;
    assert_eq!(status, 204, "{b}");
    let (status, replay) = call(&f.app, &f.author, Method::POST, "/skus", body, Some("k1")).await;
    assert_eq!(status, 201);
    assert_eq!(replay["id"], id);
    let (status, _) = call(
        &f.app,
        &f.author,
        Method::GET,
        &format!("/skus/{id}"),
        json!({}),
        None,
    )
    .await;
    assert_eq!(status, 404);
}

/// P-D-207 (owner option b): usage types are read as the caller. A collector that refuses the caller is
/// 403 `USAGE_TYPE_FORBIDDEN` at submit and at approve, never the 503 of an outage; nothing is
/// recorded and no key is claimed.
#[tokio::test]
async fn a_collector_denial_is_403_at_submit_and_at_approve() {
    let f = Fixture::new(1).await;
    // A ref the collector adapter would ask about: a well-formed usage-record GTS id.
    let (status, s) = call(
        &f.app,
        &f.author,
        Method::POST,
        "/skus",
        json!({"code":"METERED","name":"Metered","type":"usage","unit":"GB",
               "usage_type_ref":"gts.cf.core.uc.usage_record.v1~cf.e2e.pricebook.storage.v1"}),
        None,
    )
    .await;
    assert_eq!(status, 201, "{s}");
    let submit = format!("/skus/{}/submit", s["id"].as_str().unwrap());
    let denied = second_app(&f, denying_collector_catalog()).await;
    let (status, b) = call(
        &denied,
        &f.author,
        Method::POST,
        &submit,
        json!({}),
        Some("denied-submit"),
    )
    .await;
    assert_eq!(status, 403, "{b}");
    assert_eq!(problem_code(&b), "USAGE_TYPE_FORBIDDEN");
    assert_eq!(
        raw_i64(&f.dsn, "SELECT count(*) AS v FROM products_approval_unit").await,
        0
    );
    assert_eq!(idempotency_rows_for(&f.dsn, "denied-submit").await, 0);
    let (status, u) = call(&f.app, &f.author, Method::POST, &submit, json!({}), None).await;
    assert_eq!(status, 200, "{u}");
    let approve = format!(
        "/approval-units/{}/approve",
        u["unit"]["id"].as_str().unwrap()
    );
    let (status, b) = call(
        &denied,
        &f.reviewer,
        Method::POST,
        &approve,
        json!({"generation":1}),
        None,
    )
    .await;
    assert_eq!(status, 403, "{b}");
    assert_eq!(problem_code(&b), "USAGE_TYPE_FORBIDDEN");
    let (status, b) = call(
        &f.app,
        &f.reviewer,
        Method::POST,
        &approve,
        json!({"generation":1}),
        None,
    )
    .await;
    assert_eq!(
        status, 200,
        "a reviewer the collector admits still decides: {b}"
    );
    assert_eq!(b["outcome"], "applied", "{b}");
}

#[path = "sku_history_tests.rs"]
mod sku_history_tests;
#[path = "submit_note_tests.rs"]
mod submit_note_tests;

#[path = "caps_tests.rs"]
mod caps_tests;
