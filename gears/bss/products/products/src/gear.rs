//! `BssProductsGear` — the toolkit gear declaration.
//!
//! One deployable gear over one `toolkit-db` backend. This increment declares
//! the `db` capability alone: the Foundation tables and their guards are
//! migrations, and they are what everything else in the gear is built on.
//!
//! **No `rest` capability yet, and the absence is deliberate rather than
//! pending.** A route whose handler has nothing to call is not a route. The
//! authoring doors arrive with the repositories and the authorization gate they
//! need, as their own definitions of done; mounting a router before then would
//! be a surface that answers 500 to a caller who should have got 403.
//!
//! @cpt-cf-bss-products-component-registry-foundation

use async_trait::async_trait;
use toolkit::{Gear, GearCtx};

use crate::config::ProductsConfig;

/// The products gear.
#[toolkit::gear(name = "bss-products", capabilities = [db])]
#[derive(Default)]
pub struct BssProductsGear;

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
        Ok(())
    }
}
