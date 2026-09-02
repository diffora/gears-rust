//! The Foundation event envelope, shared by every create/save/publish door
//! this gear opens (`design/01-foundation.md` §4.5, P-D-27).
//!
//! # One home for the body core, so `SkuCreated` does not duplicate it
//!
//! §4.5 fixes one body core across all eight Foundation events — and 04's
//! announced pair rides the same core —
//! `{tenantId, entityKind, entityId, internalRevision, lifecycleState}` — and
//! names anything beyond it where the act that adds it is specified (only
//! `*Published`, with `publishedVersion`). [`EventBodyCore`] is that shape;
//! six of the Foundation's eight carry **only** the core, built through this same type,
//! rather than each redeclaring the five fields a second time.
//!
//! # `publishedVersion` sits **outside** the core, not inside it
//!
//! §4.5's sentence is two clauses, and the second one is what fixes the
//! shape: *"every one of the eight carries the same body core"*, and
//! `ProductPublished`/`SkuPublished` ***additionally*** *carry
//! `publishedVersion`*. A sixth field on [`EventBodyCore`] would satisfy the
//! two publish events and break the other six, which would then announce a
//! `publishedVersion` §4.5 does not put on them — and, worse, would have to
//! invent a value for it on a `ProductDiscarded`, whose act writes no
//! version at all. So the extra field lives on
//! [`PublishedEventBody`], which **borrows** a core and adds the one field
//! beside it through `serde`'s `flatten`. The wire shape is a single flat
//! object either way, which is what §4.5 describes; the type is what keeps
//! "additionally" from quietly becoming "always".
//!
//! [`enqueue`] is the core-only entry and [`enqueue_published`] the
//! publish one; both go through the same private body writer, so neither can
//! drift from the other on the partition formula or the envelope.
//!
//! # One home for the partition formula, so the SKU door does not grow a
//! second copy
//!
//! P-D-22 fixes `partition = hash(tenant_id, aggregate_id) mod N`: every
//! event of one aggregate lands in one partition of **this gear's own
//! toolkit outbox**. [`partition_for`] is the one function that computes it;
//! a door calls it, never re-derives it.
//!
//! **It is not the ordering AC #28 gets.** §4.4 is explicit under **P-D-47**:
//! *"Ordering comes from the broker's partition selection, not from a
//! column"* — the gear sets no `partition_key`, so the broker's ADR-0002
//! default applies (`MurmurHash3-32` over `tenant_id`, modulo
//! `topic.partitions`, re-computed authoritatively at ingest), and the
//! consumer-visible operand beyond the idempotency window is the **broker's**
//! read-side `sequence`, server-assigned per `(topic, partition)`. What the
//! partition below orders is the local pipeline: the toolkit outbox's `seq`,
//! which the SDK sends on as the producer chain's `meta.sequence`. So this
//! formula is a *pipeline* invariant that P-D-47 supersedes for the guarantee
//! a consumer actually reads, and §4.4 records the broker's ordering as
//! **stronger** than the `(tenant, aggregate)` key the envelope promises. The hash itself is the same idiom
//! `gears/mini-chat`'s own `InfraOutboxEnqueuer::compute_partition` uses for
//! its single-operand case (`tenant_id.as_u128() % num_partitions`) —
//! extended here to **two** operands, since P-D-22's key is the pair, not
//! `tenant_id` alone. It is a plain, unsalted combination
//! (`tenant_id.as_u128() ^ aggregate_id.as_u128().rotate_left(64)`, then `%
//! N`) rather than a cryptographic hash: nothing here needs collision
//! resistance, only a deterministic, stable-across-restarts spread over `[0,
//! N)`, and every input is already a high-entropy `Uuid`.
//!
//! # What this module does not do
//!
//! It does not register a queue, does not run a consumer, and does not
//! decide **which** running [`toolkit_db::outbox::Outbox`] a door enqueues
//! against — that instance is [`crate::api::rest::ApiState`]'s to hold and
//! `crate::gear::BssProductsGear`'s to build and hand to the router.
//! `gear.rs` registers the queue from this module's own
//! [`OUTBOX_TABLE_PREFIX`], [`QUEUE_NAME`] and [`PARTITIONS`], so the three
//! have one definition site between them.
//!
//! **Whether delivery happens is decided at boot, not here.** `gear.rs` binds
//! the broker SDK's producer as this queue's processor when the `ClientHub`
//! carries an `EventBrokerApi` (**P-D-47**), and [`PendingBrokerProducer`] —
//! which holds every message rather than publishing it — when it does not.
//! `crate::infra::broker` owns that fork and records why the second arm exists
//! at all. This module writes the interim envelope the second arm carries; the
//! first carries the SDK's, built from `broker`'s typed events.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-outbox-eventing:p1

use serde::Serialize;
use toolkit_db::outbox::{Outbox, OutboxError};
use toolkit_db::secure::DBRunner;

use crate::infra::broker::{self, EventSink};
use uuid::Uuid;

/// Table-family prefix this door's events are enqueued under.
///
/// **The one definition.** `crate::gear` imports this constant rather than
/// declaring its own, so the prefix the pipeline creates its tables under and
/// the prefix this door enqueues against cannot disagree. The duplication an
/// earlier revision of this doc warned about was closed when `gear.rs` came
/// into scope; the warning outlived it and is removed here.
pub(crate) const OUTBOX_TABLE_PREFIX: &str = "bss_products_outbox";

/// The one queue every Foundation event on this gear's Product/SKU surface
/// enqueues onto. One queue rather than one per entity: P-D-27's ordering
/// key is `(tenant, aggregate)`, not `(tenant, aggregate, entity_kind)`, and
/// splitting the queue would not change the partitioning, only the registry
/// entry a consumer subscribes to.
pub(crate) const QUEUE_NAME: &str = "bss_products_events";

/// The fixed partition count P-D-22's modulus divides by. Chosen once, here,
/// so [`partition_for`] and the queue's own registration in `gear.rs` — which
/// reads this very constant (`Gear::init`) — never disagree on `N`; a
/// registration with a
/// different count fails closed with `OutboxError::PartitionCountMismatch`
/// rather than silently reassigning aggregates to different partitions.
pub(crate) const PARTITIONS: u16 = 8;

