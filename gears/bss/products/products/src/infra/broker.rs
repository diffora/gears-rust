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

/// `07`'s producer set, the subject of `ReferenceProducerSetChanged` — one
/// per tenant (**P-D-71**), so the subject value is the tenant id.
pub(crate) const REFERENCE_PRODUCER_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.reference_producer.v1";

/// `06`'s catalog version — the subject of `CatalogVersionPublished` and
/// `FreezeForceCompleted` (**P-D-125** row 47, P-D-94's naming rule).
pub(crate) const CATALOG_VERSION_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.catalog_version.v1";
/// `06`'s freeze-participant set — the subject of `FreezeParticipantSetChanged`.
pub(crate) const FREEZE_PARTICIPANT_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.freeze_participant.v1";

/// The subject type for the batch-completion summary — the subject is the
/// batch id. Derived from `cf.bss.products.bulk.v1~`, the GTS type `05`
/// §3.2's authz catalog declares for the bulk grants (P-D-94's derivation
/// rule, applied a second time).
pub(crate) const BULK_SUBJECT_TYPE: &str = "gts.cf.core.events.subject.v1~cf.bss.products.bulk.v1";

/// The subject type for an event about a category — derived from
/// `cf.bss.products.category.v1~`, the GTS type `05` §3.2's authz catalog
/// declares for the category grants P-D-106 doored (P-D-94's derivation
/// rule, applied as for the set and bulk types; **P-D-122** records it).
pub(crate) const CATEGORY_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.category.v1";

/// The subject type for an event about an attribute definition — derived
/// from `cf.bss.products.attribute_definition.v1~` the same way.
pub(crate) const ATTRIBUTE_DEFINITION_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.attribute_definition.v1";

/// The subject type for `MetadataUpdated` — derived from
/// `cf.bss.products.metadata.v1~`, the grant's own type. The **subject** is
/// the owning entity's id and the payload's `entityKind` says which table;
/// a `TypedEvent`'s subject type is a constant, so it cannot follow the
/// owner's kind, and the metadata map is the resource the act is on
/// (**P-D-122**).
pub(crate) const METADATA_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.metadata.v1";

/// The subject type for `ActorErased` — derived from
/// `cf.bss.products.erasure.v1~`, the GTS type `05` §3.2's authz catalog
/// declares for `erasure × execute` (**P-D-94**'s derivation rule, the same
/// application P-D-122 recorded for the taxonomy's three). The **subject** is
/// the retired `actor_ref`: the pseudonym is the only identifier this event
/// may carry, which is the whole of why it exists.
pub(crate) const ERASURE_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.erasure.v1";

/// The subject type for `PiiAllowlistChanged` — derived from
/// `cf.bss.products.pii_allowlist.v1~`, the type declared for
/// `pii_allowlist × write`. The subject is the **entry**, which is also the
/// aggregate the act serializes on (**P-D-118** item 26).
pub(crate) const PII_ALLOWLIST_SUBJECT_TYPE: &str =
    "gts.cf.core.events.subject.v1~cf.bss.products.pii_allowlist.v1";

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

/// `07`'s correction, announced from the correcting transaction
/// (`dod-reference-events`; P-D-147): the core plus the corrected field, its
/// value, the lane and `quorumReduced` (**P-D-13**).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkuImmutableFieldCorrected {
    #[serde(flatten)]
    pub core: CatalogEventCore,
    pub field: String,
    pub value: Option<String>,
    pub lane: String,
    pub quorum_reduced: bool,
    pub correction_ref: Uuid,
}

impl TypedEvent for SkuImmutableFieldCorrected {
    const TYPE_ID: &'static str =
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_immutable_field_corrected.v1";
    const TOPIC: &'static str = TOPIC;
    const SUBJECT_TYPE: &'static str = SKU_SUBJECT_TYPE;
    const SOURCE: &'static str = SOURCE;

    fn subject(&self) -> Cow<'_, str> {
        Cow::Owned(self.core.entity_id.to_string())
    }

    fn tenant_id(&self) -> Option<Uuid> {
        Some(self.core.tenant_id)
    }

    fn trace_parent(&self) -> Option<Cow<'_, str>> {
        crate::infra::events::traceparent().map(Cow::Owned)
    }
}

/// A break-glass correction's evidence row, announced beside the correction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SkuCorrectionOverride {
    #[serde(flatten)]
    pub core: CatalogEventCore,
    pub arm: String,
    pub field: String,
    pub ceremony_ref: Uuid,
}

