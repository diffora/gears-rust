//! The eight Foundation events as the broker SDK's own typed events (P-D-47).
//!
//! `infra::events` writes the interim envelope this gear controls; this module
//! is what replaces it once a broker is reachable. The two coexist on purpose:
//! `Gear::init` binds the SDK producer when `ClientHub` carries an
//! `EventBrokerApi` and falls back to `events::PendingBrokerProducer` when it
//! does not, so a deployment with no event-broker still boots. That fallback is
//! a **deviation from P-D-47 as written** — the decision says the processor *is*
//! the SDK's producer, with no "when configured" clause — taken on the owner's
//! call and safe by construction: the holding processor answers `Retry` and
//! never reports a delivery.
//!
//! # Where the three GTS ids come from
//!
//! `TypedEvent` demands `TYPE_ID`, `TOPIC` and `SUBJECT_TYPE` as compile-time
//! constants, and **no document in this gear's design set declares any of the
//! three**. `design/01-foundation.md` §6 item 12 registers one of them as an
//! open owner question in as many words: *"Which GTS type does the envelope's
//! `subject_type` name for a Product or a SKU?"*. They are derived here, on the
//! owner's call, from what **is** declared, and the derivation is recorded so a
//! reader can check it rather than take it:
//!
//! - the **shapes** are the platform's, not invented. The workspace's real ids
//!   are `gts.cf.core.events.topic.v1~<name>`,
//!   `gts.cf.core.events.event_type.v1~<name>` and
//!   `gts.cf.core.events.subject.v1~<name>`, and the `<name>` convention in
//!   platform-owned ids is `cf.<domain>.<thing>.v1` (`cf.core.oagw.http.v1`).
//!
//!   **`event_type.v1~`, not `event.v1~`,** and the distinction is the broker's
//!   own: `event-broker/src/domain/model.rs` documents
//!   `gts.cf.core.events.event_type.v1~` as *"schema and constraints for one"*
//!   event type and `gts.cf.core.events.event.v1~` as *"an immutable record in
//!   a"* stream. [`TypedEvent::TYPE_ID`] names a **type**, so it takes the
//!   former. The SDK's own `TypedEvent` doc-example uses the latter and is
//!   misleading on this point; the broker's model is the authority, and the
//!   SDK's `api_tests` matches event-type *patterns* against `event_type.v1~`
//!   ids;
//! - the **names** are this set's own. `DESIGN.md` declares six domain GTS
//!   types, two of which are the entities these events are about —
//!   `gts.cf.bss.products.product.v1~` and `gts.cf.bss.products.sku.v1~` — so
//!   the subject types carry those names, and the event types carry §4.5's own
//!   eight tokens;
//! - **one topic, not eight.** P-D-27's ordering key is `(tenant, aggregate)`,
//!   not `(tenant, aggregate, entity_kind)`; splitting the topic would not
//!   change the partitioning, only what a consumer subscribes to. This is the
//!   same argument [`crate::infra::events::QUEUE_NAME`] already carries for the
//!   toolkit queue.
//!
//! **What is still owed**: none of the three is a *registration*. A topic and
//! an event type are broker-side resources, and `subject_type` is checked at
//! ingest against the `allowed_subject_types` list registered **with the event
//! type** — so these constants are one half of an agreement whose other half
//! this gear does not own. Until the event types are registered under these
//! ids, a `prepare_all()` against a live broker fails, and that failure is the
//! one this module wants to be loud rather than silent.
//!
//! # What the payload carries, and what left it
//!
//! Three fields of `events::EventEnvelope` do not appear below, because the
//! broker's own envelope owns them (P-D-47): the event `id` — the SDK mints it
//! at enqueue and repeats it on every delivery attempt, which is what makes it
//! the idempotency key — the schema reference, which is [`TypedEvent::TYPE_ID`],
//! and the correlation id, which rides `trace_parent`. What stays in the
//! payload is what P-D-01 obliges and the broker's `Event` has no field for: a
//! causation id and the pseudonymous `actor_ref`.

use std::borrow::Cow;
use std::sync::Arc;

use event_broker_sdk::{ProducerOutbox, ProducerOutboxHandle, TypedEvent};
use serde::{Deserialize, Serialize};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// The one topic all eight events publish onto.
pub(crate) const TOPIC: &str = "gts.cf.core.events.topic.v1~cf.bss.products.catalog.v1";

/// The `source` every event of this gear carries: the gear's own name, the
/// same string `#[toolkit::gear(name = ...)]` registers it under.
pub(crate) const SOURCE: &str = "bss-products";

/// The subject type for an event about a Product.
pub(crate) const PRODUCT_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.product.v1";

/// The subject type for an event about a SKU.
pub(crate) const SKU_SUBJECT_TYPE: &str = "gts.cf.core.events.subject.v1~cf.bss.products.sku.v1";