/// The queue's processor until Phase 8 binds the real one.
///
/// A queue cannot be declared without a handler — every finishing path on
/// `QueueBuilder` registers a processor factory — but the processor this
/// queue is *supposed* to have is not this gear's to write. **P-D-47**: the
/// processor is the **broker SDK's** outbox producer
/// (`gears/system/event-broker/event-broker-sdk`: a `DbProducer` bound to a
/// `toolkit_db::outbox` queue, in managed monotonic mode), and the plan puts
/// that wiring in Phase 8 with `dod-outbox-eventing`.
///
/// So this handler exists to make the queue declarable while delivery is
/// still owed, and it answers [`toolkit_db::outbox::MessageResult::Retry`] to every message:
/// a transient failure, which leaves the row in the queue and dead-letters
/// nothing. That is the honest shape of "enqueued, not yet deliverable".
///
/// It deliberately does **not** answer `Ok`. An `Ok` would mark the message
/// delivered and hand it to the vacuum, so every event this gear enqueues
/// before the producer lands would be reclaimed having reached no broker at
/// all — a silent loss of exactly the events `fr-registry-eventing-audit`
/// requires. A queue that visibly cannot deliver is recoverable; one that
/// quietly discards is not.
///
/// # Two things this handler's absence leaves owed, recorded here
///
/// 1. **"Emitted" before durable broker acceptance.** The requirement
///    (`fr-event-delivery-resilience`, registry-side half) is the *handler's*
///    contract, not a column to mark, so it cannot be discharged until the
///    handler exists. Today nothing is reported emitted at all, which is the
///    safe side of that requirement rather than a breach of it.
/// 2. **The sub-3-second publication-propagation probe is owed**, and the
///    01/06 split of that budget is open at the PRD owner. No measurement in
///    the design set establishes it, and none can be taken here: the elapsed
///    time from a committed act to a consumer-visible event is dominated by
///    the broker leg this handler does not yet make. Recorded rather than
///    estimated — a number produced against a handler that holds every
///    message would describe this stub, not the system.
pub(crate) struct PendingBrokerProducer;

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

/// `ProductCreated`'s payload type token, carried on the outbox row and read
/// back by whatever eventually drains this queue. Named for the event
/// itself, matching `design/01-foundation.md` §4.5's own name for it (P-D-27
/// renamed only the two `*DraftSaved` events; `ProductCreated` was never
/// renamed).
pub(crate) const PRODUCT_CREATED_PAYLOAD_TYPE: &str = "ProductCreated";

/// `SkuCreated`'s payload type token — [`PRODUCT_CREATED_PAYLOAD_TYPE`]'s SKU
/// sibling, carrying the identical [`EventBodyCore`] shape (this module's
/// doc, "One home for the body core").
pub(crate) const SKU_CREATED_PAYLOAD_TYPE: &str = "SkuCreated";

/// `ProductPublished`'s payload type token (§4.5, `inst-fd-publish-emit`).
///
/// It and the three below were each declared inside the door that emits
/// them, by two slices running in parallel, and each of the four carried a
/// note saying it belonged here beside [`PRODUCT_CREATED_PAYLOAD_TYPE`].
/// They are here now, so this gear's payload-type roster reads in one place
/// and a consumer contract can be checked against one list.
///
/// Its body is [`PublishedEventBody`] — the core plus `publishedVersion` —
/// and it is enqueued through [`enqueue_published`].
pub(crate) const PRODUCT_PUBLISHED_PAYLOAD_TYPE: &str = "ProductPublished";

/// `ProductDiscarded`'s payload type token (§4.5, `inst-fd-discard`). Its
/// body is the bare [`EventBodyCore`]: §4.5 names nothing beyond the core
/// for it, and a discard writes no version there could be a
/// `publishedVersion` to announce.
pub(crate) const PRODUCT_DISCARDED_PAYLOAD_TYPE: &str = "ProductDiscarded";

/// `SkuPublished`'s payload type token — [`PRODUCT_PUBLISHED_PAYLOAD_TYPE`]'s
/// SKU sibling, on the same [`PublishedEventBody`] shape.
pub(crate) const SKU_PUBLISHED_PAYLOAD_TYPE: &str = "SkuPublished";

/// `SkuDiscarded`'s payload type token — [`PRODUCT_DISCARDED_PAYLOAD_TYPE`]'s
/// SKU sibling, carrying the bare core for the same reason.
pub(crate) const SKU_DISCARDED_PAYLOAD_TYPE: &str = "SkuDiscarded";

/// `ProductHeadSaved`'s payload type token (§4.5's roster of eight).
///
/// It and its SKU twin below spent two phases declared inside the doors that
/// emit them, each carrying a note saying it belonged here. Both notes gave
/// the same reason — that this module was outside that slice's target paths
/// — and that reason expired with the slice. They are here now, so the
/// roster of eight reads as one list and [`SCHEMA_REFS`] can be checked
/// against it.
pub(crate) const PRODUCT_HEAD_SAVED_PAYLOAD_TYPE: &str = "ProductHeadSaved";

/// `SkuHeadSaved`'s payload type token — [`PRODUCT_HEAD_SAVED_PAYLOAD_TYPE`]'s
/// SKU sibling, carrying the bare core.
pub(crate) const SKU_HEAD_SAVED_PAYLOAD_TYPE: &str = "SkuHeadSaved";

/// `ProductDeprecated`'s payload type token — and **not one of §4.5's
/// eight**.
///
/// `design/01` §4.5 records that the floor's three remaining edges —
/// `published→deprecated`, `deprecated→published`, `deprecated→retired` —
/// carry *"no event here"*, and that **04 announces them**
/// (`design/04-lifecycle.md` §3 `inst-lc-deprecate`, its Events roster).
/// So this token and its SKU twin widen the gear's roster past the
/// Foundation's eight rather than filling a gap in it, and `events_tests`
/// names the two rosters separately for exactly that reason: a token
/// wrongly attributed to §4.5 would make the Foundation's own completeness
/// check unfalsifiable.
///
/// Its body is [`DeprecatedEventBody`] — the core plus `provenance`, which
/// `dod-deprecation-provenance` requires *"in its payload"*.
pub(crate) const PRODUCT_DEPRECATED_PAYLOAD_TYPE: &str = "ProductDeprecated";

/// `SkuDeprecated`'s payload type token — [`PRODUCT_DEPRECATED_PAYLOAD_TYPE`]'s
/// SKU sibling, on the same body shape.
///
/// This is the one pricing AC #82 keys on, and `design/04` has the
/// retirement arm emit it too, with `direct` or `cascaded` provenance
/// according to who drove the act.
pub(crate) const SKU_DEPRECATED_PAYLOAD_TYPE: &str = "SkuDeprecated";

/// `RecognizedUnitUpdated`'s payload type token — `design/03` §4's roster,
/// the metering-unit set's own event, emitted **in the same transaction** as
/// the membership mutation (`inst-rs-shape`). Not one of §4.5's eight and
/// not 04's pair: a third declared roster, 03's, and `events_tests` names it
/// separately for the same reason as the other two.
pub(crate) const RECOGNIZED_UNIT_UPDATED_PAYLOAD_TYPE: &str = "RecognizedUnitUpdated";

/// `RecognizedCodeUpdated`'s payload type token — the tax-category and
/// GL-code sets share it (`design/03` §4).
pub(crate) const RECOGNIZED_CODE_UPDATED_PAYLOAD_TYPE: &str = "RecognizedCodeUpdated";

/// `PlanTierUpdated`'s payload type token — PRD-named; the tier set's own
/// event by design.
pub(crate) const PLAN_TIER_UPDATED_PAYLOAD_TYPE: &str = "PlanTierUpdated";

