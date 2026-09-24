//! Toolkit outbox plumbing retained for phase 1c event producers.

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

/// Event error vocabulary; phase 1c adds the enqueue failure variants.
#[derive(Debug, thiserror::Error)]
pub enum EventsError {}
