//! The writer of `pricing_outbox` — the transactional enqueue, and the one
//! definition of the `PlanPublished` payload.
//!
//! Transactional is the whole point of the table: **an event exists if and only
//! if its commit happened**. So this module, like
//! [`audit_repo`](super::audit_repo), is stateless and takes a **runner** rather
//! than a provider. An enqueue in a transaction of its own could commit while
//! the publish rolled back — a `PlanPublished` for a plan that is still `draft`,
//! delivered at-least-once to every consumer — or fail while the publish
//! committed, which is a version nothing is ever told about.
//!
//! # Three things this file decides, each with its reason
//!
//! **The sequence is per aggregate and never global.** `seq` is
//! `MAX(seq) + 1` over `(tenant_id, aggregate_id)`, guarded by
//! `uq_pricing_outbox_sequence`. A global sequence would serialize every
//! tenant's publishing behind one counter, and no consumer needs cross-aggregate
//! order — `01-foundation.md` §1.2 orders the frozen event set per
//! `(tenantId, aggregateId)` and says nothing about a total order. The posture
//! is the audit chain's: the read of the head is not the guard, the unique index
//! is, and the loser's whole transaction rolls back.
//!
//! **The dedup key is derived here, never at the call site.** For a plan
//! publish the natural key is the publish unit itself — `(event name, plan_id,
//! revision)` — because a revision publishes exactly **once**: D-90 gives a
//! draft revision one edge out, and the publish commit's compare-and-swap
//! refuses the second attempt. Under `uq_pricing_outbox_dedup_key
//! (tenant_id, dedup_key)` a second `PlanPublished` for one revision therefore
//! fails at the **writer** rather than at every consumer, which is what that
//! index is for. Derived in [`plan_published_dedup_key`] so that no caller can
//! spell it a second way; a dedup key spelled twice is a dedup key that dedups
//! nothing.
//!
//! **`published_at` is NULL and stays NULL.** Draining is the relay's job and no
//! part of the publish commit; `idx_pricing_outbox_undrained` is that relay's
//! cursor. Note also that `config.events_enabled` (default `false`) gates
//! **fan-out, not the row** — [`crate::config`] says so in as many words — so
//! [`enqueue`] is unconditional and never reads the flag. A publish whose row
//! was skipped because fan-out was off would be a publish nothing could ever
//! replay.
//!
//! # The event name comes from one place
//!
//! `NewOutboxEvent` carries a [`CatalogEvent`] rather than a `String`, and the
//! column is written from [`CatalogEvent::as_str`].
//! `chk_pricing_outbox_event_name` pins the same thirteen names, so a second
//! spelling *would* be caught — but caught as a driver error inside a publish
//! transaction, which is not where a frozen contract should be discovered.
//!
//! # A gap this file named, and what filled it
//!
//! [`PricingSnapshotRef`](crate::domain::snapshot::PricingSnapshotRef) has three
//! parts: the version ref, the resolved price ids, and an
//! `evaluation_policy_version`. Until D-162 the commit produced the first two
//! and this file stamped nothing for the third, because no document in this gear
//! or in Rating named a producer or a format for it and a placeholder in a
//! published payload is a value a consumer pins against — the first thing to pin
//! against an invented policy version being the last thing that could tell it
//! was invented (D-161).
//!
//! D-162 gives it a producer: the **evaluation-policy generation**, a declared
//! constant of this gear ([`EVALUATION_POLICY_GENERATION`]) naming which
//! evaluation-policy field set the frozen row content is to be read under. So
//! the payload now carries all three segments. The ban stands — what is stamped
//! is the generation the gear declares and whose roster
//! `01-foundation.md` §4.4 pins, never a filler.
//!
//! # One more naming decision, recorded because nothing else records it
//!
//! **No document declares the `PlanPublished` payload's field list.** The design
//! set fixes the event *name*, the ordering key, the at-least-once delivery and
//! the requirement that the event carry a pending version ref and a correlation
//! key — and nothing else. The wire keys below are therefore this module's, in
//! the `camelCase` the design set spells consumer-visible fields in
//! (`pricingSnapshotRef`, `planId`); they are written here once so a later
//! document has something concrete to contradict.

use bss_pricing_sdk::CatalogVersion;
use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use serde_json::{Value as JsonValue, json};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

use crate::domain::evaluation_policy::EVALUATION_POLICY_GENERATION;
use crate::domain::events::CatalogEvent;
use crate::domain::scope_key::PlanId;
use crate::infra::storage::entity::outbox;
use crate::infra::storage::{RepoError, contention_or_db};

