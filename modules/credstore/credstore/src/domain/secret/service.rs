use std::sync::Arc;
use std::time::{Duration, Instant};

use authz_resolver_sdk::PolicyEnforcer;
use credstore_sdk::{
    CredStoreError, CredStorePluginClientV1, GetSecretResponse, OwnerId, SecretRef, SecretValue,
    SharingMode, TenantId,
};
use modkit_macros::domain_model;
use modkit_security::{AccessScope, SecurityContext};
use tokio::time::sleep;
use uuid::Uuid;

use crate::domain::authz::{actions, scope_for};
use crate::domain::error::DomainError;
use crate::domain::ports::metrics::{CredStoreMetricsPort, Dep, DepOp, Outcome, ReadOutcome};
use crate::domain::ports::plugin::PluginSelector;
use crate::domain::resolver::TenantDirectory;
use crate::domain::secret::model::{NewSecret, WritePrecondition};
use crate::domain::secret::repo::SecretRepo;

/// Maps a [`CredStoreError`] from the plugin layer to a [`DomainError`].
#[must_use]
pub fn map_plugin_err(e: CredStoreError) -> DomainError {
    match e {
        CredStoreError::NotFound => DomainError::NotFound,
        CredStoreError::AccessDenied => DomainError::AccessDenied { cause: None },
        CredStoreError::ServiceUnavailable {
            detail,
            retry_after,
        } => DomainError::ServiceUnavailable {
            detail,
            retry_after,
            cause: None,
        },
        // Operator misconfiguration, not a transient outage: keep a stable,
        // distinguishable detail and no `retry_after` so callers don't retry.
        CredStoreError::NoPluginAvailable => DomainError::ServiceUnavailable {
            detail: "no storage plugin registered".into(),
            retry_after: None,
            cause: None,
        },
        CredStoreError::Conflict => DomainError::Conflict,
        CredStoreError::InvalidSecretRef { reason } => DomainError::Internal {
            diagnostic: format!("plugin returned InvalidSecretRef: {reason}"),
            cause: None,
        },
        // A plugin should never return UnsupportedTransition; treat as contract violation.
        CredStoreError::UnsupportedTransition { detail } => DomainError::Internal {
            diagnostic: format!("plugin returned UnsupportedTransition: {detail}"),
            cause: None,
        },
        CredStoreError::Internal(s) => DomainError::Internal {
            diagnostic: s,
            cause: None,
        },
    }
}

/// Bounded retries when a PUT loses the create race (the winner holds a
/// provisioning row invisible to `find_for_write`); the winner marks active
/// within ms, so a small bounded budget resolves the race.
const CREATE_RACE_MAX_RETRIES: u32 = 3;

/// Small backoff between create-race retries; the winner marks active within ms.
const CREATE_RACE_RETRY_BACKOFF_MS: u64 = 25;

/// `CredStore` domain service — get / put / delete with walk-up, saga, and `AuthZ`.
#[domain_model]
pub struct Service {
    repo: Arc<dyn SecretRepo>,
    dir: Arc<dyn TenantDirectory>,
    enforcer: PolicyEnforcer,
    plugins: Arc<dyn PluginSelector>,
    metrics: Arc<dyn CredStoreMetricsPort>,
    reaper_tick_secs: u64,
    provisioning_timeout_secs: u64,
}

/// Arguments for [`Service::overwrite_existing`] — bundled so the backend-first
/// overwrite doesn't carry a long positional argument list.
#[domain_model]
struct OverwriteArgs<'a> {
    ctx: &'a SecurityContext,
    scope: &'a AccessScope,
    plugin: &'a Arc<dyn CredStorePluginClientV1>,
    key: &'a SecretRef,
    value: SecretValue,
    sharing: SharingMode,
    existing_id: Uuid,
    expected_version: Option<i64>,
}

impl Service {
    /// Creates a new [`Service`].
    #[must_use]
    pub fn new(
        repo: Arc<dyn SecretRepo>,
        dir: Arc<dyn TenantDirectory>,
        enforcer: PolicyEnforcer,
        plugins: Arc<dyn PluginSelector>,
        metrics: Arc<dyn CredStoreMetricsPort>,
        reaper_tick_secs: u64,
        provisioning_timeout_secs: u64,
    ) -> Self {
        Self {
            repo,
            dir,
            enforcer,
            plugins,
            metrics,
            reaper_tick_secs,
            provisioning_timeout_secs,
        }
    }