impl TypedEvent for SkuCorrectionOverride {
    const TYPE_ID: &'static str =
        "gts.cf.core.events.event_type.v1~cf.bss.products.sku_correction_override.v1";
    const TOPIC: &'static str = TOPIC;
    const SUBJECT_TYPE: &'static str = SKU_SUBJECT_TYPE;
    const SOURCE: &'static str = SOURCE;

    fn subject(&self) -> Cow<'_, str> {
        Cow::Owned(self.core.entity_id.to_string())
    }

    fn tenant_id(&self) -> Option<Uuid> {
        Some(self.core.tenant_id)
    }

    fn trace_parent(&self) -> Option<Cow<'_, str>> {
        crate::infra::events::traceparent().map(Cow::Owned)
    }
}

/// One frozen entity as `CatalogVersionPublished` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChangedEntityPayload {
    pub entity_kind: String,
    pub entity_id: Uuid,
    pub published_version: i64,
}

/// `06`'s catalog-version body (**P-D-125** row 27): no entity dimension.
/// `actor_ref` rides the payload for [`CatalogEventCore`]'s reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogVersionPayload {
    pub tenant_id: Uuid,
    pub catalog_version_id: Option<i64>,
    pub act: String,
    pub participants: Vec<String>,
    pub changed_entities: Vec<ChangedEntityPayload>,
    pub satisfied_requests: u32,
    pub checksum: Option<String>,
    pub quorum_reduced: Option<bool>,
    pub actor_ref: Uuid,
}

/// The three version-subjected events share the payload and differ in type
/// id and subject type; the subject value is the version id (or the
/// participant, for the set change).
macro_rules! catalog_version_event {
    ($(#[$doc:meta])* $name:ident, $type_id:literal, $subject_type:expr, $subject:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(crate) struct $name {
            #[serde(flatten)]
            pub payload: CatalogVersionPayload,
        }

        impl TypedEvent for $name {
            const TYPE_ID: &'static str = $type_id;
            const TOPIC: &'static str = TOPIC;
            const SUBJECT_TYPE: &'static str = $subject_type;
            const SOURCE: &'static str = SOURCE;

            fn subject(&self) -> Cow<'_, str> {
                let f: fn(&CatalogVersionPayload) -> String = $subject;
                Cow::Owned(f(&self.payload))
            }

            fn tenant_id(&self) -> Option<Uuid> {
                Some(self.payload.tenant_id)
            }

            fn trace_parent(&self) -> Option<Cow<'_, str>> {
                crate::infra::events::traceparent().map(Cow::Owned)
            }
        }
    };
}

fn version_subject(payload: &CatalogVersionPayload) -> String {
    payload
        .catalog_version_id
        .map_or_else(|| payload.tenant_id.to_string(), |id| id.to_string())
}

fn participant_subject(payload: &CatalogVersionPayload) -> String {
    payload
        .participants
        .first()
        .cloned()
        .unwrap_or_else(|| payload.tenant_id.to_string())
}

catalog_version_event! {
    /// A catalog version was published: the freeze protocol's opening fact.
    CatalogVersionPublished,
    "gts.cf.core.events.event_type.v1~cf.bss.products.catalog_version_published.v1",
    CATALOG_VERSION_SUBJECT_TYPE,
    version_subject
}
catalog_version_event! {
    /// A force-completion ceremony closed a timed-out freeze.
    FreezeForceCompleted,
    "gts.cf.core.events.event_type.v1~cf.bss.products.freeze_force_completed.v1",
    CATALOG_VERSION_SUBJECT_TYPE,
    version_subject
}
catalog_version_event! {
    /// The tenant's freeze-participant set moved.
    FreezeParticipantSetChanged,
    "gts.cf.core.events.event_type.v1~cf.bss.products.freeze_participant_set_changed.v1",
    FREEZE_PARTICIPANT_SUBJECT_TYPE,
    participant_subject
}

catalog_event! {
    /// The inbound composition signal cleared a bundle's `composition_pending`
    /// (`06`; rides beside the re-publish's own `SkuPublished`, P-D-60).
    SkuCompositionCleared,
    "gts.cf.core.events.event_type.v1~cf.bss.products.sku_composition_cleared.v1",
    SKU_SUBJECT_TYPE
}

