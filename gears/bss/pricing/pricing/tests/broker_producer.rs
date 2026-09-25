//! With an `EventBrokerApi` in the `ClientHub`, pricing binds the broker SDK's `DbProducer`
//! to its outbox queue (D-400, Products' pattern): committed events reach the broker, an
//! interrupted dispatch is retried from the durable envelope, and a rolled-back transaction
//! delivers nothing.
#![allow(clippy::expect_used, clippy::unwrap_used)]
mod entry_support;
use bss_pricing::{
    api::rest::authoring::AuthoringState,
    infra::{
        events::{self, APPROVAL_UNIT_SUBJECT_TYPE, PRICE_BOOK_SUBJECT_TYPE, TOPIC},
        reference_events::PriceBookEntryReferenceLost,
    },
};
use entry_support::{Script, app_for, request, test_db, user_of};
use event_broker_sdk::{TypedEvent, api::EventBrokerApi, mock::MockBroker};
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

const PUBLISHED: &str = "gts.cf.core.events.event.v1~cf.bss.pricing.prices_published.v1~";
const DECIDED: &str = "gts.cf.core.events.event.v1~cf.bss.pricing.approval_unit_decided.v1~";
const ENTRY_SUBJECT_TYPE: &str = "gts.cf.core.events.subject.v1~cf.bss.pricing.price_book_entry.v1";

/// Every event the broker stored on pricing's topic: its type and its data.
async fn stored(mock: &MockBroker) -> Vec<(String, serde_json::Value)> {
    let handle = mock.handle();
    let mut all = Vec::new();
    for partition in 0..8 {
        for stored in handle.stored(TOPIC, partition).await {
            all.push((
                stored.event.type_id.clone(),
                stored.event.data.clone().unwrap_or_default(),
            ));
        }
    }
    all
}
fn lost(tenant: Uuid) -> PriceBookEntryReferenceLost {
    PriceBookEntryReferenceLost {
        tenant_id: tenant,
        price_book_entry_id: Uuid::new_v4(),
        sku_id: Uuid::new_v4(),
        reservation_id: Uuid::new_v4(),
        actor_ref: Uuid::new_v4(),
    }
}
/// Enqueue one event on its own transaction; roll it back when `commit` is false.
async fn enqueue(state: &AuthoringState, event: PriceBookEntryReferenceLost, commit: bool) {
    let outbox = state.outbox.clone();
    let result = state
        .db
        .db()
        .transaction_with_retry(
            toolkit_db::secure::TxConfig::default(),
            |_: &anyhow::Error| None,
            move |tx| {
                let (outbox, event) = (outbox.clone(), event.clone());
                Box::pin(async move {
                    events::enqueue(&outbox, tx, &event, time::OffsetDateTime::now_utc()).await?;
                    if commit {
                        Ok(())
                    } else {
                        Err(anyhow::anyhow!(
                            "the act failed after its event was written"
                        ))
                    }
                })
            },
        )
        .await;
    assert_eq!(result.is_ok(), commit);
}