/// `CatalogBulkOperationCompleted`'s payload type token — **slice 09's only
/// event**, and the fourth declared roster.
///
/// `design/09`'s eight state-changing instructions carry an inline *no
/// event* marker on 01's convention (**P-D-61**): a row's own act is
/// announced by the 01 and 04 doors it drives, and the batch's history —
/// the ledger, the `ChangeReport`, 05's approval record — is audit-plane
/// (**P-D-21**). This one summary is the exception, and it is **additive**:
/// what it coalesces is per-row progress noise, never a row's domain event,
/// so `12`'s bookkeeping lint reads it as an addition to the register
/// rather than as events withheld.
pub(crate) const CATALOG_BULK_OPERATION_COMPLETED_PAYLOAD_TYPE: &str =
    "CatalogBulkOperationCompleted";

/// Every payload type this gear emits, paired with the **versioned schema
/// reference** its envelope carries (P-D-01: *"versioned (semver) schema
/// references — the broker-native equivalent of `dataschema`"*).
///
/// One list rather than a constant beside each token, because the property
/// that matters is *coverage*: an added event, or a renamed token, must not
/// be able to reach the wire with no schema reference. [`schema_ref_for`] is
/// total over this array and nothing else.
///
/// **This array is the gear's roster, not the Foundation's.** It carries
/// `01` §4.5's **eight** and, since `04-lifecycle`'s deprecation act landed,
/// the **two** that slice announces on the edges §4.5 leaves eventless
/// ([`PRODUCT_DEPRECATED_PAYLOAD_TYPE`] and its twin). `events_tests` checks
/// the two rosters separately — §4.5's eight must all be here, and every
/// token here must belong to one of the two named rosters — because a single
/// "exactly eight" assertion would either refuse a legitimate addition or,
/// once widened, stop testing §4.5's completeness at all.
///
/// **The version is per event, not per gear.** §4.5's own rule makes an added
/// optional field a minor bump, so one event's schema may move while the
/// others stand still; a single gear-wide version would force false bumps or
/// hide a real one. All thirteen read `1.0.0` today because none has shipped a
/// second shape.
pub(crate) const SCHEMA_REFS: &[(&str, &str)] = &[
    (
        PRODUCT_CREATED_PAYLOAD_TYPE,
        "bss-products.ProductCreated.v1.0.0",
    ),
    (SKU_CREATED_PAYLOAD_TYPE, "bss-products.SkuCreated.v1.0.0"),
    (
        PRODUCT_HEAD_SAVED_PAYLOAD_TYPE,
        "bss-products.ProductHeadSaved.v1.0.0",
    ),
    (
        SKU_HEAD_SAVED_PAYLOAD_TYPE,
        "bss-products.SkuHeadSaved.v1.0.0",
    ),
    (
        PRODUCT_PUBLISHED_PAYLOAD_TYPE,
        "bss-products.ProductPublished.v1.0.0",
    ),
    (
        SKU_PUBLISHED_PAYLOAD_TYPE,
        "bss-products.SkuPublished.v1.0.0",
    ),
    (
        PRODUCT_DISCARDED_PAYLOAD_TYPE,
        "bss-products.ProductDiscarded.v1.0.0",
    ),
    (
        SKU_DISCARDED_PAYLOAD_TYPE,
        "bss-products.SkuDiscarded.v1.0.0",
    ),
    (
        PRODUCT_DEPRECATED_PAYLOAD_TYPE,
        "bss-products.ProductDeprecated.v1.0.0",
    ),
    (
        SKU_DEPRECATED_PAYLOAD_TYPE,
        "bss-products.SkuDeprecated.v1.0.0",
    ),
    (
        RECOGNIZED_UNIT_UPDATED_PAYLOAD_TYPE,
        "bss-products.RecognizedUnitUpdated.v1.0.0",
    ),
    (
        RECOGNIZED_CODE_UPDATED_PAYLOAD_TYPE,
        "bss-products.RecognizedCodeUpdated.v1.0.0",
    ),
    (
        PLAN_TIER_UPDATED_PAYLOAD_TYPE,
        "bss-products.PlanTierUpdated.v1.0.0",
    ),
    (
        CATALOG_BULK_OPERATION_COMPLETED_PAYLOAD_TYPE,
        "bss-products.CatalogBulkOperationCompleted.v1.0.0",
    ),
];

/// The versioned schema reference for a payload type, or `None` for a token
/// [`SCHEMA_REFS`] does not name.
///
/// `None` rather than a woven-in default: a default would let an unregistered
/// event reach a consumer announcing a schema it does not have, which is the
/// one failure a schema reference exists to prevent. [`enqueue_body`] turns
/// the `None` into [`EventsError::UnregisteredSchema`] and refuses the write,
/// so the act rolls back rather than emitting an unidentifiable event.
#[must_use]
pub(crate) fn schema_ref_for(payload_type: &str) -> Option<&'static str> {
    SCHEMA_REFS
        .iter()
        .find(|(token, _)| *token == payload_type)
        .map(|(_, schema_ref)| *schema_ref)
}

/// Which entity a body core describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntityKind {
    Product,
    Sku,
}

impl EntityKind {
    /// The wire spelling `entityKind` carries.
    ///
    /// **The same two bytes the SDK already publishes**, and that is the fact
    /// to hold on to: `bss_products_sdk::models::EntityKind::as_str` renders
    /// `"product"`/`"sku"` too, and its own doc calls that *"the stable wire
    /// spelling, which is also the value the `entity_kind` column and the
    /// event body core carry"*. Two definitions, one value. An earlier
    /// revision of this doc called the value provisional while the SDK's
    /// called it stable; the SDK's is the one a consumer reads, so **stable**
    /// is the reading, and §4.5's silence on casing is not a licence for this
    /// copy to drift from it.
    ///
    /// Why a second definition at all: this enum is `pub(crate)` and the SDK's
    /// is the published contract. Collapsing them is a real simplification and
    /// it is **owed**, not declined — it belongs with slice 12's consumer
    /// contract, which is where the SDK type's own audience is decided. Until
    /// then the guard is `events_tests`, which asserts the rendered value
    /// rather than this function.
    #[must_use]
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Sku => "sku",
        }
    }
}

/// The body core every Foundation event carries (§4.5, P-D-27):
/// `{tenantId, entityKind, entityId, internalRevision, lifecycleState}`.
///
/// # The consumer contract, stated (`dod-body-core`)
///
/// Two sentences a consumer can get wrong while every field is present:
///
/// - **`internalRevision` is the value AS COMMITTED by the act** (P-D-29),
///   never the pre-act number. A consumer correlating an event to an `ETag`
///   compares the two **directly** — adjusting by one re-introduces exactly
///   the off-by-one this sentence exists to rule out. For a create, that is
///   always the freshly inserted row's own `1`, since nothing before this
///   event could have moved it.
/// - **`lifecycleState` is the discriminator on
///   `ProductHeadSaved`/`SkuHeadSaved`**: one event type covers a save on a
///   `draft`, `published` or `deprecated` head alike, and this field — not
///   the event type — is what tells them apart.
///
/// @cpt-dod:cpt-cf-bss-products-dod-body-core:p1
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventBodyCore {
    pub tenant_id: Uuid,
    pub entity_kind: &'static str,
    pub entity_id: Uuid,
    pub internal_revision: i64,
    pub lifecycle_state: &'static str,
}