/// The tenant's registered producer set moved — entity-less, the tenant's
/// set being the aggregate (**P-D-71**), so the subject is the tenant id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReferenceProducerSetChanged {
    pub tenant_id: Uuid,
    pub producer: String,
    pub state: String,
    pub actor_ref: Uuid,
}

impl TypedEvent for ReferenceProducerSetChanged {
    const TYPE_ID: &'static str =
        "gts.cf.core.events.event_type.v1~cf.bss.products.reference_producer_set_changed.v1";
    const TOPIC: &'static str = TOPIC;
    const SUBJECT_TYPE: &'static str = REFERENCE_PRODUCER_SUBJECT_TYPE;
    const SOURCE: &'static str = SOURCE;

    fn subject(&self) -> Cow<'_, str> {
        Cow::Owned(self.tenant_id.to_string())
    }

    fn tenant_id(&self) -> Option<Uuid> {
        Some(self.tenant_id)
    }

    fn trace_parent(&self) -> Option<Cow<'_, str>> {
        crate::infra::events::traceparent().map(Cow::Owned)
    }
}

/// `02`'s eight events share one body: which entity, which act, which state
/// it now carries (`design/02` §4.3; **P-D-122** fixed the shape). Owned for
/// [`CatalogEventCore`]'s reason — a consumer deserializes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TaxonomyEventPayload {
    /// The owning tenant — the partition input, through [`TypedEvent::tenant_id`].
    pub tenant_id: Uuid,
    /// `category`, `attribute_definition`, or the metadata map's owner
    /// (`product` / `sku`).
    pub entity_kind: String,
    /// The entity the act touched; also the subject.
    pub entity_id: Uuid,
    /// The act, in the slice's vocabulary.
    pub act: String,
    /// The entity's state after the act.
    pub state: String,
    /// The token the category live-value door spent, where the act spent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutation_seq: Option<i64>,
    /// The `GovernedLiveOp` kind, where the act rode an envelope.
    /// `inst-tx-event`'s *"op envelope id"* has no operand — the envelope
    /// carries none (**P-D-122**) — and this is what does exist.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_kind: Option<String>,
    /// The acting principal, pseudonymously — payload-borne for
    /// [`CatalogEventCore`]'s reason.
    pub actor_ref: Uuid,
}

impl TaxonomyEventPayload {
    /// Owned copy of the interim body, plus the actor the broker's `Event`
    /// has no field for.
    pub(crate) fn from_body(
        body: &crate::infra::events::TaxonomyEventBody<'_>,
        actor_ref: Uuid,
    ) -> Self {
        Self {
            tenant_id: body.tenant_id,
            entity_kind: body.entity_kind.to_owned(),
            entity_id: body.entity_id,
            act: body.act.to_owned(),
            state: body.state.to_owned(),
            mutation_seq: body.mutation_seq,
            operation_kind: body.operation_kind.map(str::to_owned),
            actor_ref,
        }
    }
}

/// A taxonomy event: [`TaxonomyEventPayload`] flat, the subject the entity's
/// id, the subject type the entity's kind.
///
/// A fifth macro rather than a field on [`catalog_event`]'s expansion for the
/// reason the set events give: the body is not entity-core-shaped — no
/// revision, no lifecycle state, a state machine of its own — so a shared
/// core would carry fields these events cannot fill honestly.
macro_rules! taxonomy_event {
    ($(#[$doc:meta])* $name:ident, $type_id:literal, $subject_type:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(crate) struct $name {
            /// `02`'s body, flat.
            #[serde(flatten)]
            pub payload: TaxonomyEventPayload,
        }

        impl TypedEvent for $name {
            const TYPE_ID: &'static str = $type_id;
            const TOPIC: &'static str = TOPIC;
            const SUBJECT_TYPE: &'static str = $subject_type;
            const SOURCE: &'static str = SOURCE;

            fn subject(&self) -> Cow<'_, str> {
                Cow::Owned(self.payload.entity_id.to_string())
            }

            /// The entity's tenant, not the producer's — the catalog events'
            /// own override, for the same partitioning reason.
            fn tenant_id(&self) -> Option<Uuid> {
                Some(self.payload.tenant_id)
            }

            fn trace_parent(&self) -> Option<Cow<'_, str>> {
                crate::infra::events::traceparent().map(Cow::Owned)
            }
        }
    };
}