/// The `PlanPublished` payload, as one type rather than a map built at a call
/// site.
///
/// The version ref is the registry's **pending** handle and never a committed
/// version: §4.2 step 4 is explicit that `PlanPublished` carries a pending ref,
/// because the registry batches and the version does not exist yet. A consumer
/// that received a committed version here would be reading a number this gear
/// invented.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanPublishedPayload {
    /// The plan that published.
    pub plan_id: PlanId,
    /// The revision that became current.
    pub revision: u64,
    /// The registry's pending handle.
    pub pending_version_ref: String,
    /// The price rows this commit moved into `published`.
    pub price_ids: Vec<Uuid>,
    /// The correlation id of the causing request.
    pub correlation_id: Uuid,
}

impl PlanPublishedPayload {
    /// Render the payload for its `jsonb` column.
    ///
    /// Built with `json!` rather than a `Serialize` derive so the **wire** keys
    /// are spelled in exactly one place and are visibly a different vocabulary
    /// from the Rust field names — the sibling precedent is `price_repo`'s
    /// `allowance_json`, which persists the D-45 declaration in the spelling the
    /// design set uses rather than the one the struct happens to have.
    ///
    /// `evaluationPolicyVersion` is read from the constant rather than carried
    /// on the struct (D-162): it is a property of the gear that published, not
    /// of the publish, so a caller able to supply one could stamp a period with
    /// a semantics its rows were never frozen under. The three segments are
    /// stamped **flat**, beside the payload's other keys, because no document
    /// declares a `pricingSnapshotRef` envelope for this event and inventing
    /// one here would put a wire structure on the record that nothing agreed to.
    #[must_use]
    pub fn to_value(&self) -> JsonValue {
        json!({
            "planId": self.plan_id.get(),
            "revision": self.revision,
            "pendingVersionRef": self.pending_version_ref,
            "priceIds": self.price_ids,
            "evaluationPolicyVersion": EVALUATION_POLICY_GENERATION,
            "correlationId": self.correlation_id,
        })
    }
}

/// One event, as its writer is handed it.
///
/// `seq` is deliberately **not** here: it is the aggregate's, computed from the
/// rows already enqueued for it, and a caller that could supply one could
/// reorder another caller's events.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewOutboxEvent {
    /// The owning tenant.
    pub tenant_id: Uuid,
    /// The aggregate the event is ordered within — the plan, for a plan
    /// publish.
    pub aggregate_id: Uuid,
    /// Which of the thirteen frozen names.
    pub event: CatalogEvent,
    /// The rendered payload.
    pub payload: JsonValue,
    /// What makes a repeat of *this* publish the same event.
    pub dedup_key: String,
    /// The correlation id of the causing request.
    pub correlation_id: Uuid,
    /// When the event was enqueued, UTC — the caller's instant, for the reason
    /// [`NewAuditEntry`](super::audit_repo::NewAuditEntry) gives.
    pub enqueued_at: DateTime<Utc>,
}

