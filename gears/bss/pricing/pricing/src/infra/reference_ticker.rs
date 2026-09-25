//! Bounded recovery and periodic receipt reconciliation under the pricing system actor.
//!
//! @cpt-dod:cpt-cf-bss-pricing-dod-confirmation-retry:p1
use super::{
    reference_work::{self, Caller, Clock},
    storage::{
        entity::price_book_entry,
        repo::{price_book_entry_repo, reference_op_repo as ops},
    },
};
use crate::{
    api::rest::authoring::{
        AuthoringState,
        support::{self, DoorError},
    },
    domain::{
        price_book_entry::{ReferenceState, charge_kind_for},
        reference_op::{OpKind, RefKind},
    },
};
use bss_products_sdk::{
    PRICING_SYSTEM_ACTOR,
    models::{Lifecycle, ReferenceState as RegistryState},
};
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
/// One gear-owned ticker. Its cursor bounds confirmed-entry work across ticks.
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
        let batch = price_book_entry_repo::reconcile_batch(
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
        let mut tenants: BTreeMap<Uuid, Vec<price_book_entry::Model>> = BTreeMap::new();
        for entry in batch {
            tenants.entry(entry.tenant_id).or_default().push(entry);
        }
        if tenants.is_empty() {
            self.cursor = None;
            return Ok(());
        }
        let registry = super::reference_registry::resolve(&self.state.hub)?;
        for (tenant, entries) in tenants {
            // One tenant's divergence (an unreachable or disagreeing registry, a storage
            // error) never halts reconciliation for the others; the cursor moves on.
            if let Err(error) = self
                .reconcile_tenant(registry.as_ref(), tenant, entries)
                .await
            {
                tracing::warn!(%tenant, error=%error, "pricing reconciliation skipped a tenant");
            }
        }
        self.cursor = next_cursor;
        Ok(())
    }
    /// One tenant's slice of the batch: re-reserve every confirmed entry whose receipt
    /// Products reports released and every lost entry whose SKU admits a reservation again.
    async fn reconcile_tenant(
        &self,
        registry: &dyn bss_products_sdk::ReferenceRegistryV1,
        tenant: Uuid,
        entries: Vec<price_book_entry::Model>,
    ) -> Result<(), CanonicalError> {
        let ctx = system_actor(tenant)?;
        let (lost, confirmed): (Vec<_>, Vec<_>) = entries
            .into_iter()
            .partition(|p| p.reference_state == ReferenceState::Lost.as_str());
        let mut due = Vec::new();
        if !confirmed.is_empty() {
            let ids: Vec<_> = confirmed.iter().map(|p| p.reservation_id).collect();
            let states = receipt_states(registry, &ctx, tenant, &ids).await?;
            due.extend(confirmed.into_iter().filter(|entry| {
                states.iter().any(|(id, state)| {
                    *id == entry.reservation_id && *state == RegistryState::Released
                })
            }));
        }
        for entry in lost {
            if admits(registry, &ctx, &entry).await {
                due.push(entry);
            }
        }
        for entry in due {
            self.rereserve(&ctx, entry).await?;
        }
        Ok(())
    }
    /// Mint one re-reservation and drive it now; a deferred op stays durable and due.
    async fn rereserve(
        &self,
        ctx: &SecurityContext,
        entry: price_book_entry::Model,
    ) -> Result<(), CanonicalError> {
        let Some(id) = self.begin_rereserve(ctx, entry).await? else {
            return Ok(());
        };
        if let Err(error) =
            reference_work::drive(&self.state, ctx, id, self.clock.clone(), Caller::Ticker).await
        {
            tracing::warn!(op_id=%id, error=%error, "pricing re-reservation deferred");
        }
        Ok(())
    }
    /// Start one re-reservation. A confirmed entry is claimed by moving it to
    /// `confirmation_pending` at its observed version; a lost entry stays lost (it admits no
    /// prices) until the new reservation is written, and one open op per entry is the guard.
    async fn begin_rereserve(
        &self,
        ctx: &SecurityContext,
        observed: price_book_entry::Model,
    ) -> Result<Option<Uuid>, CanonicalError> {
        let (ctx, now) = (ctx.clone(), self.clock.now());
        support::transaction(&self.state.db.db(), move |tx| {
            let (ctx, observed) = (ctx.clone(), observed.clone());
            Box::pin(async move {
                let scope = AccessScope::for_tenant(ctx.subject_tenant_id());
                let current =
                    price_book_entry_repo::find(tx, &scope, observed.tenant_id, observed.id)
                        .await?;
                if current.as_ref() != Some(&observed) {
                    return Ok(None);
                }
                let op = reference_work::rereserve_op(
                    &ctx,
                    &observed,
                    now,
                    now + reference_work::IN_FLIGHT_GRACE,
                )?;
                let id = op.op_id;
                if observed.reference_state == ReferenceState::Lost.as_str() {
                    if ops::open_for_ref(
                        tx,
                        &scope,
                        observed.tenant_id,
                        RefKind::Entry,
                        observed.id,
                        OpKind::Rereserve,
                    )
                    .await?
                    {
                        return Ok(None);
                    }
                } else {
                    // Claim this entry for reconciliation atomically; another ticker cannot
                    // mint competing recovery work and deletion cannot strand a new receipt.
                    price_book_entry_repo::set_reference(
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
                }
                ops::insert(tx, &scope, op).await?;
                Ok(Some(id))
            })
        })
        .await
    }
}
/// Products' view of confirmed receipts. A batch answered 404 names a reservation Products
/// does not know (for example after a restore): each id is then asked alone, and an unknown
/// one counts as released, so its entry is re-reserved like any other released receipt.
async fn receipt_states(
    registry: &dyn bss_products_sdk::ReferenceRegistryV1,
    ctx: &SecurityContext,
    tenant: Uuid,
    ids: &[Uuid],
) -> Result<Vec<(Uuid, RegistryState)>, CanonicalError> {
    match registry.states(ctx, tenant, ids).await {
        Err(error) if error.status_code() == 404 => {}
        other => return other,
    }
    let mut states = Vec::with_capacity(ids.len());
    for id in ids {
        match registry.states(ctx, tenant, &[*id]).await {
            Ok(found) => states.extend(found),
            Err(error) if error.status_code() == 404 => states.push((*id, RegistryState::Released)),
            Err(error) => return Err(error),
        }
    }
    Ok(states)
}
/// Whether a lost entry's SKU admits its reservation again: published or deprecated, not
/// fenced, and of the entry's charge kind (a changed type cannot be healed by a reservation).
async fn admits(
    registry: &dyn bss_products_sdk::ReferenceRegistryV1,
    ctx: &SecurityContext,
    entry: &price_book_entry::Model,
) -> bool {
    match registry
        .sku_for_write(ctx, entry.tenant_id, entry.sku_id)
        .await
    {
        Ok(sku) => {
            matches!(sku.lifecycle, Lifecycle::Published | Lifecycle::Deprecated)
                && !sku.type_change_pending
                && charge_kind_for(sku.r#type).is_ok_and(|kind| kind.as_str() == entry.charge_kind)
        }
        Err(error) => {
            tracing::warn!(price_book_entry_id=%entry.id, error=%error, "pricing lost-entry check deferred");
            false
        }
    }
}