#[tokio::test]
async fn a_bound_producer_delivers_committed_events_retries_dispatch_and_drops_rollbacks() {
    let mock = MockBroker::new();
    let control = mock.handle();
    control.register_topic(TOPIC, 8).await;
    let object = json!({"type":"object"});
    for (type_id, subject) in [
        (PriceBookEntryReferenceLost::TYPE_ID, ENTRY_SUBJECT_TYPE),
        (PUBLISHED, PRICE_BOOK_SUBJECT_TYPE),
        (DECIDED, APPROVAL_UNIT_SUBJECT_TYPE),
    ] {
        control
            .register_event_type(TOPIC, type_id, object.clone(), &[subject])
            .await;
    }
    let (db, _, tenant, _) = test_db().await;
    let hub = Arc::new(toolkit::ClientHub::default());
    hub.register::<bss_products_sdk::PricingReferenceRegistry>(Arc::new(
        bss_products_sdk::PricingReferenceRegistry(Arc::new(Script::default())),
    ));
    hub.register::<dyn EventBrokerApi>(Arc::new(mock.clone()));
    let state = Arc::new(AuthoringState::new(db, hub).await.unwrap());

    // The broker refuses every dispatch for now (429): the committed envelope stays durable.
    control.set_publish_rate_limit(Some(0)).await;
    let committed = lost(tenant);
    enqueue(&state, committed.clone(), true).await;
    let rolled_back = lost(tenant);
    enqueue(&state, rolled_back.clone(), false).await;

    // A real door's transaction: quorum zero applies at submit and announces both events.
    let (app, ctx) = (app_for(state.clone(), tenant), user_of(tenant));
    let call = |method: &'static str,
                path: String,
                body: serde_json::Value,
                tag: Option<String>,
                key: Option<&'static str>| {
        let (app, ctx) = (app.clone(), ctx.clone());
        async move { request(&app, &ctx, method, &path, body, tag.as_deref(), key).await }
    };
    let (_, _, tag) = call("GET", "/approval-policy".into(), json!({}), None, None).await;
    assert_eq!(
        call(
            "PUT",
            "/approval-policy".into(),
            json!({"quorum":0}),
            Some(tag),
            None
        )
        .await
        .0,
        200
    );
    let book = call(
        "POST",
        "/price-books".into(),
        json!({"code":"standard","name":"Standard","currency":"EUR"}),
        None,
        Some("book"),
    )
    .await;
    assert_eq!(book.0, 201, "{book:?}");
    let entry = call(
        "POST",
        format!("/price-books/{}/entries", book.1["id"].as_str().unwrap()),
        json!({"sku_id":Uuid::new_v4()}),
        None,
        Some("entry"),
    )
    .await;
    assert_eq!(entry.0, 201, "{entry:?}");
    let prices = call(
        "POST",
        format!("/price-book-entries/{}/prices", entry.1["id"].as_str().unwrap()),
        json!({"model":"per_unit","price":{"rate":"0.10"},"eligibility":"all","effective_from":"2031-03-01"}),
        None,
        Some("price"),
    )
    .await;
    assert_eq!(prices.0, 201, "{prices:?}");
    let submitted = call(
        "POST",
        format!(
            "/prices/{}/submit",
            prices.1["items"][0]["id"].as_str().unwrap()
        ),
        json!({}),
        None,
        Some("submit"),
    )
    .await;
    assert_eq!(submitted.0, 201, "{submitted:?}");

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    assert!(
        stored(&mock).await.is_empty(),
        "no dispatch succeeded while the broker refused"
    );
    // Delivery resumes: the durable envelopes are retried.
    control.set_publish_rate_limit(None).await;
    let mut delivered = Vec::new();
    for _ in 0..300 {
        delivered = stored(&mock).await;
        if delivered.len() >= 3 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let types: Vec<_> = delivered.iter().map(|(t, _)| t.as_str()).collect();
    assert_eq!(delivered.len(), 3, "{delivered:#?}");
    assert!(types.contains(&PUBLISHED), "{types:?}");
    assert!(types.contains(&DECIDED), "{types:?}");
    let lost_events: Vec<_> = delivered
        .iter()
        .filter(|(t, _)| t == PriceBookEntryReferenceLost::TYPE_ID)
        .collect();
    assert_eq!(lost_events.len(), 1, "{delivered:#?}");
    assert_eq!(
        lost_events[0].1["priceBookEntryId"],
        committed.price_book_entry_id.to_string(),
        "the committed event, retried after the refused dispatch"
    );
    assert!(
        !delivered.iter().any(
            |(_, data)| data["priceBookEntryId"] == rolled_back.price_book_entry_id.to_string()
        ),
        "a rolled-back transaction delivers nothing"
    );
}