    /// Returns the configured reaper tick interval in seconds.
    #[must_use]
    pub fn reaper_tick_secs(&self) -> u64 {
        self.reaper_tick_secs
    }

    /// Evaluate the PDP scope for `action`, recording it as a timed dependency.
    ///
    /// The PDP call gates every get/put/delete and drives 503s, so it is timed
    /// like the plugin and tenant-resolver dependencies.
    async fn scope_for_timed(
        &self,
        ctx: &SecurityContext,
        action: &str,
    ) -> Result<AccessScope, DomainError> {
        let t0 = Instant::now();
        let result = scope_for(&self.enforcer, ctx, action).await;
        let outcome = if result.is_ok() {
            Outcome::Success
        } else {
            Outcome::Error
        };
        self.metrics.dependency(
            Dep::Pdp,
            DepOp::Evaluate,
            outcome,
            t0.elapsed().as_secs_f64(),
        );
        result
    }

    /// Retrieve a secret, walking up the tenant hierarchy.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::AccessDenied`] if the caller is out of scope.
    /// Returns [`DomainError::NotFound`] if the plugin has no value for a resolved row.
    pub async fn get(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
    ) -> Result<Option<GetSecretResponse>, DomainError> {
        let scope = self.scope_for_timed(ctx, actions::READ).await?;
        let req = TenantId(ctx.subject_tenant_id());

        if !self.repo.scope_includes_tenant(&scope, req.0).await? {
            self.metrics.cross_tenant_denied();
            return Err(DomainError::AccessDenied { cause: None });
        }

        let subject = OwnerId(ctx.subject_id());
        let chain = self.dir.ancestor_chain(ctx, req).await?;
        let row = self.repo.resolve_for_get(req, subject, key, &chain).await?;

        let Some(row) = row else {
            self.metrics.read_outcome(ReadOutcome::Miss);
            return Ok(None);
        };

        let depth = chain
            .iter()
            .position(|c| *c == row.tenant_id.0)
            .unwrap_or(chain.len());
        self.metrics.walkup_depth(depth as u64);

        let owner = if row.sharing == SharingMode::Private {
            Some(row.owner_id)
        } else {
            None
        };

        let plugin = self.plugins.resolve().await?;
        let t0 = Instant::now();
        let result = plugin
            .get(ctx, &row.tenant_id, key, owner.as_ref())
            .await
            .map_err(map_plugin_err);
        let secs = t0.elapsed().as_secs_f64();
        let outcome = match &result {
            Ok(Some(_)) => Outcome::Success,
            Ok(None) => Outcome::NotFound,
            Err(_) => Outcome::Error,
        };
        self.metrics
            .dependency(Dep::Plugin, DepOp::PluginGet, outcome, secs);
        let value: SecretValue = result?.ok_or(DomainError::NotFound)?;

        let is_inherited = row.tenant_id != req;
        self.metrics.read_outcome(if is_inherited {
            ReadOutcome::HitInherited
        } else {
            ReadOutcome::HitOwn
        });

        Ok(Some(GetSecretResponse {
            value,
            owner_tenant_id: row.tenant_id,
            sharing: row.sharing,
            is_inherited,
            version: row.version,
        }))
    }

