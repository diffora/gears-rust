use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;
use toolkit_security::SecurityContext;
use tracing::{trace, warn};

use futures_util::{Stream, StreamExt};
use gts::{GtsIdPattern, GtsInstanceId};
use toolkit_gts::gts_id;

#[cfg(feature = "db")]
use super::commit::TxCommitHandleParts;
use crate::api::{
    AssignedPartition, EventBrokerApi, FrameStream, JoinRequest, Position, SubscriptionAssignment,
};
use crate::api::{
    BarrierMode, ControlCode, Filter, PartitionPosition, SeekPosition,
    SubscriptionInterest as BrokerSubscriptionInterest, TenantTraversalDepth, WireFrame,
};
use crate::consumer::builder::{ConsumerBuilder, subscription_filter_ref_to_filter};
use crate::consumer::offset_manager::{CommitOffset, OffsetStore};
#[cfg(feature = "db")]
use crate::consumer::types::DeliverTxBatch;
use crate::consumer::types::{
    DeliverBatch, Delivered, PartitionFrontier, SkippedEvent, SubscriptionInterest,
};
use crate::consumer::{
    BatchHandlerOutcome, ConnectionDropReason, ConsumerCommitMode, ConsumerGroupRef,
    ConsumerRuntimeEvent, ConsumerRuntimeListener, ConsumerSettings, Delivery, EventBatch,
    PartitionBufferState, PartitionBufferStateSnapshot, PartitionProgress, SlowConsumerTrigger,
};
#[cfg(feature = "db")]
use crate::consumer::{CommitOffsetInTx, TxCommitHandle};
use crate::error::EventBrokerError;
use crate::gts::{EventV1, StoredPosition};
use crate::ids::{ConsumerGroupId, SubscriptionId, TopicId};
use crate::sequence::Sequence;

/// Per-partition in-memory cursor for the contiguous processed frontier.
#[derive(Default)]
pub(crate) struct PartitionCursor {
    frontier: Option<PartitionFrontier>,
    committed: Sequence, // the last offset actually committed to CommitOffset
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct TopicPartitionKey {
    topic: GtsInstanceId,
    topic_id: TopicId,
    partition: u32,
}

/// How many times a pre-stream seek re-reads the subscription and re-seeks when
/// the broker fences it with `TopologyVersionMismatch` (a concurrent rebalance
/// bumped the group's topology between reading the assignment and seeking).
/// Bounded so a rebalance storm surfaces to the caller's re-JOIN budget rather
/// than looping forever.
pub(super) const RESEEK_ON_TOPOLOGY_MISMATCH_ATTEMPTS: u32 = 5;

impl TopicPartitionKey {
    pub(crate) fn new(topic: GtsInstanceId, topic_id: TopicId, partition: u32) -> Self {
        Self {
            topic,
            topic_id,
            partition,
        }
    }
}

pub(crate) struct PartitionEventBuffer {
    capacity: usize,
    events: VecDeque<Buffered>,
}

/// A buffered event with the position it was delivered at. The runtime reads the
/// position from here, so it never unwraps the event's own optional members.
pub(crate) struct Buffered {
    pub(crate) event: EventV1,
    pub(crate) at: StoredPosition,
}

#[derive(Debug)]
pub(crate) struct EnqueuedPartitionBatch {
    pub(crate) buffered_count: usize,
}

impl PartitionEventBuffer {
    fn new(capacity: usize) -> Self {
        Self {
            capacity,
            events: VecDeque::with_capacity(capacity.min(1024)),
        }
    }

    fn push(&mut self, key: &TopicPartitionKey, event: Buffered) -> Result<(), EventBrokerError> {
        if self.events.len() >= self.capacity {
            return Err(EventBrokerError::InvalidConsumerOptions {
                detail: format!(
                    "partition buffer capacity {} exceeded for topic '{}' partition {}",
                    self.capacity, key.topic, key.partition
                ),
            });
        }
        self.events.push_back(event);
        Ok(())
    }

    fn len(&self) -> usize {
        self.events.len()
    }

