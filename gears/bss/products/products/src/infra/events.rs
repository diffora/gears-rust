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
        EventSink::Broker(producer) => producer
            .enqueue(runner, event)
            .await
            .map(|_| ())
            .map_err(|e| EventsError::Producer(e.to_string())),
        EventSink::Interim(outbox) => {
            let payload =
                serde_json::to_vec(&event).map_err(|e| EventsError::Serialize(e.to_string()))?;
            let partition = event.tenant_id().map_or(0, |t| {
                u32::from(u16::from_le_bytes([t.as_bytes()[14], t.as_bytes()[15]]) % PARTITIONS)
            });
            outbox
                .enqueue(runner, QUEUE_NAME, partition, payload, E::TYPE_ID)
                .await
                .map(|_| ())
                .map_err(|e| EventsError::Outbox(e.to_string()))
        }
    }
}
