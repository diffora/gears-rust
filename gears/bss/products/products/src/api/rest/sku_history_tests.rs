//! P-D-213: every act that touches a SKU's lifecycle stamps the move on its audit row
//! (`from_lifecycle`, `to_lifecycle`), and `GET /skus/{id}/history` reads the SKU's rows back.
//!
//! A child of the governance suite, so it drives the real doors through its `Fixture`.
#![allow(clippy::expect_used, clippy::unwrap_used)]
use super::*;
use axum::http::Method;

/// The SKU's audit rows as the history reads them — the SKU's own rows and its units' rows — raw,
/// one `action unit_kind from>to` line each (`-` for none), in `(written_at, audit_id)` order.
async fn moves(f: &Fixture, sku: Uuid) -> Vec<String> {
    let hex = sku.simple().to_string().to_uppercase();
    let conn = sea_orm::Database::connect(&f.dsn).await.unwrap();
    let rows = {
        use sea_orm::ConnectionTrait;
        conn.query_all_raw(sea_orm::Statement::from_string(
            sea_orm::DbBackend::Sqlite,
            format!(
                "SELECT a.action || ' ' || coalesce(u.kind, '-') || ' ' || \
                 coalesce(a.from_lifecycle, '-') || '>' || coalesce(a.to_lifecycle, '-') AS v \
                 FROM products_audit_log a LEFT JOIN products_approval_unit u \
                 ON a.subject_kind = 'approval_unit' AND u.id = a.subject_id \
                 WHERE (a.subject_kind = 'sku' AND hex(a.subject_id) = '{hex}') \
                 OR (a.subject_kind = 'approval_unit' AND hex(u.ref_id) = '{hex}') \
                 ORDER BY a.written_at, a.audit_id"
            ),
        ))
        .await
        .unwrap()
        .iter()
        .map(|row| row.try_get::<String>("", "v").unwrap())
        .collect()
    };
    conn.close().await.unwrap();
    rows
}

/// A unit's decision as `ctx`.
async fn decide(f: &Fixture, ctx: &SecurityContext, unit: &Value, action: &str) -> Value {
    let (status, b) = call(
        &f.app,
        ctx,
        Method::POST,
        &format!(
            "/approval-units/{}/{action}",
            unit["unit"]["id"].as_str().unwrap()
        ),
        json!({"generation":unit["unit"]["generation"],"note":format!("{action} note")}),
        None,
    )
    .await;
    assert_eq!(status, 200, "{action}: {b}");
    b
}

/// An orphan fence the test leaves behind (no unit holds it), `age` old: the state a lost submit
/// leaves, and the only way to one through the doors' own tables.
async fn orphan_fence(f: &Fixture, kind: repo::Fence, age: time::Duration) {
    let (db, scope) = repo_connection(&f.dsn, f.tenant).await;
    let fenced = repo::fence_sku(
        &db.conn().unwrap(),
        &scope,
        f.tenant,
        f.id,
        kind,
        Uuid::new_v4(),
        time::OffsetDateTime::now_utc() - age,
    )
    .await
    .unwrap();
    assert!(matches!(fenced, repo::HeadWrite::Written(_)), "fenced");
}