taxonomy_event! {
    /// A category row was created (`inst-tx-event`).
    CategoryCreated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.category_created.v1",
    CATEGORY_SUBJECT_TYPE
}
taxonomy_event! {
    /// A category was renamed.
    CategoryRenamed,
    "gts.cf.core.events.event_type.v1~cf.bss.products.category_renamed.v1",
    CATEGORY_SUBJECT_TYPE
}
taxonomy_event! {
    /// A category was re-parented.
    CategoryReparented,
    "gts.cf.core.events.event_type.v1~cf.bss.products.category_reparented.v1",
    CATEGORY_SUBJECT_TYPE
}
taxonomy_event! {
    /// A category was retired.
    CategoryRetired,
    "gts.cf.core.events.event_type.v1~cf.bss.products.category_retired.v1",
    CATEGORY_SUBJECT_TYPE
}
taxonomy_event! {
    /// A retired, empty, unreferenced category row was deleted.
    CategoryDeleted,
    "gts.cf.core.events.event_type.v1~cf.bss.products.category_deleted.v1",
    CATEGORY_SUBJECT_TYPE
}
taxonomy_event! {
    /// A category's display values moved (`inst-av-category-branch`).
    CategoryDisplayUpdated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.category_display_updated.v1",
    CATEGORY_SUBJECT_TYPE
}
taxonomy_event! {
    /// An attribute definition was created, flipped or re-labelled
    /// (`inst-ad-event`).
    AttributeDefinitionUpdated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.attribute_definition_updated.v1",
    ATTRIBUTE_DEFINITION_SUBJECT_TYPE
}
taxonomy_event! {
    /// An entity's metadata map was merged (`inst-md-*`).
    MetadataUpdated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.metadata_updated.v1",
    METADATA_SUBJECT_TYPE
}

/// `10`'s two events share one body, for the reason `02`'s eight do: they
/// announce the same kind of thing — *an act this feature performed on a
/// governed identity object* — and differ in which act and which subject
/// (`dod-retention-events`; **P-D-118** item 26 fixed the aggregates).
///
/// **It carries no identity, and that is the feature's own rule made
/// structural.** `ActorErased` names the retired pseudonym; there is no
/// payload field an identity could reach, so the *"defensive cache-buster"*
/// cannot become a leak by a later caller filling one in. The allow-list arm
/// carries the entry's id and never its value: the value is a person-named
/// string, which is precisely what the write block exists to keep out of
/// records erasure cannot rewrite.
///
/// Owned for [`CatalogEventCore`]'s reason — a consumer deserializes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RetentionEventPayload {
    /// The owning tenant — the partition input, through [`TypedEvent::tenant_id`].
    pub tenant_id: Uuid,
    /// The aggregate this act serializes on, rendered: the erased principal's
    /// `principal_ref` for `ActorErased`, the entry's id for
    /// `PiiAllowlistChanged` (**P-D-118** item 26). Also the subject.
    pub subject_ref: String,
    /// The act, in the slice's vocabulary: `erased`, `signed_off`, `revoked`.
    pub act: String,
    /// The **retired** pseudonym, on the erasure arm only. `None` on the
    /// allow-list arm, where no ref was retired.
    ///
    /// Not spelled `actorRef`, and that is the gear's convention rather than
    /// a preference: every typed event's payload carries `actorRef` as
    /// P-D-01's *acting* principal — `broker_tests` asserts it across the
    /// whole roster — so an erased subject under that name would collide with
    /// a field every consumer already reads as "who did this".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub erased_actor_ref: Option<Uuid>,
    /// The acting principal, pseudonymously — payload-borne for
    /// [`CatalogEventCore`]'s reason. On the age-triggered erasure this is
    /// `gear::system_actor_ref()` (**P-D-113** arm 2), because the act had no
    /// requester.
    pub actor_ref: Uuid,
}

impl RetentionEventPayload {
    /// Owned copy of the interim body, plus the actor the broker's `Event`
    /// has no field for.
    pub(crate) fn from_body(
        body: &crate::infra::events::RetentionEventBody<'_>,
        actor_ref: Uuid,
    ) -> Self {
        Self {
            tenant_id: body.tenant_id,
            subject_ref: body.subject_ref.to_owned(),
            act: body.act.to_owned(),
            erased_actor_ref: body.erased_actor_ref,
            actor_ref,
        }
    }
}