impl NewOutboxEvent {
    /// The `PlanPublished` event of one publish unit, whole.
    ///
    /// A named constructor rather than a struct literal at the call site,
    /// because three of the fields are not free: the event name is
    /// [`CatalogEvent::PlanPublished`], the aggregate is the plan, and the dedup
    /// key is [`plan_published_dedup_key`]. A call site free to choose them is a
    /// call site free to enqueue a `PlanPublished` under a dedup key that dedups
    /// against nothing.
    #[must_use]
    pub fn plan_published(
        tenant_id: Uuid,
        payload: &PlanPublishedPayload,
        enqueued_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id,
            aggregate_id: payload.plan_id.get(),
            event: CatalogEvent::PlanPublished,
            payload: payload.to_value(),
            dedup_key: plan_published_dedup_key(payload.plan_id, payload.revision),
            correlation_id: payload.correlation_id,
            enqueued_at,
        }
    }

    /// The `PlanPublishDegraded` event of one publish whose subject has not
    /// warmed.
    ///
    /// A named constructor for [`NewOutboxEvent::plan_published`]'s reason: the
    /// event name, the aggregate and the dedup key are not the caller's to
    /// choose. The aggregate is the **plan**, so the degradation is ordered
    /// with that plan's other events under the `(tenantId, aggregateId)` rule
    /// rather than arriving on a stream of its own.
    #[must_use]
    pub fn plan_publish_degraded(
        tenant_id: Uuid,
        payload: &PlanPublishDegradedPayload,
        enqueued_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id,
            aggregate_id: payload.plan_id.get(),
            event: CatalogEvent::PlanPublishDegraded,
            payload: payload.to_value(),
            dedup_key: plan_publish_degraded_dedup_key(payload.plan_id, payload.catalog_version),
            correlation_id: payload.correlation_id,
            enqueued_at,
        }
    }

    /// The `PriceWindowActivated` of one window taking effect
    /// (`inst-ws-activate`).
    ///
    /// A named constructor for [`NewOutboxEvent::plan_published`]'s reason. What
    /// distinguishes this event from its sibling is entirely the **name and the
    /// dedup key**: the body is one rendering of one window's facts, and which
    /// boundary was crossed is what the name says (see
    /// [`PriceWindowTransitionPayload::to_value`] for why it is not also a field).
    ///
    /// The aggregate is the **plan** — §7 orders `PriceWindow*` events per
    /// `(tenant, plan)`, and the window's own id would give every window a
    /// stream of two events on which nothing is ordered against anything.
    #[must_use]
    pub fn price_window_activated(
        tenant_id: Uuid,
        payload: &PriceWindowTransitionPayload,
        enqueued_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id,
            aggregate_id: payload.plan_id.get(),
            event: CatalogEvent::PriceWindowActivated,
            payload: payload.to_value(),
            dedup_key: price_window_transition_dedup_key(
                CatalogEvent::PriceWindowActivated,
                payload.window_id,
            ),
            correlation_id: payload.correlation_id,
            enqueued_at,
        }
    }

    /// The `PriceWindowExpired` of one window reaching its end
    /// (`inst-ws-expire`).
    ///
    /// [`NewOutboxEvent::price_window_activated`]'s sibling, and the reason the
    /// dedup key names the transition: this event and that one are about one
    /// window, so a key that named only the window would let the activation
    /// swallow the expiry.
    #[must_use]
    pub fn price_window_expired(
        tenant_id: Uuid,
        payload: &PriceWindowTransitionPayload,
        enqueued_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id,
            aggregate_id: payload.plan_id.get(),
            event: CatalogEvent::PriceWindowExpired,
            payload: payload.to_value(),
            dedup_key: price_window_transition_dedup_key(
                CatalogEvent::PriceWindowExpired,
                payload.window_id,
            ),
            correlation_id: payload.correlation_id,
            enqueued_at,
        }
    }

    /// A window **mutation**'s event — `PriceWindowScheduled` or
    /// `PriceWindowCancelled` (D-99, `inst-ws-publishunit`).
    ///
    /// The other two of §7's four frozen names, and the last of the four to get a
    /// producer. Unlike its two siblings above it takes the event as an argument
    /// rather than being two constructors, and the reason is the inverse of theirs:
    /// there the event was a total function of *which sweep boundary* was crossed
    /// and a caller choosing it could have produced an `active` body under the
    /// expired name; here the event is a total function of **which of §5's three
    /// surfaces** was called, which is a fact only the caller has. `infra::window`'s
    /// `Op::event` is the single place that decides it, and it is a `const fn` over a
    /// closed enumeration — so the pairing is still held in one place, just one
    /// layer up.
    ///
    /// **It refuses the other two names by assertion rather than by type**, because
    /// [`CatalogEvent`] is the gear's whole event vocabulary and narrowing it to a
    /// two-member subset here would mint a type for one call site.
    ///
    /// # Panics
    /// When handed anything but the two mutation events — a caller passing
    /// `PriceWindowActivated` here would emit a sweep's event from an operator's
    /// transaction, which is `inst-ws-publishunit`'s negative half inverted: the
    /// time-driven flips are **not** publish units and must never carry a pending
    /// ref. This is a programming error in one crate, so it fails loudly rather than
    /// widening the signature with an error a route would have to render.
    #[must_use]
    pub fn price_window_mutation(
        tenant_id: Uuid,
        event: CatalogEvent,
        payload: &PriceWindowTransitionPayload,
        enqueued_at: DateTime<Utc>,
        act: &str,
    ) -> Self {
        assert!(
            matches!(
                event,
                CatalogEvent::PriceWindowScheduled | CatalogEvent::PriceWindowCancelled
            ),
            "a window mutation emits PriceWindowScheduled or PriceWindowCancelled, never {}",
            event.as_str()
        );
        Self {
            tenant_id,
            aggregate_id: payload.plan_id.get(),
            event,
            payload: payload.to_value(),
            // The **act**, not the transition. `price_window_transition_dedup_key`
            // is the sweep's shape and says "one activation and one expiry per
            // window, by §4's edge set" — true of the two time-driven flips and
            // false of the operator's acts, because `Op::event` maps a schedule
            // **and** an adjustment to `PriceWindowScheduled`. So an adjustment of
            // a window that was scheduled through the route deduped against its
            // own schedule and was refused by `uq_pricing_outbox_dedup_key` — a
            // 409 on a legal act, and the reason no window could be adjusted twice.
            //
            // Widening the key rather than minting `PriceWindowAdjusted`: a fifth
            // event name is a wire fact §7 does not declare and
            // `chk_pricing_outbox_event_name` does not carry, so it owes a decision
            // and a migration. The dedup key is internal to this table, so naming
            // the act in it settles the defect without deciding the naming
            // question — which stays owed, and is now a naming gap rather than a
            // correctness one.
            dedup_key: format!(
                "{}/{act}",
                price_window_transition_dedup_key(event, payload.window_id)
            ),
            correlation_id: payload.correlation_id,
            enqueued_at,
        }
    }

    /// The `PriceUpdated` of one row superseding another on its key
    /// (`inst-su-return`, D-88).
    ///
    /// A named constructor for [`NewOutboxEvent::plan_published`]'s reason, and the
    /// aggregate is the **plan** — Foundation §1.2 orders the frozen set per
    /// `(tenantId, aggregateId)`, and a price row's aggregate is the plan it prices,
    /// which is the same answer `audit_repo::plan_chain` gives one store over.
    /// `PriceCreated` for a row this gear has just authored (S3 §17.5).
    ///
    /// **The payload is deliberately shorter than [`PriceUpdatedPayload`]'s**, and
    /// the missing member is the informative one: a created row carries **no**
    /// `pending_version_ref`, because authoring is on the draft plane and a draft is
    /// addressable at no `CatalogVersion` at all. A supersession's `PriceUpdated`
    /// carries one because that act publishes. Filling this with a placeholder would
    /// be the lie this crate has already paid for once.
    #[must_use]
    pub fn price_created(
        tenant_id: Uuid,
        payload: &PriceCreatedPayload,
        enqueued_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id,
            aggregate_id: payload.plan_id.get(),
            event: CatalogEvent::PriceCreated,
            payload: payload.to_value(),
            dedup_key: price_created_dedup_key(payload.price_id),
            correlation_id: payload.correlation_id,
            enqueued_at,
        }
    }

    #[must_use]
    pub fn price_updated(
        tenant_id: Uuid,
        payload: &PriceUpdatedPayload,
        enqueued_at: DateTime<Utc>,
    ) -> Self {
        Self {
            tenant_id,
            aggregate_id: payload.plan_id.get(),
            event: CatalogEvent::PriceUpdated,
            payload: payload.to_value(),
            dedup_key: price_updated_dedup_key(payload.price_id),
            correlation_id: payload.correlation_id,
            enqueued_at,
        }
    }
}

