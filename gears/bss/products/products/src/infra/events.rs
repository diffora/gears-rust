//! The Foundation event envelope, shared by every create/save/publish door
//! this gear opens (`design/01-foundation.md` §4.5, P-D-27).
//!
//! # One home for the body core, so `SkuCreated` does not duplicate it
//!
//! §4.5 fixes one body core across all eight Foundation events —
//! `{tenantId, entityKind, entityId, internalRevision, lifecycleState}` — and
//! names anything beyond it where the act that adds it is specified (only
//! `*Published`, with `publishedVersion`). [`EventBodyCore`] is that shape;
//! six of the eight carry **only** the core, built through this same type,
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
//! event of one aggregate lands in one partition, which is what keeps their
//! relative order — the ordering key `fr-registry-eventing-audit`'s AC #28
//! asks for. [`partition_for`] is the one function that computes it; a door
//! calls it, never re-derives it. The hash itself is the same idiom
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
//! `crate::gear::BssProductsGear`'s to build and hand to the router. See
//! `api/rest.rs`'s module doc for the wiring gap this leaves and who owns
//! closing it: `gear.rs` is outside every target path this slice was allowed
//! to touch, and the queue this door needs registered (with a consumer
//! handler — [`OUTBOX_TABLE_PREFIX`]'s sibling constant, [`QUEUE_NAME`], and
//! [`PARTITIONS`]) lives in its builder chain, not here.

use serde::Serialize;
use toolkit_db::outbox::{Outbox, OutboxError};
use toolkit_db::secure::DBRunner;
use uuid::Uuid;

/// Table-family prefix this door's events are enqueued under.
///
/// MUST equal `crate::gear::OUTBOX_TABLE_PREFIX` (currently a private
/// constant of that module, duplicated here rather than imported because
/// `gear.rs` is outside this slice's target paths) — a mismatch would point
/// this door's `enqueue` at tables the running pipeline never created.
pub(crate) const OUTBOX_TABLE_PREFIX: &str = "bss_products_outbox";

/// The one queue every Foundation event on this gear's Product/SKU surface
/// enqueues onto. One queue rather than one per entity: P-D-27's ordering
/// key is `(tenant, aggregate)`, not `(tenant, aggregate, entity_kind)`, and
/// splitting the queue would not change the partitioning, only the registry
/// entry a consumer subscribes to.
pub(crate) const QUEUE_NAME: &str = "bss_products_events";

/// The fixed partition count P-D-22's modulus divides by. Chosen once, here,
/// so [`partition_for`] and the queue's own registration (owed to `gear.rs`,
/// see this module's doc) never disagree on `N` — a registration with a
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
/// still owed, and it answers [`MessageResult::Retry`] to every message:
/// a transient failure, which leaves the row in the queue and dead-letters
/// nothing. That is the honest shape of "enqueued, not yet deliverable".
///
/// It deliberately does **not** answer `Ok`. An `Ok` would mark the message
/// delivered and hand it to the vacuum, so every event this gear enqueues
/// before Phase 8 would be reclaimed having reached no broker at all — a
/// silent loss of exactly the events `fr-registry-eventing-audit` requires.
/// A queue that visibly cannot deliver is recoverable; one that quietly
/// discards is not.
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
            "bss-products: outbox delivery is owed to Phase 8's broker producer (P-D-47); \
             holding the message in the queue"
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

/// Which entity a body core describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EntityKind {
    Product,
    Sku,
}

impl EntityKind {
    /// The wire spelling `entityKind` carries. Lower-case, matching
    /// `LifecycleState::as_str()`'s own convention on this payload's sibling
    /// field — §4.5 does not pin a casing, so this is a documented choice
    /// slice 12's consumer contract may yet override, not a spec citation.
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
/// `internal_revision` is the value **as committed by the act** (P-D-29) —
/// for a create, that is always the freshly inserted row's own `1`, since
/// nothing before this event could have moved it.
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
}

/// P-D-22's partition formula, the one place it is computed.
///
/// `N` is fixed at [`PARTITIONS`]; a door never passes its own count, which
/// is what keeps this the single source `gear.rs`'s eventual queue
/// registration (see this module's doc) must be kept equal to.
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
    outbox: &Outbox,
    runner: &(impl DBRunner + Sync),
    aggregate_id: Uuid,
    payload_type: &str,
    core: &EventBodyCore,
) -> Result<(), EventsError> {
    enqueue_body(
        outbox,
        runner,
        core.tenant_id,
        aggregate_id,
        payload_type,
        core,
    )
    .await
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
    outbox: &Outbox,
    runner: &(impl DBRunner + Sync),
    aggregate_id: Uuid,
    payload_type: &str,
    core: &EventBodyCore,
    published_version: i64,
) -> Result<(), EventsError> {
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
    )
    .await
}

/// The one place a body is rendered, partitioned and handed to the outbox.
///
/// Both public entry points above go through it, so the envelope, the queue
/// and P-D-22's partition formula are written once and a new body shape
/// cannot arrive with its own copy of any of the three. `tenant_id` is an
/// argument rather than read off `body` because a `Serialize` value has no
/// field a function can read; both callers pass their own core's
/// `tenant_id`, which is the same value the body itself carries.
///
/// # Errors
/// [`EventsError::Serialize`] if `body` cannot be rendered as JSON;
/// [`EventsError::Outbox`] on a queue/partition/storage failure.
async fn enqueue_body(
    outbox: &Outbox,
    runner: &(impl DBRunner + Sync),
    tenant_id: Uuid,
    aggregate_id: Uuid,
    payload_type: &str,
    body: &impl Serialize,
) -> Result<(), EventsError> {
    let payload = serde_json::to_vec(body)?;
    let partition = partition_for(tenant_id, aggregate_id);
    outbox
        .enqueue(runner, QUEUE_NAME, partition, payload, payload_type)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{PARTITIONS, partition_for};
    use uuid::Uuid;

    /// `partition_for` must never answer outside `[0, PARTITIONS)` — an
    /// out-of-range answer would make `Outbox::enqueue`'s own
    /// `resolve_partition` fail every call with `PartitionOutOfRange`,
    /// turning every create into a `500` regardless of anything the door
    /// itself does right.
    #[test]
    fn partition_for_answers_within_the_configured_partition_count() {
        for _ in 0..64 {
            let tenant_id = Uuid::new_v4();
            let aggregate_id = Uuid::new_v4();
            let partition = partition_for(tenant_id, aggregate_id);
            assert!(
                partition < u32::from(PARTITIONS),
                "partition {partition} must be < PARTITIONS ({PARTITIONS})"
            );
        }
    }

    /// The same `(tenant_id, aggregate_id)` pair must always land on the
    /// same partition — this is the entire property P-D-22 asks the formula
    /// for: every event of one aggregate keeps its relative order because
    /// they all land in one partition.
    #[test]
    fn partition_for_is_deterministic_for_the_same_pair() {
        let tenant_id = Uuid::new_v4();
        let aggregate_id = Uuid::new_v4();
        assert_eq!(
            partition_for(tenant_id, aggregate_id),
            partition_for(tenant_id, aggregate_id),
            "the same (tenant_id, aggregate_id) pair must hash to the same partition every time"
        );
    }
}