/// A retention event: [`RetentionEventPayload`] flat, the subject the
/// aggregate the act serializes on.
///
/// A sixth macro rather than a field on [`taxonomy_event`]'s expansion, for
/// the reason that one gives against folding into [`catalog_event`]: neither
/// of these bodies is entity-core-shaped, and neither subject is an entity —
/// `EntityKind` is exactly `Product | Sku`, and a principal is neither.
macro_rules! retention_event {
    ($(#[$doc:meta])* $name:ident, $type_id:literal, $subject_type:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(crate) struct $name {
            /// `10`'s body, flat.
            #[serde(flatten)]
            pub payload: RetentionEventPayload,
        }

        impl TypedEvent for $name {
            const TYPE_ID: &'static str = $type_id;
            const TOPIC: &'static str = TOPIC;
            const SUBJECT_TYPE: &'static str = $subject_type;
            const SOURCE: &'static str = SOURCE;

            fn subject(&self) -> Cow<'_, str> {
                Cow::Borrowed(&self.payload.subject_ref)
            }

            /// The subject's tenant, not the producer's — the catalog events'
            /// own override, for the same partitioning reason.
            fn tenant_id(&self) -> Option<Uuid> {
                Some(self.payload.tenant_id)
            }

            fn trace_parent(&self) -> Option<Cow<'_, str>> {
                crate::infra::events::traceparent().map(Cow::Owned)
            }
        }
    };
}

retention_event! {
    /// A principal's map entry was tombstoned, by request or by age
    /// (`inst-er-event`). A defensive cache-buster: it carries the retired
    /// pseudonym and no identity, and its consumer set is legitimately empty
    /// because no projection in the design set materializes identities.
    ActorErased,
    "gts.cf.core.events.event_type.v1~cf.bss.products.actor_erased.v1",
    ERASURE_SUBJECT_TYPE
}
retention_event! {
    /// An allow-list entry was signed off or revoked (`inst-pp-allowlist`).
    /// Carries the entry's id and never its value.
    PiiAllowlistChanged,
    "gts.cf.core.events.event_type.v1~cf.bss.products.pii_allowlist_changed.v1",
    PII_ALLOWLIST_SUBJECT_TYPE
}

/// The batch-completion summary as a typed event — slice 09's only one.
///
/// Its own declaration rather than a macro's expansion: no other event of
/// this gear carries per-disposition counts, and a macro for a population
/// of one would hide the shape rather than share it. The subject is the
/// **batch**, so the subject type derives from
/// `cf.bss.products.bulk.v1~` — the GTS type `05` §3.2's authz catalog
/// already declares for `bulk × execute|read`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CatalogBulkOperationCompleted {
    /// The owning tenant — the ordering key's first half.
    pub tenant_id: Uuid,
    /// The batch this summary closes, and the subject.
    pub batch_id: Uuid,
    /// The import door's idempotency operand.
    pub batch_key: String,
    /// The digest over the completed ledger.
    pub ledger_digest: String,
    /// Rows whose entity published.
    pub published: u32,
    /// Rows whose live-entity operation applied.
    pub applied: u32,
    /// Rows already in the requested state.
    pub no_op: u32,
    /// Rows that failed row-locally.
    pub failed: u32,
    /// The acting principal, pseudonymously.
    pub actor_ref: Uuid,
}

impl TypedEvent for CatalogBulkOperationCompleted {
    const TYPE_ID: &'static str =
        "gts.cf.core.events.event_type.v1~cf.bss.products.catalog_bulk_operation_completed.v1";
    const TOPIC: &'static str = TOPIC;
    const SUBJECT_TYPE: &'static str = BULK_SUBJECT_TYPE;
    const SOURCE: &'static str = SOURCE;

    fn subject(&self) -> Cow<'_, str> {
        Cow::Owned(self.batch_id.to_string())
    }

    /// The batch's tenant, not the producer's — the catalog events' own
    /// override, for the same partitioning reason.
    fn tenant_id(&self) -> Option<Uuid> {
        Some(self.tenant_id)
    }

    fn trace_parent(&self) -> Option<Cow<'_, str>> {
        crate::infra::events::traceparent().map(Cow::Owned)
    }
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