/// A `*Published` body: the shared [`EventBodyCore`], **plus** the one field
/// §4.5 puts on `ProductPublished` and `SkuPublished` beyond it.
///
/// See this module's doc, "`publishedVersion` sits outside the core", for
/// why this is a second type rather than a sixth field on the core. The
/// `flatten` is what keeps the wire object flat: a consumer reads
/// `{tenantId, entityKind, entityId, internalRevision, lifecycleState,
/// publishedVersion}`, one object, exactly as §4.5 writes it.
///
/// The core is **borrowed**, not owned: every caller already built one for
/// its own act and there is nothing here to take ownership of.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PublishedEventBody<'core> {
    /// The five fields every Foundation event carries.
    #[serde(flatten)]
    pub core: &'core EventBodyCore,
    /// The version the publish act **produced** — `N + 1`, the key the
    /// frozen `products_entity_version` row was written at, never the `N`
    /// the head carried before the act. `06` reads this as the content
    /// pointer and `08`'s projector keys on it, so a body carrying the
    /// pre-act number would point both at a version that is not the one this
    /// event announces.
    pub published_version: i64,
}

/// A `*Deprecated` body: the shared [`EventBodyCore`], **plus** the
/// provenance `dod-deprecation-provenance` requires *"in its payload"*.
///
/// [`PublishedEventBody`]'s shape and for its reasons — flattened, so a
/// consumer reads one object, and the core borrowed because the act already
/// built one.
///
/// # Why the provenance is on the wire and not only on the row
///
/// A consumer's own reaction differs by cause. `design/04` has the registry
/// marks and exposes while the new-adoption block is the consumer's
/// (pricing AC #82), and a consumer that had to re-read the head to learn
/// whether a deprecation was the operator's or a parent's would be reading a
/// row that may have moved again by then. The cause travels with the
/// announcement or it is not reliably knowable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DeprecatedEventBody<'core> {
    /// The five fields every event of this gear carries.
    #[serde(flatten)]
    pub core: &'core EventBodyCore,
    /// `direct` or `cascaded` — the value written to the row in the very
    /// statement this event announces, so the two cannot disagree.
    pub provenance: &'static str,
}

/// Failures constructing or enqueuing an event. Never a `DomainError`: an
/// event that cannot be serialized or enqueued is an infrastructure fault of
/// this door's own mutation, not a business refusal of the caller's request
/// — the caller sees a `500`, mapped by whichever door calls this module,
/// the same way [`crate::infra::storage::RepoError::Db`] renders one.
#[derive(Debug, thiserror::Error)]
pub(crate) enum EventsError {
    /// Every field on [`EventBodyCore`] is a plain scalar, a `Uuid` or a
    /// `&'static str` — none can fail to serialize as JSON — so this arm is
    /// unreached in practice. It exists because `serde_json::to_vec` returns
    /// a `Result`, and this door does not reach for `.expect()` (a denied
    /// restriction lint) to discharge one it cannot prove is impossible from
    /// the type alone.
    #[error("serialize event payload: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error("enqueue event: {0}")]
    Outbox(#[from] OutboxError),
    /// [`schema_ref_for`] did not recognise the payload type, so the event
    /// has no versioned schema reference to announce.
    ///
    /// Refused rather than defaulted: see [`schema_ref_for`]'s own doc. It is
    /// unreachable while every caller passes one of [`SCHEMA_REFS`]' thirteen
    /// tokens, and `events_tests` holds that roster equal to the union of the
    /// three declared ones — but a fourteenth event added without an entry
    /// lands here, at its first enqueue, instead of on a consumer.
    #[error("no versioned schema reference registered for payload type {0}")]
    UnregisteredSchema(String),
    /// A `*Published` token reached [`enqueue`], whose body has no
    /// `publishedVersion` to carry.
    ///
    /// The two entry points exist precisely so a publish cannot be announced
    /// without the version it published at (see [`enqueue_published`]'s own
    /// doc). Until this variant the wrong entry point emitted a body §4.5 calls
    /// incomplete and nothing noticed; the SDK's typed events made the
    /// distinction a compile-time one on the broker path, and this makes it a
    /// runtime one on both.
    #[error("{0} carries a publishedVersion and must be enqueued through enqueue_published")]
    PublishNeedsVersion(String),
    /// A core-only token reached [`enqueue_published`], which would attach a
    /// `publishedVersion` §4.5 does not put on it.
    #[error("{0} carries no publishedVersion and must be enqueued through enqueue")]
    NotAPublishEvent(String),
    /// A token that is not one of the two `*Deprecated` events reached
    /// [`enqueue_deprecated`], which would attach a `provenance` no design
    /// document puts on it.
    ///
    /// The third arm of the same fail-closed rule as the two above: each
    /// entry point admits exactly the tokens whose body shape it builds, so a
    /// mis-routed call is a refusal rather than a wire body with a surplus
    /// field.
    #[error("{0} carries no provenance and belongs to the entry point owning its body shape")]
    NotADeprecationEvent(String),
    /// A `*Deprecated` token reached [`enqueue`], whose core-only body would
    /// drop the one field `dod-deprecation-provenance` requires on the wire.
    ///
    /// [`Self::PublishNeedsVersion`]'s reciprocal for the deprecation pair:
    /// without it the broker arm's typed-event `match` refuses the mis-route
    /// while the interim arm writes the bare core straight through — the two
    /// sinks disagreeing about a rule that is the body shape's, not the
    /// sink's.
    #[error("{0} carries a provenance and must be enqueued through enqueue_deprecated")]
    DeprecationNeedsProvenance(String),
    /// A token outside the three set events reached [`enqueue_set_event`],
    /// whose set-shaped body no other event carries — the fourth arm of the
    /// entry points' fail-closed rule.
    #[error(
        "{0} is not a recognized-set event and belongs to the entry point owning its body shape"
    )]
    NotASetEvent(String),
    /// A set token reached [`enqueue`], whose entity-shaped
    /// [`EventBodyCore`] carries none of the set's operands.
    ///
    /// The reciprocal the deprecation pair already had, and its absence
    /// was the same sink divergence: the broker arm refused a mis-routed
    /// set token as [`Self::NoTypedEvent`] while the interim arm wrote the
    /// entity core straight through under the set event's schema
    /// reference.
    #[error("{0} carries a set body and must be enqueued through enqueue_set_event")]
    SetEventNeedsSetBody(String),
    /// A token that is not the batch summary reached
    /// [`enqueue_bulk_completed`], whose batch-shaped body no other event
    /// carries — the fifth arm of the entry points' fail-closed rule.
    #[error(
        "{0} is not the bulk completion summary and belongs to the entry point owning its body shape"
    )]
    NotABulkEvent(String),
    /// The batch summary reached [`enqueue`], whose entity-shaped core
    /// carries neither the batch id nor the digest.
    #[error("{0} carries a batch body and must be enqueued through enqueue_bulk_completed")]
    BulkEventNeedsBatchBody(String),
    /// The broker arm has no [`crate::infra::broker`] typed event for this
    /// payload type.
    ///
    /// Distinct from [`Self::UnregisteredSchema`], which is the interim arm's
    /// roster miss: the two arms resolve a token through two different rosters
    /// — `SCHEMA_REFS` there, a `match` over the thirteen typed events here — and
    /// naming both misses the same thing would send a reader to the wrong one.
    /// A fourteenth event registered in `SCHEMA_REFS` but not wired here reaches
    /// this variant, and a no-broker deployment would have emitted it.
    #[error("no typed event is declared for payload type {0} on the broker arm")]
    NoTypedEvent(String),
    /// The broker SDK refused the enqueue.
    ///
    /// Reached only on [`crate::infra::broker::EventSink::Broker`]. The door
    /// maps it exactly as it maps [`Self::Outbox`]: the act's transaction
    /// rolls back, because an entity row whose announcement was refused is the
    /// split `dod-create-doors` exists to prevent.
    #[error("the broker producer refused the enqueue: {0}")]
    Broker(#[from] event_broker_sdk::EventBrokerError),
}