    fn drain_batch(&mut self, max_events: usize) -> Vec<Buffered> {
        let count = max_events.max(1).min(self.events.len());
        self.events.drain(..count).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlowConsumerReason {
    BufferHighWatermark,
    HandlerLatencyStrikes,
}

fn slow_consumer_trigger(reason: SlowConsumerReason) -> SlowConsumerTrigger {
    match reason {
        SlowConsumerReason::BufferHighWatermark => SlowConsumerTrigger::BufferHighWatermark,
        SlowConsumerReason::HandlerLatencyStrikes => SlowConsumerTrigger::HandlerLatencyStrikes,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SlowConsumerSignal {
    pub reason: SlowConsumerReason,
    pub buffered_count: usize,
    pub consecutive_slow_handlers: u16,
    pub latest_observed_offset: Option<Sequence>,
    pub last_delivered_offset: Option<Sequence>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PartitionSlowState {
    buffered_count: usize,
    latest_observed_offset: Option<Sequence>,
    last_delivered_offset: Option<Sequence>,
    consecutive_slow_handlers: u16,
    slow_detected: bool,
}

type PartitionCursors = Arc<RwLock<HashMap<TopicPartitionKey, PartitionCursor>>>;
type PartitionBuffers = Arc<RwLock<HashMap<TopicPartitionKey, PartitionEventBuffer>>>;
type PartitionSlowStates = Arc<RwLock<HashMap<TopicPartitionKey, PartitionSlowState>>>;

#[derive(Clone, Copy)]
struct DispatchState<'a> {
    cursors: &'a PartitionCursors,
    buffers: &'a PartitionBuffers,
    slow_states: &'a PartitionSlowStates,
}

#[derive(Clone, Copy)]
struct ActiveSubscription<'a> {
    ctx: &'a SecurityContext,
    group_id: &'a ConsumerGroupId,
    sub_id: SubscriptionId,
    affected: &'a [AssignedPartition],
}

impl PartitionSlowState {
    pub(crate) fn observe_enqueue(
        &mut self,
        buffered_count: usize,
        latest_observed_offset: Sequence,
        high_watermark: usize,
    ) -> Option<SlowConsumerSignal> {
        self.buffered_count = buffered_count;
        self.latest_observed_offset = Some(latest_observed_offset);
        if buffered_count >= high_watermark {
            self.detect(SlowConsumerReason::BufferHighWatermark)
        } else {
            None
        }
    }

    pub(crate) fn observe_handler_completion(
        &mut self,
        elapsed: Duration,
        handler_latency: Duration,
        handler_strikes: u16,
        last_delivered_offset: Sequence,
    ) -> Option<SlowConsumerSignal> {
        self.last_delivered_offset = Some(last_delivered_offset);
        if elapsed >= handler_latency {
            self.consecutive_slow_handlers = self.consecutive_slow_handlers.saturating_add(1);
        } else {
            self.consecutive_slow_handlers = 0;
        }

        if self.consecutive_slow_handlers >= handler_strikes {
            self.detect(SlowConsumerReason::HandlerLatencyStrikes)
        } else {
            None
        }
    }

    fn detect(&mut self, reason: SlowConsumerReason) -> Option<SlowConsumerSignal> {
        if self.slow_detected {
            return None;
        }
        self.slow_detected = true;
        Some(SlowConsumerSignal {
            reason,
            buffered_count: self.buffered_count,
            consecutive_slow_handlers: self.consecutive_slow_handlers,
            latest_observed_offset: self.latest_observed_offset,
            last_delivered_offset: self.last_delivered_offset,
        })
    }
}

pub(crate) async fn enqueue_partition_batch(
    buffers: &PartitionBuffers,
    key: TopicPartitionKey,
    event: Buffered,
    capacity: usize,
) -> Result<EnqueuedPartitionBatch, EventBrokerError> {
    let mut guard = buffers.write().await;
    let buffer = guard
        .entry(key.clone())
        .or_insert_with(|| PartitionEventBuffer::new(capacity));
    buffer.push(&key, event)?;
    let buffered_count = buffer.len();
    Ok(EnqueuedPartitionBatch { buffered_count })
}

async fn drain_partition_batch(
    buffers: &PartitionBuffers,
    key: &TopicPartitionKey,
    max_events: usize,
) -> Vec<Buffered> {
    let mut guard = buffers.write().await;
    guard
        .get_mut(key)
        .map(|buffer| buffer.drain_batch(max_events))
        .unwrap_or_default()
}

async fn next_buffered_partition_batch(
    buffers: &PartitionBuffers,
    max_events: usize,
) -> Option<(TopicPartitionKey, Vec<Buffered>)> {
    let mut guard = buffers.write().await;
    let key = guard
        .iter()
        .find_map(|(key, buffer)| (!buffer.events.is_empty()).then(|| key.clone()))?;
    let batch = guard
        .get_mut(&key)
        .map(|buffer| buffer.drain_batch(max_events))
        .unwrap_or_default();
    Some((key, batch))
}

pub(crate) async fn emit_runtime_event_to(
    listeners: &[Arc<dyn ConsumerRuntimeListener>],
    timeout: Duration,
    event: ConsumerRuntimeEvent,
) {
    // Listener failures are diagnostic only. They must never decide ack,
    // commit, reject, or stream-drop behavior.
    for listener in listeners {
        match tokio::time::timeout(timeout, listener.on_consumer_event(&event)).await {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                warn!(error = %err, "consumer runtime listener failed");
            }
            Err(_) => {
                warn!(
                    timeout_ms = timeout.as_millis(),
                    "consumer runtime listener timed out"
                );
            }
        }
    }
}

fn partition_progress(key: &TopicPartitionKey, offset: Sequence) -> PartitionProgress {
    PartitionProgress {
        topic_id: key.topic_id,
        topic: key.topic.clone(),
        partition: key.partition,
        offset,
    }
}

impl PartitionCursor {
    pub(crate) fn latest_offset(&self) -> Sequence {
        self.frontier
            .as_ref()
            .map(PartitionFrontier::committed)
            .unwrap_or(self.committed)
    }

    pub(crate) fn advance_through_delivered_prefix(
        &mut self,
        positions: &[StoredPosition],
    ) -> Sequence {
        let Some(last) = positions.last() else {
            return self.latest_offset();
        };
        let frontier = last.sequence;
        self.frontier = Some(PartitionFrontier::new(frontier));
        frontier
    }
}

/// The GTS pattern matching every concrete event type: the event envelope base
/// with a trailing `*`. `gts_id!` validates it at compile time, so the parse here
/// cannot fail.
fn all_event_types_pattern() -> GtsIdPattern {
    GtsIdPattern::try_new(gts_id!("cf.core.events.event.v1~*"))
        .expect("event envelope wildcard is a valid GTS pattern")
}

/// What one subscription slot runs with, shared by the plain and the transactional
/// slot: the broker, the JOIN shape, the buffering, retry and slow-consumer settings
/// and the listeners.
#[derive(Clone)]
pub(crate) struct SlotSettings {
    pub slot_idx: u32,
    pub broker: Arc<dyn EventBrokerApi>,
    /// Shared across slots: every slot of one consumer resolves the same types.
    pub type_cache: Arc<super::type_cache::ConsumerTypeCache>,
    pub group_ref: ConsumerGroupRef,
    pub topics: Vec<GtsInstanceId>,
    pub subscription_interests: Vec<SubscriptionInterest>,
    pub tenant_id: Option<uuid::Uuid>,
    pub tenant_depth: TenantTraversalDepth,
    pub barrier_mode: BarrierMode,
    pub event_type_patterns: Vec<GtsIdPattern>,
    pub client_agent: String,
    pub session_timeout: Option<Duration>,
    pub filter: Option<Filter>,
    pub heartbeat_drop_threshold: usize,
    pub retry_base: Duration,
    pub retry_max: Duration,
    pub partition_buffer_capacity: usize,
    pub buffer_high_watermark: usize,
    pub buffer_low_watermark: usize,
    pub batch_max_events: usize,
    pub handler_latency: Duration,
    pub handler_strikes: u16,
    pub listeners: Vec<Arc<dyn ConsumerRuntimeListener>>,
    pub listener_timeout: Duration,
    pub max_rejoin_attempts: u32,
    pub subscription_id: Arc<tokio::sync::Mutex<Option<SubscriptionId>>>,
}

impl SlotSettings {
    /// The settings all slots of the consumer `builder` describes share, with its
    /// resolved `settings`. Its slot index and subscription-id cell are placeholders
    /// that [`Self::for_slot`] replaces.
    pub(crate) fn for_consumer<M>(
        builder: &ConsumerBuilder<M>,
        settings: &ConsumerSettings,
        broker: Arc<dyn EventBrokerApi>,
        type_cache: Arc<super::type_cache::ConsumerTypeCache>,
    ) -> Self {
        Self {
            slot_idx: 0,
            broker,
            type_cache,
            group_ref: builder
                .group
                .clone()
                .unwrap_or(ConsumerGroupRef::AutoAnonymous {
                    alias: builder.client_agent.clone(),
                }),
            topics: builder.topics.clone(),
            subscription_interests: builder.subscription_interests.clone(),
            tenant_id: builder.tenant_id,
            tenant_depth: builder.tenant_depth,
            barrier_mode: builder.barrier_mode,
            event_type_patterns: builder.event_type_patterns.clone(),
            client_agent: builder.client_agent.clone(),
            session_timeout: builder.session_timeout,
            filter: builder.filter.clone(),
            heartbeat_drop_threshold: builder.heartbeat_drop_threshold,
            retry_base: settings.retry.base_delay,
            retry_max: settings.retry.max_delay,
            partition_buffer_capacity: settings.buffering.partition_capacity,
            buffer_high_watermark: settings.buffering.high_watermark,
            buffer_low_watermark: settings.buffering.low_watermark,
            batch_max_events: settings.batching.max_events,
            handler_latency: settings.slow_detection.handler_latency,
            handler_strikes: settings.slow_detection.handler_strikes,
            listeners: builder.listeners.clone(),
            listener_timeout: settings.listener.timeout,
            max_rejoin_attempts: 16,
            subscription_id: Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    /// These settings for slot `slot_idx`, which publishes its subscription id to
    /// `subscription_id`.
    pub(crate) fn for_slot(
        &self,
        slot_idx: u32,
        subscription_id: Arc<tokio::sync::Mutex<Option<SubscriptionId>>>,
    ) -> Self {
        Self {
            slot_idx,
            subscription_id,
            ..self.clone()
        }
    }
}

/// Runs one subscription slot: JOIN, poll, dispatch, retry, re-JOIN.
pub(crate) struct SlotDispatcher<H, OM>
where
    H: DeliverBatch + 'static,
    OM: CommitOffset + 'static,
{
    pub settings: SlotSettings,
    pub offset_manager: Arc<OM>,
    pub handler: Arc<H>,
    pub commit_mode: ConsumerCommitMode,
}

#[cfg(feature = "db")]
/// Runs one transactional subscription slot. Unlike [`SlotDispatcher`], this path
/// never creates an auto-commit timer; offset durability is handler-driven through
/// `TxCommitHandle::commit_offset_in_tx`.
pub(crate) struct TxSlotDispatcher<H, OM>
where
    H: DeliverTxBatch<OM> + 'static,
    OM: CommitOffsetInTx + 'static,
{
    pub settings: SlotSettings,
    pub offset_manager: Arc<OM>,
    pub handler: Arc<H>,
}

/// The log lines and the re-JOIN exhaustion detail of one slot flavour.
struct SlotTexts {
    starting: &'static str,
    open_failed: &'static str,
    heartbeat_drop: &'static str,
    terminal: &'static str,
    transport_error: &'static str,
    frame_error: &'static str,
    buffer_rejected: &'static str,
    draining: &'static str,
    slow_detected: &'static str,
    handler_retry: &'static str,
    handler_err: &'static str,
    rejoin_exhausted: &'static str,
}

/// The plain slot's texts.
const PLAIN_TEXTS: SlotTexts = SlotTexts {
    starting: "slot dispatcher starting",
    open_failed: "stream open failed; re-joining",
    heartbeat_drop: "drop-on-Nth-heartbeat triggered; voluntary disconnect + re-JOIN",
    terminal: "terminal control frame; re-JOIN",
    transport_error: "stream transport error; re-joining",
    frame_error: "stream frame error; ending stream",
    buffer_rejected: "partition buffer rejected event",
    draining: "draining buffered events before re-JOIN",
    slow_detected: "slow consumer predicate detected",
    handler_retry: "handler returned Retry; backing off",
    handler_err: "handler returned Err; treating as Retry",
    rejoin_exhausted: "max re-JOIN attempts exceeded",
};

/// The transactional slot's texts.
#[cfg(feature = "db")]
const TX_TEXTS: SlotTexts = SlotTexts {
    starting: "transactional slot dispatcher starting",
    open_failed: "transactional stream open failed; re-joining",
    heartbeat_drop: "transactional drop-on-Nth-heartbeat triggered",
    terminal: "transactional terminal control frame; re-JOIN",
    transport_error: "transactional stream transport error; re-joining",
    frame_error: "transactional stream frame error; ending stream",
    buffer_rejected: "transactional partition buffer rejected event",
    draining: "draining transactional buffered events before re-JOIN",
    slow_detected: "transactional slow consumer predicate detected",
    handler_retry: "transactional handler returned Retry; backing off",
    handler_err: "transactional handler returned Err; treating as Retry",
    rejoin_exhausted: "max transactional re-JOIN attempts exceeded",
};

/// One handler call on a batch, as the shared retry loop reads it.
enum BatchAttempt {
    /// The handler settled a prefix of the batch.
    Settled(SettledBatch),
    /// The handler settled the batch in a way the runtime cannot apply; reported as
    /// `HandlerFailed` and the stream is dropped.
    Unusable(String),
    /// The batch is delivered again after a backoff.
    Again(RetryCause),
}

enum RetryCause {
    /// The handler asked for the batch again.
    Retry { reason: String },
    /// The handler failed; treated as a retry.
    Failed(String),
}

struct SettledBatch {
    outcome: BatchHandlerOutcome,
    processed_count: usize,
    skipped: Option<SkippedEvent>,
    /// Whether the handler already made the processed prefix durable, so the cursor
    /// counts it as committed and `OffsetCommitted` is reported with it.
    committed: bool,
}

/// How [`SlotFlavour::consume_stream`] left an open stream.
enum StreamEnd {
    /// The slot was cancelled; the run ends without a re-JOIN.
    Cancelled,
    /// The stream ended or a frame ended it; the slot re-joins.
    Rejoin,
    /// Dispatching an event dropped the stream (a slow partition or an unusable
    /// batch); the slot drains its buffers, leaves and re-joins.
    DrainThenRejoin,
}

/// What differs between the plain and the transactional slot: the handler call,
/// where frame positions go, the auto-commit timer and the log texts. The JOIN,
/// stream, dispatch, retry and re-JOIN loop is shared, as the provided methods.
trait SlotFlavour: Sync {
    const TEXTS: SlotTexts;
    /// Whether a cancel stops the batch loop before each handler call and each
    /// drained batch. The transactional slot checks cancel only around its backoff.
    const CANCEL_STOPS_DELIVERY: bool;
    /// Whether a stream open refused with `SubscriptionRecoveryExhausted` ends the
    /// run instead of re-joining.
    const OPEN_EXHAUSTION_ENDS_RUN: bool;

    fn settings(&self) -> &SlotSettings;

    fn offsets(&self) -> &dyn OffsetStore;

    /// Starts the background committer of the in-memory cursors, if this slot has one;
    /// by default it has none.
    fn spawn_commit_task(
        &self,
        _ctx: &Arc<SecurityContext>,
        _group_id: ConsumerGroupId,
        _assignment: &SubscriptionAssignment,
        _cursors: &PartitionCursors,
        _cancel: &CancellationToken,
    ) -> Option<tokio::task::JoinHandle<()>> {
        None
    }

    /// Takes the positions a control or topology frame carries.
    async fn take_frame_positions(
        &self,
        group_id: &ConsumerGroupId,
        subscription_id: SubscriptionId,
        positions: &[PartitionPosition],
    );

    /// Calls the handler once on a batch of `key`.
    async fn deliver(
        &self,
        group_id: ConsumerGroupId,
        key: &TopicPartitionKey,
        events: &[EventV1],
        positions: &[StoredPosition],
        delivery: &Delivery,
    ) -> BatchAttempt;

    async fn run_slot(
        &self,
        ctx: Arc<SecurityContext>,
        cancel: CancellationToken,
    ) -> Result<(), EventBrokerError> {
        let slot = self.settings();
        trace!(slot_idx = slot.slot_idx, "{}", Self::TEXTS.starting);
        let group_id = slot.ensure_group(&ctx).await?;
        let mut assignment = slot.join_subscription(&ctx, &group_id).await?;
        slot.publish_subscription_id(assignment.subscription_id)
            .await;

        // Pre-stream SEEK: resolve a starting position per assigned partition
        // via OffsetStore::load_position(...) and seed them on the broker before
        // opening the stream. Without this, the broker returns `409
        // PositionsNotSet` on stream-open (defensive backstop).
        self.resolve_and_seek(&ctx, &group_id, &assignment).await?;

        let cursors: PartitionCursors = Arc::new(RwLock::new(HashMap::new()));
        let buffers: PartitionBuffers = Arc::new(RwLock::new(HashMap::new()));
        let slow_states: PartitionSlowStates = Arc::new(RwLock::new(HashMap::new()));
        let state = DispatchState {
            cursors: &cursors,
            buffers: &buffers,
            slow_states: &slow_states,
        };

        let _commit_task = self.spawn_commit_task(&ctx, group_id, &assignment, &cursors, &cancel);

        let mut consecutive_failures = 0u32;
        loop {
            if cancel.is_cancelled() {
                break;
            }

            let sub_id = assignment.subscription_id;
            let opened = slot.open_stream(&ctx, sub_id).await;
            if cancel.is_cancelled() {
                break;
            }
            let (first, rest) = match opened {
                Ok(opened) => opened,
                Err(EventBrokerError::SubscriptionRecoveryExhausted { .. })
                    if Self::OPEN_EXHAUSTION_ENDS_RUN =>
                {
                    return Err(EventBrokerError::SubscriptionRecoveryExhausted {
                        attempts: consecutive_failures,
                        detail: "stream open: recovery exhausted".into(),
                    });
                }
                Err(e) => {
                    warn!(
                        slot_idx = slot.slot_idx,
                        error = %e,
                        "{}", Self::TEXTS.open_failed
                    );
                    // A member whose stream never opened, or never got past its first
                    // frame, may still hold a share of the partitions; a JOIN evicts only
                    // `Disconnected` members. Leave it first; a failed leave leaves it to
                    // the broker's timeouts.
                    let _ = slot.broker.leave(&ctx, sub_id).await;
                    assignment = self
                        .rejoin_and_seek(&ctx, &group_id, sub_id, &mut consecutive_failures)
                        .await?;
                    continue;
                }
            };
            let mut stream = futures_util::stream::iter([Ok(first)]).chain(rest);
            let active = ActiveSubscription {
                ctx: &ctx,
                group_id: &group_id,
                sub_id,
                affected: &assignment.assigned,
            };

            // Only an open stream starts the re-JOIN budget over.
            consecutive_failures = 0;
            let drain_before_rejoin = match self
                .consume_stream(&cancel, &mut stream, state, active)
                .await
            {
                StreamEnd::Cancelled => break,
                StreamEnd::Rejoin => false,
                StreamEnd::DrainThenRejoin => true,
            };

            // The stream ended, a frame ended it, or a dispatched event dropped it. Re-JOIN
            // unless cancelled.
            if cancel.is_cancelled() {
                break;
            }
            if drain_before_rejoin {
                self.drain_subscription_buffers(&cancel, state, active)
                    .await;
                let _ = slot.broker.leave(&ctx, sub_id).await;
            }
            // End the old stream before the re-JOIN. The broker evicts a member whose
            // stream has closed (`Disconnected`) when the next JOIN arrives, but splits
            // the partitions with one whose stream is still open, and that one then
            // keeps its share until its stream closes, then `session_timeout`. Closing
            // the stream is enough without a LEAVE, which a broken transport may not
            // carry. That is proven in process; over REST the server notices the close
            // when hyper does, a delay that is not measured (#5819).
            drop(stream);
            slot.emit_runtime_event(ConsumerRuntimeEvent::SubscriptionTerminated {
                subscription_id: sub_id,
                reason: ConnectionDropReason::StreamTerminated,
            })
            .await;
            assignment = self
                .rejoin_and_seek(&ctx, &group_id, sub_id, &mut consecutive_failures)
                .await?;
        }

        // Graceful shutdown: leave subscription.
        let _ = slot.broker.leave(&ctx, assignment.subscription_id).await;
        Ok(())
    }

    /// Reads the open stream's frames until the stream ends, the slot is cancelled,
    /// or a frame or a dispatched event calls for a re-JOIN.
    async fn consume_stream(
        &self,
        cancel: &CancellationToken,
        stream: &mut (impl Stream<Item = Result<WireFrame, EventBrokerError>> + Unpin + Send),
        state: DispatchState<'_>,
        active: ActiveSubscription<'_>,
    ) -> StreamEnd {
        let slot = self.settings();
        let mut consecutive_heartbeats: usize = 0;
        while let Some(frame_result) = stream.next().await {
            if cancel.is_cancelled() {
                return StreamEnd::Cancelled;
            }
            match frame_result {
                Ok(WireFrame::Event(event)) => {
                    consecutive_heartbeats = 0;
                    if self.dispatch_event(cancel, event, state, active).await {
                        return StreamEnd::DrainThenRejoin;
                    }
                }
                Ok(WireFrame::Heartbeat { .. }) => {
                    consecutive_heartbeats += 1;
                    if consecutive_heartbeats >= slot.heartbeat_drop_threshold {
                        trace!(
                            slot_idx = slot.slot_idx,
                            threshold = slot.heartbeat_drop_threshold,
                            "{}",
                            Self::TEXTS.heartbeat_drop
                        );
                        return StreamEnd::Rejoin;
                    }
                }
                Ok(WireFrame::Control {
                    code,
                    positions,
                    reason,
                }) => {
                    self.take_frame_positions(active.group_id, active.sub_id, &positions)
                        .await;
                    if code == ControlCode::Terminal {
                        // Gain / lose-all: the subscription is terminating; re-JOIN.
                        trace!(
                            slot_idx = slot.slot_idx,
                            ?reason,
                            "{}",
                            Self::TEXTS.terminal
                        );
                        return StreamEnd::Rejoin;
                    }
                    // Progress: sparse mid-stream update; keep streaming.
                }
                Ok(WireFrame::Topology {
                    topology_version: _,
                    assigned,
                }) => {
                    // Non-terminal: a partition loss or a `topology_version`-only
                    // change; keep streaming. Gains never arrive as a topology
                    // frame - they terminate via a Terminal control frame and a
                    // re-JOIN.
                    self.take_frame_positions(active.group_id, active.sub_id, &assigned)
                        .await;
                }
                Err(e @ EventBrokerError::Transport(_)) => {
                    warn!(
                        slot_idx = slot.slot_idx,
                        error = %e,
                        "{}", Self::TEXTS.transport_error
                    );
                    return StreamEnd::Rejoin;
                }
                Err(e) => {
                    warn!(
                        slot_idx = slot.slot_idx,
                        error = %e,
                        "{}", Self::TEXTS.frame_error
                    );
                    return StreamEnd::Rejoin;
                }
            }
        }
        StreamEnd::Rejoin
    }

    async fn dispatch_event(
        &self,
        cancel: &CancellationToken,
        event: EventV1,
        state: DispatchState<'_>,
        active: ActiveSubscription<'_>,
    ) -> bool {
        let slot = self.settings();
        // A transport stamps every event it can place, so one without a position is
        // skipped like an event no topic can be attributed to.
        let (Some(partition), Some(sequence), Some(sequence_time)) =
            (event.partition(), event.sequence(), event.sequence_time())
        else {
            warn!(
                event_id = %event.id(),
                type_id = %event.event_type(),
                "a delivered event carries no stamped position; skipping it"
            );
            return false;
        };
        // A topic is what an event's type declares, so it is resolved rather than
        // read off the frame. A type this consumer has not seen costs one call
        // here and none afterwards.
        let topic = match slot
            .type_cache
            .topic_of(&slot.broker, active.ctx, event.event_type())
            .await
        {
            Ok(topic) => topic,
            Err(err) => {
                warn!(
                    error = %err,
                    type_id = %event.event_type(),
                    "cannot attribute a delivered event to a topic; skipping it"
                );
                return false;
            }
        };
        let topic_id = TopicId::from_gts(&topic);
        let key = TopicPartitionKey::new(topic, topic_id, partition);
        let buffered = Buffered {
            event,
            at: StoredPosition {
                partition,
                sequence,
                sequence_time,
            },
        };
        let enqueued = match enqueue_partition_batch(
            state.buffers,
            key.clone(),
            buffered,
            slot.partition_buffer_capacity,
        )
        .await
        {
            Ok(events) => events,
            Err(e) => {
                warn!(error = %e, "{}", Self::TEXTS.buffer_rejected);
                return false;
            }
        };
        if self
            .observe_enqueue_slow_signal(
                state,
                key.clone(),
                enqueued.buffered_count,
                sequence,
                active,
            )
            .await
        {
            return true;
        }
        let batch_events = drain_partition_batch(state.buffers, &key, slot.batch_max_events).await;
        self.process_batch(cancel, state, key, batch_events, active)
            .await
    }

    async fn drain_subscription_buffers(
        &self,
        cancel: &CancellationToken,
        state: DispatchState<'_>,
        active: ActiveSubscription<'_>,
    ) {
        let slot = self.settings();
        trace!(
            slot_idx = slot.slot_idx,
            low_watermark = slot.buffer_low_watermark,
            "{}",
            Self::TEXTS.draining
        );
        loop {
            if Self::CANCEL_STOPS_DELIVERY && cancel.is_cancelled() {
                break;
            }
            let Some((key, batch_events)) =
                next_buffered_partition_batch(state.buffers, slot.batch_max_events).await
            else {
                break;
            };
            let _ = self
                .process_batch(cancel, state, key, batch_events, active)
                .await;
        }
    }

    async fn process_batch(
        &self,
        cancel: &CancellationToken,
        state: DispatchState<'_>,
        key: TopicPartitionKey,
        batch_events: Vec<Buffered>,
        active: ActiveSubscription<'_>,
    ) -> bool {
        let slot = self.settings();
        if batch_events.is_empty() {
            return false;
        }
        let (events, positions): (Vec<EventV1>, Vec<StoredPosition>) = batch_events
            .into_iter()
            .map(|buffered| (buffered.event, buffered.at))
            .unzip();
        let batch_partition = positions[0].partition;
        let batch_offset = positions
            .last()
            .map(|at| at.sequence)
            .unwrap_or_else(|| positions[0].sequence);
        let topic_id = key.topic_id;

        let mut attempts: u16 = 1;
        let mut backoff = slot.retry_base;

        loop {
            if Self::CANCEL_STOPS_DELIVERY && cancel.is_cancelled() {
                return false;
            }
            slot.emit_runtime_event(ConsumerRuntimeEvent::HandlerBatchStarted {
                topic_id,
                topic: key.topic.clone(),
                partition: batch_partition,
                len: events.len(),
            })
            .await;
            let delivery = Delivery::new(key.topic.clone(), key.partition, attempts);
            let started_at = Instant::now();
            let cause = match self
                .deliver(*active.group_id, &key, &events, &positions, &delivery)
                .await
            {
                BatchAttempt::Settled(settled) => {
                    slot.emit_runtime_event(ConsumerRuntimeEvent::HandlerBatchCompleted {
                        topic_id,
                        topic: key.topic.clone(),
                        partition: batch_partition,
                        outcome: settled.outcome,
                    })
                    .await;
                    if let Some(skipped) = settled.skipped {
                        slot.report_skipped_event(&key, skipped).await;
                    }
                    let drop_stream = self
                        .observe_handler_slow_signal(
                            state,
                            key.clone(),
                            started_at.elapsed(),
                            batch_offset,
                            active,
                        )
                        .await;
                    let frontier = {
                        let mut guard = state.cursors.write().await;
                        let c = guard.entry(key.clone()).or_default();
                        let frontier = c.advance_through_delivered_prefix(
                            &positions[..settled.processed_count],
                        );
                        if settled.committed {
                            c.committed = frontier;
                        }
                        frontier
                    };
                    if settled.committed {
                        slot.emit_runtime_event(ConsumerRuntimeEvent::OffsetCommitted {
                            topic_id,
                            topic: key.topic.clone(),
                            partition: batch_partition,
                            offset: frontier,
                        })
                        .await;
                    }
                    slot.emit_runtime_event(ConsumerRuntimeEvent::ProgressAdvanced {
                        subscription_id: active.sub_id,
                        progress: vec![partition_progress(&key, frontier)],
                    })
                    .await;
                    return drop_stream;
                }
                BatchAttempt::Unusable(error) => {
                    slot.emit_runtime_event(ConsumerRuntimeEvent::HandlerFailed {
                        topic_id,
                        topic: key.topic.clone(),
                        partition: batch_partition,
                        error,
                    })
                    .await;
                    return true;
                }
                BatchAttempt::Again(cause) => {
                    let event = match &cause {
                        RetryCause::Retry { reason } => {
                            ConsumerRuntimeEvent::HandlerBatchCompleted {
                                topic_id,
                                topic: key.topic.clone(),
                                partition: batch_partition,
                                outcome: BatchHandlerOutcome::Retry {
                                    reason: reason.clone(),
                                },
                            }
                        }
                        RetryCause::Failed(error) => ConsumerRuntimeEvent::HandlerFailed {
                            topic_id,
                            topic: key.topic.clone(),
                            partition: batch_partition,
                            error: error.clone(),
                        },
                    };
                    slot.emit_runtime_event(event).await;
                    cause
                }
            };
            if self
                .observe_handler_slow_signal(
                    state,
                    key.clone(),
                    started_at.elapsed(),
                    batch_offset,
                    active,
                )
                .await
            {
                return true;
            }
            if cancel.is_cancelled() {
                return false;
            }
            match cause {
                RetryCause::Retry { reason } => {
                    trace!(reason, attempts, "{}", Self::TEXTS.handler_retry);
                }
                RetryCause::Failed(error) => {
                    warn!(error = %error, "{}", Self::TEXTS.handler_err);
                }
            }
            slot.emit_runtime_event(ConsumerRuntimeEvent::RetryScheduled {
                topic_id,
                topic: key.topic.clone(),
                partition: batch_partition,
                attempt: attempts,
                delay: backoff,
            })
            .await;
            // Giving the batch up here advances no cursor: unless the handler
            // committed it, the next start delivers it again (at-least-once).
            tokio::select! {
                _ = cancel.cancelled() => return false,
                _ = tokio::time::sleep(backoff) => {}
            }
            backoff = (backoff * 2).min(slot.retry_max);
            attempts = attempts.saturating_add(1);
        }
    }

    async fn observe_enqueue_slow_signal(
        &self,
        state: DispatchState<'_>,
        key: TopicPartitionKey,
        buffered_count: usize,
        latest_observed_offset: Sequence,
        active: ActiveSubscription<'_>,
    ) -> bool {
        let slot = self.settings();
        let mut guard = state.slow_states.write().await;
        let signal = guard.entry(key.clone()).or_default().observe_enqueue(
            buffered_count,
            latest_observed_offset,
            slot.buffer_high_watermark,
        );
        drop(guard);
        if let Some(signal) = signal {
            warn!(
                slot_idx = slot.slot_idx,
                topic = %key.topic,
                partition = key.partition,
                reason = ?signal.reason,
                buffered_count = signal.buffered_count,
                latest_observed_offset = ?signal.latest_observed_offset,
                "{}", Self::TEXTS.slow_detected
            );
            slot.emit_slow_consumer_events(active, key, signal).await;
            return true;
        }
        false
    }

    async fn observe_handler_slow_signal(
        &self,
        state: DispatchState<'_>,
        key: TopicPartitionKey,
        elapsed: Duration,
        last_delivered_offset: Sequence,
        active: ActiveSubscription<'_>,
    ) -> bool {
        let slot = self.settings();
        let mut guard = state.slow_states.write().await;
        let signal = guard
            .entry(key.clone())
            .or_default()
            .observe_handler_completion(
                elapsed,
                slot.handler_latency,
                slot.handler_strikes,
                last_delivered_offset,
            );
        drop(guard);
        if let Some(signal) = signal {
            warn!(
                slot_idx = slot.slot_idx,
                topic = %key.topic,
                partition = key.partition,
                reason = ?signal.reason,
                consecutive_slow_handlers = signal.consecutive_slow_handlers,
                last_delivered_offset = ?signal.last_delivered_offset,
                "{}", Self::TEXTS.slow_detected
            );
            slot.emit_slow_consumer_events(active, key, signal).await;
            return true;
        }
        false
    }

    /// Resolve a starting position for each assigned partition via
    /// `OffsetStore::load_position` and SEEK the broker. Used after JOIN (initial
    /// seed) and after every re-JOIN.
    async fn resolve_and_seek(
        &self,
        ctx: &SecurityContext,
        group_id: &ConsumerGroupId,
        assignment: &SubscriptionAssignment,
    ) -> Result<(), EventBrokerError> {
        let slot = self.settings();
        // A concurrent rebalance can bump the group's topology_version between
        // the moment we read our assignment and the moment we seek, so the
        // broker fences a stale seek with TopologyVersionMismatch. Re-read the
        // subscription for the fresh version + assignment and re-seek. A gained
        // partition is reached via the terminal -> re-JOIN path, not here; this
        // only re-seeks the (possibly reduced) current assignment.
        let subscription_id = assignment.subscription_id;
        let mut version = assignment.topology_version;
        let mut slots: Vec<AssignedPartition> = assignment.assigned.clone();
        let mut attempts = 0u32;
        loop {
            if slots.is_empty() {
                return Ok(());
            }
            let mut positions: Vec<SeekPosition> = Vec::with_capacity(slots.len());
            for assigned in &slots {
                let topic = assigned.topic.clone();
                let topic_id = TopicId::from_gts(&topic);
                let value = self
                    .offsets()
                    .load_position(group_id, &topic_id, assigned.partition)
                    .await?;
                slot.emit_runtime_event(ConsumerRuntimeEvent::OffsetLoaded {
                    topic_id,
                    topic: topic.clone(),
                    partition: assigned.partition,
                    position: value.clone(),
                })
                .await;
                positions.push(SeekPosition {
                    topic,
                    partition: assigned.partition,
                    value,
                });
            }
            match slot
                .broker
                .seek(ctx, subscription_id, version, &positions)
                .await
            {
                Ok(_) => return Ok(()),
                Err(EventBrokerError::TopologyVersionMismatch { .. })
                    if attempts < RESEEK_ON_TOPOLOGY_MISMATCH_ATTEMPTS =>
                {
                    attempts += 1;
                    let sub = slot.broker.get_subscription(ctx, subscription_id).await?;
                    version = sub.topology_version;
                    slots = sub
                        .assigned
                        .into_iter()
                        .map(|p| AssignedPartition {
                            topic: p.topic.clone(),
                            partition: p.partition,
                        })
                        .collect();
                }
                // Budget exhausted (or any other error): surface it so the
                // caller's outer re-JOIN loop can take over.
                Err(e) => return Err(e),
            }
        }
    }

    /// Re-JOINs within the re-JOIN budget and seeds the new subscription's
    /// positions: a fresh subscription id has no SEEK history on the broker.
    async fn rejoin_and_seek(
        &self,
        ctx: &SecurityContext,
        group_id: &ConsumerGroupId,
        previous_subscription_id: SubscriptionId,
        consecutive_failures: &mut u32,
    ) -> Result<SubscriptionAssignment, EventBrokerError> {
        let slot = self.settings();
        *consecutive_failures += 1;
        if *consecutive_failures > slot.max_rejoin_attempts {
            return Err(EventBrokerError::SubscriptionRecoveryExhausted {
                attempts: *consecutive_failures,
                detail: Self::TEXTS.rejoin_exhausted.into(),
            });
        }
        slot.emit_runtime_event(ConsumerRuntimeEvent::SubscriptionRejoining {
            group_id: *group_id,
            previous_subscription_id,
        })
        .await;
        tokio::time::sleep(Duration::from_millis(250) * (*consecutive_failures)).await;
        let assignment = slot.join_subscription(ctx, group_id).await?;
        slot.publish_subscription_id(assignment.subscription_id)
            .await;
        self.resolve_and_seek(ctx, group_id, &assignment).await?;
        Ok(assignment)
    }
}

impl SlotSettings {
    /// Opens the subscription's stream and reads its first frame. The stream is open
    /// only at its first frame: a transport that opens lazily (REST) reports a
    /// refused open as the first item, and a stream that ends before any frame never
    /// opened, so both are an error here, with the stream already dropped.
    async fn open_stream(
        &self,
        ctx: &SecurityContext,
        subscription_id: SubscriptionId,
    ) -> Result<(WireFrame, FrameStream), EventBrokerError> {
        let mut stream = self.broker.stream(ctx, subscription_id).await?;
        match stream.next().await {
            Some(Ok(first)) => Ok((first, stream)),
            Some(Err(e)) => Err(e),
            None => Err(EventBrokerError::Transport(
                "stream ended before its first frame".to_owned(),
            )),
        }
    }

    async fn publish_subscription_id(&self, subscription_id: SubscriptionId) {
        let mut guard = self.subscription_id.lock().await;
        *guard = Some(subscription_id);
    }

    async fn emit_slow_consumer_events(
        &self,
        active: ActiveSubscription<'_>,
        key: TopicPartitionKey,
        signal: SlowConsumerSignal,
    ) {
        let trigger = slow_consumer_trigger(signal.reason);
        let state = PartitionBufferStateSnapshot {
            group_id: *active.group_id,
            subscription_id: active.sub_id,
            topic_id: key.topic_id,
            topic: key.topic.clone(),
            partition: key.partition,
            state: PartitionBufferState::SlowDetected,
            trigger: Some(trigger),
            buffered_count: signal.buffered_count,
            capacity: self.partition_buffer_capacity,
            latest_observed_offset: signal.latest_observed_offset,
            last_delivered_offset: signal.last_delivered_offset,
            consecutive_slow_handlers: signal.consecutive_slow_handlers,
        };
        self.emit_runtime_event(ConsumerRuntimeEvent::PartitionBufferStateChanged { state })
            .await;
        self.emit_runtime_event(ConsumerRuntimeEvent::SubscriptionConnectionDropped {
            subscription_id: active.sub_id,
            reason: ConnectionDropReason::SlowConsumer {
                topic_id: key.topic_id,
                topic: key.topic,
                partition: key.partition,
                trigger,
            },
            affected: active.affected.to_vec(),
        })
        .await;
    }

    async fn emit_runtime_event(&self, event: ConsumerRuntimeEvent) {
        emit_runtime_event_to(&self.listeners, self.listener_timeout, event).await;
    }

    /// Reports an event a typed route's `Skip` policy advanced past: the runtime event,
    /// then a warn naming its id, type and decode category, never its data.
    async fn report_skipped_event(&self, key: &TopicPartitionKey, skipped: SkippedEvent) {
        let SkippedEvent {
            event_id,
            sequence,
            type_id,
            category,
        } = skipped;
        self.emit_runtime_event(ConsumerRuntimeEvent::EventSkipped {
            topic: key.topic.clone(),
            partition: key.partition,
            sequence,
            type_id: type_id.clone(),
            category,
        })
        .await;
        warn!(
            event_id = %event_id,
            type_id = %type_id,
            category = %category,
            "a delivered event does not decode as its route's payload type; skipping it"
        );
    }

    async fn ensure_group(
        &self,
        ctx: &SecurityContext,
    ) -> Result<ConsumerGroupId, EventBrokerError> {
        match &self.group_ref {
            ConsumerGroupRef::Id(id) => Ok(*id),
            ConsumerGroupRef::Gts(gts) => Err(EventBrokerError::InvalidConsumerOptions {
                detail: format!(
                    "consumer group GTS reference '{gts}' must be resolved before startup"
                ),
            }),
            ConsumerGroupRef::AutoAnonymous { alias } => {
                let group = self
                    .broker
                    .create_consumer_group(
                        ctx,
                        crate::models::CreateConsumerGroupRequest {
                            client_agent: alias.clone(),
                            description: None,
                        },
                    )
                    .await?;
                Ok(group.id)
            }
        }
    }

    /// The interests a JOIN of this slot declares: one per configured subscription
    /// interest, else one per topic with the configured event-type patterns (every
    /// event type when none) and filter. A missing tenant is the nil tenant.
    fn join_interests(&self) -> Result<Vec<BrokerSubscriptionInterest>, EventBrokerError> {
        let tenant_id = self.tenant_id.unwrap_or_else(uuid::Uuid::nil);
        if !self.subscription_interests.is_empty() {
            return self
                .subscription_interests
                .iter()
                .map(|interest| {
                    let mut builder = BrokerSubscriptionInterest::builder()
                        .topic(interest.topic.clone())
                        .tenant_id(tenant_id)
                        .tenant_depth(self.tenant_depth)
                        .barrier_mode(self.barrier_mode)
                        .types(interest.event_types.clone());
                    if let Some(filter) = interest.filter.as_ref() {
                        builder = builder.filter(subscription_filter_ref_to_filter(filter));
                    }
                    builder.build()
                })
                .collect();
        }

        // `.topics(..)` with no explicit patterns means "every event type on the
        // topic". Expressed as the event envelope's trailing-`*` GTS pattern, which
        // the broker's `GtsId::matches_pattern` covers for every derived event type;
        // topic-equality in the interest scopes it to the topic.
        let event_type_patterns = if self.event_type_patterns.is_empty() {
            vec![all_event_types_pattern()]
        } else {
            self.event_type_patterns.clone()
        };
        self.topics
            .iter()
            .map(|topic| {
                let mut builder = BrokerSubscriptionInterest::builder()
                    .topic(topic.clone())
                    .tenant_id(tenant_id)
                    .tenant_depth(self.tenant_depth)
                    .barrier_mode(self.barrier_mode)
                    .types(event_type_patterns.clone());
                if let Some(filter) = self.filter.clone() {
                    builder = builder.filter(filter);
                }
                builder.build()
            })
            .collect()
    }

    async fn join_subscription(
        &self,
        ctx: &SecurityContext,
        group_id: &ConsumerGroupId,
    ) -> Result<SubscriptionAssignment, EventBrokerError> {
        self.emit_runtime_event(ConsumerRuntimeEvent::SubscriptionJoining {
            group_id: *group_id,
        })
        .await;
        let interests = self.join_interests()?;

        let assignment = self
            .broker
            .join(
                ctx,
                JoinRequest {
                    group: *group_id,
                    client_agent: self.client_agent.clone(),
                    interests,
                    session_timeout: self.session_timeout,
                },
            )
            .await?;
        self.emit_runtime_event(ConsumerRuntimeEvent::SubscriptionStarted {
            group_id: *group_id,
            subscription_id: assignment.subscription_id,
            assigned: assignment.assigned.clone(),
        })
        .await;
        self.emit_runtime_event(ConsumerRuntimeEvent::AssignmentChanged {
            subscription_id: assignment.subscription_id,
            assigned: assignment.assigned.clone(),
        })
        .await;
        Ok(assignment)
    }
}

impl<H, OM> SlotDispatcher<H, OM>
where
    H: DeliverBatch + 'static,
    OM: CommitOffset + 'static,
{
    pub async fn run(
        &self,
        ctx: Arc<SecurityContext>,
        cancel: CancellationToken,
    ) -> Result<(), EventBrokerError> {
        self.run_slot(ctx, cancel).await
    }
}

impl<H, OM> SlotFlavour for SlotDispatcher<H, OM>
where
    H: DeliverBatch + 'static,
    OM: CommitOffset + 'static,
{
    const TEXTS: SlotTexts = PLAIN_TEXTS;
    const CANCEL_STOPS_DELIVERY: bool = true;
    const OPEN_EXHAUSTION_ENDS_RUN: bool = true;

    fn settings(&self) -> &SlotSettings {
        &self.settings
    }

    fn offsets(&self) -> &dyn OffsetStore {
        self.offset_manager.as_ref()
    }

    fn spawn_commit_task(
        &self,
        ctx: &Arc<SecurityContext>,
        group_id: ConsumerGroupId,
        assignment: &SubscriptionAssignment,
        cursors: &PartitionCursors,
        cancel: &CancellationToken,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let ConsumerCommitMode::Auto { interval } = self.commit_mode else {
            return None;
        };
        let seek_cursors = cursors.clone();
        let seek_broker = self.settings.broker.clone();
        let seek_om = self.offset_manager.clone();
        let seek_ctx = ctx.clone();
        let seek_group = group_id;
        let seek_sub = assignment.subscription_id;
        // Captured with the subscription id it belongs to; both go stale
        // together after a re-JOIN, and a stale seek here is ignored.
        let seek_topology_version = assignment.topology_version;
        let seek_cancel = cancel.clone();
        let seek_listeners = self.settings.listeners.clone();
        let seek_listener_timeout = self.settings.listener_timeout;
        Some(tokio::spawn(async move {
            let mut interval = tokio::time::interval(interval);
            loop {
                tokio::select! {
                    _ = seek_cancel.cancelled() => break,
                    _ = interval.tick() => {
                        let pending: Vec<_> = {
                            let guard = seek_cursors.read().await;
                            guard
                                .iter()
                                .filter_map(|(key, cursor)| {
                                    let latest_offset = cursor.latest_offset();
                                    (latest_offset > cursor.committed)
                                        .then(|| (key.clone(), latest_offset))
                                })
                                .collect()
                        };

                        for (key, latest_offset) in pending {
                            if seek_om.commit(
                                &seek_group,
                                &key.topic_id,
                                key.partition,
                                latest_offset,
                            ).await.is_ok() {
                                {
                                    let mut guard = seek_cursors.write().await;
                                    if let Some(cursor) = guard.get_mut(&key)
                                        && cursor.latest_offset() >= latest_offset
                                    {
                                        cursor.committed = latest_offset;
                                    }
                                }
                                emit_runtime_event_to(
                                    &seek_listeners,
                                    seek_listener_timeout,
                                    ConsumerRuntimeEvent::OffsetCommitted {
                                        topic_id: key.topic_id,
                                        topic: key.topic.clone(),
                                        partition: key.partition,
                                        offset: latest_offset,
                                    },
                                ).await;
                                emit_runtime_event_to(
                                    &seek_listeners,
                                    seek_listener_timeout,
                                    ConsumerRuntimeEvent::ProgressAdvanced {
                                        subscription_id: seek_sub,
                                        progress: vec![partition_progress(
                                            &key,
                                            latest_offset,
                                        )],
                                    },
                                ).await;
                            }
                            let _ = seek_broker.seek(
                                &seek_ctx,
                                seek_sub,
                                seek_topology_version,
                                &[SeekPosition {
                                    topic: key.topic.clone(),
                                    partition: key.partition,
                                    value: Position::Exact(latest_offset),
                                }],
                            ).await;
                        }
                    }
                }
            }
        }))
    }

    /// Feed control / topology-frame positions into the consumer's own offset
    /// store. Persists `last_examined` (the scan frontier) so a later reconnect
    /// re-SEEKs past server-side-filtered events (R57). Best-effort: a failure to
    /// persist is logged, not fatal.
    async fn take_frame_positions(
        &self,
        group_id: &ConsumerGroupId,
        subscription_id: SubscriptionId,
        positions: &[PartitionPosition],
    ) {
        for p in positions {
            let topic = p.topic.clone();
            let topic_id = TopicId::from_gts(&topic);
            if let Err(e) = self
                .offset_manager
                .commit(group_id, &topic_id, p.partition, p.last_examined)
                .await
            {
                warn!(error = %e, "failed to commit control-frame position to offset store");
            } else {
                self.settings
                    .emit_runtime_event(ConsumerRuntimeEvent::OffsetCommitted {
                        topic_id,
                        topic: topic.clone(),
                        partition: p.partition,
                        offset: p.last_examined,
                    })
                    .await;
                self.settings
                    .emit_runtime_event(ConsumerRuntimeEvent::ProgressAdvanced {
                        subscription_id,
                        progress: vec![PartitionProgress {
                            topic_id,
                            topic,
                            partition: p.partition,
                            offset: p.last_examined,
                        }],
                    })
                    .await;
            }
        }
    }

    async fn deliver(
        &self,
        _group_id: ConsumerGroupId,
        _key: &TopicPartitionKey,
        events: &[EventV1],
        positions: &[StoredPosition],
        delivery: &Delivery,
    ) -> BatchAttempt {
        let batch = EventBatch::new(events);
        match self.handler.deliver(&batch, positions, delivery).await {
            Ok(Delivered {
                outcome:
                    outcome
                    @ (BatchHandlerOutcome::Success | BatchHandlerOutcome::AdvanceThrough { .. }),
                skipped,
            }) => match outcome.processed_count(positions) {
                Ok(Some(processed_count)) => BatchAttempt::Settled(SettledBatch {
                    outcome,
                    processed_count,
                    skipped,
                    committed: false,
                }),
                Ok(None) => unreachable!("retry outcomes are handled separately"),
                Err(err) => BatchAttempt::Unusable(err.to_string()),
            },
            Ok(Delivered {
                outcome: BatchHandlerOutcome::Retry { reason },
                ..
            }) => BatchAttempt::Again(RetryCause::Retry { reason }),
            Err(e) => BatchAttempt::Again(RetryCause::Failed(e.to_string())),
        }
    }
}

#[cfg(feature = "db")]
impl<H, OM> TxSlotDispatcher<H, OM>
where
    H: DeliverTxBatch<OM> + 'static,
    OM: CommitOffsetInTx + 'static,
{
    pub async fn run(
        &self,
        ctx: Arc<SecurityContext>,
        cancel: CancellationToken,
    ) -> Result<(), EventBrokerError> {
        self.run_slot(ctx, cancel).await
    }
}

#[cfg(feature = "db")]
impl<H, OM> SlotFlavour for TxSlotDispatcher<H, OM>
where
    H: DeliverTxBatch<OM> + 'static,
    OM: CommitOffsetInTx + 'static,
{
    const TEXTS: SlotTexts = TX_TEXTS;
    const CANCEL_STOPS_DELIVERY: bool = false;
    const OPEN_EXHAUSTION_ENDS_RUN: bool = false;

    fn settings(&self) -> &SlotSettings {
        &self.settings
    }

    fn offsets(&self) -> &dyn OffsetStore {
        self.offset_manager.as_ref()
    }

    /// Only traces the positions: offsets are committed in the handler's transaction.
    async fn take_frame_positions(
        &self,
        _group_id: &ConsumerGroupId,
        _subscription_id: SubscriptionId,
        positions: &[PartitionPosition],
    ) {
        for p in positions {
            trace!(
                topic = p.topic.as_ref(),
                partition = p.partition,
                offset = %p.offset,
                last_examined = %p.last_examined,
                "transactional position observed without out-of-tx commit"
            );
        }
    }

    async fn deliver(
        &self,
        group_id: ConsumerGroupId,
        key: &TopicPartitionKey,
        events: &[EventV1],
        positions: &[StoredPosition],
        delivery: &Delivery,
    ) -> BatchAttempt {
        let commit_handle = TxCommitHandle::new(TxCommitHandleParts {
            partition: positions[0].partition,
            offsets: positions.iter().map(|at| at.sequence).collect(),
            offset_manager: self.offset_manager.clone(),
            group: group_id,
            topic: key.topic_id,
        });
        let committed_offset = commit_handle.committed_offset.clone();
        let batch = EventBatch::new(events);
        match self
            .handler
            .deliver(&batch, positions, delivery, commit_handle)
            .await
        {
            Ok(crate::consumer::HandlerOutcome::Success) => {
                let Ok(committed_offset) = committed_offset.lock().map(|guard| *guard) else {
                    return BatchAttempt::Unusable("tx commit state mutex poisoned".to_owned());
                };
                let Some(committed_offset) = committed_offset else {
                    return BatchAttempt::Unusable(
                        "transactional handler returned success without committing an offset"
                            .to_owned(),
                    );
                };
                match BatchHandlerOutcome::processed_count_through(committed_offset, positions) {
                    Ok(processed_count) => BatchAttempt::Settled(SettledBatch {
                        outcome: if processed_count == positions.len() {
                            BatchHandlerOutcome::Success
                        } else {
                            BatchHandlerOutcome::AdvanceThrough {
                                offset: committed_offset,
                            }
                        },
                        processed_count,
                        skipped: None,
                        committed: true,
                    }),
                    Err(err) => BatchAttempt::Unusable(err.to_string()),
                }
            }
            Ok(crate::consumer::HandlerOutcome::Retry { reason }) => {
                BatchAttempt::Again(RetryCause::Retry { reason })
            }
            Err(e) => BatchAttempt::Again(RetryCause::Failed(e.to_string())),
        }
    }
}
