//! Trusted in-process transport. Ownership is selected only by Products wiring.
use crate::api::rest::{self, ApiState, TxError, governance as g, references as service};
use crate::authz::actions;
use crate::domain::references::RefKind;
use crate::infra::storage::{RepoError, repo};
use async_trait::async_trait;
use authz_resolver_sdk::PolicyEnforcer;
use bss_products_sdk::{
    PRICING_SYSTEM_ACTOR, ReferenceKind, ReferenceRegistryV1, ReferenceState, ReservationReceipt,
    Sku, SkuVersion,
};
use std::sync::Arc;
use toolkit_canonical_errors::CanonicalError;
use toolkit_db::secure::AccessScope;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// Bound owner awaiting the provider's runtime dependencies.
pub struct ReferenceRegistryOwner(String);
impl ReferenceRegistryOwner {
    /// Attach the same dependencies as the REST reference handlers.
    #[must_use]
    pub fn with_runtime(
        self,
        state: Arc<ApiState>,
        enforcer: Arc<PolicyEnforcer>,
    ) -> LocalReferenceRegistry {
        LocalReferenceRegistry {
            owner: self.0,
            state,
            enforcer,
        }
    }
}
/// Same-binary trust boundary; this constructor is called by Products, not request input.
pub struct LocalReferenceRegistry {
    owner: String,
    state: Arc<ApiState>,
    enforcer: Arc<PolicyEnforcer>,
}
impl LocalReferenceRegistry {
    /// Bind a consumer owner before attaching the Products runtime.
    #[must_use]
    pub fn for_owner(owner: &str) -> ReferenceRegistryOwner {
        ReferenceRegistryOwner(owner.into())
    }
    async fn scope(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        action: &str,
    ) -> Result<AccessScope, CanonicalError> {
        // A missing subject type is a human principal (the static-authn default, third-party
        // OIDC tokens): it goes through the PDP below like any other. Only the system branch
        // interprets the subject type, and it checks it itself.
        if tenant != ctx.subject_tenant_id() || tenant.is_nil() || ctx.subject_id().is_nil() {
            return Err(service::forbidden().into());
        }
        if ctx.subject_type().is_some_and(|s| s.ends_with(".system")) {
            if self.owner != "pricing"
                || ctx.subject_type() != Some("bss-pricing.system")
                || ctx.subject_id() != PRICING_SYSTEM_ACTOR
            {
                return Err(service::forbidden().into());
            }
            return Ok(AccessScope::for_tenant(tenant));
        }
        g::scope(&self.enforcer, ctx, action, false).await
    }
    async fn change(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
        release: bool,
    ) -> Result<(), CanonicalError> {
        let scope = self.scope(ctx, tenant, actions::REFERENCE).await?;
        let ctx = ctx.clone();
        let owner = self.owner.clone();
        let ttl = self.state.fence_ttl_minutes;
        self.state
            .db
            .db()
            .transaction_with_retry(
                rest::category_tx_config(&self.state),
                rest::contention_db_err,
                move |tx| {
                    let (scope, ctx, owner) = (scope.clone(), ctx.clone(), owner.clone());
                    Box::pin(async move {
                        if release {
                            service::release_tx(tx, &scope, &ctx, &owner, id, ttl).await?;
                        } else {
                            service::confirm_tx(tx, &scope, &ctx, &owner, id, ttl).await?;
                        }
                        Ok(())
                    })
                },
            )
            .await
            .map_err(rest::tx_to_canonical)
    }
}
fn state(token: &str) -> Result<ReferenceState, CanonicalError> {
    match token {
        "reserved" => Ok(ReferenceState::Reserved),
        "confirmed" => Ok(ReferenceState::Confirmed),
        "released" => Ok(ReferenceState::Released),
        _ => Err(CanonicalError::internal("invalid stored reference state").create()),
    }
}
#[async_trait]
impl ReferenceRegistryV1 for LocalReferenceRegistry {
    async fn reserve(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        sku_id: Uuid,
        kind: ReferenceKind,
        ref_id: Uuid,
    ) -> Result<ReservationReceipt, CanonicalError> {
        let scope = self.scope(ctx, tenant, actions::REFERENCE).await?;
        let kind = match kind {
            ReferenceKind::Price => RefKind::Price,
            ReferenceKind::PlanItem => RefKind::PlanItem,
            ReferenceKind::SoldAs => RefKind::SoldAs,
        };
        for attempt in 0..2 {
            let (scope, ctx, owner, ttl) = (
                scope.clone(),
                ctx.clone(),
                self.owner.clone(),
                self.state.fence_ttl_minutes,
            );
            let result = self
                .state
                .db
                .db()
                .transaction_with_retry(
                    rest::category_tx_config(&self.state),
                    rest::contention_db_err,
                    move |tx| {
                        let (scope, ctx, owner) = (scope.clone(), ctx.clone(), owner.clone());
                        Box::pin(async move {
                            service::reserve_tx(tx, &scope, &ctx, &owner, sku_id, kind, ref_id, ttl)
                                .await
                        })
                    },
                )
                .await;
            match result {
                Err(TxError::Repo(RepoError::Db(code)))
                    if code == "REFERENCE_EXISTS" && attempt == 0 => {}
                other => {
                    let (row, _) = other.map_err(rest::tx_to_canonical)?;
                    return Ok(ReservationReceipt {
                        reservation_id: row.id,
                        state: state(&row.state)?,
                    });
                }
            }
        }
        Err(rest::tx_to_canonical(g::conflict(
            "REFERENCE_EXISTS",
            "logical reference changed; retry",
        )))
    }
    async fn confirm(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<(), CanonicalError> {
        self.change(ctx, tenant, id, false).await
    }
    async fn release(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<(), CanonicalError> {
        self.change(ctx, tenant, id, true).await
    }
    async fn states(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        ids: &[Uuid],
    ) -> Result<Vec<(Uuid, ReferenceState)>, CanonicalError> {
        let scope = self.scope(ctx, tenant, actions::REFERENCE).await?;
        let db = self.state.db.db();
        let conn = db
            .conn()
            .map_err(|e| CanonicalError::internal(e.to_string()).create())?;
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            let row = service::owned(&conn, &scope, tenant, &self.owner, *id)
                .await
                .map_err(rest::tx_to_canonical)?;
            result.push((*id, state(&row.state)?));
        }
        Ok(result)
    }
    async fn sku_for_write(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
    ) -> Result<Sku, CanonicalError> {
        let scope = self.scope(ctx, tenant, actions::READ).await?;
        let db = self.state.db.db();
        let conn = db
            .conn()
            .map_err(|e| CanonicalError::internal(e.to_string()).create())?;
        g::find(&conn, &scope, tenant, id)
            .await
            .map_err(rest::tx_to_canonical)
    }
    async fn sku_version_as_of(
        &self,
        ctx: &SecurityContext,
        tenant: Uuid,
        id: Uuid,
        date: time::Date,
    ) -> Result<Option<SkuVersion>, CanonicalError> {
        let scope = self.scope(ctx, tenant, actions::READ).await?;
        let db = self.state.db.db();
        let conn = db
            .conn()
            .map_err(|e| CanonicalError::internal(e.to_string()).create())?;
        g::find(&conn, &scope, tenant, id)
            .await
            .map_err(rest::tx_to_canonical)?;
        repo::version_as_of(&conn, &scope, tenant, id, date)
            .await
            .map_err(|e| rest::repo_error_to_canonical(&e))
    }
}