/// The envelope every enqueued event is wrapped in.
///
/// # Why the obligations sit here and not on the broker's own envelope
///
/// P-D-01 fixes **five** semantic obligations and calls them
/// **envelope-agnostic**: versioned (semver) schema references, `vN`->`vN+1`
/// consumer compatibility, correlation/causation, per-aggregate ordering keys
/// `(tenant, aggregate)`, and pseudonymous actors. The fifth of those is not
/// this slice's: §4.5 puts the schema-versioning discipline and the
/// replay/bootstrap path in **slice 12**, and leaves the Foundation the
/// envelope itself. An earlier revision of this paragraph said "four" and
/// dropped the compatibility clause outright, which is why it is spelled out
/// here.
///
/// Measured against the platform as it stands, the broker's `Event`
/// (`gears/system/event-broker/event-broker-sdk`, `models::Event`) carries
/// `id`, `type_id`, `topic`, `tenant_id`, `source`, `subject`,
/// `subject_type`, `partition_key`, `occurred_at`, `trace_parent` and `data`
/// — and **no field for a causation id, and none for an actor**. Counted
/// honestly, that is **two values with no broker slot**, not three
/// obligations: a schema reference maps onto `type_id`, an ordering key onto
/// `partition_key`, and a correlation id onto `trace_parent`. The two that
/// do not map are discharged here, in the payload this gear controls, which
/// is exactly what "envelope-agnostic" licenses.
///
/// # This envelope is an interim shape, not one whose fields lift
///
/// It is tempting to read the struct below as a draft of what the producer
/// will hand the broker. It is not. `producer::outbox::ProducerOutboxEnvelope`
/// owns **both** ends already: it declares its own field set, stamps its own
/// fixed payload-type token (`PRODUCER_OUTBOX_PAYLOAD_TYPE`, a versioned MIME
/// string) and carries its own `broker_partition`, which the SDK computes from
/// the broker's rule and not from [`partition_for`]. When P-D-47's producer is
/// bound to [`QUEUE_NAME`] it will neither deserialize the rows this module
/// writes nor agree with the partition they were routed on. So the honest
/// reading is that the SDK **replaces** this envelope rather than lifting
/// fields out of it, and the rows written before that point are this gear's
/// own record, not a consumer's.
///
/// # `data` is a nested object, not a flattening
///
/// The body core keeps its own object rather than being `flatten`ed beside
/// these five, so a consumer reading §4.5's five fields reads them from one
/// place whatever the envelope grows next, and an envelope field can never
/// collide with a body field of the same name.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EventEnvelope<'body, B: Serialize> {
    /// The event's own identity **on this interim envelope**, minted per
    /// enqueue: a consumer that sees this id twice has seen one enqueue twice.
    ///
    /// **Not P-D-47's idempotency key, and it cannot become one.** That key is
    /// the id on the broker's `Event`, and the SDK mints it itself:
    /// `producer::event_factory` sets `id: Uuid::now_v7()` unconditionally,
    /// while `ProducerOutbox::enqueue` takes a `TypedEvent` and no id — there
    /// is no parameter through which a value minted here could reach it. So
    /// when the producer lands there will be **two** ids per event, and the
    /// consumer-visible one will be the SDK's, not this. Read this field as
    /// what it is: the interim envelope's own handle, useful for correlating
    /// an outbox row with the act that wrote it, and superseded the moment
    /// P-D-47's producer is bound.
    ///
    /// Minted rather than derived from the act, because the same act may
    /// legitimately emit more than one event and a derived id would make them
    /// indistinguishable.
    pub event_id: Uuid,
    /// The versioned schema reference for [`Self::data`]'s shape
    /// ([`SCHEMA_REFS`]).
    pub schema_ref: &'static str,
    /// The W3C trace id of the request that caused this event, where this
    /// gear is running inside a traced request.
    ///
    /// **Read from the ambient span, never minted** ([`correlation_id`]). A
    /// minted-per-event value would correlate nothing while reading, to an
    /// operator, as though it did — the same judgement
    /// `repo::AuditCommon::correlation_id` records for the audit trail's own
    /// column, and the reason this field is an `Option` that is honestly
    /// absent rather than a `Uuid` that is always present and usually a lie.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// The event that caused this one.
    ///
    /// **`None` for every event this gear emits, and that is the measurement
    /// rather than an omission**: every one of them is caused by an operator
    /// request, not by another event, so there is no event id to name. It
    /// becomes populated the first time a slice emits an event *in reaction
    /// to* one. Carrying the field with an honest `None` is what lets a
    /// consumer tell "not caused by an event" from "nobody filled this in";
    /// minting the correlation id into it would collapse the distinction the
    /// pair exists to draw.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub causation_id: Option<Uuid>,
    /// The acting principal's **pseudonymous** ref — never a direct operator
    /// identity. The value slice 10's identity-reference map minted; the same
    /// one the act's audit row would carry.
    pub actor_ref: Uuid,
    /// The event body: [`EventBodyCore`], or [`PublishedEventBody`] for the
    /// two publish events.
    pub data: &'body B,
}

