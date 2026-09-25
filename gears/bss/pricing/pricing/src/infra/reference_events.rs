//! Durable `PriceBookEntryReferenceLost` events through the shared pricing event writer.
use super::{
    events::{self, SOURCE},
    storage::{RepoError, entity::price_book_entry},
};
use event_broker_sdk::TypedEvent;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use toolkit_db::secure::DBRunner;
use uuid::Uuid;
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceBookEntryReferenceLost {
    pub tenant_id: Uuid,
    pub price_book_entry_id: Uuid,
    pub sku_id: Uuid,
    pub reservation_id: Uuid,
    pub actor_ref: Uuid,
}
impl TypedEvent for PriceBookEntryReferenceLost {
    const TYPE_ID: &'static str =
        "gts.cf.core.events.event.v1~cf.bss.pricing.price_book_entry_reference_lost.v1~";
    const SUBJECT_TYPE: &'static str =
        "gts.cf.core.events.subject.v1~cf.bss.pricing.price_book_entry.v1";
    const SOURCE: &'static str = SOURCE;
    fn subject(&self) -> Cow<'_, str> {
        Cow::Owned(self.price_book_entry_id.to_string())
    }
    fn tenant_id(&self) -> Option<Uuid> {
        Some(self.tenant_id)
    }
}
/// Enqueue on the same runner as the entry/op transition, so rollback erases the event.
/// # Errors
/// Preserves typed database errors for serializable transaction retries.
pub async fn lost(
    outbox: &events::EventSink,
    tx: &(impl DBRunner + Sync),
    entry: &price_book_entry::Model,
    actor: Uuid,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let event = PriceBookEntryReferenceLost {
        tenant_id: entry.tenant_id,
        price_book_entry_id: entry.id,
        sku_id: entry.sku_id,
        reservation_id: entry.reservation_id,
        actor_ref: actor,
    };
    events::enqueue(outbox, tx, &event, now).await
}
