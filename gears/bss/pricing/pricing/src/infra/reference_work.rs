//! Shared durable executor: local transitions are conditional, remote calls are idempotent.
//!
//! One machine drives both kinds of reference (D-407): a price book entry and a plan item. It
//! dispatches by [`RefKind`] at every hook that touches the reference itself — the work input and
//! its endpoint, the write (Tx B), the confirm (Tx C), the receipt, the refusals of the write,
//! the SKU re-read's lifecycle rules, and the loss and its event. The entry's hooks live here;
//! the plan item's in [`plan_item`].
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-reference-protocol:p1
pub mod plan_item;
use crate::{
    api::rest::authoring::{
        AuthoringState,
        dto::{PricingPlanItemCreate, PricingPriceBookEntryCreate, PricingPriceBookEntryDto},
        support::{self, DoorError},
    },
    domain::{
        price_book_entry::{OpState, ReferenceState, charge_kind_for},
        reference_op::{self, Effect, Event, Op, OpKind, RefKind},
    },
    infra::storage::{
        RepoError,
        entity::{plan_item as plan_item_entity, price_book_entry, reference_op as entity},
        repo::{idempotency_repo as idem, price_book_entry_repo, reference_op_repo as ops},
    },
};
use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
};
use bss_products_sdk::{
    ReferenceRegistryV1,
    models::{Lifecycle, ReferenceKind},
};
pub use plan_item::attach_op;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use time::OffsetDateTime;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::{AccessScope, DBRunner};
use toolkit_security::SecurityContext;
use uuid::Uuid;
/// Everything needed after losing the original door future, encoded in the op outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Work {
    pub target: Target,
    pub correlation: Uuid,
    pub refusal: Option<Receipt>,
    pub receipt: Option<Receipt>,
    /// [`CANCELLED`] once a create was given up before its reservation outcome was known:
    /// no entry was written, the Idempotency-Key claim was released in that same
    /// transaction, and the cancellation releases whatever reservation exists.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
}
/// The work input of each kind: what its create writes, and where its Idempotency-Key lives.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    /// An entry of `book_id`; every op of an entry carries its create input.
    PriceBookEntry {
        book_id: Uuid,
        input: PricingPriceBookEntryCreate,
    },
    /// An item of `revision_id`. Only a create carries its input; an attach, a rereserve and a
    /// delete find the item by the op's `ref_id`.
    PlanItem {
        revision_id: Uuid,
        input: Option<PricingPlanItemCreate>,
    },
}
/// The reference an op works for: its kind, its id and the SKU it reserves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ref {
    pub kind: RefKind,
    pub id: Uuid,
    pub sku_id: Uuid,
}
/// Products spells the reference kinds as pricing does.
#[must_use]
pub const fn products_kind(kind: RefKind) -> ReferenceKind {
    match kind {
        RefKind::Entry => ReferenceKind::PriceBookEntry,
        RefKind::PlanItem => ReferenceKind::PlanItem,
    }
}
/// The recorded outcome of a create cancelled before its reservation outcome was known.
pub const CANCELLED: &str = "cancelled";
/// Who drives an op. A door drives the work it just began, under the requesting principal;
/// the ticker resumes abandoned work under the pricing system actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    Door,
    Ticker,
}
/// Store the exact body rendering, including problem bodies, inside the JSON replay store.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub status: u16,
    pub body: String,
    pub etag: Option<String>,
}
impl Receipt {
    /// Render a stored response without serializing its body a second time.
    /// # Errors
    /// Rejects corrupt stored status or header values.
    pub fn response(&self) -> Result<Response, CanonicalError> {
        let status = StatusCode::from_u16(self.status).map_err(|_| corrupt())?;
        let mut response = (status, self.body.clone()).into_response();
        response.headers_mut().insert(
            "content-type",
            if status.is_client_error() || status.is_server_error() {
                "application/problem+json"
            } else {
                "application/json"
            }
            .parse()
            .map_err(|_| corrupt())?,
        );
        if let Some(tag) = &self.etag {
            response
                .headers_mut()
                .insert("etag", tag.parse().map_err(|_| corrupt())?);
        }
        Ok(response)
    }
    pub async fn error(error: CanonicalError) -> Result<Self, CanonicalError> {
        let response = error.into_response();
        let status = response.status().as_u16();
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .map_err(|_| corrupt())?;
        Ok(Self {
            status,
            body: String::from_utf8(bytes.to_vec()).map_err(|_| corrupt())?,
            etag: None,
        })
    }
    pub fn entry(model: price_book_entry::Model) -> Result<Self, CanonicalError> {
        let etag = Some(format!("\"{}\"", model.version));
        Ok(Self {
            status: 201,
            body: serde_json::to_string(&PricingPriceBookEntryDto::from(model))
                .map_err(|_| corrupt())?,
            etag,
        })
    }
    /// A created plan item's answer: 201, its body and its version.
    pub fn item(model: plan_item_entity::Model) -> Result<Self, CanonicalError> {
        let etag = Some(format!("\"{}\"", model.version));
        Ok(Self {
            status: 201,
            body: serde_json::to_string(
                &crate::api::rest::authoring::dto::PricingPlanItemDto::from(model),
            )
            .map_err(|_| corrupt())?,
            etag,
        })
    }
}
fn corrupt() -> CanonicalError {
    CanonicalError::internal("invalid durable pricing reference work").create()
}
impl Work {
    /// Decode persisted recovery input.
    /// # Errors
    /// A missing or malformed record is surfaced instead of dropped.
    pub fn read(op: &entity::Model) -> Result<Self, CanonicalError> {
        serde_json::from_str(op.outcome.as_deref().ok_or_else(corrupt)?).map_err(|_| corrupt())
    }
    pub fn encode(&self) -> Result<String, CanonicalError> {
        serde_json::to_string(self).map_err(|_| corrupt())
    }
    /// Where the create's Idempotency-Key lives: the door that began it.
    #[must_use]
    pub fn endpoint(&self) -> String {
        match &self.target {
            Target::PriceBookEntry { book_id, .. } => {
                format!("/bss-pricing/v1/price-books/{book_id}/entries")
            }
            Target::PlanItem { revision_id, .. } => {
                format!("/bss-pricing/v1/plan-revisions/{revision_id}/items")
            }
        }
    }
    /// Whether this create was cancelled before its reservation outcome was known.
    #[must_use]
    pub fn cancelled(&self) -> bool {
        self.outcome.as_deref() == Some(CANCELLED)
    }
}
/// Start a durable record inside Tx A or the removal's transaction.
/// # Errors
/// Fails only if the durable work record cannot be encoded.
pub fn new_op(
    ctx: &SecurityContext,
    reference: Ref,
    work: &Work,
    kind: OpKind,
    reservation_id: Option<Uuid>,
    key: Option<String>,
    now: OffsetDateTime,
) -> Result<entity::Model, CanonicalError> {
    Ok(entity::Model {
        op_id: Uuid::now_v7(),
        tenant_id: ctx.subject_tenant_id(),
        kind: kind.as_str().into(),
        ref_kind: reference.kind.as_str().into(),
        ref_id: reference.id,
        sku_id: reference.sku_id,
        reservation_id,
        idempotency_key: key,
        state: if kind == OpKind::Delete {
            OpState::Releasing
        } else {
            OpState::Reserving
        }
        .as_str()
        .into(),
        outcome: Some(work.encode()?),
        attempts: 0,
        next_attempt_at: now + IN_FLIGHT_GRACE,
        last_error: None,
        created_by: ctx.subject_id(),
        created_at: now,
        updated_at: now,
    })
}
/// How long a door that just created or advanced an op keeps it before the ticker may take
/// it over. The door drives its op to completion without waiting on `next_attempt_at`; the
/// grace only keeps the one-second ticker from racing a live request with a second registry
/// caller under another actor. An abandoned op (a crash, a dropped request) is due after it.
pub const IN_FLIGHT_GRACE: time::Duration = time::Duration::seconds(30);
/// Time and jitter are injectable so recovery tests never sleep for backoff.
pub trait Clock: Send + Sync {
    fn now(&self) -> OffsetDateTime;
    fn jitter_millis(&self) -> i64 {
        0
    }
}
pub struct WallClock;
impl Clock for WallClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
    fn jitter_millis(&self) -> i64 {
        i64::from(Uuid::new_v4().as_bytes()[0])
    }
}
/// Backoff remains bounded even after years of failures.
#[must_use]
pub fn backoff(attempts: i32) -> time::Duration {
    time::Duration::seconds((1_i64 << attempts.clamp(0, 9)).min(300))
}
fn parse_state(op: &entity::Model) -> Result<OpState, CanonicalError> {
    op.state.parse().map_err(|_| corrupt())
}
fn parse_ref_kind(op: &entity::Model) -> Result<RefKind, CanonicalError> {
    op.ref_kind.parse().map_err(|_| corrupt())
}
/// Whether the op re-reserves a reference that already exists: a rereserve, or the attach of a
/// copied item (D-413). Ending without a live receipt makes that reference lost.
fn replaces_a_receipt(op: &entity::Model) -> bool {
    op.kind == OpKind::Rereserve.as_str() || op.kind == OpKind::Attach.as_str()
}
/// Apply exactly the observation used to perform the external call. A stale observer retries.
async fn advance(
    tx: &impl DBRunner,
    observed: &entity::Model,
    work: &Work,
    event: Event,
    clock: &dyn Clock,
) -> Result<(OpState, Vec<Effect>), DoorError> {
    let scope = AccessScope::for_tenant(observed.tenant_id);
    let current = ops::find(tx, &scope, observed.tenant_id, observed.op_id)
        .await?
        .ok_or_else(corrupt)?;
    if current != *observed {
        return Err(RepoError::Conflict {
            code: "REFERENCE_OP_CONTENDED",
        }
        .into());
    }
    let from = parse_state(observed)?;
    let (next, effects) = reference_op::next(
        Op {
            state: from,
            reservation_id: observed.reservation_id,
            refusal: observed.last_error.clone(),
        },
        event,
    )
    .map_err(|_| corrupt())?;
    let retry = effects.contains(&Effect::Retry);
    let attempts = if retry {
        observed.attempts.saturating_add(1)
    } else {
        observed.attempts
    };
    if retry && attempts >= 10 {
        tracing::warn!(op_id=%observed.op_id, attempts, "pricing reference operation still requires recovery");
    }
    let now = clock.now();
    ops::transition(
        tx,
        &scope,
        observed.op_id,
        from,
        next.state,
        &ops::TransitionFields {
            reservation_id: next.reservation_id,
            outcome: Some(work.encode()?),
            attempts,
            next_attempt_at: if retry {
                now + (backoff(attempts) + time::Duration::milliseconds(clock.jitter_millis()))
                    .min(time::Duration::seconds(300))
            } else {
                now + IN_FLIGHT_GRACE
            },
            last_error: if retry {
                Some("REGISTRY_UNAVAILABLE".into())
            } else {
                next.refusal
            },
            updated_at: now,
        },
    )
    .await?;
    Ok((next.state, effects))
}
/// A re-reservation or an attach that ended without a live receipt leaves its reference lost:
/// the state, the kind's durable event and the audit record commit together.
async fn mark_lost(
    tx: &(impl DBRunner + Sync),
    outbox: &super::events::EventSink,
    ctx: &SecurityContext,
    work: &Work,
    op: &entity::Model,
    now: OffsetDateTime,
) -> Result<(), DoorError> {
    match parse_ref_kind(op)? {
        RefKind::Entry => mark_entry_lost(tx, outbox, ctx, work, op, now).await,
        RefKind::PlanItem => plan_item::mark_lost(tx, outbox, ctx, work, op, now).await,
    }
}
/// A re-reservation that ended without a live receipt leaves its entry lost: the
/// state, the durable `PriceBookEntryReferenceLost` event and the audit record commit together.
async fn mark_entry_lost(
    tx: &(impl DBRunner + Sync),
    outbox: &super::events::EventSink,
    ctx: &SecurityContext,
    work: &Work,
    op: &entity::Model,
    now: OffsetDateTime,
) -> Result<(), DoorError> {
    let scope = AccessScope::for_tenant(op.tenant_id);
    let Some(mut entry) = price_book_entry_repo::find(tx, &scope, op.tenant_id, op.ref_id).await?
    else {
        return Ok(());
    };
    if entry.reference_state == ReferenceState::Lost.as_str() {
        // A lost entry whose re-reservation is refused again stays lost, announced once.
        return Ok(());
    }
    price_book_entry_repo::set_reference(
        tx,
        &scope,
        op.tenant_id,
        op.ref_id,
        entry.version,
        ReferenceState::Lost,
        entry.reservation_id,
        now,
    )
    .await?;
    super::reference_events::lost(outbox, tx, &entry, ctx.subject_id(), now).await?;
    entry.reference_state = "lost".into();
    entry.version += 1;
    support::audit(
        tx,
        ctx,
        work.correlation,
        "PriceBookEntryReferenceLost",
        entry.id,
        entry.version,
    )
    .await?;
    Ok(())
}
/// Answer the op's Idempotency-Key with its receipt or refusal, once, in the completing transaction.
async fn answer_key(
    tx: &(impl DBRunner + Sync),
    op: &entity::Model,
    work: &Work,
) -> Result<(), DoorError> {
    let Some(key) = &op.idempotency_key else {
        return Ok(());
    };
    if work.cancelled() {
        // The claim was released when the create was cancelled; the key may now belong to a
        // fresh attempt, which this op must never answer.
        return Ok(());
    }
    let scope = AccessScope::for_tenant(op.tenant_id);
    let receipt = work
        .receipt
        .as_ref()
        .or(work.refusal.as_ref())
        .ok_or_else(corrupt)?;
    if idem::answer_idempotency_key(
        tx,
        &scope,
        op.tenant_id,
        &work.endpoint(),
        key,
        i32::from(receipt.status),
        support::value(receipt)?,
    )
    .await?
        != idem::IdempotencyAnswer::Recorded
    {
        return Err(corrupt().into());
    }
    Ok(())
}
/// Give up a create before its write (spec §13: a 503 writes nothing). In
/// one transaction the op moves `reserving → cancelling`, is recorded [`CANCELLED`], and its
/// Idempotency-Key claim is released, so a same-key retry runs afresh with a new entry id.
/// The cancellation then releases whatever reservation the unanswered call made.
async fn abandon(
    state: &AuthoringState,
    op: &entity::Model,
    mut work: Work,
    clock: Arc<dyn Clock>,
) -> Result<(), CanonicalError> {
    work.outcome = Some(CANCELLED.into());
    let op = op.clone();
    support::transaction(&state.db.db(), move |tx| {
        let (op, work, clock) = (op.clone(), work.clone(), clock.clone());
        Box::pin(async move {
            advance(tx, &op, &work, Event::ReservationUnknown, clock.as_ref()).await?;
            if let Some(key) = &op.idempotency_key {
                idem::release_idempotency_claim(
                    tx,
                    &AccessScope::for_tenant(op.tenant_id),
                    op.tenant_id,
                    &work.endpoint(),
                    key,
                )
                .await?;
            }
            Ok(())
        })
    })
    .await
}
/// A create still before its write (`reserving`), with or without a receipt.
fn reserving_create(op: &entity::Model, state: OpState) -> bool {
    state == OpState::Reserving && op.kind == OpKind::Create.as_str()
}
/// A create in `reserving` that never learned a reservation id.
fn unreserved_create(op: &entity::Model, state: OpState) -> bool {
    reserving_create(op, state) && op.reservation_id.is_none()
}
/// What the loop does with an op before any registry call.
enum Gate {
    /// The op finished: hand back its stored answer.
    Finished,
    /// A door's create was cancelled before its reservation outcome was known.
    Cancelled,
    /// The ticker found a create it must not reserve for: cancel it.
    Abandon,
    /// Observe the registry and commit the observation.
    Observe,
}
fn gate(caller: Caller, op: &entity::Model, work: &Work, current: OpState) -> Gate {
    if caller == Caller::Door && work.cancelled() {
        // Cancelled before its reservation outcome was known, by this door or, past the
        // in-flight grace, by the ticker: nothing was written and the key is free again.
        Gate::Cancelled
    } else if current == OpState::Done {
        Gate::Finished
    } else if caller == Caller::Ticker && unreserved_create(op, current) {
        Gate::Abandon
    } else {
        Gate::Observe
    }
}
/// Whether a failed transaction's error is the lost compare-and-swap of a racing driver.
fn contended(error: &CanonicalError) -> bool {
    error_code(error).as_deref() == Some("REFERENCE_OP_CONTENDED")
}
/// Drive a durable op until terminal completion or the next scheduled retry.
///
/// A door that gets no definite answer before the write cancels its create and answers 503
/// (nothing written, key released, any receipt released by the cancellation). Any other error
/// that ends a door's drive after its reserve and before its write (a 409 `CONTENDED`, a 500)
/// cancels the create the same way before it is answered. The ticker never makes a first
/// reservation on a user's behalf: it cancels a create still `reserving` without a reservation
/// id the same way.
/// # Errors
/// Returns registry unavailability or a storage failure; the operation remains durable.
pub async fn drive(
    state: &Arc<AuthoringState>,
    ctx: &SecurityContext,
    id: Uuid,
    clock: Arc<dyn Clock>,
    caller: Caller,
) -> Result<Option<Receipt>, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let scope = AccessScope::for_tenant(tenant);
    for _ in 0..32 {
        let op = ops::find(&state.db.conn().map_err(|_| corrupt())?, &scope, tenant, id)
            .await
            .map_err(|e| CanonicalError::from(DoorError::Repo(e)))?
            .ok_or_else(corrupt)?;
        let work = Work::read(&op)?;
        let current = parse_state(&op)?;
        match gate(caller, &op, &work, current) {
            Gate::Cancelled => return Err(support::unavailable()),
            Gate::Finished => return Ok(work.receipt.or(work.refusal)),
            Gate::Abandon => match abandon(state, &op, work, clock.clone()).await {
                Err(error) if !contended(&error) => return Err(error),
                _ => continue,
            },
            Gate::Observe => {}
        }
        step(state, ctx, &op, work, current, caller, clock.clone()).await?;
    }
    Err(unavailable())
}
/// One observation and its commit. `Ok` loops again; `Err` ends the drive with that answer.
async fn step(
    state: &AuthoringState,
    ctx: &SecurityContext,
    op: &entity::Model,
    work: Work,
    current: OpState,
    caller: Caller,
    clock: Arc<dyn Clock>,
) -> Result<(), CanonicalError> {
    warn_past_threshold(op);
    let registry = super::reference_registry::resolve(&state.hub);
    let (event, write, refusal) = match observe(registry, ctx, op).await {
        Ok(observation) => observation,
        Err(error) => return cancel_then(state, caller, op, work, current, clock, error).await,
    };
    if caller == Caller::Door
        && event == Event::RegistryUnavailable
        && reserving_create(op, current)
    {
        return give_up(state, op, work, clock).await;
    }
    let retry = matches!(
        event,
        Event::RegistryUnavailable | Event::ConfirmFailed | Event::ReleaseFailed
    );
    let mut observed = work.clone();
    if let Some(refusal) = refusal {
        observed.refusal = Some(refusal);
    }
    match commit_observation(state, ctx, op, observed, event, write, clock.clone()).await {
        Ok(()) if retry => Err(unavailable()),
        Err(error)
            if refuses_the_write(parse_ref_kind(op)?, &error) && current == OpState::Reserving =>
        {
            cancel(state, op, work, error, clock).await
        }
        Err(error) if !contended(&error) => {
            cancel_then(state, caller, op, work, current, clock, error).await
        }
        _ => Ok(()),
    }
}
/// A door's create that holds a receipt but is not written yet, and whose drive ends with
/// `error` (409 `CONTENDED`, a 500): cancel it first, exactly as [`give_up`] does, so an
/// answered error never becomes an entry later. A lost race means another driver moved the op
/// first, and the loop re-reads it. A failed cancellation is logged and the error is answered
/// as it is: the ticker rule applies to that op unchanged.
async fn cancel_then(
    state: &AuthoringState,
    caller: Caller,
    op: &entity::Model,
    work: Work,
    current: OpState,
    clock: Arc<dyn Clock>,
    error: CanonicalError,
) -> Result<(), CanonicalError> {
    if caller != Caller::Door || !reserving_create(op, current) || op.reservation_id.is_none() {
        return Err(error);
    }
    match abandon(state, op, work, clock).await {
        Ok(()) => Err(error),
        Err(lost) if contended(&lost) => Ok(()),
        Err(failed) => {
            tracing::warn!(op_id=%op.op_id, error=%failed, "pricing create not cancelled before its error answer");
            Err(error)
        }
    }
}
fn warn_past_threshold(op: &entity::Model) {
    if op.attempts >= 10 {
        tracing::warn!(op_id=%op.op_id, attempts=op.attempts, "pricing reference operation retry threshold reached");
    }
}
/// The door got no definite answer before the write (the reserve, or the SKU re-read after a
/// successful reserve): cancel the create and answer 503, so a 503 never becomes an entry. A
/// lost race means another driver moved the op first; the loop re-reads it.
async fn give_up(
    state: &AuthoringState,
    op: &entity::Model,
    work: Work,
    clock: Arc<dyn Clock>,
) -> Result<(), CanonicalError> {
    match abandon(state, op, work, clock).await {
        Ok(()) => Err(support::unavailable()),
        Err(error) if contended(&error) => Ok(()),
        Err(error) => Err(error),
    }
}
/// A local refusal of Tx B that cancels the op (and releases its reservation), per kind.
fn refuses_the_write(kind: RefKind, error: &CanonicalError) -> bool {
    let Some(code) = error_code(error) else {
        return false;
    };
    match kind {
        RefKind::Entry => matches!(
            code.as_str(),
            "ENTRY_KEY_TAKEN"
                | "DIM_NOT_DECLARED"
                | "BOOK_NOT_FOUND"
                | "CHARGE_KIND_SKU_TYPE"
                | "ENTRY_NOT_FOUND"
        ),
        RefKind::PlanItem => plan_item::REFUSES_THE_WRITE.contains(&code.as_str()),
    }
}
/// Commit one observation in one transaction: the entry write it carries, the completion
/// work of a finishing op and the op's own compare-and-swap transition, or none of them.
async fn commit_observation(
    state: &AuthoringState,
    ctx: &SecurityContext,
    op: &entity::Model,
    work: Work,
    event: Event,
    write: Option<Write>,
    clock: Arc<dyn Clock>,
) -> Result<(), CanonicalError> {
    let (op, ctx, outbox) = (op.clone(), ctx.clone(), state.outbox.clone());
    support::transaction(&state.db.db(), move |tx| {
        let (op, work, ctx, clock, event, write, outbox) = (
            op.clone(),
            work.clone(),
            ctx.clone(),
            clock.clone(),
            event.clone(),
            write.clone(),
            outbox.clone(),
        );
        Box::pin(
            async move { commit(tx, &outbox, &ctx, &op, work, event, write, clock.as_ref()).await },
        )
    })
    .await
}
#[allow(
    clippy::too_many_arguments,
    reason = "the observed op, its work and the observation are the transaction's operands"
)]
async fn commit(
    tx: &(impl DBRunner + Sync),
    outbox: &super::events::EventSink,
    ctx: &SecurityContext,
    op: &entity::Model,
    mut work: Work,
    event: Event,
    write: Option<Write>,
    clock: &dyn Clock,
) -> Result<(), DoorError> {
    let scope = AccessScope::for_tenant(op.tenant_id);
    if ops::find(tx, &scope, op.tenant_id, op.op_id)
        .await?
        .as_ref()
        != Some(op)
    {
        return Err(RepoError::Conflict {
            code: "REFERENCE_OP_CONTENDED",
        }
        .into());
    }
    let now = clock.now();
    let (planned, effects) = reference_op::next(
        Op {
            state: parse_state(op)?,
            reservation_id: op.reservation_id,
            refusal: op.last_error.clone(),
        },
        event.clone(),
    )
    .map_err(|_| corrupt())?;
    match write {
        Some(Write::Entry(entry)) => write_entry(tx, &scope, op, entry, now).await?,
        Some(Write::Item(item)) => plan_item::write(tx, &scope, item).await?,
        Some(Write::ItemReceipt(receipt)) => {
            plan_item::write_receipt(tx, &scope, op, receipt, now).await?;
        }
        None => {}
    }
    if planned.state == OpState::Done {
        if op.state == OpState::Written.as_str() {
            work.receipt = Some(match parse_ref_kind(op)? {
                RefKind::Entry => finish_written(tx, ctx, op, &work, &effects, now).await?,
                RefKind::PlanItem => {
                    plan_item::finish_written(tx, ctx, op, &work, &effects, now).await?
                }
            });
        } else if replaces_a_receipt(op) {
            mark_lost(tx, outbox, ctx, &work, op, now).await?;
        }
        answer_key(tx, op, &work).await?;
    }
    advance(tx, op, &work, event, clock).await?;
    Ok(())
}
/// Tx B: a create inserts its entry; a rereserve re-points its entry at the new receipt.
async fn write_entry(
    tx: &(impl DBRunner + Sync),
    scope: &AccessScope,
    op: &entity::Model,
    entry: price_book_entry::Model,
    now: OffsetDateTime,
) -> Result<(), DoorError> {
    if op.kind != OpKind::Rereserve.as_str() {
        price_book_entry_repo::insert(tx, scope, entry).await?;
        return Ok(());
    }
    // A lost entry may be deleted while its re-reservation is in flight: cancel, which
    // releases the new reservation.
    let current = price_book_entry_repo::find(tx, scope, op.tenant_id, op.ref_id)
        .await?
        .ok_or_else(|| support::conflict("ENTRY_NOT_FOUND"))?;
    if current.charge_kind != entry.charge_kind {
        return Err(support::conflict("CHARGE_KIND_SKU_TYPE").into());
    }
    price_book_entry_repo::set_reference(
        tx,
        scope,
        op.tenant_id,
        op.ref_id,
        current.version,
        ReferenceState::ConfirmationPending,
        entry.reservation_id,
        now,
    )
    .await?;
    Ok(())
}
/// Tx C. A confirmed receipt confirms the entry. A receipt released before its confirm keeps
/// the entry `confirmation_pending` and starts a `rereserve_entry` op in this transaction;
/// that op alone decides between confirmed and lost (D-401), so a create is never answered
/// `lost` for a reservation that can still be replaced.
async fn finish_written(
    tx: &(impl DBRunner + Sync),
    ctx: &SecurityContext,
    op: &entity::Model,
    work: &Work,
    effects: &[Effect],
    now: OffsetDateTime,
) -> Result<Receipt, DoorError> {
    let scope = AccessScope::for_tenant(op.tenant_id);
    let mut entry = price_book_entry_repo::find(tx, &scope, op.tenant_id, op.ref_id)
        .await?
        .ok_or_else(corrupt)?;
    if effects.contains(&Effect::Rereserve) {
        // Due at once: no door drives this op, so no in-flight grace applies.
        ops::insert(tx, &scope, rereserve_op(ctx, &entry, now, now)?).await?;
        return Ok(Receipt::entry(entry)?);
    }
    price_book_entry_repo::set_reference(
        tx,
        &scope,
        op.tenant_id,
        op.ref_id,
        entry.version,
        ReferenceState::Confirmed,
        entry.reservation_id,
        now,
    )
    .await?;
    entry.reference_state = ReferenceState::Confirmed.as_str().into();
    entry.version += 1;
    entry.updated_at = now;
    support::audit(
        tx,
        ctx,
        work.correlation,
        "price_book_entry.confirm",
        entry.id,
        entry.version,
    )
    .await?;
    Ok(Receipt::entry(entry)?)
}
/// A `rereserve_entry` op for a live entry, due at `due`.
/// # Errors
/// Fails only if the durable work record cannot be encoded.
pub fn rereserve_op(
    ctx: &SecurityContext,
    entry: &price_book_entry::Model,
    now: OffsetDateTime,
    due: OffsetDateTime,
) -> Result<entity::Model, CanonicalError> {
    let work = Work {
        target: Target::PriceBookEntry {
            book_id: entry.book_id,
            input: PricingPriceBookEntryCreate {
                sku_id: entry.sku_id,
                period: entry.period.clone(),
                dimension_key: entry.dimension_key.clone(),
                invoice_line_override: entry.invoice_line_override.clone(),
            },
        },
        correlation: Uuid::now_v7(),
        refusal: None,
        receipt: None,
        outcome: None,
    };
    let reference = Ref {
        kind: RefKind::Entry,
        id: entry.id,
        sku_id: entry.sku_id,
    };
    let mut op = new_op(ctx, reference, &work, OpKind::Rereserve, None, None, now)?;
    op.tenant_id = entry.tenant_id;
    op.next_attempt_at = due;
    Ok(op)
}
/// Products refusals that mean the SKU admits no reservation: fenced, retiring or retired.
/// Only these make a live entry lost; every other refusal of a re-reservation is retried.
pub const LOSING_REFUSALS: [&str; 3] = ["SKU_FENCED", "SKU_RETIRING", "SKU_RETIRED"];
fn unavailable() -> CanonicalError {
    CanonicalError::service_unavailable()
        .with_detail("REGISTRY_UNAVAILABLE: reference work will be retried")
        .create()
}
/// Stable registry business reason, independent of its resource error type.
#[must_use]
pub fn error_code(error: &CanonicalError) -> Option<String> {
    match error {
        CanonicalError::Aborted { ctx, .. } => Some(ctx.reason.clone()),
        _ => None,
    }
}
async fn cancel(
    state: &AuthoringState,
    op: &entity::Model,
    mut work: Work,
    error: CanonicalError,
    clock: Arc<dyn Clock>,
) -> Result<(), CanonicalError> {
    let code = error_code(&error).unwrap_or_else(|| "WRITE_REFUSED".into());
    // What the door refuses as input stays an input refusal (400, D-403) when Tx B finds it: a
    // key removed from the registry since the door checked it, an item's entry that is no
    // longer of its revision's book or never was of its SKU, or a revision already full.
    let error = match (parse_ref_kind(op)?, code.as_str()) {
        (RefKind::Entry, "DIM_NOT_DECLARED") => {
            support::invalid("dimension_key", "DIM_NOT_DECLARED")
        }
        (RefKind::PlanItem, "ITEM_BOOK_FOREIGN" | "ITEM_ENTRY_SKU_MISMATCH") => {
            support::invalid("price_book_entry_id", &code)
        }
        (RefKind::PlanItem, "REVISION_ITEMS_TOO_MANY") => support::invalid("items", &code),
        _ => error,
    };
    work.refusal = Some(Receipt::error(error).await?);
    let op = op.clone();
    support::transaction(&state.db.db(), move |tx| {
        let (op, work, clock, code) = (op.clone(), work.clone(), clock.clone(), code.clone());
        Box::pin(async move {
            advance(tx, &op, &work, Event::SkuRefused { code }, clock.as_ref()).await?;
            Ok(())
        })
    })
    .await
}
/// What Tx B writes for an observation, per kind.
#[derive(Debug, Clone)]
enum Write {
    /// A created entry, or a rereserved entry carrying its new receipt.
    Entry(price_book_entry::Model),
    /// A created plan item.
    Item(plan_item_entity::Model),
    /// The new receipt of an attached or rereserved plan item.
    ItemReceipt(Uuid),
}
type Observation = (Event, Option<Write>, Option<Receipt>);
/// Products 409 codes a retry can clear: a lost race, not a refusal.
const RETRYABLE_CONFLICTS: [&str; 2] = ["UNIT_CONTENDED", "CONTENDED"];
/// A Products answer that settles the call: a client error, except rate limiting (429) and
/// a contention conflict. Those, 5xx and timeouts are unavailability, never a refusal.
#[must_use]
pub fn definite_refusal(error: &CanonicalError) -> bool {
    let status = error.status_code();
    (400..500).contains(&status)
        && status != 429
        && !error_code(error).is_some_and(|code| RETRYABLE_CONFLICTS.contains(&code.as_str()))
}
/// Products answers 404 for a reservation id it does not hold (a restore from an older backup
/// is the known case). The reconciliation reads the same answer as released.
fn unknown_reservation(error: &CanonicalError) -> bool {
    error.status_code() == 404
}
async fn observe(
    registry: Result<Arc<dyn ReferenceRegistryV1>, CanonicalError>,
    ctx: &SecurityContext,
    op: &entity::Model,
) -> Result<Observation, CanonicalError> {
    let current = parse_state(op)?;
    let unavailable = || {
        (
            match current {
                OpState::Reserving => Event::RegistryUnavailable,
                OpState::Written => Event::ConfirmFailed,
                _ => Event::ReleaseFailed,
            },
            None,
            None,
        )
    };
    let cancelled = Work::read(op)?.cancelled();
    if matches!(current, OpState::Cancelling | OpState::Releasing)
        && op.reservation_id.is_none()
        && !cancelled
    {
        // A definite refusal before any receipt: nothing was reserved.
        return Ok((Event::Released, None, None));
    }
    let Ok(registry) = registry else {
        return Ok(unavailable());
    };
    let tenant = op.tenant_id;
    match current {
        OpState::Reserving if op.reservation_id.is_none() => {
            match registry
                .reserve(
                    ctx,
                    tenant,
                    op.sku_id,
                    products_kind(parse_ref_kind(op)?),
                    op.ref_id,
                )
                .await
            {
                Ok(receipt) => Ok((
                    Event::Reserved {
                        id: receipt.reservation_id,
                    },
                    None,
                    None,
                )),
                Err(error) if definite_refusal(&error) => {
                    let code = error_code(&error).unwrap_or_else(|| "SKU_REFUSED".into());
                    if replaces_a_receipt(op) && !LOSING_REFUSALS.contains(&code.as_str()) {
                        // Only a SKU that admits no reservation loses a live entry or a copied
                        // item: an attach has the rereserve shape (D-413). Any other refusal (the
                        // door caller's own grant, say) is retried, and the ticker finishes it as
                        // the system actor.
                        return Ok(unavailable());
                    }
                    Ok((
                        Event::ReserveRefused { code },
                        None,
                        Some(Receipt::error(error).await?),
                    ))
                }
                Err(_) => Ok(unavailable()),
            }
        }
        OpState::Reserving => observe_sku(registry.as_ref(), ctx, op).await,
        OpState::Written => {
            let event = match registry
                .confirm(ctx, tenant, op.reservation_id.ok_or_else(corrupt)?)
                .await
            {
                Ok(()) => Event::Confirmed,
                // A reservation Products does not know (404, for example after a restore from
                // an older backup) is gone just like a released one: re-reserve the entry.
                Err(error)
                    if error_code(&error).as_deref() == Some("REFERENCE_RELEASED")
                        || unknown_reservation(&error) =>
                {
                    Event::ReleasedOnConfirm
                }
                Err(_) => Event::ConfirmFailed,
            };
            Ok((event, None, None))
        }
        OpState::Cancelling | OpState::Releasing => {
            let result = match op.reservation_id {
                // A reservation Products does not know holds nothing: it counts as released.
                Some(id) => match registry.release(ctx, tenant, id).await {
                    Err(error) if unknown_reservation(&error) => Ok(()),
                    other => other,
                },
                // The reserve outcome was never learned. Reserve is idempotent per logical
                // reference, so it answers the reservation the lost call made (or makes one),
                // and releasing that leaves none. A definite refusal means none can exist:
                // a fence requires zero live references.
                None => match registry
                    .reserve(
                        ctx,
                        tenant,
                        op.sku_id,
                        products_kind(parse_ref_kind(op)?),
                        op.ref_id,
                    )
                    .await
                {
                    Ok(receipt) => registry.release(ctx, tenant, receipt.reservation_id).await,
                    Err(error) if definite_refusal(&error) => Ok(()),
                    Err(error) => Err(error),
                },
            };
            Ok((
                if result.is_ok() {
                    Event::Released
                } else {
                    Event::ReleaseFailed
                },
                None,
                None,
            ))
        }
        OpState::Done => Err(corrupt()),
    }
}