    /// Create or update a secret.
    ///
    /// A write targets the row of its own sharing class — `private` →
    /// `(tenant, ref, owner)`, `tenant`/`shared` → `(tenant, ref)` — so a private
    /// and a tenant/shared secret coexist under one reference (per design §4.1);
    /// a write of one class never affects the other.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::Conflict`] if `create_only` and a secret of the same
    /// sharing class already exists.
    // Saga orchestration (validate -> scope -> resolve -> backend -> commit) is
    // inherently branchy; kept as one function for readability of the flow.
    #[allow(clippy::cognitive_complexity)]
    pub async fn put(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        value: SecretValue,
        sharing: SharingMode,
        create_only: bool,
        precondition: Option<WritePrecondition>,
    ) -> Result<(), DomainError> {
        let scope = self.scope_for_timed(ctx, actions::WRITE).await?;
        let tenant = TenantId(ctx.subject_tenant_id());
        let owner = OwnerId(ctx.subject_id());

        // Own-tenant gate, mirroring the read path: insert_provisioning uses
        // scope_unchecked, so an allow-with-constraints scope that excludes the
        // caller's tenant would otherwise insert an orphan provisioning row
        // (the later scope_with clamp only fails after the side effects).
        if !self.repo.scope_includes_tenant(&scope, tenant.0).await? {
            self.metrics.cross_tenant_denied();
            return Err(DomainError::AccessDenied { cause: None });
        }

        // Fail fast if no plugin is available before touching metadata.
        let plugin = self.plugins.resolve().await?;

        if let Some(existing) = self
            .repo
            .find_for_write(&scope, tenant, owner, key, sharing)
            .await?
        {
            if create_only {
                return Err(DomainError::Conflict);
            }
            // find_for_write addresses the target sharing class, so a write never
            // crosses the private boundary here — a private and a tenant/shared
            // secret coexist under one ref (per design §4.1). The guard stays as a
            // defensive invariant; tenant↔shared is the only real in-place change.
            if (existing.sharing == SharingMode::Private) != (sharing == SharingMode::Private) {
                return Err(DomainError::UnsupportedTransition {
                    detail: "cannot move between private and tenant/shared".into(),
                });
            }
            let expected_version = Self::precheck_version(precondition, existing.version)?;
            return self
                .overwrite_existing(OverwriteArgs {
                    ctx,
                    scope: &scope,
                    plugin: &plugin,
                    key,
                    value,
                    sharing,
                    existing_id: existing.id,
                    expected_version,
                })
                .await;
        }

        // Create path: the target does not exist. An If-Match precondition
        // requires an existing target, so any precondition fails here.
        if precondition.is_some() {
            return Err(DomainError::VersionConflict);
        }

        // Create saga: insert provisioning → plugin.put → mark_active.
        let id = Uuid::new_v4();
        let new = NewSecret {
            id,
            tenant_id: tenant,
            reference: key.clone(),
            sharing,
            owner_id: owner,
        };
        match self.repo.insert_provisioning(&scope, &new).await {
            Ok(()) => {}
            Err(DomainError::Conflict) => {
                // Lost the create race. A create-only POST surfaces the 409
                // directly; a PUT resolves to an update once the winner (whose
                // provisioning row is invisible to find_for_write) marks active.
                if create_only {
                    return Err(DomainError::Conflict);
                }
                for _ in 0..CREATE_RACE_MAX_RETRIES {
                    if let Some(existing) = self
                        .repo
                        .find_for_write(&scope, tenant, owner, key, sharing)
                        .await?
                    {
                        let expected_version =
                            Self::precheck_version(precondition, existing.version)?;
                        return self
                            .overwrite_existing(OverwriteArgs {
                                ctx,
                                scope: &scope,
                                plugin: &plugin,
                                key,
                                value,
                                sharing,
                                existing_id: existing.id,
                                expected_version,
                            })
                            .await;
                    }
                    sleep(Duration::from_millis(CREATE_RACE_RETRY_BACKOFF_MS)).await;
                }
                tracing::warn!(
                    tenant = %tenant.0,
                    key = %key.as_ref(),
                    "credstore put: create-race retry budget exhausted; winner appears stuck mid-saga, returning retry-safe Conflict"
                );
                return Err(DomainError::Conflict);
            }
            Err(e) => return Err(e),
        }

        let plugin_owner = if sharing == SharingMode::Private {
            Some(owner)
        } else {
            None
        };
        let t0 = Instant::now();
        let result = plugin
            .put(ctx, &tenant, key, value, plugin_owner.as_ref())
            .await
            .map_err(map_plugin_err);
        let secs = t0.elapsed().as_secs_f64();
        let outcome = if result.is_ok() {
            Outcome::Success
        } else {
            Outcome::Error
        };
        self.metrics
            .dependency(Dep::Plugin, DepOp::PluginPut, outcome, secs);
        if let Err(e) = result {
            // Compensate the saga: the backend write failed, so roll back the
            // provisioning row. The partial unique index ignores status, so a
            // lingering provisioning row wedges the reference until the reaper
            // runs — a misleading 409 for POST, exhausted retries for PUT.
            // Best-effort: a failed cleanup just defers to the reaper.
            let rollback = match self.repo.delete_by_id(&scope, id, None).await {
                Ok(()) => Outcome::Success,
                // Row already gone (concurrent reap/delete): not wedged.
                Err(DomainError::NotFound) => Outcome::NotFound,
                Err(cleanup_err) => {
                    tracing::warn!(
                        tenant = %tenant.0,
                        key = %key.as_ref(),
                        err = %cleanup_err,
                        "credstore put: provisioning rollback after backend write failure failed; reference stays wedged until reaped"
                    );
                    Outcome::Error
                }
            };
            self.metrics.provisioning_rollback(rollback);
            return Err(e);
        }

        // KNOWN DEBT (orphaned backend value): a mark_active failure after a
        // successful plugin.put leaves the value in the backend while the row
        // stays provisioning and is later reaped — so the value persists with no
        // metadata pointer. A client retry overwrites it; absent a retry it
        // lingers until the backend's own TTL/GC (if any). It is never readable
        // (no active row), so this is a storage leak, not a disclosure.
        // Reconciliation plan: have the reaper issue a best-effort plugin.delete
        // for each provisioning row it reaps (owner derived from the row), making
        // cleanup self-healing rather than retry-dependent. Tracked separately;
        // surfaced operationally via the warn below and the provisioning_reaped
        // metric.
        if let Err(e) = self.repo.mark_active(&scope, id).await {
            tracing::warn!(
                tenant = %tenant.0,
                key = %key.as_ref(),
                err = %e,
                "credstore put: mark_active failed after backend write; backend value orphaned until reaped/retried"
            );
            return Err(e);
        }
        Ok(())
    }

