//! Durable `PriceReferenceLost` events using the toolkit outbox and broker envelope.
use super::storage::{RepoError, entity::price};
use event_broker_sdk::TypedEvent;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use toolkit_db::secure::DBRunner;
use uuid::Uuid;
pub const QUEUE: &str = "bss_pricing_events";
/// Held until the pricing broker producer is bound by the eventing task (2c.8).
pub struct PendingProducer;
#[async_trait::async_trait]
impl toolkit_db::outbox::LeasedMessageHandler for PendingProducer {
    async fn handle(
        &self,
        _: &toolkit_db::outbox::OutboxMessage,
    ) -> toolkit_db::outbox::MessageResult {
        toolkit_db::outbox::MessageResult::Retry
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceReferenceLost {
    pub tenant_id: Uuid,
    pub price_id: Uuid,
    pub sku_id: Uuid,
    pub reservation_id: Uuid,
    pub actor_ref: Uuid,
}
impl TypedEvent for PriceReferenceLost {
    const TYPE_ID: &'static str =
        "gts.cf.core.events.event.v1~cf.bss.pricing.price_reference_lost.v1~";
    const SUBJECT_TYPE: &'static str = "gts.cf.core.events.subject.v1~cf.bss.pricing.price.v1";
    const SOURCE: &'static str = "bss-pricing";
    fn subject(&self) -> Cow<'_, str> {
        Cow::Owned(self.price_id.to_string())
    }
    fn tenant_id(&self) -> Option<Uuid> {
        Some(self.tenant_id)
    }
}
/// Enqueue on the same runner as the price/op transition, so rollback erases the event.
/// # Errors
/// Preserves typed database errors for serializable transaction retries.
pub async fn lost(
    outbox: &toolkit_db::outbox::Outbox,
    tx: &(impl DBRunner + Sync),
    price: &price::Model,
    actor: Uuid,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let event = PriceReferenceLost {
        tenant_id: price.tenant_id,
        price_id: price.id,
        sku_id: price.sku_id,
        reservation_id: price.reservation_id,
        actor_ref: actor,
    };
    let envelope = serde_json::json!({
        "version":1, "event_id":Uuid::now_v7(), "type":PriceReferenceLost::TYPE_ID,
        "topic":"gts.cf.core.events.topic.v1~cf.bss.pricing.catalog.v1", "tenant_id":event.tenant_id(),
        "source":PriceReferenceLost::SOURCE, "subject":event.subject(), "subject_type":PriceReferenceLost::SUBJECT_TYPE,
        "occurred_at":now.format(&time::format_description::well_known::Rfc3339).map_err(|e| RepoError::Db(e.to_string()))?,
        "trace_parent":null, "data":event, "broker_partition":0, "producer_mode":"stateless", "diagnostic_metadata":{"sdk_client_agent":"bss-pricing"}
    });
    outbox
        .enqueue(
            tx,
            QUEUE,
            0,
            serde_json::to_vec(&envelope).map_err(|e| RepoError::Db(e.to_string()))?,
            "application/vnd.constructorfabric.event-broker.producer-outbox+json;version=1",
        )
        .await
        .map_err(|e| match e {
            toolkit_db::outbox::OutboxError::Database(source) => RepoError::Driver {
                context: "price reference event".into(),
                source,
            },
            other => RepoError::Db(other.to_string()),
        })?;
    Ok(())
}