/// The W3C trace id of the request in scope, where there is one.
///
/// `None` outside a traced request — a background task, or a test that
/// installed no subscriber — which is why every caller carries it as an
/// `Option` rather than substituting a value.
///
/// The idiom, not an invention: `gears/mini-chat`'s
/// `domain::service::current_otel_trace_id` reads the same id the same way,
/// and `toolkit`'s `api::canonical_error_layer::extract_trace_id` puts the
/// same 32-hex trace-id segment on every canonical error. Rendering it as
/// that hex string rather than as a `Uuid` is what keeps this value
/// **grep-equal** to the one in the access log, the `OTel` span and the error
/// envelope; a `Uuid` rendering of the same 128 bits would carry hyphens and
/// join to none of them by string equality.
///
/// # What has to be true of the host for this to answer anything
///
/// This reads a layer it does not install. Three conditions of the *host*
/// binary, none of them this gear's to satisfy, each of which makes every
/// answer here a permanent `None`:
///
/// - `toolkit::telemetry::init_tracing` — the only builder of the
///   `OpenTelemetryLayer` in this workspace (`libs/toolkit/src/telemetry/
///   init.rs:184`) — is `#[cfg(feature = "otel")]`, so it is absent from the
///   API of a `toolkit` built without it. That feature is in `toolkit`'s own
///   `default` set (`libs/toolkit/Cargo.toml:28`), so this bites only a host
///   that opts out with `default-features = false`;
/// - it returns `Err` outright when `opentelemetry.tracing.enabled` is false;
/// - it otherwise builds an OTLP exporter from the resolved endpoint, and a
///   failure there is an `Err` too.
///
/// In each case the host's subscriber carries no `OpenTelemetry` layer,
/// `Span::current()` has no `OTel` context to hand back, and this function
/// answers `None` for every request for the life of the process — the events
/// still emit, and their `correlationId` is absent rather than wrong. The
/// in-crate positive control installs the layer itself precisely so that a
/// `None` caused by *this* function, rather than by the host, is a red test.
#[must_use]
pub(crate) fn correlation_id() -> Option<String> {
    use tracing_opentelemetry::OpenTelemetrySpanExt as _;

    let context = tracing::Span::current().context();
    let trace_id = opentelemetry::trace::TraceContextExt::span(&context)
        .span_context()
        .trace_id();
    (trace_id != opentelemetry::trace::TraceId::INVALID).then(|| trace_id.to_string())
}

/// The full W3C `traceparent` of the request in scope, where there is one.
///
/// [`correlation_id`]'s sibling, and **not** interchangeable with it. That one
/// answers the bare 32-hex trace id, which is the value the interim envelope
/// calls `correlationId` and which stays grep-equal to the access log. This one
/// answers the header form — `00-<trace-id>-<span-id>-<flags>` — because that
/// is what the broker's `Event.trace_parent` field is named for, and putting a
/// bare trace id in a field called `trace_parent` would be a claim about the
/// value that is not true of it.
///
/// `None` under exactly the conditions [`correlation_id`] answers `None`; see
/// that function's doc for the three host preconditions.
#[must_use]
pub(crate) fn traceparent() -> Option<String> {
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

/// P-D-22's partition formula, the one place it is computed.
///
/// `N` is fixed at [`PARTITIONS`]; a door never passes its own count, which
/// is what keeps this the single source `gear.rs`'s queue registration — made,
/// not eventual — is kept equal to.
#[must_use]
pub(crate) fn partition_for(tenant_id: Uuid, aggregate_id: Uuid) -> u32 {
    let combined = tenant_id.as_u128() ^ aggregate_id.as_u128().rotate_left(64);
    #[allow(clippy::cast_possible_truncation)]
    {
        (combined % u128::from(PARTITIONS)) as u32
    }
}

/// Enqueue one Foundation event on [`QUEUE_NAME`], in the caller's own
/// transaction.
///
/// `runner` MUST be the door's own mutation transaction — the same one the
/// entity insert this event announces just committed into — since
/// `dod-create-doors` requires the entity row and its creation outbox row in
/// one transaction. This function does not open one of its own, for the same
/// reason every function in `infra::storage::repo` does not (see that
/// module's doc): it takes whatever runner the caller hands it.
///
/// # Errors
/// [`EventsError::Serialize`] if `core` cannot be rendered as JSON (see that
/// variant's own doc for why this is unreached in practice);
/// [`EventsError::Outbox`] on a queue/partition/storage failure from
/// [`Outbox::enqueue`].
pub(crate) async fn enqueue(
    sink: &EventSink,
    runner: &(impl DBRunner + Sync),
    aggregate_id: Uuid,
    payload_type: &str,
    core: &EventBodyCore,
    actor_ref: Uuid,
) -> Result<(), EventsError> {
    if matches!(
        payload_type,
        PRODUCT_PUBLISHED_PAYLOAD_TYPE | SKU_PUBLISHED_PAYLOAD_TYPE
    ) {
        return Err(EventsError::PublishNeedsVersion(payload_type.to_owned()));
    }
    if matches!(
        payload_type,
        PRODUCT_DEPRECATED_PAYLOAD_TYPE | SKU_DEPRECATED_PAYLOAD_TYPE
    ) {
        return Err(EventsError::DeprecationNeedsProvenance(
            payload_type.to_owned(),
        ));
    }
    if matches!(
        payload_type,
        RECOGNIZED_UNIT_UPDATED_PAYLOAD_TYPE
            | RECOGNIZED_CODE_UPDATED_PAYLOAD_TYPE
            | PLAN_TIER_UPDATED_PAYLOAD_TYPE
    ) {
        return Err(EventsError::SetEventNeedsSetBody(payload_type.to_owned()));
    }
    if payload_type == CATALOG_BULK_OPERATION_COMPLETED_PAYLOAD_TYPE {
        return Err(EventsError::BulkEventNeedsBatchBody(
            payload_type.to_owned(),
        ));
    }
    match sink {
        EventSink::Interim(outbox) => {
            enqueue_body(
                outbox,
                runner,
                core.tenant_id,
                aggregate_id,
                payload_type,
                core,
                actor_ref,
            )
            .await
        }
        EventSink::Broker(producer) => {
            let body = broker::CatalogEventCore::from_core(core, actor_ref);
            match payload_type {
                PRODUCT_CREATED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(runner, broker::ProductCreated { core: body })
                        .await
                }
                SKU_CREATED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(runner, broker::SkuCreated { core: body })
                        .await
                }
                PRODUCT_HEAD_SAVED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(runner, broker::ProductHeadSaved { core: body })
                        .await
                }
                SKU_HEAD_SAVED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(runner, broker::SkuHeadSaved { core: body })
                        .await
                }
                PRODUCT_DISCARDED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(runner, broker::ProductDiscarded { core: body })
                        .await
                }
                SKU_DISCARDED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(runner, broker::SkuDiscarded { core: body })
                        .await
                }
                // Not `UnregisteredSchema`: that variant's own doc says
                // `schema_ref_for` did not recognise the token, and on this arm
                // `schema_ref_for` was never called. The condition here is "no
                // `TypedEvent` is declared for this token", which is a different
                // repair — the rosters are equal today and a fourteenth event
                // would have to be added to both.
                other => return Err(EventsError::NoTypedEvent(other.to_owned())),
            }
            .map(|_| ())
            .map_err(EventsError::Broker)
        }
    }
}