    /// Optimistic-concurrency pre-check before any backend write: returns the
    /// `version = ?` filter to gate `touch` on, or `VersionConflict` if the
    /// caller's `If-Match` version already disagrees with the current row.
    fn precheck_version(
        precondition: Option<WritePrecondition>,
        existing_version: i64,
    ) -> Result<Option<i64>, DomainError> {
        match precondition {
            Some(WritePrecondition::Version(v)) if existing_version != v => {
                Err(DomainError::VersionConflict)
            }
            Some(WritePrecondition::Version(v)) => Ok(Some(v)),
            Some(WritePrecondition::Exists) | None => Ok(None),
        }
    }

    /// Overwrite an existing secret: write the backend value first, then bump
    /// the metadata version (and sharing). Backend-first means a plugin failure
    /// leaves metadata untouched; a `touch` failure after a successful backend
    /// write self-heals on the next put. Shared by the normal overwrite path and
    /// the create-race resolution path.
    #[allow(clippy::cognitive_complexity)]
    async fn overwrite_existing(&self, args: OverwriteArgs<'_>) -> Result<(), DomainError> {
        let OverwriteArgs {
            ctx,
            scope,
            plugin,
            key,
            value,
            sharing,
            existing_id,
            expected_version,
        } = args;
        let tenant = TenantId(ctx.subject_tenant_id());
        let owner = OwnerId(ctx.subject_id());
        let plugin_owner = if sharing == SharingMode::Private {
            Some(owner)
        } else {
            None
        };
        let t0 = Instant::now();
        let result = plugin
            .put(ctx, &tenant, key, value, plugin_owner.as_ref())
            .await
            .map_err(map_plugin_err);
        let secs = t0.elapsed().as_secs_f64();
        let outcome = if result.is_ok() {
            Outcome::Success
        } else {
            Outcome::Error
        };
        self.metrics
            .dependency(Dep::Plugin, DepOp::PluginPut, outcome, secs);
        result?;

        // Version bump second. A failure here (after the backend write committed)
        // leaves version/sharing lagging the backend until the next put retries.
        match self
            .repo
            .touch(scope, existing_id, sharing, expected_version)
            .await
        {
            Ok(Some(_)) => Ok(()),
            // 0 rows under an If-Match precondition: the version moved (concurrent
            // write) or the row vanished between our pre-check and the commit. The
            // backend write already committed, but surface the optimistic-lock
            // failure so the caller retries against the current version.
            Ok(None) if expected_version.is_some() => {
                tracing::warn!(
                    tenant = %tenant.0,
                    key = %key.as_ref(),
                    "credstore put: If-Match version precondition lost between pre-check and commit (concurrent write)"
                );
                Err(DomainError::VersionConflict)
            }
            // Row concurrently deleted/reaped: the backend write already committed
            // and there is no metadata row to bump — treat as success, but record
            // the vanish so the otherwise-silent race is observable.
            Ok(None) => {
                tracing::debug!(
                    tenant = %tenant.0,
                    key = %key.as_ref(),
                    "credstore put: row vanished before version bump (concurrent delete/reap); backend write already committed"
                );
                Ok(())
            }
            Err(e) => {
                tracing::warn!(
                    tenant = %tenant.0,
                    key = %key.as_ref(),
                    err = %e,
                    "credstore put: version bump (touch) failed after backend write; version/sharing metadata lags the backend until the next put retries"
                );
                Err(e)
            }
        }
    }

