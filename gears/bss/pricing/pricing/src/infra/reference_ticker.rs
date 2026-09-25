//! Bounded recovery and periodic receipt reconciliation under the pricing system actor.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-confirmation-retry:p1
use super::{
    reference_work::{self, Caller, Clock, Work},
    storage::{
        entity::price,
        repo::{price_repo, reference_op_repo as ops},
    },
};
use crate::{
    api::rest::authoring::{
        AuthoringState,
        dto::PricingPriceCreate,
        support::{self, DoorError},
    },
    domain::price::{OpKind, ReferenceState},
};
use bss_products_sdk::{PRICING_SYSTEM_ACTOR, models::ReferenceState as RegistryState};
use std::{collections::BTreeMap, sync::Arc};
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;
/// A fixed identity scoped anew for every operation's tenant.
/// # Errors
/// Propagates invalid security context construction.
pub fn system_actor(tenant: Uuid) -> Result<SecurityContext, CanonicalError> {
    SecurityContext::builder()
        .subject_id(PRICING_SYSTEM_ACTOR)
        .subject_tenant_id(tenant)
        .subject_type("bss-pricing.system")
        .build()
        .map_err(|_| CanonicalError::internal("pricing recovery identity failed").create())
}
/// One gear-owned ticker. Its cursor bounds confirmed-price work across ticks.
pub struct Ticker {
    state: Arc<AuthoringState>,
    clock: Arc<dyn Clock>,
    limit: u64,
    reconcile_every: u64,
    ticks: u64,
    cursor: Option<Uuid>,
}
impl Ticker {
    /// Construct a bounded ticker; tests inject a clock with no jitter.
    #[must_use]
    pub fn new(
        state: Arc<AuthoringState>,
        clock: Arc<dyn Clock>,
        limit: u64,
        reconcile_every: u64,
    ) -> Self {
        Self {
            state,
            clock,
            limit: limit.clamp(1, 1000),
            reconcile_every: reconcile_every.max(1),
            ticks: 0,
            cursor: None,
        }
    }
    /// Resume a bounded due batch, then reconcile a bounded confirmed batch every N ticks.
    /// # Errors
    /// Scan failures are surfaced; per-op failures remain due at their scheduled retry.
    pub async fn tick(&mut self) -> Result<(), CanonicalError> {
        // Only this trusted scheduler scans all tenants. Every mutation and Products call
        // below has a tenant-only scope and the fixed pricing system actor.
        let due = ops::due(
            &self
                .state
                .db
                .conn()
                .map_err(|e| CanonicalError::from(DoorError::from(e)))?,
            &AccessScope::allow_all(),
            self.clock.now(),
            self.limit,
        )
        .await
        .map_err(|e| CanonicalError::from(DoorError::from(e)))?;
        for op in due {
            let ctx = system_actor(op.tenant_id)?;
            if let Err(error) = reference_work::drive(
                &self.state,
                &ctx,
                op.op_id,
                self.clock.clone(),
                Caller::Ticker,
            )
            .await
            {
                tracing::warn!(op_id=%op.op_id, attempts=op.attempts, error=%error, "pricing reference recovery deferred");
            }
        }
        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks.is_multiple_of(self.reconcile_every) {
            self.reconcile().await?;
        }
        Ok(())
    }
    async fn reconcile(&mut self) -> Result<(), CanonicalError> {
        let batch = price_repo::confirmed_batch(
            &self
                .state
                .db
                .conn()
                .map_err(|e| CanonicalError::from(DoorError::from(e)))?,
            &AccessScope::allow_all(),
            self.cursor,
            self.limit,
        )
        .await
        .map_err(|e| CanonicalError::from(DoorError::from(e)))?;
        let next_cursor = if u64::try_from(batch.len()).unwrap_or(u64::MAX) == self.limit {
            batch.last().map(|p| p.id)
        } else {
            None
        };
        let mut tenants: BTreeMap<Uuid, Vec<price::Model>> = BTreeMap::new();
        for price in batch {
            tenants.entry(price.tenant_id).or_default().push(price);
        }
        if tenants.is_empty() {
            self.cursor = None;
            return Ok(());
        }
        let registry = super::reference_registry::resolve(&self.state.hub)?;
        for (tenant, prices) in tenants {
            let ctx = system_actor(tenant)?;
            let ids: Vec<_> = prices.iter().map(|p| p.reservation_id).collect();
            let states = registry.states(&ctx, tenant, &ids).await?;
            for price in prices {
                if states.iter().any(|(id, state)| {
                    *id == price.reservation_id && *state == RegistryState::Released
                }) {
                    let id = self.begin_rereserve(&ctx, price).await?;
                    if let Some(id) = id
                        && let Err(error) = reference_work::drive(
                            &self.state,
                            &ctx,
                            id,
                            self.clock.clone(),
                            Caller::Ticker,
                        )
                        .await
                    {
                        // The rereserve op stays durable and due; the next tick resumes it.
                        tracing::warn!(op_id=%id, error=%error, "pricing re-reservation deferred");
                    }
                }
            }
        }
        self.cursor = next_cursor;
        Ok(())
    }
    async fn begin_rereserve(
        &self,
        ctx: &SecurityContext,
        observed: price::Model,
    ) -> Result<Option<Uuid>, CanonicalError> {
        let (ctx, now) = (ctx.clone(), self.clock.now());
        support::transaction(&self.state.db.db(), move |tx| {
            let (ctx, observed) = (ctx.clone(), observed.clone());
            Box::pin(async move {
                let scope = AccessScope::for_tenant(ctx.subject_tenant_id());
                let current = price_repo::find(tx, &scope, observed.tenant_id, observed.id).await?;
                if current.as_ref() != Some(&observed) {
                    return Ok(None);
                }
                let work = Work {
                    book_id: observed.book_id,
                    input: PricingPriceCreate {
                        sku_id: observed.sku_id,
                        period: observed.period.clone(),
                        dimension_key: observed.dimension_key.clone(),
                        invoice_line_override: observed.invoice_line_override.clone(),
                    },
                    correlation: Uuid::now_v7(),
                    refusal: None,
                    receipt: None,
                    outcome: None,
                };
                let op = reference_work::new_op(
                    &ctx,
                    observed.id,
                    &work,
                    OpKind::Rereserve,
                    None,
                    None,
                    now,
                )?;
                let id = op.op_id;
                // Claim this price for reconciliation atomically; another ticker cannot mint
                // competing recovery work and deletion cannot strand a new remote receipt.
                price_repo::set_reference(
                    tx,
                    &scope,
                    observed.tenant_id,
                    observed.id,
                    observed.version,
                    ReferenceState::ConfirmationPending,
                    observed.reservation_id,
                    now,
                )
                .await?;
                ops::insert(tx, &scope, op).await?;
                Ok(Some(id))
            })
        })
        .await
    }
}
