//! @cpt-dod:cpt-cf-bss-products-dod-events-in-outbox-tx:p1
//! @cpt-dod:cpt-cf-bss-products-dod-outbox-same-tx:p1
//! Typed event enqueueing on the caller's transaction runner.
use crate::infra::broker::EventSink;
use event_broker_sdk::TypedEvent;
use toolkit_db::secure::DBRunner;

pub const OUTBOX_TABLE_PREFIX: &str = "bss_products_outbox";
pub const QUEUE_NAME: &str = "bss_products_events";
pub const PARTITIONS: u16 = 8;

/// Hold messages until a broker is available.
pub struct PendingBrokerProducer;

#[async_trait::async_trait]
impl toolkit_db::outbox::LeasedMessageHandler for PendingBrokerProducer {
    async fn handle(
        &self,
        msg: &toolkit_db::outbox::OutboxMessage,
    ) -> toolkit_db::outbox::MessageResult {
        tracing::debug!(
            queue = QUEUE_NAME,
            payload_type = %msg.payload_type,
            "bss-products: no EventBrokerApi was present at boot, so P-D-47's SDK producer \
             was not bound; holding the message in the queue"
        );
        toolkit_db::outbox::MessageResult::Retry
    }
}

/// Ambient W3C trace context, if present.
#[must_use]
pub fn traceparent() -> Option<String> {
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;

    let context = tracing::Span::current().context();
    let span = opentelemetry::trace::TraceContextExt::span(&context);
    let span_context = span.span_context();
    (span_context.trace_id() != opentelemetry::trace::TraceId::INVALID).then(|| {
        format!(
            "00-{}-{}-{:02x}",
            span_context.trace_id(),
            span_context.span_id(),
            span_context.trace_flags().to_u8()
        )
    })
}

/// Serialization or durable enqueue failure.
#[derive(Debug, thiserror::Error)]
pub enum EventsError {
    #[error("event serialization: {0}")]
    Serialize(String),
    #[error("broker producer: {0}")]
    Producer(String),
    #[error("event database: {0}")]
    Db(#[source] sea_orm::DbErr),
    #[error("interim outbox: {0}")]
    Outbox(String),
}

/// Write a typed event through the bound broker producer or the interim outbox.
/// # Errors
/// Returns serialization or enqueue failures; the caller must roll back its transaction.
#[allow(
    dead_code,
    reason = "Called by the business doors introduced in Tasks 7-10"
)]
pub(crate) async fn enqueue_typed<E: TypedEvent>(
    sink: &EventSink,
    runner: &(impl DBRunner + Sync),
    event: E,
) -> Result<(), EventsError> {
    match sink {
        // SDK ProducerOutbox::enqueue currently erases OutboxError into
        // EventBrokerError::Internal(String), exposing no DbErr/source to recover.
        EventSink::Broker(producer) => producer
            .enqueue(runner, event)
            .await
            .map(|_| ())
            .map_err(EventsError::from),
        EventSink::Interim(outbox) => {
            // The SDK constructor is crate-private and DbProducer::outbox_envelope
            // needs a prepared broker/schema cache. Persist its v1 wire format here.
            // Stateless backlog needs no producer registration; the later processor
            // honors the mode in each envelope and publishes this durable event id.
            let envelope = serde_json::json!({
                "version": 1,
                "event_id": uuid::Uuid::now_v7(),
                "type": E::TYPE_ID,
                "topic": crate::infra::broker::TOPIC,
                "tenant_id": event.tenant_id(),
                "source": E::SOURCE,
                "subject": event.subject(),
                "subject_type": E::SUBJECT_TYPE,
                "occurred_at": time::OffsetDateTime::now_utc().format(&time::format_description::well_known::Rfc3339).map_err(|e| EventsError::Serialize(e.to_string()))?,
                "trace_parent": event.trace_parent(),
                "data": serde_json::to_value(&event).map_err(|e| EventsError::Serialize(e.to_string()))?,
                "broker_partition": 0,
                "producer_mode": "stateless",
                "diagnostic_metadata": {"sdk_client_agent": crate::infra::broker::SOURCE}
            });
            let payload =
                serde_json::to_vec(&envelope).map_err(|e| EventsError::Serialize(e.to_string()))?;
            let partition = event.tenant_id().map_or(0, |t| {
                u32::from(u16::from_le_bytes([t.as_bytes()[14], t.as_bytes()[15]]) % PARTITIONS)
            });
            outbox
                .enqueue(
                    runner,
                    QUEUE_NAME,
                    partition,
                    payload,
                    "application/vnd.constructorfabric.event-broker.producer-outbox+json;version=1",
                )
                .await
                .map(|_| ())
                .map_err(EventsError::from)
        }
    }
}

impl From<toolkit_db::outbox::OutboxError> for EventsError {
    fn from(error: toolkit_db::outbox::OutboxError) -> Self {
        match error {
            toolkit_db::outbox::OutboxError::Database(source) => Self::Db(source),
            other => Self::Outbox(other.to_string()),
        }
    }
}
impl From<EventsError> for bss_approval::ApprovalError {
    fn from(error: EventsError) -> Self {
        match error {
            EventsError::Db(source) => Self::Db(source),
            other => Self::Store(other.to_string()),
        }
    }
}

impl From<event_broker_sdk::EventBrokerError> for EventsError {
    fn from(error: event_broker_sdk::EventBrokerError) -> Self {
        // Recover any typed source the SDK does expose. Its local enqueue
        // Internal(String) case has already lost the source and stays opaque.
        let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(&error);
        while let Some(error) = cause {
            if let Some(db) = error.downcast_ref::<sea_orm::DbErr>() {
                return Self::Db(db.clone());
            }
            // SDK offset-manager errors store their source in an Arc. Walking
            // Arc::source directly would skip the wrapped error itself.
            cause = error
                .downcast_ref::<std::sync::Arc<dyn std::error::Error + Send + Sync>>()
                .map_or_else(|| error.source(), |shared| Some(shared.as_ref()));
        }
        Self::Producer(error.to_string())
    }
}
