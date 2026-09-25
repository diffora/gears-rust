//! Durable `PriceReferenceLost` events through the shared pricing event writer.
use super::{
    events::{self, SOURCE},
    storage::{RepoError, entity::price},
};
use event_broker_sdk::TypedEvent;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use toolkit_db::secure::DBRunner;
use uuid::Uuid;
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
    const SOURCE: &'static str = SOURCE;
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
    outbox: &events::EventSink,
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
    events::enqueue(outbox, tx, &event, now).await
}
