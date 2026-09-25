//! Price authoring protocol branches through the production router.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod price_support;
use price_support::{Fixture, Script};
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;
async fn setup(mode: usize) -> (Fixture, Arc<Script>, String, Value) {
    let script = Arc::new(Script::default());
    script.set(mode);
    let f = Fixture::new(script.clone()).await;
    let (book, _) = f.book().await;
    let path = format!("/price-books/{}/prices", book["id"].as_str().unwrap());
    (
        f,
        script,
        path,
        if mode == 11 {
            json!({"sku_id":Uuid::new_v4(),"period":"month"})
        } else {
            json!({"sku_id":Uuid::new_v4()})
        },
    )
}
#[tokio::test]
async fn create_replay_current_kind_unique_key_and_delete() {
    let (f, script, path, input) = setup(11).await;
    assert_eq!(
        f.call("POST", &path, input.clone(), None, None).await.0,
        400
    );
    let first = f
        .call("POST", &path, input.clone(), None, Some("one"))
        .await;
    assert_eq!(first.0, 201, "{first:?}");
    assert_eq!(first.1["reference_state"], "confirmed");
    assert_eq!(first.1["charge_kind"], "recurring");
    assert_eq!(
        f.call("POST", &path, input.clone(), None, Some("one"))
            .await,
        first
    );
    assert_eq!(Script::count(&script.reserve_calls), 1);
    let dup = f.call("POST", &path, input, None, Some("two")).await;
    assert_eq!(dup.0, 409, "{dup:?}");
    assert!(dup.1.to_string().contains("PRICE_KEY_TAKEN"));
    let id = first.1["id"].as_str().unwrap();
    assert_eq!(
        f.call("GET", &format!("/prices/{id}"), json!({}), None, None)
            .await
            .1,
        first.1
    );
    assert_eq!(
        f.call("DELETE", &format!("/prices/{id}"), json!({}), None, None)
            .await
            .0,
        204
    );
    assert_eq!(Script::count(&script.releases), 2);
}
#[tokio::test]
async fn refusal_branches_answer_key_and_release_only_after_reservation() {
    for (mode, code, releases) in [
        (4, "SKU_FENCED", 0),
        (8, "BUNDLE_SKU_NOT_PRICEABLE", 1),
        (9, "SKU_DEPRECATED", 1),
        (10, "SKU_DRAFT", 1),
    ] {
        let (f, script, path, input) = setup(mode).await;
        let refused = f
            .call("POST", &path, input.clone(), None, Some("one"))
            .await;
        assert_eq!(refused.0, 409, "{refused:?}");
        assert!(refused.1.to_string().contains(code), "{refused:?}");
        script.set(0);
        assert_eq!(
            f.call("POST", &path, input, None, Some("one")).await,
            refused
        );
        assert_eq!(Script::count(&script.releases), releases);
    }
}
#[tokio::test]
async fn lost_reserve_and_confirm_timeout_leave_unanswered_keys() {
    for mode in [5, 6] {
        let (f, script, path, input) = setup(mode).await;
        let result = f
            .call("POST", &path, input.clone(), None, Some("one"))
            .await;
        assert_eq!(result.0, 503, "{result:?}");
        let retry = f.call("POST", &path, input, None, Some("one")).await;
        assert_eq!(retry.0, 409);
        assert!(
            retry.1.to_string().contains("IDEMPOTENCY_KEY_IN_FLIGHT"),
            "{retry:?}"
        );
        assert_eq!(Script::count(&script.releases), 0);
    }
}
#[tokio::test]
async fn released_on_confirm_is_lost_and_replayable() {
    let (f, script, path, input) = setup(7).await;
    let first = f
        .call("POST", &path, input.clone(), None, Some("one"))
        .await;
    assert_eq!(first.0, 201, "{first:?}");
    assert_eq!(first.1["reference_state"], "lost");
    assert_eq!(f.call("POST", &path, input, None, Some("one")).await, first);
    assert_eq!(Script::count(&script.releases), 0);
}
#[tokio::test]
async fn patch_template_and_dimension_preconditions() {
    let (f, _, path, input) = setup(0).await;
    let first = f.call("POST", &path, input, None, Some("one")).await;
    assert_eq!(first.0, 201, "{first:?}");
    let path = format!("/prices/{}", first.1["id"].as_str().unwrap());
    assert_eq!(f.call("PATCH", &path, json!({}), None, None).await.0, 400);
    assert_eq!(
        f.call(
            "PATCH",
            &path,
            json!({"invoice_line_override":"{phase}"}),
            Some(&first.2),
            None
        )
        .await
        .0,
        400
    );
    let changed = f
        .call(
            "PATCH",
            &path,
            json!({"invoice_line_override":"{sku} {unit}"}),
            Some(&first.2),
            None,
        )
        .await;
    assert_eq!(changed.0, 200, "{changed:?}");
    assert_eq!(
        f.call("PATCH", &path, json!({}), Some(&first.2), None)
            .await
            .0,
        409
    );
    assert_eq!(
        f.call(
            "PATCH",
            &path,
            json!({"invoice_line_override":null}),
            Some(&changed.2),
            None
        )
        .await
        .1["invoice_line_override"],
        Value::Null
    );
}

#[tokio::test]
async fn translated_dimension_key_change_rejected_and_approved_delete_refused() {
    use bss_pricing::infra::storage::repo::{price_repo, row_repo};
    let (f, _, path, input) = setup(0).await;
    let (_, _, tag) = f
        .call("GET", "/dimension-keys", json!({}), None, None)
        .await;
    assert_eq!(
        f.call(
            "PUT",
            "/dimension-keys",
            json!({"items":[{"key":"region","values":["eu","us"]}]}),
            Some(&tag),
            None
        )
        .await
        .0,
        200
    );
    let first = f.call("POST", &path, input, None, Some("one")).await;
    let id = first.1["id"].as_str().unwrap().parse().unwrap();
    let path = format!("/prices/{id}");
    let changed = f
        .call(
            "PATCH",
            &path,
            json!({"dimension_key":"region"}),
            Some(&first.2),
            None,
        )
        .await;
    assert_eq!(changed.0, 200, "{changed:?}");
    let scope = toolkit_db::secure::AccessScope::for_tenant(f.ctx.subject_tenant_id());
    let conn = f.db.conn().unwrap();
    let p = price_repo::find(&conn, &scope, f.ctx.subject_tenant_id(), id)
        .await
        .unwrap()
        .unwrap();
    let mut row = price_support::row(&p);
    row.dim_value = Some("eu".into());
    row.state = "approved".into();
    row_repo::insert(&conn, &scope, row).await.unwrap();
    let refusal = f
        .call(
            "PATCH",
            &path,
            json!({"dimension_key":null}),
            Some(&changed.2),
            None,
        )
        .await;
    assert_eq!(refusal.0, 409);
    assert!(refusal.1.to_string().contains("DIMENSION_KEY_IN_USE"));
    assert_eq!(
        f.call(
            "PATCH",
            &path,
            json!({"invoice_line_override":"{sku}"}),
            Some(&changed.2),
            None
        )
        .await
        .0,
        200
    );
    assert_eq!(f.call("DELETE", &path, json!({}), None, None).await.0, 409);
}
