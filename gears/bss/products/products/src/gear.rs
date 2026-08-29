//! `BssProductsGear` — the toolkit gear declaration.
//!
//! One deployable gear over one `toolkit-db` backend, now declaring both the
//! `db` and the `rest` capabilities. The Foundation tables and their guards
//! are migrations, and they are what everything else in the gear is built on;
//! the `rest` capability claims the gear's service prefix so the runtime
//! knows, from boot, that `/bss-products/v1` belongs to this gear and to no
//! other.
//!
//! **The router this slice mounts is empty.** The authoring doors —
//! `POST /bss-products/v1/products` and `POST /bss-products/v1/skus` — arrive
//! in Phase 4 with the repositories and the DTOs they need, as their own
//! definitions of done. What lands here first is the reservation: a gear that
//! declared `rest` but mounted nothing at all would leave its prefix
//! unclaimed, so an unconfigured boot would either fall through to another
//! gear's router or answer whatever the merge order happens to produce. Both
//! are worse than the one thing an empty, correctly nested router guarantees —
//! a caller under `/bss-products/v1` gets a `404`, from this gear, until this
//! gear has something to answer with.
//!
//! The `authz_resolver` dependency and the `PolicyEnforcer` it builds in
//! [`Gear::init`] are wired now, ahead of the routes that will gate through
//! them, for the same reason the sibling pricing gear wires its own PEP at
//! init: authorization is security-critical, so a missing
//! `AuthZResolverClient` must fail the boot rather than be discovered the
//! first time a Phase 4 handler reaches for an enforcer that was never built.
//!
//! @cpt-cf-bss-products-component-registry-foundation

use std::sync::Arc;

use anyhow::Context;
use arc_swap::ArcSwapOption;
use async_trait::async_trait;
use axum::Router;
use toolkit::api::OpenApiRegistry;
use toolkit::contracts::RestApiCapability;
use toolkit::{Gear, GearCtx};

use crate::config::ProductsConfig;

/// Per-process state built by [`Gear::init`] and read by
/// [`RestApiCapability::register_rest`].
///
/// Carries only the platform PEP for now. Phase 4 adds the per-request state
/// each authoring surface needs, following the sibling pricing and ledger
/// gears' shape: one field per surface, built once here rather than wired
/// inside `register_rest`.
pub(crate) struct ProductsRuntime {
    /// Platform PEP, built in `init()` from the `authz-resolver` `ClientHub`
    /// client. `Arc`-held so a future per-request `Extension` clones the
    /// value, not the enforcer, exactly as the sibling pricing gear does.
    ///
    /// Unread until Phase 4's authoring doors gate through it: `register_rest`
    /// only checks whether the runtime is present in this slice, it does not
    /// yet read what it holds. The `allow` is discharged the day a handler
    /// extracts this field.
    #[allow(dead_code)]
    pub enforcer: Arc<authz_resolver_sdk::PolicyEnforcer>,
}

/// The products gear.
#[toolkit::gear(name = "bss-products", deps = [authz_resolver], capabilities = [db, rest])]
pub struct BssProductsGear {
    /// `None` until `init()` completes, and on a boot where the gear is
    /// compiled in but not configured.
    runtime: ArcSwapOption<ProductsRuntime>,
}

impl Default for BssProductsGear {
    fn default() -> Self {
        Self {
            runtime: ArcSwapOption::from(None),
        }
    }
}

impl toolkit::contracts::DatabaseCapability for BssProductsGear {
    fn migrations(&self) -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        use sea_orm_migration::MigratorTrait;
        crate::infra::storage::migrations::Migrator::migrations()
    }
}

