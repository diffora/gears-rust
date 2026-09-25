//! The approval doors dispatch by the unit's kind (run 3.4, Task 3.4.1): the policy names the
//! `plan_revision` kind, each kind shows its own impact, and a stored unit of a kind pricing does
//! not record is a corrupt row (500), never treated as a `prices` unit.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod plan_support;
use bss_pricing::infra::storage::repo::approval_repo;
use plan_support::{scope, setup, text, unit, unit_of_kind};
use serde_json::json;

#[tokio::test]
async fn the_policy_names_the_plan_revision_kind_and_refuses_the_deferred_ones() {
    let (f, _) = setup().await;
    let (_, _, tag) = f
        .call("GET", "/approval-policy", json!({}), None, None)
        .await;
    let saved = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"kind":"plan_revision","quorum":0}),
            Some(&tag),
            None,
        )
        .await;
    assert_eq!(saved.0, 200, "{saved:?}");
    assert_eq!(
        saved.1,
        json!({"default_quorum":1,"overrides":{"plan_revision":0}})
    );
    // Promotions (D-409) and migration requests (D-410) are deferred: no kind of theirs.
    for kind in ["promotion", "migration", "price", "bogus"] {
        let refused = f
            .call(
                "PUT",
                "/approval-policy",
                json!({"kind":kind,"quorum":1}),
                Some(&saved.2),
                None,
            )
            .await;
        assert_eq!(refused.0, 400, "{kind}: {refused:?}");
        assert!(
            text(&refused.1).contains("POLICY_KIND_INVALID"),
            "{refused:?}"
        );
    }
    let both = f
        .call(
            "PUT",
            "/approval-policy",
            json!({"kind":"prices","quorum":2}),
            Some(&saved.2),
            None,
        )
        .await;
    assert_eq!(both.0, 200, "{both:?}");
    assert_eq!(
        both.1,
        json!({"default_quorum":1,"overrides":{"plan_revision":0,"prices":2}})
    );
}

#[tokio::test]
async fn a_plan_revision_unit_shows_its_own_impact_on_the_card_and_in_the_queue() {
    let (f, _) = setup().await;
    let id = unit(&f).await;
    let impact = json!({"subscriptions":"unavailable until the Subscriptions integration"});
    let (s, card, _) = f
        .call(
            "GET",
            &format!("/approval-units/{id}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{card}");
    assert_eq!(card["kind"], "plan_revision");
    assert_eq!(card["impact"], impact, "{card}");
    let (s, list, _) = f
        .call(
            "GET",
            "/approval-units?kind=plan_revision",
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 200, "{list}");
    assert_eq!(list["items"].as_array().unwrap().len(), 1, "{list}");
    assert_eq!(list["items"][0]["impact"], impact, "{list}");
}

#[tokio::test]
async fn a_unit_of_an_unknown_kind_is_a_corrupt_row_never_a_prices_unit() {
    let (f, _) = setup().await;
    let id = unit_of_kind(&f, "promotion").await;
    let (s, b, _) = f
        .call(
            "GET",
            &format!("/approval-units/{id}"),
            json!({}),
            None,
            None,
        )
        .await;
    assert_eq!(s, 500, "the card: {b}");
    let (s, b, _) = f
        .call("GET", "/approval-units", json!({}), None, None)
        .await;
    assert_eq!(s, 500, "the queue: {b}");
    for (action, body) in [
        ("approve", json!({"generation":1})),
        ("reject", json!({"generation":1,"note":"no"})),
        ("withdraw", json!({})),
    ] {
        let (s, b, _) = f
            .call_as(
                &f.user(),
                "POST",
                &format!("/approval-units/{id}/{action}"),
                body,
                None,
                Some(action),
            )
            .await;
        assert_eq!(s, 500, "{action}: {b}");
    }
    let stored = approval_repo::find_unit(
        &f.db.conn().unwrap(),
        &scope(&f),
        f.ctx.subject_tenant_id(),
        id,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        (stored.state.as_str(), stored.generation, stored.version),
        ("pending", 1, 1),
        "nothing about the unit changed"
    );
}