    /// Delete an owned secret.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::NotFound`] if no own-tenant row exists for the key.
    /// Returns [`DomainError::VersionConflict`] if `precondition` is set and the
    /// current version does not satisfy it.
    pub async fn delete(
        &self,
        ctx: &SecurityContext,
        key: &SecretRef,
        precondition: Option<WritePrecondition>,
    ) -> Result<(), DomainError> {
        let scope = self.scope_for_timed(ctx, actions::DELETE).await?;
        let tenant = TenantId(ctx.subject_tenant_id());
        let owner = OwnerId(ctx.subject_id());

        let Some(row) = self.repo.find_own(&scope, tenant, owner, key).await? else {
            return Err(DomainError::NotFound);
        };

        // Optimistic-concurrency gate, in two layers mirroring the put path.
        // `Exists` is satisfied by find_own; a `Version` precondition becomes:
        //   1. a pre-check here, before the backend delete, so the common
        //      stale-client case has no side effect; and
        //   2. the same `version = ?` filter on delete_by_id below.
        let expected_version = match precondition {
            Some(WritePrecondition::Version(v)) => Some(v),
            Some(WritePrecondition::Exists) | None => None,
        };
        if let Some(v) = expected_version
            && row.version != v
        {
            return Err(DomainError::VersionConflict);
        }

        let plugin_owner = if row.sharing == SharingMode::Private {
            Some(row.owner_id)
        } else {
            None
        };

        // Delete the backend value FIRST. A plugin failure then leaves the
        // metadata row intact, so the caller gets an error and can retry — no
        // unreachable orphan in the backend. A missing backend value is treated
        // as success (idempotent). If the subsequent row delete fails, the row
        // lingers with no backend value (`get` → 404) and a later `put`
        // re-populates it. Backend-first also means that under a version
        // precondition a concurrent bump between the pre-check and the gated row
        // delete leaves the backend value deleted but the (now-newer) row in
        // place — we surface VersionConflict rather than a misleading 404, and
        // `get` 404s until the next put re-populates it.
        let t0 = Instant::now();
        let plugin_result = self
            .plugins
            .resolve()
            .await?
            .delete(ctx, &tenant, key, plugin_owner.as_ref())
            .await;
        let secs = t0.elapsed().as_secs_f64();
        let plugin_result = match plugin_result {
            Ok(()) | Err(CredStoreError::NotFound) => Ok(()),
            Err(e) => Err(map_plugin_err(e)),
        };
        self.metrics.dependency(
            Dep::Plugin,
            DepOp::PluginDelete,
            if plugin_result.is_ok() {
                Outcome::Success
            } else {
                Outcome::Error
            },
            secs,
        );
        plugin_result?;

        match self
            .repo
            .delete_by_id(&scope, row.id, expected_version)
            .await
        {
            Ok(()) => Ok(()),
            // 0 rows under a version precondition: the row moved/vanished between
            // the pre-check and the gated delete. The backend value is already
            // gone (backend-first); surface the optimistic-lock conflict.
            Err(DomainError::NotFound) if expected_version.is_some() => {
                Err(DomainError::VersionConflict)
            }
            Err(e) => Err(e),
        }
    }

    /// Reap stuck provisioning rows and refresh inventory gauges.
    ///
    /// Called by the reaper loop on a periodic tick. Errors are logged, not propagated.
    pub async fn reap_and_refresh(&self) {
        match self
            .repo
            .reap_provisioning(self.provisioning_timeout_secs)
            .await
        {
            Ok(n) => {
                if n > 0 {
                    self.metrics.provisioning_reaped(n);
                }
            }
            Err(e) => {
                tracing::warn!(err = %e, "reap_provisioning failed");
            }
        }
        match self.repo.inventory().await {
            Ok(counts) => self.metrics.record_inventory(counts),
            Err(e) => {
                tracing::warn!(err = %e, "inventory failed");
            }
        }
    }
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod service_tests;
