//! `BssProductsGear` — the toolkit gear declaration.
//!
//! One deployable gear over one `toolkit-db` backend, now declaring both the
//! `db` and the `rest` capabilities. The Foundation tables and their guards
//! are migrations, and they are what everything else in the gear is built on;
//! the `rest` capability claims the gear's service prefix so the runtime
//! knows, from boot, that `/bss-products/v1` belongs to this gear and to no
//! other.
//!
//! **This slice (Phase 4, Slice C) mounts the read door**:
//! `GET /bss-products/v1/products/{id}` and `GET /bss-products/v1/skus/{id}`
//! (`crate::api::rest::products`, `crate::api::rest::skus`). It is deliberately
//! the first door in the phase: an author who has not just written a row has
//! no `ETag` to send back as `If-Match`, so every mutating door this phase
//! still owes — the create doors first, then save/publish/discard — depends
//! on this one existing already. Before this slice the router this gear
//! mounted was empty; a gear that declared `rest` but mounted nothing at all
//! would leave its prefix unclaimed, so an unconfigured boot would either
//! fall through to another gear's router or answer whatever the merge order
//! happens to produce, which is why the runtime-absent branch below still
//! nests an empty router rather than erroring — an unconfigured boot still
//! answers `404` from this gear, not a fall-through.
//!
//! The `authz_resolver` dependency and the `PolicyEnforcer` it builds in
//! [`Gear::init`] were wired ahead of the routes that would gate through
//! them, for the same reason the sibling pricing gear wires its own PEP at
//! init: authorization is security-critical, so a missing
//! `AuthZResolverClient` must fail the boot rather than be discovered the
//! first time a handler reaches for an enforcer that was never built. This
//! slice is the first to read it, cloning it once per boot into its own
//! `Extension` layer in `RestApiCapability::register_rest`.
//!
//! **The transactional outbox is wired the same way, and deliberately not the
//! way the sibling gears wired theirs.** `DECISIONS.md` P-D-22 struck
//! `products_outbox` as a gear-authored table: pricing's own `pricing_outbox`
//! is a private re-invention of a platform facility, measured against
//! mini-chat, the one gear that imports `toolkit_db::outbox::Outbox` directly.
//! This gear follows mini-chat's donor shape, not pricing's. [`Gear::init`]
//! builds the pipeline with the outbox's own table prefix
//! ([`OUTBOX_TABLE_PREFIX`]) and stores the running handle in
//! [`ProductsRuntime`] beside the enforcer; `DatabaseCapability::migrations`
//! appends the facility's own migrations
//! (`toolkit_db::outbox::outbox_migrations_with_prefix`) rather than declaring
//! any outbox table in this gear's own migration chain. No queue is
//! registered yet — there is no door with a handler to register one for —
//! so the pipeline starts with an empty queue set; a door slice adds
//! `.queue(name, partitions).transactional(handler)` to the builder chain in
//! [`Gear::init`] as it lands, per P-D-23 (`leased`, not `transactional`, is
//! the mode owed once a handler exists).
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