/// Enqueue a `*Deprecated` event — [`enqueue`]'s twin for the one body shape
/// that carries a cause ([`DeprecatedEventBody`]).
///
/// # Why a third function and not a `provenance: Option<&str>` on [`enqueue`]
///
/// [`enqueue_published`]'s own argument, unchanged: an `Option` would make
/// every other `enqueue` call site pass a `None` that means nothing to
/// them, and would let a `*Deprecated` event reach the wire with no
/// provenance — the one field `dod-deprecation-provenance` requires it to
/// carry. The token guard below is the same fail-closed shape: this function
/// refuses any payload type that is not one of the two, so a caller cannot
/// route a `ProductCreated` through the provenance-carrying body.
///
/// # Errors
///
/// [`EventsError::NotADeprecationEvent`] for any other token;
/// otherwise as [`enqueue`].
pub(crate) async fn enqueue_deprecated(
    sink: &EventSink,
    runner: &(impl DBRunner + Sync),
    aggregate_id: Uuid,
    payload_type: &str,
    core: &EventBodyCore,
    provenance: &'static str,
    actor_ref: Uuid,
) -> Result<(), EventsError> {
    if !matches!(
        payload_type,
        PRODUCT_DEPRECATED_PAYLOAD_TYPE | SKU_DEPRECATED_PAYLOAD_TYPE
    ) {
        return Err(EventsError::NotADeprecationEvent(payload_type.to_owned()));
    }
    match sink {
        EventSink::Interim(outbox) => {
            let body = DeprecatedEventBody { core, provenance };
            enqueue_body(
                outbox,
                runner,
                core.tenant_id,
                aggregate_id,
                payload_type,
                &body,
                actor_ref,
            )
            .await
        }
        EventSink::Broker(producer) => {
            let body = broker::CatalogEventCore::from_core(core, actor_ref);
            let provenance = provenance.to_owned();
            match payload_type {
                PRODUCT_DEPRECATED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(
                            runner,
                            broker::ProductDeprecated {
                                core: body,
                                provenance,
                            },
                        )
                        .await
                }
                SKU_DEPRECATED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(
                            runner,
                            broker::SkuDeprecated {
                                core: body,
                                provenance,
                            },
                        )
                        .await
                }
                // Unreachable while the guard above owns the condition; kept
                // total so a third `*Deprecated` token added to that guard and
                // forgotten here is a refusal, not a body published under
                // `sku_deprecated.v1`'s id with the wrong subject.
                other => return Err(EventsError::NoTypedEvent(other.to_owned())),
            }
            .map(|_| ())
            .map_err(EventsError::Broker)
        }
    }
}

/// A recognized-set event's body: which set, which member, which state it
/// now carries. Set-shaped, not entity-shaped — no id, no revision, no
/// lifecycle state — so it is its own type rather than a
/// [`EventBodyCore`] wearing blanks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetEventBody<'a> {
    /// The owning tenant — the ordering key's first half.
    pub tenant_id: Uuid,
    /// The set — the ordering key's second half.
    pub set_kind: &'a str,
    /// The member the mutation touched.
    pub member_code: &'a str,
    /// The member's state as this mutation committed it.
    pub state: &'a str,
}

/// Enqueue a recognized-set event — the fourth entry point, owning the
/// set-shaped body the same way [`enqueue_deprecated`] owns the
/// provenance-carrying one.
///
/// # The interim aggregate id is derived, and the derivation is the ordering
///
/// `design/03` §4 keys these events `(tenant, set_kind)`, and the interim
/// outbox partitions by `(tenant, aggregate_id)` — so the aggregate id is a
/// **v5 UUID of the set kind in the tenant's namespace**: deterministic, one
/// per `(tenant, set_kind)`, which makes the outbox's ordering exactly the
/// declared key. The broker arm needs no such derivation — its typed events
/// carry the set kind as the subject.
///
/// # Errors
///
/// [`EventsError::NotASetEvent`] for any token outside the three; otherwise
/// as [`enqueue`].
pub(crate) async fn enqueue_set_event(
    sink: &EventSink,
    runner: &(impl DBRunner + Sync),
    payload_type: &str,
    body: SetEventBody<'_>,
    actor_ref: Uuid,
) -> Result<(), EventsError> {
    if !matches!(
        payload_type,
        RECOGNIZED_UNIT_UPDATED_PAYLOAD_TYPE
            | RECOGNIZED_CODE_UPDATED_PAYLOAD_TYPE
            | PLAN_TIER_UPDATED_PAYLOAD_TYPE
    ) {
        return Err(EventsError::NotASetEvent(payload_type.to_owned()));
    }
    match sink {
        EventSink::Interim(outbox) => {
            let aggregate_id = Uuid::new_v5(&body.tenant_id, body.set_kind.as_bytes());
            enqueue_body(
                outbox,
                runner,
                body.tenant_id,
                aggregate_id,
                payload_type,
                &body,
                actor_ref,
            )
            .await
        }
        EventSink::Broker(producer) => {
            let tenant_id = body.tenant_id;
            let set_kind = body.set_kind.to_owned();
            let member_code = body.member_code.to_owned();
            let state = body.state.to_owned();
            match payload_type {
                RECOGNIZED_UNIT_UPDATED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(
                            runner,
                            broker::RecognizedUnitUpdated {
                                tenant_id,
                                set_kind,
                                member_code,
                                state,
                                actor_ref,
                            },
                        )
                        .await
                }
                RECOGNIZED_CODE_UPDATED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(
                            runner,
                            broker::RecognizedCodeUpdated {
                                tenant_id,
                                set_kind,
                                member_code,
                                state,
                                actor_ref,
                            },
                        )
                        .await
                }
                PLAN_TIER_UPDATED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(
                            runner,
                            broker::PlanTierUpdated {
                                tenant_id,
                                set_kind,
                                member_code,
                                state,
                                actor_ref,
                            },
                        )
                        .await
                }
                // Total for the guard's own reason: a fourth set event added
                // to the guard and forgotten here is a refusal, not a body
                // published under another event's type id.
                other => return Err(EventsError::NoTypedEvent(other.to_owned())),
            }
            .map(|_| ())
            .map_err(EventsError::Broker)
        }
    }
}

/// The batch-completion summary's body: which batch, and the digest over
/// the ledger it completed.
///
/// Batch-shaped, like the set events are set-shaped: there is no entity id,
/// no revision and no lifecycle state, because the subject is the batch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkCompletedEventBody<'a> {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The batch the summary closes — the subject.
    pub batch_id: Uuid,
    /// The import door's idempotency operand, echoed so a caller that only
    /// holds its own key can match the summary to its request.
    pub batch_key: &'a str,
    /// The digest over the completed ledger. **What it covers is `§7`'s
    /// open question**, not this type's: the design names *"the ledger
    /// digest"* and defines no computation, so the producer states its
    /// covered set and the register carries the question.
    pub ledger_digest: &'a str,
    /// How many rows the ledger closed, by the four terminal dispositions —
    /// the summary a consumer would otherwise re-derive by reading the whole
    /// ledger back.
    pub rows: BulkCompletedRows,
}

