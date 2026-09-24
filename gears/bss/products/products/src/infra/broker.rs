//! Broker binding and outbox sink; typed registry events return in phase 1c.
use event_broker_sdk::{ProducerOutbox, ProducerOutboxHandle};
use std::sync::Arc;
use toolkit_security::SecurityContext;
use uuid::Uuid;

pub const TOPIC: &str = "gts.cf.core.events.topic.v1~cf.bss.products.catalog.v1";
pub const SOURCE: &str = "bss-products";
pub const SKU_SUBJECT_TYPE: &str = "gts.cf.core.events.subject.v1~cf.bss.products.sku.v1";

fn producer_system_actor() -> SecurityContext {
    /// Hand-picked, version nibble `0`. `62 73 73 70` is `bssp`.
    const PRODUCER_ACTOR: Uuid = uuid::uuid!("00000000-0000-0f01-0000-627373702d70");
    /// The subject type an `AuthZ` policy may key on to route this gear's
    /// producer traffic separately from tenant traffic.
    const SUBJECT_TYPE: &str = "bss-products.system";

    tracing::info!(
        target: "bss_products.system_actor",
        site = "broker_producer",
        "bss-products system actor constructed"
    );
    #[allow(clippy::expect_used)]
    SecurityContext::builder()
        .subject_id(PRODUCER_ACTOR)
        .subject_type(SUBJECT_TYPE)
        .subject_tenant_id(Uuid::nil())
        .build()
        .expect("both required builder fields are set unconditionally above")
}

/// The pipeline a route enqueues into.
#[derive(Clone)]
pub enum EventSink {
    /// **P-D-47's own shape.** The toolkit outbox whose processor is the SDK's
    /// producer; the envelope is the broker's and the id is the SDK's.
    Broker(Box<ProducerOutbox>),
    /// The interim envelope on this gear's own toolkit queue, held by
    /// [`crate::infra::events::PendingBrokerProducer`] and never delivered.
    ///
    /// The fallback for a deployment whose `ClientHub` carries no
    /// `EventBrokerApi`. Safe by construction: the holding processor answers
    /// `Retry` to every message, so nothing is ever reported delivered and the
    /// `dod-outbox-eventing` clause *"emission success MUST NOT be reported
    /// before the event is durably accepted"* holds trivially — nothing is
    /// reported at all.
    Interim(Arc<toolkit_db::outbox::Outbox>),
}

pub(crate) async fn bind_producer(
    hub: &toolkit::client_hub::ClientHub,
    db: toolkit_db::Db,
    table_prefix: &str,
    partitions: toolkit_db::outbox::Partitions,
) -> anyhow::Result<Option<(EventSink, ProducerOutboxHandle)>> {
    let broker = match hub.get::<dyn event_broker_sdk::EventBrokerApi>() {
        Ok(broker) => broker,
        // The configured-out case, and the only silent one.
        Err(toolkit::client_hub::ClientHubError::NotFound { .. }) => return Ok(None),
        // A registration that is there but wrong is an operator's mistake, not a
        // deployment without a broker. Collapsing the two — which a
        // `let ... else` does — is the degradation this function's doc forbids.
        Err(other) => {
            return Err(anyhow::Error::new(other)
                .context("bss-products: the ClientHub holds an unusable EventBrokerApi"));
        }
    };

    let producer = event_broker_sdk::DbProducer::builder()
        .broker(broker)
        .db(db.clone())
        .security_context(producer_system_actor())
        .identity(
            event_broker_sdk::ProducerIdentity::new()
                .source(SOURCE)
                // **No version here.** `ProducerRegistration::validate_matches`
                // compares the stored `client_agent` against the supplied one and
                // returns `InvalidProducerOptions` on any difference — before
                // `on_missing`/`on_unknown` are consulted, so neither policy is an
                // escape. A version in this string would make an ordinary
                // `version = "0.1.0"` in the manifest an unbootable gear against
                // an existing registration row, recoverable only by hand-editing
                // the SDK's table. The crate is at `0.0.0` today, so the version
                // carries no diagnostic value either.
                .client_agent(SOURCE),
        )
        // **Monotonic, not chained** (P-D-47: "managed monotonic mode"). The
        // toolkit outbox's `seq` is the durable local sequence the chain's
        // `meta.sequence` is built from, write-only, for ingest-side dedup.
        .deduplication(
            event_broker_sdk::DbDeduplication::managed(event_broker_sdk::ProducerMode::Monotonic)
                .key(SOURCE)
                .on_missing(event_broker_sdk::MissingProducerRegistration::RegisterNew)
                .on_unknown(event_broker_sdk::UnknownProducerRegistration::RegisterNew)
                .build()?,
        )
        .topics([TOPIC])
        .event_type_patterns(["gts.cf.core.events.event.v1~cf.bss.products.*"])
        .prepare_all()
        .await?;

    // No typed events are prepared in phase 1b; phase 1c adds its four events.

    // The queue name is the table prefix's own, so the producer's queue and
    // this gear's tables are named from one constant.
    // **`QUEUE_NAME`, not the table prefix.** `outbox_queue`'s first argument is
    // the *queue name*, and passing the prefix here gave the two arms two
    // different queue names over one table family — so rows an interim boot had
    // accumulated under `QUEUE_NAME` had no processor once a broker appeared,
    // and stopped moving. They are not lost (`Dialect::vacuum_cleanup` is scoped
    // by `partition_id`, which is per queue), but they are stranded, and the
    // boot is green either way. One name across both arms is what makes an arm
    // switch survivable.
    let queue = producer.outbox_queue(crate::infra::events::QUEUE_NAME, partitions)?;
    let handle = queue
        .start(toolkit_db::outbox::Outbox::builder(db).table_prefix(table_prefix)?)
        .await?;
    let sink = EventSink::Broker(Box::new(handle.outbox().clone()));
    Ok(Some((sink, handle)))
}