/// The dedup key of a plan publish: `PlanPublished/<plan_id>/<revision>`.
///
/// The publish unit itself, because a revision publishes exactly once. Written
/// down rather than computed at the call site so the same publish always
/// produces the same key and two different publishes never produce one.
#[must_use]
pub fn plan_published_dedup_key(plan_id: PlanId, revision: u64) -> String {
    format!(
        "{}/{}/{}",
        CatalogEvent::PlanPublished.as_str(),
        plan_id,
        revision
    )
}

/// The `PlanPublishDegraded` payload — the second event this file defines, and
/// the second whose field list no document declares.
///
/// §1.2 requires the event and names nothing about its shape, exactly as it did
/// for `PlanPublished`; the keys below are therefore this module's, written
/// here once so a later document has something concrete to contradict.
///
/// It names the publish by its **plan and committed version** and carries the
/// handle as lineage. Under D-166 the degraded condition is *the commit was
/// observed and the warm has not landed*, so the version is known at every
/// instant the condition is observable — which is what the earlier
/// handle-keyed form could not assume, having been written when the degraded
/// predicate fired on a ref nobody had resolved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanPublishDegradedPayload {
    /// The plan whose subject is not warm.
    pub plan_id: PlanId,
    /// The version the registry committed the publish into.
    pub catalog_version: CatalogVersion,
    /// The registry handle its publish is still waiting on — lineage, so an
    /// operator can find the ref row this was observed from.
    pub pending_version_ref: String,
    /// When this gear first saw the registry's answer: **what the degraded age
    /// is measured from** (D-166 clause 2).
    ///
    /// `requested_at` was here before and was the wrong instant, not merely a
    /// coarser one: `fr-publish-fanout-atomicity` puts the pre-commit batching
    /// wait **outside** degraded handling, and an age measured from the request
    /// includes nothing but that wait for the first five minutes of every
    /// publish.
    pub commit_observed_at: DateTime<Utc>,
    /// The correlation id of the sweep pass that observed it.
    pub correlation_id: Uuid,
}