/// The completion summary's per-disposition counts.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BulkCompletedRows {
    /// Rows whose entity published.
    pub published: u32,
    /// Rows whose live-entity operation applied.
    pub applied: u32,
    /// Rows that were already in the requested state.
    pub no_op: u32,
    /// Rows that failed row-locally. **A batch with failed rows still
    /// completes** — parts-succeeded is the honest end state.
    pub failed: u32,
}

/// Enqueue the batch-completion summary — the fifth entry point, owning the
/// batch-shaped body.
///
/// # Errors
///
/// [`EventsError::NotABulkEvent`] for any other token; otherwise as
/// [`enqueue`].
pub(crate) async fn enqueue_bulk_completed(
    sink: &EventSink,
    runner: &(impl DBRunner + Sync),
    payload_type: &str,
    body: BulkCompletedEventBody<'_>,
    actor_ref: Uuid,
) -> Result<(), EventsError> {
    if payload_type != CATALOG_BULK_OPERATION_COMPLETED_PAYLOAD_TYPE {
        return Err(EventsError::NotABulkEvent(payload_type.to_owned()));
    }
    match sink {
        EventSink::Interim(outbox) => {
            enqueue_body(
                outbox,
                runner,
                body.tenant_id,
                body.batch_id,
                payload_type,
                &body,
                actor_ref,
            )
            .await
        }
        EventSink::Broker(producer) => producer
            .enqueue(
                runner,
                broker::CatalogBulkOperationCompleted {
                    tenant_id: body.tenant_id,
                    batch_id: body.batch_id,
                    batch_key: body.batch_key.to_owned(),
                    ledger_digest: body.ledger_digest.to_owned(),
                    published: body.rows.published,
                    applied: body.rows.applied,
                    no_op: body.rows.no_op,
                    failed: body.rows.failed,
                    actor_ref,
                },
            )
            .await
            .map(|_| ())
            .map_err(EventsError::Broker),
    }
}

/// [`enqueue`] for the two `*Published` events, whose body is the core
/// **plus** `publishedVersion` (§4.5, [`PublishedEventBody`]).
///
/// A separate entry point rather than an `Option<i64>` on [`enqueue`]: the
/// six core-only events have no version to pass and would each have to write
/// a `None` that means nothing, and a publish that passed `None` by mistake
/// would silently emit a body §4.5 says is incomplete. Here the field is a
/// plain `i64` the caller cannot omit.
///
/// `published_version` MUST be the **post-act** version — the key the frozen
/// row was written at; see [`PublishedEventBody::published_version`].
///
/// # Errors
/// [`EventsError::Serialize`] and [`EventsError::Outbox`], exactly as
/// [`enqueue`] raises them.
pub(crate) async fn enqueue_published(
    sink: &EventSink,
    runner: &(impl DBRunner + Sync),
    aggregate_id: Uuid,
    payload_type: &str,
    core: &EventBodyCore,
    published_version: i64,
    actor_ref: Uuid,
) -> Result<(), EventsError> {
    // Hoisted above the match, like [`enqueue`]'s twin guard. It used to sit
    // inside the `Interim` arm while the `Broker` arm relied on its own
    // fallthrough — two copies of one rule, so a third publish event added to
    // the broker's match and forgotten in this list would have been accepted on
    // one arm and refused on the other.
    if !matches!(
        payload_type,
        PRODUCT_PUBLISHED_PAYLOAD_TYPE | SKU_PUBLISHED_PAYLOAD_TYPE
    ) {
        return Err(EventsError::NotAPublishEvent(payload_type.to_owned()));
    }
    match sink {
        EventSink::Interim(outbox) => {
            let body = PublishedEventBody {
                core,
                published_version,
            };
            enqueue_body(
                outbox,
                runner,
                core.tenant_id,
                aggregate_id,
                payload_type,
                &body,
                actor_ref,
            )
            .await
        }
        EventSink::Broker(producer) => {
            let body = broker::CatalogEventCore::from_core(core, actor_ref);
            match payload_type {
                PRODUCT_PUBLISHED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(
                            runner,
                            broker::ProductPublished {
                                core: body,
                                published_version,
                            },
                        )
                        .await
                }
                SKU_PUBLISHED_PAYLOAD_TYPE => {
                    producer
                        .enqueue(
                            runner,
                            broker::SkuPublished {
                                core: body,
                                published_version,
                            },
                        )
                        .await
                }
                // Unreachable while the hoisted guard above owns the
                // condition; kept so the `match` stays total if the roster of
                // publish events ever grows past that guard's list.
                other => return Err(EventsError::NotAPublishEvent(other.to_owned())),
            }
            .map(|_| ())
            .map_err(EventsError::Broker)
        }
    }
}

/// The one place a body is wrapped, rendered, partitioned and handed to the
/// outbox.
///
/// Both public entry points above go through it, so the envelope, the queue
/// and P-D-22's partition formula are written once and a new body shape
/// cannot arrive with its own copy of any of the three. `tenant_id` is an
/// argument rather than read off `body` because a `Serialize` value has no
/// field a function can read; both callers pass their own core's
/// `tenant_id`, which is the same value the body itself carries.
///
/// The schema reference is resolved **before** anything is written, so an
/// event with no registered schema fails the caller's transaction rather than
/// reaching the queue unidentifiable.
///
/// # Errors
/// [`EventsError::UnregisteredSchema`] if `payload_type` is not one of
/// [`SCHEMA_REFS`]'; [`EventsError::Serialize`] if the envelope cannot be
/// rendered as JSON; [`EventsError::Outbox`] on a queue/partition/storage
/// failure.
async fn enqueue_body(
    outbox: &Outbox,
    runner: &(impl DBRunner + Sync),
    tenant_id: Uuid,
    aggregate_id: Uuid,
    payload_type: &str,
    body: &impl Serialize,
    actor_ref: Uuid,
) -> Result<(), EventsError> {
    let schema_ref = schema_ref_for(payload_type)
        .ok_or_else(|| EventsError::UnregisteredSchema(payload_type.to_owned()))?;
    let envelope = EventEnvelope {
        event_id: Uuid::new_v4(),
        schema_ref,
        correlation_id: correlation_id(),
        // See the field's own doc: an operator request causes every one,
        // and a request is not an event.
        causation_id: None,
        actor_ref,
        data: body,
    };
    let payload = serde_json::to_vec(&envelope)?;
    let partition = partition_for(tenant_id, aggregate_id);
    outbox
        .enqueue(runner, QUEUE_NAME, partition, payload, payload_type)
        .await?;
    Ok(())
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod events_tests;