/// §4.5's five fields, owned.
///
/// Owned rather than borrowed because [`TypedEvent`] requires
/// `DeserializeOwned`: a consumer deserializes this, and a `&'static str` has
/// no owned counterpart to deserialize into. `events::EventBodyCore` keeps the
/// borrowed shape for the interim envelope, which is only ever serialized.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogEventCore {
    /// Owning tenant, and — since [`TypedEvent::tenant_id`] returns it — the
    /// broker's partition input under ADR-0002's default.
    pub tenant_id: Uuid,
    /// `"product"` or `"sku"`, matching `events::EntityKind::as_str`.
    pub entity_kind: String,
    /// The entity this event is about; also its `subject`.
    pub entity_id: Uuid,
    /// The revision **as committed by the act** (P-D-29).
    pub internal_revision: i64,
    /// The head's state after the act.
    pub lifecycle_state: String,
    /// P-D-01's causation half. Absent rather than null, and never an echo of
    /// the correlation id: an operator request causes these eight, and a
    /// request is not an event.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<String>,
    /// The acting principal, pseudonymously. The broker's `Event` has no field
    /// for an actor, which is why this obligation is discharged in the payload.
    pub actor_ref: Uuid,
}

impl CatalogEventCore {
    /// Owned copy of the interim envelope's borrowed core, plus the two
    /// obligations the broker's `Event` has no field for.
    ///
    /// `causation_id` is `None` for all eight: an operator request causes them,
    /// and a request is not an event. The field exists so a later slice that
    /// emits an event *caused by* another has somewhere to put it.
    pub(crate) fn from_core(core: &crate::infra::events::EventBodyCore, actor_ref: Uuid) -> Self {
        Self {
            tenant_id: core.tenant_id,
            entity_kind: core.entity_kind.to_owned(),
            entity_id: core.entity_id,
            internal_revision: core.internal_revision,
            lifecycle_state: core.lifecycle_state.to_owned(),
            causation_id: None,
            actor_ref,
        }
    }
}

macro_rules! catalog_event {
    ($(#[$doc:meta])* $name:ident, $type_id:literal, $subject_type:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(crate) struct $name {
            /// §4.5's five fields plus P-D-01's two payload-borne obligations.
            #[serde(flatten)]
            pub core: CatalogEventCore,
        }

        impl TypedEvent for $name {
            const TYPE_ID: &'static str = $type_id;
            const TOPIC: &'static str = TOPIC;
            const SUBJECT_TYPE: &'static str = $subject_type;
            const SOURCE: &'static str = SOURCE;

            fn subject(&self) -> Cow<'_, str> {
                Cow::Owned(self.core.entity_id.to_string())
            }

            /// The **entity's** tenant, not the producer's.
            ///
            /// Left as `None` the SDK would use the authenticated
            /// security-context tenant — this gear's own service identity — and
            /// every event of every tenant would hash to one partition,
            /// destroying exactly the per-tenant ordering P-D-47 relies on.
            fn tenant_id(&self) -> Option<Uuid> {
                Some(self.core.tenant_id)
            }

            /// Deliberately **not** overridden beyond the default `None`
            /// ([`partition_key`](TypedEvent::partition_key) is inherited).
            /// P-D-47: *"the gear sets no `partition_key`, so ADR-0002's default
            /// puts every event of one tenant on one partition in publish
            /// order"*. Setting one here is the named amendment path, not a
            /// tuning knob.
            fn trace_parent(&self) -> Option<Cow<'_, str>> {
                crate::infra::events::traceparent().map(Cow::Owned)
            }
        }
    };
}

macro_rules! catalog_publish_event {
    ($(#[$doc:meta])* $name:ident, $type_id:literal, $subject_type:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(crate) struct $name {
            /// §4.5's five fields plus P-D-01's two payload-borne obligations.
            #[serde(flatten)]
            pub core: CatalogEventCore,
            /// §4.5's "additionally carry": the **post-act** version, the key
            /// the frozen row was written at.
            pub published_version: i64,
        }

        impl TypedEvent for $name {
            const TYPE_ID: &'static str = $type_id;
            const TOPIC: &'static str = TOPIC;
            const SUBJECT_TYPE: &'static str = $subject_type;
            const SOURCE: &'static str = SOURCE;

            fn subject(&self) -> Cow<'_, str> {
                Cow::Owned(self.core.entity_id.to_string())
            }

            /// See the core-only twin: the entity's tenant, not the producer's.
            fn tenant_id(&self) -> Option<Uuid> {
                Some(self.core.tenant_id)
            }

            fn trace_parent(&self) -> Option<Cow<'_, str>> {
                crate::infra::events::traceparent().map(Cow::Owned)
            }
        }
    };
}