impl PlanPublishDegradedPayload {
    /// Render the payload for its `jsonb` column, `camelCase` as its sibling's.
    #[must_use]
    pub fn to_value(&self) -> JsonValue {
        json!({
            "planId": self.plan_id.get(),
            "catalogVersion": self.catalog_version.get(),
            "pendingVersionRef": self.pending_version_ref,
            "commitObservedAt": self.commit_observed_at,
            "correlationId": self.correlation_id,
        })
    }
}

/// The dedup key of a degraded publish:
/// `PlanPublishDegraded/<plan_id>/<catalog_version>`.
///
/// **The plan and its version**, which is what D-166 clause (2) makes available:
/// the degraded condition is now *observed committed and still unwarm*, so the
/// pass holding the condition is the pass holding the registry's answer. The
/// earlier form keyed on the pending handle because the predicate it served
/// fired on refs nothing had resolved — there was no version to name.
///
/// Naming the version rather than the handle is also what makes the key mean the
/// **publish**: two revisions of one plan landing in one D-47 batch degrade as
/// one version's failure to warm, and an operator paging on it acts once.
///
/// Under `uq_pricing_outbox_dedup_key (tenant_id, dedup_key)` that makes a
/// repeat of one degradation **one** event, which is the whole requirement.
#[must_use]
pub fn plan_publish_degraded_dedup_key(plan_id: PlanId, catalog_version: CatalogVersion) -> String {
    format!(
        "{}/{}/{}",
        CatalogEvent::PlanPublishDegraded.as_str(),
        plan_id,
        catalog_version.get()
    )
}

/// The `PriceWindowActivated` / `PriceWindowExpired` payload — one type for both,
/// and now **one rendering** for both: they are one fact about one window, and
/// which boundary it crossed is the event's name.
///
/// **The third payload whose field list no document declares.** §7 names the
/// four `PriceWindow*` events, says they are ordered per `(tenant, plan)`,
/// idempotency-keyed and at-least-once, and says nothing about their content.
/// The keys below are therefore this module's, in the `camelCase` the design set
/// spells consumer-visible fields in, written here once so a later document has
/// something concrete to contradict — the posture this file already took twice.
///
/// It carries the window's **interval** rather than the instant of the flip, and
/// that is the same choice D-99 makes one plane over: the interval is the fact,
/// and "active at `t`" is derived from it. The flip instant is not dropped — it
/// is `effectiveFrom` for an activation and `effectiveTo` for an expiry by
/// construction of §4's conditions, so a field for it would be a second
/// spelling of a value already here, free to disagree with it. When the flip was
/// *recorded* is the row's own `enqueued_at`, which is where the sweep's lag is
/// visible.
///
/// # `state` is gone, because that rule was applied to one of two candidates
///
/// The payload used to carry `"state": "active" | "expired"`, and the paragraph
/// above refuses the flip instant on a ground that covers it exactly: a second
/// spelling of a value already carried, free to disagree with it. `state` was a
/// **total function of the event name** — the two constructors are the only
/// callers and there is one activation and one expiry per window by §4's edge set
/// — so the rule is applied consistently and the field is dropped rather than
/// kept and tested.
///
/// The counter-argument, and why it does not save the field: the flip instant
/// would have duplicated a value *inside* the payload, while `state` duplicated
/// the **envelope** — `pricing_outbox.event_name`, which is not in this JSON. That
/// is a real difference and it makes the duplication *worse*, not admissible. The
/// event name is `NOT NULL`, pinned to thirteen values by
/// `chk_pricing_outbox_event_name`, and is what any consumer dispatches on; a body
/// contradicting its own envelope has no resolution rule at all, whereas two
/// disagreeing payload keys at least both live in one document. A test asserting
/// the pairing was the other option and is not equivalent: the rule as this doc
/// block states it refuses a second spelling because it *can* disagree, not
/// because nothing checks it.
///
/// **The design set constrains neither choice**: §7 names the four `PriceWindow*`
/// events, their `(tenant, plan)` ordering, their idempotency keys and their
/// at-least-once delivery, and declares no payload field — no `state`, no
/// `windowId`. Nothing is owed to a consumer either way, no relay existing. So
/// dropping the field mints nothing and removes nothing anything agreed to; it is
/// decided by the rule this file already wrote down.
///
/// It carries **no canonical scope key**. The key is eight axes of
/// `pricing_price` and resolving one per event would put a query per flip on a
/// sweep's path; a consumer that needs the key has the `priceId` and the pinned
/// delta, which carries the window facts grouped by key already (D-121).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceWindowTransitionPayload {
    /// The window that moved.
    pub window_id: Uuid,
    /// The plan the window's row is on — the aggregate the event is ordered
    /// within.
    pub plan_id: PlanId,
    /// The price row the window is bound to.
    pub price_id: Uuid,
    /// Inclusive start of the half-open interval, UTC.
    pub effective_from: DateTime<Utc>,
    /// Exclusive end, UTC; `None` is open-ended.
    pub effective_to: Option<DateTime<Utc>>,
    /// The correlation id of the sweep pass that observed the boundary.
    pub correlation_id: Uuid,
}

