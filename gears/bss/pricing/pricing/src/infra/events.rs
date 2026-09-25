//! Pricing's typed events (D-400): broker `TypedEvent` payloads written through the toolkit
//! outbox on the caller's transaction, in the broker's producer-outbox envelope.
//!
//! The runner a writer passes is the transaction of the act the event reports, so a rollback
//! erases the event with the act, and a failed outbox insert fails the act. No broker
//! producer is bound in phase 2: the queue's processor is [`PendingProducer`], which holds
//! every envelope, so nothing is reported delivered before a broker exists (Products'
//! interim pattern).
use super::storage::RepoError;
use event_broker_sdk::TypedEvent;
use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use toolkit_db::secure::DBRunner;
use uuid::Uuid;

/// The toolkit outbox table family of this gear.
pub const OUTBOX_TABLE_PREFIX: &str = "bss_pricing_outbox";
/// The one queue every pricing event is enqueued on.
pub const QUEUE: &str = "bss_pricing_events";
/// The broker topic pricing publishes to.
pub const TOPIC: &str = "gts.cf.core.events.topic.v1~cf.bss.pricing.catalog.v1";
/// The producer source of every pricing event.
pub const SOURCE: &str = "bss-pricing";
/// `PriceRowsPublished` is about a book.
pub const PRICE_BOOK_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.pricing.price_book.v1";
/// `ApprovalUnitDecided` is about a unit.
pub const APPROVAL_UNIT_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.pricing.approval_unit.v1";
const CONTENT_TYPE: &str =
    "application/vnd.constructorfabric.event-broker.producer-outbox+json;version=1";

/// Holds every envelope until a broker producer is bound (a later phase).
pub struct PendingProducer;
#[async_trait::async_trait]
impl toolkit_db::outbox::LeasedMessageHandler for PendingProducer {
    async fn handle(
        &self,
        msg: &toolkit_db::outbox::OutboxMessage,
    ) -> toolkit_db::outbox::MessageResult {
        tracing::debug!(
            queue = QUEUE,
            payload_type = %msg.payload_type,
            "bss-pricing: no broker producer is bound; holding the message in the queue"
        );
        toolkit_db::outbox::MessageResult::Retry
    }
}

/// One row a `price_rows` unit approved, with the window the chain was approved with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedRow {
    pub row_id: Uuid,
    pub price_id: Uuid,
    /// The chain: the dimension value, `None` for the default chain. (A value may itself be
    /// spelled `default`, so the chain is not a string.)
    pub dim_value: Option<String>,
    /// ISO date, inclusive.
    pub effective_from: String,
    /// ISO date, exclusive; `None` is an open tail.
    pub effective_to: Option<String>,
    /// `all` or `new`.
    pub eligibility: String,
}

/// A `price_rows` unit was applied: its rows are approved in the book.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PriceRowsPublished {
    pub tenant_id: Uuid,
    pub book_id: Uuid,
    pub unit_id: Uuid,
    /// In ascending row id.
    pub rows: Vec<PublishedRow>,
    /// The principal whose act applied the unit.
    pub actor_ref: Uuid,
}
impl TypedEvent for PriceRowsPublished {
    const TYPE_ID: &'static str =
        "gts.cf.core.events.event.v1~cf.bss.pricing.price_rows_published.v1~";
    const SUBJECT_TYPE: &'static str = PRICE_BOOK_SUBJECT_TYPE;
    const SOURCE: &'static str = SOURCE;
    fn subject(&self) -> Cow<'_, str> {
        Cow::Owned(self.book_id.to_string())
    }
    fn tenant_id(&self) -> Option<Uuid> {
        Some(self.tenant_id)
    }
}

/// A unit reached a terminal state: approved (applied), rejected or withdrawn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalUnitDecided {
    pub tenant_id: Uuid,
    pub unit_id: Uuid,
    pub kind: String,
    /// The outcome: `approved`, `rejected` or `withdrawn`.
    pub state: String,
    pub generation: i32,
    /// The deciding principals: every current-generation voter and the actor, ascending.
    pub actors: Vec<Uuid>,
}
impl TypedEvent for ApprovalUnitDecided {
    const TYPE_ID: &'static str =
        "gts.cf.core.events.event.v1~cf.bss.pricing.approval_unit_decided.v1~";
    const SUBJECT_TYPE: &'static str = APPROVAL_UNIT_SUBJECT_TYPE;
    const SOURCE: &'static str = SOURCE;
    fn subject(&self) -> Cow<'_, str> {
        Cow::Owned(self.unit_id.to_string())
    }
    fn tenant_id(&self) -> Option<Uuid> {
        Some(self.tenant_id)
    }
}

/// Enqueue one event on the caller's transaction in the broker's producer-outbox envelope.
/// # Errors
/// Serialization failures, and outbox failures with the driver error kept typed so a
/// serializable transaction can retry.
pub async fn enqueue<E: TypedEvent>(
    outbox: &toolkit_db::outbox::Outbox,
    tx: &(impl DBRunner + Sync),
    event: &E,
    now: time::OffsetDateTime,
) -> Result<(), RepoError> {
    let serialize = |e: String| RepoError::Db(format!("{} event: {e}", E::TYPE_ID));
    let envelope = serde_json::json!({
        "version": 1,
        "event_id": Uuid::now_v7(),
        "type": E::TYPE_ID,
        "topic": TOPIC,
        "tenant_id": event.tenant_id(),
        "source": E::SOURCE,
        "subject": event.subject(),
        "subject_type": E::SUBJECT_TYPE,
        "occurred_at": now
            .format(&time::format_description::well_known::Rfc3339)
            .map_err(|e| serialize(e.to_string()))?,
        "trace_parent": event.trace_parent(),
        "data": serde_json::to_value(event).map_err(|e| serialize(e.to_string()))?,
        "broker_partition": 0,
        "producer_mode": "stateless",
        "diagnostic_metadata": {"sdk_client_agent": SOURCE},
    });
    outbox
        .enqueue(
            tx,
            QUEUE,
            0,
            serde_json::to_vec(&envelope).map_err(|e| serialize(e.to_string()))?,
            CONTENT_TYPE,
        )
        .await
        .map_err(|e| match e {
            toolkit_db::outbox::OutboxError::Database(source) => RepoError::Driver {
                context: format!("{} event", E::TYPE_ID),
                source,
            },
            other => RepoError::Db(other.to_string()),
        })?;
    Ok(())
}