catalog_event! {
    /// A Product row was created.
    ProductCreated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.product_created.v1",
    PRODUCT_SUBJECT_TYPE
}
catalog_event! {
    /// A SKU row was created.
    SkuCreated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.sku_created.v1",
    SKU_SUBJECT_TYPE
}
catalog_event! {
    /// A Product head was saved.
    ProductHeadSaved,
    "gts.cf.core.events.event_type.v1~cf.bss.products.product_head_saved.v1",
    PRODUCT_SUBJECT_TYPE
}
catalog_event! {
    /// A SKU head was saved.
    SkuHeadSaved,
    "gts.cf.core.events.event_type.v1~cf.bss.products.sku_head_saved.v1",
    SKU_SUBJECT_TYPE
}
catalog_event! {
    /// A Product was discarded.
    ProductDiscarded,
    "gts.cf.core.events.event_type.v1~cf.bss.products.product_discarded.v1",
    PRODUCT_SUBJECT_TYPE
}
catalog_event! {
    /// A SKU was discarded.
    SkuDiscarded,
    "gts.cf.core.events.event_type.v1~cf.bss.products.sku_discarded.v1",
    SKU_SUBJECT_TYPE
}
catalog_publish_event! {
    /// A Product was published at [`Self::published_version`].
    ProductPublished,
    "gts.cf.core.events.event_type.v1~cf.bss.products.product_published.v1",
    PRODUCT_SUBJECT_TYPE
}
catalog_publish_event! {
    /// A SKU was published at [`Self::published_version`].
    SkuPublished,
    "gts.cf.core.events.event_type.v1~cf.bss.products.sku_published.v1",
    SKU_SUBJECT_TYPE
}

#[cfg(test)]
#[path = "broker_tests.rs"]
mod broker_tests;

/// The gear's own identity when it talks to the broker at `init()`.
///
/// Shaped on `gears/system/account-management`'s `domain::system_actor`, which
/// states the argument this file follows: a system actor is minted by a
/// **named factory per call site**, never by a crate-wide constructor, so
/// `grep` answers "where does this gear elevate?" in one search and a new site
/// is a review magnet. This gear has exactly one such site, and it is private
/// to this module rather than `pub(crate)`.
///
/// Platform-scoped: the producer registration pre-dates any tenant, so the
/// tenant binding is the platform-root sentinel, exactly as the donor's own
/// `for_gear_init` does. The per-event tenant is carried by
/// [`TypedEvent::tenant_id`] instead, which is what the broker partitions on.
///
/// The actor `UUID` is hand-picked with a zero version nibble so it cannot
/// collide with a `v4` or `v5` actor id, and is stable across processes so an
/// audit sink can correlate every producer registration under one identity —
/// the donor's reasoning, taken rather than re-derived.
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

/// Which pipeline a door's enqueue reaches.
///
/// One value, decided once at `Gear::init` and carried on
/// `api::rest::ApiState`, so no door has to know which of the two is live —
/// and so a door cannot accidentally reach the wrong one.
#[derive(Clone)]
pub(crate) enum EventSink {
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

/// Build the SDK producer and bind its queue, or answer `None` when the
/// platform has no broker to talk to.
///
/// `None` on a `ClientHub` with no `EventBrokerApi` is the **only** silent
/// path: it is the configured-out case, and the caller logs it. Every other
/// failure — a broker that is there but refuses the registration, an event type
/// this gear's ids do not match, an outbox that will not start — is an `Err`,
/// because a half-configured broker is an operator's mistake and must not
/// degrade quietly into the interim envelope.
///
/// # Errors
/// Any failure of `DbProducer::prepare_all` (registration, event-type
/// validation) or of the producer outbox's own start.
pub(crate) async fn bind_producer(
    hub: &toolkit::client_hub::ClientHub,
    db: toolkit_db::Db,
    table_prefix: &str,
    partitions: toolkit_db::outbox::Partitions,
) -> anyhow::Result<Option<(EventSink, ProducerOutboxHandle)>> {
    let Ok(broker) = hub.get::<dyn event_broker_sdk::EventBrokerApi>() else {
        return Ok(None);
    };

    let producer = event_broker_sdk::DbProducer::builder()
        .broker(broker)
        .db(db.clone())
        .security_context(producer_system_actor())
        .identity(
            event_broker_sdk::ProducerIdentity::new()
                .source(SOURCE)
                .client_agent(concat!("bss-products/", env!("CARGO_PKG_VERSION"))),
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
        .event_type_patterns(["gts.cf.core.events.event_type.v1~cf.bss.products.*"])
        .prepare_all()
        .await?;

    // The queue name is the table prefix's own, so the producer's queue and
    // this gear's tables are named from one constant.
    let queue = producer.outbox_queue(table_prefix, partitions)?;
    let handle = queue
        .start(toolkit_db::outbox::Outbox::builder(db).table_prefix(table_prefix)?)
        .await?;
    let sink = EventSink::Broker(Box::new(handle.outbox().clone()));
    Ok(Some((sink, handle)))
}