impl PriceWindowTransitionPayload {
    /// Render the payload for its `jsonb` column, `camelCase` as its two siblings'.
    ///
    /// A `to_value(&self)` like theirs, which it could not be while the renderer
    /// took a `state`: a public renderer the caller chose a state for could have
    /// produced an `active` body under the `PriceWindowExpired` name, so the
    /// pairing had to be held by a private free function and its two callers. With
    /// the field gone there is no pairing left to get wrong, and the guard is a
    /// value that does not exist rather than a shape a reader has to notice.
    #[must_use]
    pub fn to_value(&self) -> JsonValue {
        json!({
            "windowId": self.window_id,
            "planId": self.plan_id.get(),
            "priceId": self.price_id,
            "effectiveFrom": self.effective_from,
            "effectiveTo": self.effective_to,
            "correlationId": self.correlation_id,
        })
    }
}

/// The dedup key of one window transition:
/// `PriceWindowActivated/<window_id>`.
///
/// **The window *and* the transition**, which is what makes the coordination
/// lease over the activation sweep a performance measure rather than the only
/// thing standing between two replicas and two events for one flip. The event
/// name *is* the transition — there is exactly one activation and one expiry per
/// window, by §4's edge set — so naming it plus the window id covers the pair
/// with no third segment.
///
/// Both halves are load-bearing and each fails a different way:
///
/// * without the **window**, one plan's second activation would dedup against
///   its first;
/// * without the **transition**, a window's expiry would dedup against its own
///   activation and never be emitted at all — the event a consumer needs in
///   order to stop resolving a price.
///
/// Two sweeps flipping one window therefore write **one** row, refused by either
/// member of the pair this key reaches: `uq_pricing_outbox_dedup_key
/// (tenant_id, dedup_key)` and the `outbox_id` primary key, which [`outbox_id`]
/// derives from the same pair for exactly this reason. Naming only the index would
/// name the one the driver cannot confirm answered.
#[must_use]
pub fn price_window_transition_dedup_key(event: CatalogEvent, window_id: Uuid) -> String {
    format!("{}/{}", event.as_str(), window_id)
}

/// The `PriceCreated` payload — a row coming into existence on a key.
///
/// **Two producers, one name**, which is D-203's decision rather than a
/// duplication: the authoring door, where `03-price-structure.md` §17.5 puts it,
/// and the cutover's commit, which R-02 requires to announce it twice. Both are a
/// row being born; neither is the other's special case. This type's field list is
/// this module's, like its three siblings', because §17.5 declares what the event
/// *means* and not what it carries.
///
/// It carries **no** version ref, and that is the difference from
/// [`PriceUpdatedPayload`] worth stating: a draft is addressable at no
/// `CatalogVersion`, so there is no pending handle to name from the authoring door.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceCreatedPayload {
    /// The plan the row belongs to, and the event's aggregate.
    pub plan_id: PlanId,
    /// The row that was authored.
    pub price_id: Uuid,
    /// Its canonical scope key, rendered.
    pub scope_key: String,
    /// The request that authored it.
    pub correlation_id: Uuid,
}

impl PriceCreatedPayload {
    /// The payload as the outbox stores it.
    #[must_use]
    pub fn to_value(&self) -> JsonValue {
        serde_json::json!({
            "planId": self.plan_id.get(),
            "priceId": self.price_id,
            "scopeKey": self.scope_key,
            "correlationId": self.correlation_id,
        })
    }
}