/// The derivation table of P-D-213, driven through the doors: each act's row says the lifecycle it
/// found and the one it left, a unit's rows name its kind, the maintenance expiry writes its own
/// row as the system actor, and a row's `to` is the next row's `from` wherever no test fence came
/// between them.
#[tokio::test]
#[allow(clippy::too_many_lines, reason = "one SKU's whole life, act by act")]
async fn every_lifecycle_move_is_stamped_by_the_act_that_made_it() {
    let f = Fixture::new(1).await;
    let tag = f.etag().await;
    let (status, _, b) = call_with(
        &f.app,
        &f.author,
        Method::PATCH,
        &format!("/skus/{}", f.id),
        json!({"description":"edited"}),
        &[("If-Match", tag)],
    )
    .await;
    assert_eq!(status, 200, "{b}");
    let (_, u) = f.post("/submit", json!({})).await;
    decide(&f, &f.reviewer, &u, "reject").await;
    f.policy(2).await;
    let (_, u) = f.post("/submit", json!({})).await;
    assert_eq!(
        decide(&f, &f.reviewer, &u, "approve").await["outcome"],
        "pending"
    );
    let second = authed_ctx(f.tenant);
    assert_eq!(
        decide(&f, &second, &u, "approve").await["outcome"],
        "applied"
    );
    f.policy(0).await;
    let (status, b) = f.post("/changes", json!({"lifecycle":"deprecated"})).await;
    assert_eq!((status, &b["applied"]), (200, &json!(true)), "{b}");
    f.policy(1).await;
    let (status, u) = f.post("/retire", json!({})).await;
    assert_eq!(status, 200, "{u}");
    decide(&f, &f.author, &u, "withdraw").await;
    orphan_fence(&f, repo::Fence::Retire, time::Duration::ZERO).await;
    assert_eq!(f.post("/unfence", json!({})).await.0, 200);
    orphan_fence(&f, repo::Fence::TypeChange, time::Duration::ZERO).await;
    assert_eq!(f.post("/unfence", json!({})).await.0, 200);
    // The card's maintenance expires an orphan fence, and so does the list's.
    orphan_fence(&f, repo::Fence::Retire, time::Duration::hours(2)).await;
    assert_eq!(f.card().await["lifecycle"], "deprecated");
    orphan_fence(&f, repo::Fence::Retire, time::Duration::hours(2)).await;
    let (status, b) = call(&f.app, &f.author, Method::GET, "/skus", json!({}), None).await;
    assert_eq!(status, 200, "{b}");
    f.policy(0).await;
    let (status, b) = f.post("/retire", json!({})).await;
    assert_eq!((status, &b["applied"]), (200, &json!(true)), "{b}");

    assert_eq!(
        moves(&f, f.id).await,
        [
            "sku.create - ->draft",
            "sku.draft_update - draft>draft",
            "approval.submit sku_publish draft>draft",
            "approval.rejected sku_publish draft>draft",
            "approval.submit sku_publish draft>draft",
            "approval.vote sku_publish draft>draft",
            "approval.approved sku_publish draft>published",
            "approval.submit sku_change published>published",
            "approval.applied sku_change published>deprecated",
            "approval.submit sku_retire deprecated>retiring",
            "approval.withdrawn sku_retire retiring>deprecated",
            // a test fence (retire) came between: no act of the doors made it
            "sku.unfence - retiring>deprecated",
            // a test fence (type change) came between: it moves no lifecycle
            "sku.unfence - deprecated>deprecated",
            // a test fence (retire, two hours old) came between, expired by the card's read
            "sku.fence_expired - retiring>deprecated",
            // and again, expired by the list's read
            "sku.fence_expired - retiring>deprecated",
            "approval.submit sku_retire deprecated>retiring",
            "approval.applied sku_retire retiring>retired",
        ]
    );
    assert_eq!(
        raw_i64(
            &f.dsn,
            &format!(
                "SELECT COUNT(*) AS v FROM products_audit_log WHERE action = 'sku.fence_expired' \
                 AND {} AND reason = 'fence_ttl_minutes=30'",
                id_matches("actor_ref", repo::SYSTEM_ACTOR)
            ),
        )
        .await,
        2,
        "the expiry is the system's act, and says which TTL it applied"
    );
    // The acts on no SKU stamp nothing: the category, the policy writes.
    assert_eq!(
        raw_i64(
            &f.dsn,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE subject_kind NOT IN ('sku', \
             'approval_unit') AND (from_lifecycle IS NOT NULL OR to_lifecycle IS NOT NULL)"
        )
        .await,
        0
    );
    assert!(
        raw_i64(
            &f.dsn,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE subject_kind IN ('category', \
             'approval_policy')"
        )
        .await
            >= 5
    );
}

/// The derivation table's other rows: a draft delete leaves nothing, a refresh moves nothing, a
/// rejected retire returns to the lifecycle it came from, and a reference act stamps nothing.
#[tokio::test]
async fn a_delete_a_refresh_a_rejected_retire_and_a_reference_stamp_their_rows() {
    let f = Fixture::new(1).await;
    // A refresh, then the rejection of the refreshed generation.
    let (_, u) = f.post("/submit", json!({})).await;
    let (db, scope) = repo_connection(&f.dsn, f.tenant).await;
    let conn = db.conn().unwrap();
    let mut content = bss_products_sdk::models::SkuContent::from(
        &repo::find_sku(&conn, &scope, f.tenant, f.id)
            .await
            .unwrap()
            .unwrap(),
    );
    content.description = "drifted".into();
    repo::write_sku_content(
        &conn,
        &scope,
        f.tenant,
        f.id,
        &content,
        time::OffsetDateTime::now_utc(),
    )
    .await
    .unwrap();
    let (status, body) = f.vote(&u, "reject", 1).await;
    assert_eq!((status, problem_code(&body).as_str()), (400, "UNIT_STALE"));
    assert_eq!(f.vote(&u, "reject", 2).await.0, 200);
    // A published SKU whose retire is rejected, then a reservation on it.
    f.publish().await;
    f.policy(1).await;
    let (_, u) = f.post("/retire", json!({})).await;
    decide(&f, &f.reviewer, &u, "reject").await;
    let (status, reference) = f.reserve(Uuid::new_v4()).await;
    assert_eq!(status, 201, "{reference}");
    assert_eq!(
        moves(&f, f.id).await,
        [
            "sku.create - ->draft",
            "approval.submit sku_publish draft>draft",
            "approval.refreshed sku_publish draft>draft",
            "approval.rejected sku_publish draft>draft",
            "approval.submit sku_publish draft>draft",
            "approval.applied sku_publish draft>published",
            "approval.submit sku_retire published>retiring",
            "approval.rejected sku_retire retiring>published",
        ]
    );
    assert_eq!(
        raw_i64(
            &f.dsn,
            "SELECT COUNT(*) AS v FROM products_audit_log WHERE action = 'reference.reserve' \
             AND from_lifecycle IS NULL AND to_lifecycle IS NULL"
        )
        .await,
        1
    );
    // A never-published draft deleted: its create and its delete.
    let (status, s) = call(
        &f.app,
        &f.author,
        Method::POST,
        "/skus",
        json!({"code":"GONE","name":"Gone","type":"recurring"}),
        None,
    )
    .await;
    assert_eq!(status, 201, "{s}");
    let gone = Uuid::parse_str(s["id"].as_str().unwrap()).unwrap();
    let (status, _, b) = call_with(
        &f.app,
        &f.author,
        Method::DELETE,
        &format!("/skus/{gone}"),
        json!({}),
        &[("If-Match", "\"1\"".to_owned())],
    )
    .await;
    assert_eq!(status, 204, "{b}");
    assert_eq!(
        moves(&f, gone).await,
        ["sku.create - ->draft", "sku.delete - draft>-"]
    );
}
