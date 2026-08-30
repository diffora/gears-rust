//! The Foundation event envelope, shared by every create/save/publish door
//! this gear opens (`design/01-foundation.md` §4.5, P-D-27).
//!
//! # One home for the body core, so `SkuCreated` does not duplicate it
//!
//! §4.5 fixes one body core across all eight Foundation events —
//! `{tenantId, entityKind, entityId, internalRevision, lifecycleState}` — and
//! names anything beyond it where the act that adds it is specified (only
//! `*Published`, with `publishedVersion`, so far). [`EventBodyCore`] is that
//! shape; `ProductCreated` ([`PRODUCT_CREATED_PAYLOAD_TYPE`]) and `SkuCreated`
//! ([`SKU_CREATED_PAYLOAD_TYPE`]) both carry **only** the core, built through
//! this same type, rather than each redeclaring the five fields a second
//! time.
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
    let payload = serde_json::to_vec(core)?;
    let partition = partition_for(core.tenant_id, aggregate_id);
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
