//! Shared durable executor: local transitions are conditional, remote calls are idempotent.
use crate::{
    api::rest::authoring::{
        AuthoringState,
        dto::{PricingPriceCreate, PricingPriceDto},
        support::{self, DoorError},
    },
    domain::{
        price::{OpKind, OpState, ReferenceState, charge_kind_for},
        reference_op::{self, Effect, Event, Op},
    },
    infra::storage::{
        RepoError,
        entity::{price, reference_op as entity},
        repo::{idempotency_repo as idem, price_repo, reference_op_repo as ops},
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
    pub book_id: Uuid,
    pub input: PricingPriceCreate,
    pub correlation: Uuid,
    pub refusal: Option<Receipt>,
    pub receipt: Option<Receipt>,
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
    pub fn price(model: price::Model) -> Result<Self, CanonicalError> {
        let etag = Some(format!("\"{}\"", model.version));
        Ok(Self {
            status: 201,
            body: serde_json::to_string(&PricingPriceDto::from(model)).map_err(|_| corrupt())?,
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
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("/bss-pricing/v1/price-books/{}/prices", self.book_id)
    }
}
/// Start a durable record inside Tx A or the price-delete transaction.
pub fn new_op(
    ctx: &SecurityContext,
    price_id: Uuid,
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
        price_id,
        sku_id: work.input.sku_id,
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
/// A re-reservation that ended without a live receipt leaves its price lost: the
/// state, the durable `PriceReferenceLost` event and the audit record commit together.
async fn mark_rereserve_lost(
    tx: &(impl DBRunner + Sync),
    outbox: &toolkit_db::outbox::Outbox,
    ctx: &SecurityContext,
    work: &Work,
    op: &entity::Model,
    now: OffsetDateTime,
) -> Result<(), DoorError> {
    let scope = AccessScope::for_tenant(op.tenant_id);
    let Some(mut price) = price_repo::find(tx, &scope, op.tenant_id, op.price_id).await? else {
        return Ok(());
    };
    price_repo::set_reference(
        tx,
        &scope,
        op.tenant_id,
        op.price_id,
        price.version,
        ReferenceState::Lost,
        price.reservation_id,
        now,
    )
    .await?;
    super::reference_events::lost(outbox, tx, &price, ctx.subject_id(), now).await?;
    price.reference_state = "lost".into();
    price.version += 1;
    support::audit(
        tx,
        ctx,
        work.correlation,
        "PriceReferenceLost",
        price.id,
        price.version,
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
/// Drive a durable op until terminal completion or the next scheduled retry.
/// # Errors
/// Returns registry unavailability or a storage failure; the operation remains durable.
pub async fn drive(
    state: &Arc<AuthoringState>,
    ctx: &SecurityContext,
    id: Uuid,
    clock: Arc<dyn Clock>,
) -> Result<Option<Receipt>, CanonicalError> {
    let tenant = ctx.subject_tenant_id();
    let scope = AccessScope::for_tenant(tenant);
    for _ in 0..32 {
        let op = ops::find(&state.db.conn().map_err(|_| corrupt())?, &scope, tenant, id)
            .await
            .map_err(|e| CanonicalError::from(DoorError::Repo(e)))?
            .ok_or_else(corrupt)?;
        let work = Work::read(&op)?;
        if parse_state(&op)? == OpState::Done {
            return Ok(work.receipt.or(work.refusal));
        }
        if op.attempts >= 10 {
            tracing::warn!(op_id=%op.op_id, attempts=op.attempts, "pricing reference operation retry threshold reached");
        }
        let registry = super::reference_registry::resolve(&state.hub);
        let observation = observe(registry, ctx, &op).await;
        let (event, price, refusal) = observation?;
        let retry = matches!(
            event,
            Event::RegistryUnavailable | Event::ConfirmFailed | Event::ReleaseFailed
        );
        let (op2, mut work2, ctx2, clock2) = (op.clone(), work.clone(), ctx.clone(), clock.clone());
        if let Some(refusal) = refusal {
            work2.refusal = Some(refusal);
        }
        let outbox = state.outbox.clone();
        let result = support::transaction(&state.db.db(), move |tx| {
            let (op, mut work, ctx, clock, event, price) = (
                op2.clone(),
                work2.clone(),
                ctx2.clone(),
                clock2.clone(),
                event.clone(),
                price.clone(),
            );
            let outbox = outbox.clone();
            Box::pin(async move {
                let scope = AccessScope::for_tenant(op.tenant_id);
                if ops::find(tx, &scope, op.tenant_id, op.op_id)
                    .await?
                    .as_ref()
                    != Some(&op)
                {
                    return Err(RepoError::Conflict {
                        code: "REFERENCE_OP_CONTENDED",
                    }
                    .into());
                }
                let now = clock.now();
                let (planned, effects) = reference_op::next(
                    Op {
                        state: parse_state(&op)?,
                        reservation_id: op.reservation_id,
                        refusal: op.last_error.clone(),
                    },
                    event.clone(),
                )
                .map_err(|_| corrupt())?;
                let next = planned.state;
                let scope = AccessScope::for_tenant(op.tenant_id);
                if let Some(price) = price {
                    if op.kind == OpKind::Rereserve.as_str() {
                        let current = price_repo::find(tx, &scope, op.tenant_id, op.price_id)
                            .await?
                            .ok_or_else(corrupt)?;
                        if current.charge_kind != price.charge_kind {
                            return Err(support::conflict("CHARGE_KIND_SKU_TYPE").into());
                        }
                        price_repo::set_reference(
                            tx,
                            &scope,
                            op.tenant_id,
                            op.price_id,
                            current.version,
                            ReferenceState::ConfirmationPending,
                            price.reservation_id,
                            now,
                        )
                        .await?;
                    } else {
                        price_repo::insert(tx, &scope, price).await?;
                    }
                }
                if next == OpState::Done {
                    if op.state == OpState::Written.as_str() {
                        let mut price = price_repo::find(tx, &scope, op.tenant_id, op.price_id)
                            .await?
                            .ok_or_else(corrupt)?;
                        let reference = if effects.contains(&Effect::MarkLost) {
                            ReferenceState::Lost
                        } else {
                            ReferenceState::Confirmed
                        };
                        price_repo::set_reference(
                            tx,
                            &scope,
                            op.tenant_id,
                            op.price_id,
                            price.version,
                            reference,
                            price.reservation_id,
                            now,
                        )
                        .await?;
                        price.reference_state = reference.as_str().into();
                        price.version += 1;
                        price.updated_at = now;
                        support::audit(
                            tx,
                            &ctx,
                            work.correlation,
                            if reference == ReferenceState::Lost {
                                "PriceReferenceLost"
                            } else {
                                "price.confirm"
                            },
                            price.id,
                            price.version,
                        )
                        .await?;
                        if reference == ReferenceState::Lost {
                            super::reference_events::lost(
                                &outbox,
                                tx,
                                &price,
                                ctx.subject_id(),
                                now,
                            )
                            .await?;
                        }
                        work.receipt = Some(Receipt::price(price)?);
                    } else if op.kind == OpKind::Rereserve.as_str() {
                        mark_rereserve_lost(tx, &outbox, &ctx, &work, &op, now).await?;
                    }
                    answer_key(tx, &op, &work).await?;
                }
                advance(tx, &op, &work, event, clock.as_ref()).await?;
                Ok(())
            })
        })
        .await;
        if let Err(error) = result {
            let code = error_code(&error);
            if code.as_deref() == Some("REFERENCE_OP_CONTENDED") {
                continue;
            }
            if matches!(
                code.as_deref(),
                Some(
                    "PRICE_KEY_TAKEN"
                        | "DIM_NOT_DECLARED"
                        | "BOOK_NOT_FOUND"
                        | "CHARGE_KIND_SKU_TYPE"
                )
            ) && op.state == OpState::Reserving.as_str()
            {
                cancel(state, &op, work, error, clock.clone()).await?;
                continue;
            }
            return Err(error);
        }
        if retry {
            return Err(unavailable());
        }
    }
    Err(unavailable())
}
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
    let code = error_code(&error).unwrap_or_else(|| "PRICE_WRITE_REFUSED".into());
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
type Observation = (Event, Option<price::Model>, Option<Receipt>);
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
    if matches!(current, OpState::Cancelling | OpState::Releasing) && op.reservation_id.is_none() {
        return Ok((Event::Released, None, None));
    }
    let Ok(registry) = registry else {
        return Ok(unavailable());
    };
    let tenant = op.tenant_id;
    match current {
        OpState::Reserving if op.reservation_id.is_none() => {
            match registry
                .reserve(ctx, tenant, op.sku_id, ReferenceKind::Price, op.price_id)
                .await
            {
                Ok(receipt) => Ok((
                    Event::Reserved {
                        id: receipt.reservation_id,
                    },
                    None,
                    None,
                )),
                Err(error) if (400..500).contains(&error.status_code()) => {
                    let code = error_code(&error).unwrap_or_else(|| "SKU_REFUSED".into());
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
                Err(error) if error_code(&error).as_deref() == Some("REFERENCE_RELEASED") => {
                    Event::ReleasedOnConfirm
                }
                Err(_) => Event::ConfirmFailed,
            };
            Ok((event, None, None))
        }
        OpState::Cancelling | OpState::Releasing => {
            let result = if let Some(id) = op.reservation_id {
                registry.release(ctx, tenant, id).await
            } else {
                Ok(())
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
        Err(error) if (400..500).contains(&error.status_code()) => {
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
    let kind = charge_kind_for(sku.r#type);
    let refusal = match sku.lifecycle {
        Lifecycle::Draft => Some("SKU_DRAFT"),
        Lifecycle::Deprecated if op.kind == OpKind::Create.as_str() => Some("SKU_DEPRECATED"),
        Lifecycle::Retiring | Lifecycle::Retired => Some("SKU_RETIRING"),
        _ => kind.as_ref().err().map(|e| e.code),
    };
    if let Some(code) = refusal {
        return Ok((
            Event::SkuRefused { code: code.into() },
            None,
            Some(Receipt::error(support::conflict(code)).await?),
        ));
    }
    let work = Work::read(op)?;
    let period_valid = if sku.r#type == bss_products_sdk::models::SkuType::Recurring {
        matches!(work.input.period.as_deref(), Some("month" | "year"))
    } else {
        work.input.period.is_none()
    };
    if !period_valid {
        return Ok((
            Event::SkuRefused {
                code: "PRICE_PERIOD_INVALID".into(),
            },
            None,
            Some(Receipt::error(support::conflict("PRICE_PERIOD_INVALID")).await?),
        ));
    }
    let price = price::Model {
        id: op.price_id,
        tenant_id: tenant,
        book_id: work.book_id,
        sku_id: op.sku_id,
        charge_kind: kind.map_err(|_| corrupt())?.as_str().into(),
        period: work.input.period,
        dimension_key: work.input.dimension_key,
        invoice_line_override: work.input.invoice_line_override,
        reservation_id: op.reservation_id.ok_or_else(corrupt)?,
        reference_state: ReferenceState::ConfirmationPending.as_str().into(),
        version: 1,
        created_at: op.created_at,
        updated_at: op.updated_at,
    };
    Ok((Event::Written, Some(price), None))
}
