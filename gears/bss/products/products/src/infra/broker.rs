//! The gear's events as the broker SDK's own typed events (P-D-47): `01`
//! §4.5's eight, `04-lifecycle`'s announced deprecation pair, and
//! `03-sku-classification`'s set trio (P-D-94's broker identity).
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
//! # Nothing registers `EventBrokerApi` yet, so this arm is inert today
//!
//! Measured 2026-08-30 over the whole workspace: the **only**
//! `register::<dyn EventBrokerApi>` anywhere is in this module's own test. The
//! `event-broker` gear's `init` registers no client, and the platform's
//! convention is that a provider registers itself in its own `init`
//! (`authz-resolver`'s does). The runtime's `run_init_phase()` also completes
//! before `run_proxy_wiring_phase()`, so `#[toolkit::consumes]` proxy wiring
//! cannot fill this slot either — it fires after every `init` has returned.
//!
//! **So every real deployment takes the interim arm today**, and the producer
//! below is exercised only by `broker_tests`. Two consequences worth stating
//! rather than discovering: this gear declares no `deps` edge to the broker
//! gear (it cannot — the dependency is optional), so when a provider does
//! appear the init order is the topological sort's tie-break rather than a
//! declared edge; and until then "no broker configured" and "the broker gear
//! booted after us" are the same observation. `ProductsConfig::require_broker`
//! exists so an operator can make the second one fatal.
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
//!   the subject types carry those names, and the event types carry the
//!   gear's own tokens — §4.5's eight, 04's pair and 03's trio, the last
//!   deriving its subject from `cf.bss.products.recognized_set.v1~`, a GTS
//!   type `05` §3.2's authz catalog declares (P-D-94 arm 2);
//! - **one topic, not one per event.** P-D-27's ordering key is `(tenant, aggregate)`,
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
//!
//! # That last part is a third deviation, and it is recorded here
//!
//! P-D-01 calls its obligations envelope-agnostic, but two other documents
//! **place** them, and both put these two on the envelope rather than in the
//! payload. `design/01-foundation.md` §4.4's **Payloads** bullet reads
//! *"broker-native envelope (P-D-01) with versioned schema ref,
//! correlation/causation, idempotency key (the event `id`, P-D-47) and
//! `actor_ref`; **in the payload** body core (§4.5) …"* — it draws the line
//! explicitly and leaves `actor_ref` on the envelope side of it. And
//! `dod-outbox-eventing` reads *"The **envelope** MUST carry correlation and
//! causation ids, a versioned schema reference, and the acting principal's
//! `actor_ref`."* The interim [`crate::infra::events::EventEnvelope`] obeys
//! both; this arm cannot, because the broker's `Event` has no slot for either
//! value and the SDK owns that struct.
//!
//! So **as built, the broker arm does not satisfy the `DoD`'s envelope clause as
//! written**, and that is a third deviation rather than a reading of P-D-01.
//! The other two — the `ClientHub` fork and the derived GTS ids — were put to
//! the owner; this one was not, and it is registered here so the `DoD` is not
//! ticked against a clause the code cannot meet. Reconciling it means moving
//! either §4.4's placement or the `DoD`'s wording, the third option — a slot on
//! the broker's `Event` — not being this gear's to add.

use std::borrow::Cow;
use std::sync::Arc;

use event_broker_sdk::{ProducerOutbox, ProducerOutboxHandle, TypedEvent};
use serde::{Deserialize, Serialize};
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// The one topic every event of this gear publishes onto.
pub(crate) const TOPIC: &str = "gts.cf.core.events.topic.v1~cf.bss.products.catalog.v1";

/// The `source` every event of this gear carries: the gear's own name, the
/// same string `#[toolkit::gear(name = ...)]` registers it under.
pub(crate) const SOURCE: &str = "bss-products";

/// The subject type for an event about a Product.
pub(crate) const PRODUCT_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.product.v1";

/// The subject type for an event about a SKU.
pub(crate) const SKU_SUBJECT_TYPE: &str = "gts.cf.core.events.subject.v1~cf.bss.products.sku.v1";

/// The subject type for an event about a recognized set — the subject is the
/// `set_kind`, which is P-D-27's ordering read for these events: `design/03`
/// §4 keys them `(tenant, set_kind)`, and tenant plus subject is exactly that
/// pair. The domain type it derives from is `cf.bss.products.recognized_set.v1~`,
/// declared by 05 §3.2's authz catalog and doored by P-D-90 (P-D-94 records
/// the derivation).
pub(crate) const RECOGNIZED_SET_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.recognized_set.v1";

/// §4.5's five body-core fields plus P-D-01's two payload-borne obligations, owned.
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
    /// the correlation id: an operator request causes every one of these, and
    /// a request is not an event.
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
    /// `causation_id` is `None` for every one: an operator request causes them,
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

            /// The request's W3C `traceparent`, where there is one
            /// ([`crate::infra::events::traceparent`]) — P-D-47 maps the
            /// correlation obligation onto this field.
            ///
            /// **`partition_key` is not overridden at all**, and that absence
            /// is load-bearing: P-D-47 says *"the gear sets no `partition_key`,
            /// so ADR-0002's default puts every event of one tenant on one
            /// partition in publish order"*. Setting one is the named amendment
            /// path, not a tuning knob. The argument used to be written here,
            /// on `trace_parent`, where rustdoc rendered it as this function
            /// doing nothing.
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