/// The `PriceUpdated` payload — the **fourth** whose field list no document
/// declares.
///
/// `03-price-structure.md` §17.5 says only *"`PriceCreated` on row authoring,
/// `PriceUpdated` on supersession"*, and Foundation §1.2 puts both in the frozen
/// name set ordered per `(tenantId, aggregateId)`. So what the event *means* is
/// declared and its content is not; the keys below are this module's, in the same
/// `camelCase` its three siblings use.
///
/// **This block documented [`PriceCreatedPayload`] from `e6a2edcce` until
/// 2026-08-06.** That commit inserted the sibling type between this doc and the
/// type it describes, and the symptom was treated rather than the cause: an
/// `#[allow(clippy::doc_markdown)]` was added with a `reason` explaining that the
/// block above documents the type below. It also carried the sentence
/// *"`PriceCreated` still has no producer anywhere in this gear"* — which the very
/// commit that moved it made false. A doc block that has to explain which type it
/// belongs to has already been transplanted.
///
/// # It names the predecessor, and the field is not optional
///
/// A `PriceUpdated` is *by definition* a row landing on an occupied canonical scope
/// key: both sanctioned producers of `published → superseded` set
/// `supersedes_price_id` on the successor (D-127), and there is no third. A
/// `Option<Uuid>` here would be a shape saying a price can be "updated" with nothing
/// updated — and a consumer's whole reason to read this event is to stop resolving
/// the row it replaces.
///
/// # It carries the **canonical scope key**, unlike its window sibling
///
/// [`PriceWindowTransitionPayload`] deliberately does not, on the ground that
/// resolving ten axes per event would put a query on a *sweep's* path. This event
/// is emitted from the operator transaction that already holds the key, so the
/// argument does not transfer: the cost is zero and the key is the only field that
/// tells a consumer *which price* moved without a second lookup.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriceUpdatedPayload {
    /// The plan the row belongs to — the aggregate the event is ordered within.
    pub plan_id: PlanId,
    /// The row that became current on the key.
    pub price_id: Uuid,
    /// The ten axes, canonically rendered.
    pub scope_key: String,
    /// The row it replaced, which every producer of this event has.
    pub supersedes_price_id: Uuid,
    /// The instant coverage hands over from the predecessor to this row.
    pub changeover: DateTime<Utc>,
    /// The registry's **pending** handle, for [`PlanPublishedPayload`]'s reason: the
    /// version does not exist yet and will not until the registry batches (D-47).
    pub pending_version_ref: String,
    /// The correlation id of the causing request.
    pub correlation_id: Uuid,
}

impl PriceUpdatedPayload {
    /// Render the payload for its `jsonb` column, `camelCase` as its siblings'.
    #[must_use]
    pub fn to_value(&self) -> JsonValue {
        json!({
            "planId": self.plan_id.get(),
            "priceId": self.price_id,
            "scopeKey": self.scope_key,
            "supersedesPriceId": self.supersedes_price_id,
            "changeover": self.changeover,
            "pendingVersionRef": self.pending_version_ref,
            "correlationId": self.correlation_id,
        })
    }
}

/// The dedup key of one row's supersession: `PriceUpdated/<price_id>`.
///
/// **The successor's id, and nothing else.** `draft → published` is a row's only
/// edge out of `draft` and the supersession commit's compare-and-swap refuses the
/// second attempt, so a row is superseded-onto exactly once and its id is the
/// natural key — the same argument [`plan_published_dedup_key`] makes about a
/// revision.
///
/// It is stable across a **retry** of one act for a reason worth stating, because
/// the analogous window key was not: the successor row is staged at compose and its
/// id is read back from the store by every later attempt, so a retry after an
/// approval renders this key from the same id rather than from one minted per
/// request. That is the property `window_unit_ref`'s schedule arm could not have,
/// and it is why this key needs no act segment the way
/// [`NewOutboxEvent::price_window_mutation`]'s does.
#[must_use]
pub fn price_updated_dedup_key(price_id: Uuid) -> String {
    format!("{}/{}", CatalogEvent::PriceUpdated.as_str(), price_id)
}

/// One `PriceCreated` per price row, ever.
///
/// The row id is the whole key because a row is created exactly once: a replayed
/// authoring call is refused upstream by the idempotency gate, and a re-authored
/// key is a **different** row with a different id.
#[must_use]
pub fn price_created_dedup_key(price_id: Uuid) -> String {
    format!("{}/{}", CatalogEvent::PriceCreated.as_str(), price_id)
}