/// Table-family prefix for this gear's `toolkit_db::outbox` instance
/// (P-D-22: "its tables ... carry a configurable prefix"). Names the gear
/// rather than reusing the facility's own default (`toolkit_outbox`), so this
/// gear's `_body`/`_partitions`/`_incoming`/`_outgoing`/`_dead_letters` tables
/// are identifiable on sight in a database another gear's default-prefixed
/// outbox might also live in, and never collide with it.
///
/// MUST match between this constant's use in [`Gear::init`]
/// (`Outbox::builder(..).table_prefix(..)`) and
/// `DatabaseCapability::migrations`'s `outbox_migrations_with_prefix(..)`
/// call below — a mismatch would point the running pipeline at tables this
/// chain never created.
use crate::infra::events::OUTBOX_TABLE_PREFIX;

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
    /// Read from `register_rest` since this slice's read door: `(*rt.enforcer)
    /// .clone()` is layered onto the merged router as its own `Extension`, the
    /// same way the sibling ledger gear's per-request PEP is wired.
    pub enforcer: Arc<authz_resolver_sdk::PolicyEnforcer>,

    /// The transactional-outbox pipeline (P-D-22), built in `init()` from
    /// `toolkit_db::outbox::Outbox::builder`. Held as the full
    /// [`toolkit_db::outbox::OutboxHandle`], not just the inner `Arc<Outbox>`
    /// it wraps: dropping the handle drops its `TaskSet`, which cancels the
    /// pipeline's background tasks (sequencer, processors, vacuum) on drop —
    /// so the handle must outlive the process, not just the field access.
    ///
    /// Read by [`RestApiCapability::register_rest`], which clones the inner
    /// `Arc<Outbox>` onto `api::rest::ApiState` so a door can enqueue inside
    /// its own transaction.
    pub outbox: toolkit_db::outbox::OutboxHandle,

    /// The database provider `api::rest::ApiState` clones into the read
    /// door's per-request state. Kept on the runtime rather than built fresh
    /// in `register_rest` because `ctx.db_required()` is `init()`'s to call —
    /// the same acquisition point the outbox handle above is built from —
    /// and a repeated call from `register_rest` would be a second,
    /// unnecessary place this gear's boot could fail on a missing `db`
    /// capability.
    pub db: toolkit_db::DBProvider<toolkit_db::DbError>,
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
        let mut migrations = crate::infra::storage::migrations::Migrator::migrations();

        // The outbox's own tables are migrated by the facility, not by this
        // chain (P-D-22's consequences: "C1's 'one migration per table,
        // guards defined once' does not reach these tables — they are
        // migrated by `outbox_migrations()`, and the schema oracle must
        // therefore golden them as imported rather than as gear-authored").
        // Appended, never declared: no `CreateProductsOutbox`-shaped
        // migration exists anywhere in `crate::infra::storage::migrations`.
        #[allow(clippy::expect_used)]
        let outbox_migrations =
            toolkit_db::outbox::outbox_migrations_with_prefix(OUTBOX_TABLE_PREFIX).expect(
                "OUTBOX_TABLE_PREFIX is a fixed compile-time identifier, validated once here \
                 rather than at every call site: alphabetic-first, alnum/underscore only, and \
                 well under the facility's length limit",
            );
        migrations.extend(outbox_migrations);
        migrations
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

        // Transactional outbox (P-D-22). The registry enqueues through the
        // platform's own `toolkit_db::outbox` pipeline rather than a
        // gear-authored `products_outbox` table — see this module's doc for
        // why. The gear's own database is required for the outbox exactly as
        // it is for the Foundation tables, so a missing configuration fails
        // the boot the same way the missing `AuthZResolverClient` above does.
        let db_provider = ctx
            .db_required()
            .context("bss-products: database not configured for the outbox pipeline")?;
        let outbox_db = db_provider.db();
        // The queue is declared here because `enqueue` refuses an
        // unregistered one (`OutboxError::QueueNotRegistered`), and the
        // create door enqueues inside its own transaction. Its processor is
        // a holding one: P-D-47 puts the real processor — the broker SDK's
        // `DbProducer` — in Phase 8's `dod-outbox-eventing`, so until then
        // rows accumulate undelivered rather than being discarded. See
        // `crate::infra::events::PendingBrokerProducer` for why it must not
        // answer `Ok`.
        let outbox = toolkit_db::outbox::Outbox::builder(outbox_db)
            .table_prefix(OUTBOX_TABLE_PREFIX)
            .context("bss-products: invalid outbox table prefix")?
            .queue(
                crate::infra::events::QUEUE_NAME,
                toolkit_db::outbox::Partitions::of(crate::infra::events::PARTITIONS),
            )
            .leased(crate::infra::events::PendingBrokerProducer)
            .start()
            .await
            .context("bss-products: outbox pipeline failed to start")?;

        self.runtime.store(Some(Arc::new(ProductsRuntime {
            enforcer,
            outbox,
            db: db_provider,
        })));

        Ok(())
    }
}

impl RestApiCapability for BssProductsGear {
    fn register_rest(
        &self,
        _ctx: &GearCtx,
        router: Router,
        openapi: &dyn OpenApiRegistry,
    ) -> anyhow::Result<Router> {
        let Some(rt) = self.runtime.load_full() else {
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
        // Phase 4 Slice C: the read door. `products::router`/`skus::router`
        // each register their own absolute path
        // (`/bss-products/v1/{products|skus}/{id}`), so they are `.merge()`d
        // onto the host router directly rather than nested under the
        // reserved prefix by `api::rest::router` — the same shape the
        // sibling ledger gear's own door modules use. The `PolicyEnforcer` is
        // layered once here as its own `Extension`, cloned from the `Arc` the
        // runtime holds (RMS layers the value, not the `Arc`;
        // `PolicyEnforcer: Clone`), rather than carried on `ApiState`, so
        // every door added in this and later slices reaches it the same way.
        let api_state = Arc::new(crate::api::rest::ApiState {
            db: rt.db.clone(),
            outbox: Arc::clone(rt.outbox.outbox()),
        });
        Ok(router
            .merge(crate::api::rest::products::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .merge(crate::api::rest::skus::router(
                Arc::clone(&api_state),
                openapi,
            ))
            .layer(axum::Extension((*rt.enforcer).clone())))
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