/// A retirement or flip event: the core plus `RetiredEventBody`'s fields.
///
/// A fourth catalog macro because those fields are not on every event —
/// the same argument [`catalog_deprecation_event`] makes for provenance.
/// @cpt-dod:cpt-cf-bss-products-dod-lifecycle-events:p1
macro_rules! catalog_retirement_event {
    ($(#[$doc:meta])* $name:ident, $type_id:literal, $subject_type:expr) => {
        $(#[$doc])*
        #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
        #[serde(rename_all = "camelCase")]
        pub(crate) struct $name {
            /// §4.5's five fields plus P-D-01's two payload-borne obligations.
            #[serde(flatten)]
            pub core: CatalogEventCore,
            /// The published version the retirement is taken from.
            pub from_version: i64,
            /// Operator retirement text (**P-D-46**).
            pub reason: String,
            /// Named replacement SKU. `None` on a Product event.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub replaced_by: Option<Uuid>,
            /// RFC3339 UTC effective instant.
            pub effective_at: String,
            /// Always `None` in v1; omitted so absence round-trips.
            #[serde(default, skip_serializing_if = "Option::is_none")]
            pub must_migrate_by: Option<String>,
        }

        impl TypedEvent for $name {
            const TYPE_ID: &'static str = $type_id;
            const TOPIC: &'static str = TOPIC;
            const SUBJECT_TYPE: &'static str = $subject_type;
            const SOURCE: &'static str = SOURCE;

            fn subject(&self) -> Cow<'_, str> {
                Cow::Owned(self.core.entity_id.to_string())
            }

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
    /// A Product un-deprecation — the bare core.
    ProductUndeprecated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.product_undeprecated.v1",
    PRODUCT_SUBJECT_TYPE
}
catalog_event! {
    /// A SKU un-deprecation — the bare core.
    SkuUndeprecated,
    "gts.cf.core.events.event_type.v1~cf.bss.products.sku_undeprecated.v1",
    SKU_SUBJECT_TYPE
}
catalog_retirement_event! {
    /// Product retirement initiation.
    ProductRetired,
    "gts.cf.core.events.event_type.v1~cf.bss.products.product_retired.v1",
    PRODUCT_SUBJECT_TYPE
}
catalog_retirement_event! {
    /// SKU retirement initiation.
    SkuRetired,
    "gts.cf.core.events.event_type.v1~cf.bss.products.sku_retired.v1",
    SKU_SUBJECT_TYPE
}
catalog_retirement_event! {
    /// The SKU flip.
    SkuRetirementEffective,
    "gts.cf.core.events.event_type.v1~cf.bss.products.sku_retirement_effective.v1",
    SKU_SUBJECT_TYPE
}
catalog_retirement_event! {
    /// The Product flip (**P-D-115**).
    ProductRetirementEffective,
    "gts.cf.core.events.event_type.v1~cf.bss.products.product_retirement_effective.v1",
    PRODUCT_SUBJECT_TYPE
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
    prepare_lifecycle_event_types(producer).await?;
    producer.prepare::<RecognizedUnitUpdated>().await?;
    producer.prepare::<RecognizedCodeUpdated>().await?;
    producer.prepare::<PlanTierUpdated>().await?;
    producer.prepare::<SkuImmutableFieldCorrected>().await?;
    producer.prepare::<SkuCorrectionOverride>().await?;
    producer.prepare::<ReferenceProducerSetChanged>().await?;
    prepare_catalog_version_event_types(producer).await?;
    producer.prepare::<CatalogBulkOperationCompleted>().await?;
    Ok(())
}

/// `06`'s four (`dod-cv-events`): the three on the catalog-version body and
/// the clear on the entity core.
async fn prepare_catalog_version_event_types(
    producer: &event_broker_sdk::DbProducer,
) -> Result<(), event_broker_sdk::EventBrokerError> {
    producer.prepare::<CatalogVersionPublished>().await?;
    producer.prepare::<FreezeForceCompleted>().await?;
    producer.prepare::<FreezeParticipantSetChanged>().await?;
    producer.prepare::<SkuCompositionCleared>().await?;
    Ok(())
}

/// 04's typed events — deprecation through both flips.
async fn prepare_lifecycle_event_types(
    producer: &event_broker_sdk::DbProducer,
) -> Result<(), event_broker_sdk::EventBrokerError> {
    producer.prepare::<ProductDeprecated>().await?;
    producer.prepare::<SkuDeprecated>().await?;
    producer.prepare::<ProductUndeprecated>().await?;
    producer.prepare::<SkuUndeprecated>().await?;
    producer.prepare::<ProductRetired>().await?;
    producer.prepare::<SkuRetired>().await?;
    producer.prepare::<SkuRetirementEffective>().await?;
    producer.prepare::<ProductRetirementEffective>().await?;
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