/// A `*Deprecated` event: the core plus the deprecation's **cause**.
///
/// A third macro rather than a field on [`catalog_event`]'s expansion,
/// because the cause is not a field every event of this gear carries —
/// exactly the argument [`catalog_publish_event`] makes for
/// `published_version`. `dod-deprecation-provenance` requires the provenance
/// *"in its payload"*, and `design/04` makes the consumer's own reaction
/// depend on it (pricing AC #82's new-adoption block), so it rides the
/// announcement rather than obliging a consumer to re-read a head that may
/// have moved again.
macro_rules! catalog_deprecation_event {
    ($(#[$doc:meta])* $name:ident, $type_id:literal, $subject_type:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(crate) struct $name {
            /// §4.5's five fields plus P-D-01's two payload-borne obligations.
            #[serde(flatten)]
            pub core: CatalogEventCore,
            /// `direct` or `cascaded` — written to the row in the very
            /// statement this event announces.
            pub provenance: String,
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

/// A recognized-set membership event: which set, which member, which state
/// it now carries (`design/03` §4's roster — `RecognizedUnitUpdated`,
/// `RecognizedCodeUpdated`, `PlanTierUpdated`).
///
/// Its own macro because the body is set-shaped, not entity-shaped: there is
/// no entity id, no revision and no lifecycle state — the subject is the
/// `set_kind` and the payload names the member. `actor_ref` rides the
/// payload for [`CatalogEventCore`]'s reason: the broker's `Event` has no
/// field for an actor.
macro_rules! set_event {
    ($(#[$doc:meta])* $name:ident, $type_id:literal) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(crate) struct $name {
            /// The owning tenant — the ordering key's first half.
            pub tenant_id: Uuid,
            /// The set — the ordering key's second half and the subject.
            pub set_kind: String,
            /// The member the mutation touched.
            pub member_code: String,
            /// The member's state as this mutation committed it.
            pub state: String,
            /// The acting principal, pseudonymously.
            pub actor_ref: Uuid,
        }

        impl TypedEvent for $name {
            const TYPE_ID: &'static str = $type_id;
            const TOPIC: &'static str = TOPIC;
            const SUBJECT_TYPE: &'static str = RECOGNIZED_SET_SUBJECT_TYPE;
            const SOURCE: &'static str = SOURCE;

            fn subject(&self) -> Cow<'_, str> {
                Cow::Borrowed(&self.set_kind)
            }

            /// The entity's tenant, not the producer's — the catalog events'
            /// own override, for the same partitioning reason.
            fn tenant_id(&self) -> Option<Uuid> {
                Some(self.tenant_id)
            }

            fn trace_parent(&self) -> Option<Cow<'_, str>> {
                crate::infra::events::traceparent().map(Cow::Owned)
            }
        }
    };
}

set_event! {
    /// The metering-unit set moved (`design/03` §4).
    RecognizedUnitUpdated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.recognized_unit_updated.v1"
}
set_event! {
    /// A tax-category or GL-code set moved.
    RecognizedCodeUpdated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.recognized_code_updated.v1"
}
set_event! {
    /// The plan-tier taxonomy moved — PRD-named, its own event by design.
    PlanTierUpdated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.plan_tier_updated.v1"
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
catalog_deprecation_event! {
    /// A Product was deprecated, with [`Self::provenance`] naming the cause.
    ProductDeprecated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.product_deprecated.v1",
    PRODUCT_SUBJECT_TYPE
}
catalog_deprecation_event! {
    /// A SKU was deprecated — the event pricing AC #82 keys on.
    SkuDeprecated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.sku_deprecated.v1",
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

/// Prepare each declared event type by name, so a registration missing any
/// one of them fails the boot rather than a door's transaction.
///
/// `DbProducer::prepare_all` resolves the declared *patterns* and errors only on
/// an empty match, which is a different claim. This asks, one per declared
/// event, exactly the questions the
/// gear actually needs answered.
///
/// # Errors
/// `EventBrokerError::SchemaNotPrepared` (or the SDK's own lookup error) naming
/// the first declared event the broker does not carry.
async fn prepare_every_event_type(
    producer: &event_broker_sdk::DbProducer,
) -> Result<(), event_broker_sdk::EventBrokerError> {
    producer.prepare::<ProductCreated>().await?;
    producer.prepare::<SkuCreated>().await?;
    producer.prepare::<ProductHeadSaved>().await?;
    producer.prepare::<SkuHeadSaved>().await?;
    producer.prepare::<ProductPublished>().await?;
    producer.prepare::<SkuPublished>().await?;
    producer.prepare::<ProductDiscarded>().await?;
    producer.prepare::<SkuDiscarded>().await?;
    producer.prepare::<ProductDeprecated>().await?;
    producer.prepare::<SkuDeprecated>().await?;
    producer.prepare::<RecognizedUnitUpdated>().await?;
    producer.prepare::<RecognizedCodeUpdated>().await?;
    producer.prepare::<PlanTierUpdated>().await?;
    Ok(())
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
        .event_type_patterns(["gts.cf.core.events.event_type.v1~cf.bss.products.*"])
        .prepare_all()
        .await?;

    // **`prepare_all` is not "every event".** Its schema cache errors only when the
    // declared patterns match **zero** event types
    // (`producer/schema_cache.rs`: `if selected.is_empty()`), so a broker
    // carrying one of this gear's thirteen lets the boot succeed and log
    // "publishing through the event-broker SDK producer" — and the other twelve
    // then fail at `outbox_envelope`'s `validate_prepared`, inside a door's own
    // transaction, on a live request. That is the "half-configured broker" case
    // this module's doc says is loud at bind time; it was not, until here.
    prepare_every_event_type(&producer).await?;

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