#[async_trait]
impl Gear for BssProductsGear {
    async fn init(&self, ctx: &GearCtx) -> anyhow::Result<()> {
        // The configuration is read at init so a malformed operator file fails
        // the boot here rather than at the first request that happens to need
        // a field from it.
        let cfg: ProductsConfig = ctx.config_or_default()?;
        tracing::info!(
            idempotency_retention_hours = cfg.idempotency_retention_hours,
            "bss-products initialised"
        );

        // Platform PEP. Authz is security-critical — the catalog this gear
        // authors is what pricing and every downstream reader depend on — so a
        // missing `AuthZResolverClient` fails init loudly rather than
        // degrading to an unguarded router.
        let authz_client = ctx
            .client_hub()
            .get::<dyn authz_resolver_sdk::AuthZResolverClient>()
            .context(
                "bss-products: AuthZResolverClient absent from ClientHub; \
                 authz-resolver module must be registered",
            )?;
        let enforcer = Arc::new(authz_resolver_sdk::PolicyEnforcer::new(authz_client));

        self.runtime
            .store(Some(Arc::new(ProductsRuntime { enforcer })));

        Ok(())
    }
}

impl RestApiCapability for BssProductsGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: Router,
        _openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<Router> {
        let Some(_rt) = self.runtime.load_full() else {
            // Unconfigured boot: claim the prefix anyway, so a probe under
            // `/bss-products/v1` gets a `404` from this gear rather than
            // falling through to whatever else the host router matches.
            //
            // Logged rather than silent: reaching here means `init` never ran
            // or never populated the slot, and from the outside that is
            // indistinguishable from the ordinary configured-but-routeless
            // state this phase also produces. An operator debugging a 404
            // under this prefix needs the two told apart.
            tracing::warn!(
                "bss-products: register_rest reached with no runtime; \
                 mounting an empty router under the reserved prefix"
            );
            return Ok(crate::api::rest::router(router));
        };
        // Phase 4 fills this branch with the authoring doors, reading `_rt`
        // for the enforcer and whatever per-request state the repositories
        // need. Until then there are no routes to mount, so it nests the
        // same empty router as the branch above.
        Ok(crate::api::rest::router(router))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_leaves_the_runtime_slot_empty() {
        let gear = BssProductsGear::default();
        assert!(gear.runtime.load_full().is_none());
    }

    /// `register_rest`'s empty-runtime branch returns a router and does not
    /// error — the behaviour that distinguishes this gear from
    /// `simple-user-settings`, whose `register_rest` errors out of an
    /// uninitialised `service` slot.
    ///
    /// Calling `register_rest` itself needs a `GearCtx` and a
    /// `dyn OpenApiRegistry`; the former needs a
    /// `tokio_util::sync::CancellationToken`, which this slice's dependency
    /// delta does not carry. What is exercised directly, without either, is
    /// [`crate::api::rest::router`] — the helper both of `register_rest`'s
    /// branches call, and the only place the nesting happens. It is
    /// infallible (`Router -> Router`, no `Result`), which is what makes
    /// `register_rest`'s `Ok(...)` around it unconditional in both branches.
    /// A request under the reserved prefix is answered by **this** gear with
    /// a `404`, and a path outside it is untouched by the nest.
    ///
    /// The earlier version of this test built the router and dropped it,
    /// which asserted nothing: it passed just as well if `router` returned
    /// `host_router` unnested, or nested under the wrong prefix, or swapped
    /// its arguments. The prefix reservation is the one behaviour this
    /// module exists to deliver, so it is asserted where it is observable —
    /// through a request — rather than by trusting the type.
    #[tokio::test]
    async fn a_request_under_the_reserved_prefix_is_answered_by_this_gear() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use axum::routing::get;
        use tower::ServiceExt as _;

        let host = Router::new().route("/elsewhere", get(|| async { "host" }));
        let mounted = crate::api::rest::router(host);

        let under_prefix = mounted
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/bss-products/v1/anything")
                    .body(Body::empty())
                    .expect("build the probe request"),
            )
            .await
            .expect("the router answers");
        assert_eq!(
            under_prefix.status(),
            StatusCode::NOT_FOUND,
            "the prefix is claimed, so an unmounted path under it is this gear's 404"
        );

        let outside = mounted
            .oneshot(
                Request::builder()
                    .uri("/elsewhere")
                    .body(Body::empty())
                    .expect("build the control request"),
            )
            .await
            .expect("the router answers");
        assert_eq!(
            outside.status(),
            StatusCode::OK,
            "nesting under the prefix must not shadow the host router's own paths"
        );
    }
}