/// Enqueue one event inside `runner`'s transaction, returning the `seq` it
/// landed at within its aggregate.
///
/// # Errors
/// [`RepoError::Db`] on a scope or storage failure — which **includes** the two
/// refusals this table's indexes make: a second event on one `(tenant, dedup
/// key)`, and losing a race for the next `seq` of an aggregate. Neither is
/// pre-checked, because a pre-check is a read the insert races with anyway, and
/// neither has a wire code the design set names.
/// [`RepoError::CorruptRow`] when the aggregate's greatest `seq` is not a
/// position this counter can count in or has no successor.
pub async fn enqueue(
    runner: &impl DBRunner,
    scope: &AccessScope,
    event: NewOutboxEvent,
) -> Result<u64, RepoError> {
    let seq = match read_head_seq(runner, scope, event.tenant_id, event.aggregate_id).await? {
        None => 0_u64,
        Some(head) => head.checked_add(1).ok_or_else(|| {
            RepoError::CorruptRow(format!(
                "pricing_outbox aggregate {} stands at seq {head}, which has no successor",
                event.aggregate_id
            ))
        })?,
    };
    let stored_seq = i64::try_from(seq).map_err(|_| {
        RepoError::CorruptRow(format!(
            "pricing_outbox aggregate {} reached seq {seq}, which its column cannot hold",
            event.aggregate_id
        ))
    })?;

    let am = outbox::ActiveModel {
        outbox_id: Set(outbox_id(event.tenant_id, &event.dedup_key)),
        tenant_id: Set(event.tenant_id),
        aggregate_id: Set(event.aggregate_id),
        event_name: Set(event.event.as_str().to_owned()),
        seq: Set(stored_seq),
        payload: Set(event.payload.clone()),
        dedup_key: Set(event.dedup_key.clone()),
        correlation_id: Set(event.correlation_id),
        enqueued_at: Set(event.enqueued_at),
        // The relay drains; the publish commit never does.
        published_at: Set(None),
    };
    outbox::Entity::insert(am.clone())
        .secure()
        .scope_with_model(scope, &am)
        .map_err(|e| RepoError::Db(format!("pricing_outbox scope: {e}")))?
        .exec(runner)
        .await
        .map_err(|e| {
            // D-159's second serialization point: `uq_pricing_outbox_sequence`
            // over `(tenant_id, aggregate_id)`. The dedup index is the other
            // unique constraint on this table and the driver's class does not
            // tell them apart - both mean another write of this aggregate got
            // here first, and both remedy the same way. See
            // `contention_or_db` for what that leaves owed.
            contention_or_db(
                &e,
                &format!("plan {}", event.aggregate_id),
                "enqueue pricing_outbox",
            )
        })?;
    Ok(seq)
}

/// The greatest `seq` already enqueued for `(tenant_id, aggregate_id)`.
async fn read_head_seq(
    runner: &impl DBRunner,
    scope: &AccessScope,
    tenant_id: Uuid,
    aggregate_id: Uuid,
) -> Result<Option<u64>, RepoError> {
    let row = outbox::Entity::find()
        .secure()
        .scope_with(scope)
        .filter(
            Condition::all()
                .add(outbox::Column::TenantId.eq(tenant_id))
                .add(outbox::Column::AggregateId.eq(aggregate_id)),
        )
        .order_by(outbox::Column::Seq, Order::Desc)
        .limit(1)
        .one(runner)
        .await
        .map_err(|e| RepoError::Db(format!("read outbox head: {e}")))?;
    row.map(|row| {
        u64::try_from(row.seq).map_err(|e| {
            RepoError::CorruptRow(format!(
                "pricing_outbox aggregate {aggregate_id} holds seq {}: {e}",
                row.seq
            ))
        })
    })
    .transpose()
}

/// The row's surrogate key, derived from the identity the table actually
/// states: `UNIQUE (tenant_id, dedup_key)`.
///
/// `price_repo`'s `band_id` argument, applied one table over. `outbox_id` is a
/// `PRIMARY KEY` with no default and nothing outside this module reads it, so it
/// could have been random — deriving it makes the surrogate agree with the real
/// identity, so a repeat of one publish collides on **both** keys rather than on
/// only the one that happened to be checked first.
fn outbox_id(tenant_id: Uuid, dedup_key: &str) -> Uuid {
    Uuid::new_v5(&tenant_id, dedup_key.as_bytes())
}

#[cfg(test)]
#[path = "outbox_repo_tests.rs"]
mod outbox_repo_tests;
