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
//! # A gap this file names and does not fill
//!
//! [`PricingSnapshotRef`](crate::domain::snapshot::PricingSnapshotRef) has three
//! parts: the version ref, the resolved price ids, and an
//! `evaluation_policy_version`. The commit produces the first two.
//! **Nothing in this gear produces the third, and no document says what does** —
//! `01-foundation.md` §4.4 and `fr-pricing-snapshot` both name the field, and no
//! section names its source or its format. So [`PlanPublishedPayload`] carries
//! the two halves the commit genuinely has and **does not** stamp a snapshot
//! ref: a placeholder in a published payload is a value a consumer would pin
//! against, and the first thing that pins against an invented policy version is
//! the last thing that can tell it was invented.
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

use chrono::{DateTime, Utc};
use sea_orm::ActiveValue::Set;
use sea_orm::{ColumnTrait, Condition, EntityTrait, Order};
use serde_json::{Value as JsonValue, json};
use toolkit_db::secure::{AccessScope, DBRunner, SecureEntityExt, SecureInsertExt};
use uuid::Uuid;

use crate::domain::events::CatalogEvent;
use crate::domain::scope_key::PlanId;
use crate::infra::storage::RepoError;
use crate::infra::storage::entity::outbox;

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
    #[must_use]
    pub fn to_value(&self) -> JsonValue {
        json!({
            "planId": self.plan_id.get(),
            "revision": self.revision,
            "pendingVersionRef": self.pending_version_ref,
            "priceIds": self.price_ids,
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
        .map_err(|e| RepoError::Db(format!("enqueue pricing_outbox: {e}")))?;
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