async fn observe_sku(
    registry: &dyn ReferenceRegistryV1,
    ctx: &SecurityContext,
    op: &entity::Model,
) -> Result<Observation, CanonicalError> {
    let tenant = op.tenant_id;
    let sku = match registry.sku_for_write(ctx, tenant, op.sku_id).await {
        Ok(sku) => sku,
        // An op that replaces a receipt (an attach, a rereserve) holds a reference the SKU
        // already admitted: a refusal of the READ says nothing about the SKU (the caller's own
        // grant, say), so it is retried and the ticker finishes it as the system actor. Only a
        // lifecycle answer below refuses it (D-413 "the rereserve shape").
        Err(error) if definite_refusal(&error) && replaces_a_receipt(op) => {
            return Ok((Event::RegistryUnavailable, None, None));
        }
        Err(error) if definite_refusal(&error) => {
            return Ok((
                Event::SkuRefused {
                    code: "SKU_REFUSED".into(),
                },
                None,
                Some(Receipt::error(error).await?),
            ));
        }
        Err(_) => return Ok((Event::RegistryUnavailable, None, None)),
    };
    let kind = parse_ref_kind(op)?;
    // The lifecycle rules of every kind: a draft, retiring or retired SKU takes no reference,
    // and a create takes no deprecated SKU. An attach and a rereserve do: the reference they
    // replace already protected that SKU (D-413). The kind adds its own rule on the SKU's type.
    let refusal = match sku.lifecycle {
        Lifecycle::Draft => Some("SKU_DRAFT"),
        Lifecycle::Deprecated if op.kind == OpKind::Create.as_str() => Some("SKU_DEPRECATED"),
        Lifecycle::Retiring | Lifecycle::Retired => Some("SKU_RETIRING"),
        Lifecycle::Published | Lifecycle::Deprecated => match kind {
            RefKind::Entry => charge_kind_for(sku.r#type).err().map(|e| e.code),
            RefKind::PlanItem => plan_item::type_refusal(sku.r#type),
        },
    };
    if let Some(code) = refusal {
        return Ok((
            Event::SkuRefused { code: code.into() },
            None,
            Some(Receipt::error(sku_refusal_answer(kind, code)).await?),
        ));
    }
    match kind {
        RefKind::Entry => entry_written(op, &sku).await,
        RefKind::PlanItem => plan_item::written(op),
    }
}
/// The answer a create's key records for a SKU its re-read refuses. An item's create answers what
/// the item door answers for the same SKU, a 400 on `sku_id` (D-403): `ITEM_SKU_DEPRECATED` for a
/// deprecated SKU, `ITEM_BUNDLE_SKU` for a bundle; the op's `SkuRefused` code stays the refusal's.
/// Every other refusal is a 409 with its code.
fn sku_refusal_answer(kind: RefKind, code: &'static str) -> CanonicalError {
    match (kind, code) {
        (RefKind::PlanItem, "SKU_DEPRECATED") => support::invalid("sku_id", "ITEM_SKU_DEPRECATED"),
        (RefKind::PlanItem, "ITEM_BUNDLE_SKU") => support::invalid("sku_id", "ITEM_BUNDLE_SKU"),
        _ => support::conflict(code),
    }
}
/// The entry Tx B writes once its SKU admits it: the create's new entry, or the rereserved
/// entry with its new receipt.
async fn entry_written(
    op: &entity::Model,
    sku: &bss_products_sdk::models::Sku,
) -> Result<Observation, CanonicalError> {
    let Target::PriceBookEntry { book_id, input } = Work::read(op)?.target else {
        return Err(corrupt());
    };
    // The door checked the period before reserving; this re-read repeats it against the
    // type the reservation froze. Either way it is an input refusal: 400 (D-403).
    if !crate::domain::price_book_entry::period_valid(sku.r#type, input.period.as_deref()) {
        return Ok((
            Event::SkuRefused {
                code: "ENTRY_PERIOD_INVALID".into(),
            },
            None,
            Some(Receipt::error(support::invalid("period", "ENTRY_PERIOD_INVALID")).await?),
        ));
    }
    let entry = price_book_entry::Model {
        id: op.ref_id,
        tenant_id: op.tenant_id,
        book_id,
        sku_id: op.sku_id,
        charge_kind: charge_kind_for(sku.r#type)
            .map_err(|_| corrupt())?
            .as_str()
            .into(),
        period: input.period,
        dimension_key: input.dimension_key,
        invoice_line_override: input.invoice_line_override,
        reservation_id: op.reservation_id.ok_or_else(corrupt)?,
        reference_state: ReferenceState::ConfirmationPending.as_str().into(),
        version: 1,
        created_at: op.created_at,
        updated_at: op.updated_at,
    };
    Ok((Event::Written, Some(Write::Entry(entry)), None))
}
