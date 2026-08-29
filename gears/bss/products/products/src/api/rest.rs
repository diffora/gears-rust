//! REST surface.
//!
//! The gear's reserved prefix, `/bss-products/v1`, is claimed here so
//! `gear.rs` never spells it twice. Phase 4 adds the authoring doors —
//! `POST /bss-products/v1/products` and `POST /bss-products/v1/skus`, per
//! `docs/design/01-foundation.md` — as sibling modules under `api::rest`,
//! each merged into the router this module builds, following the sibling
//! pricing and ledger gears' shape: one module per resource, composed in
//! [`crate::gear::BssProductsGear::register_rest`] rather than wired inline.
//!
//! Nothing is mounted yet, which is why [`router`] takes no per-request
//! state today: there is no handler to close over.

use axum::Router;

pub mod preconditions;

/// The gear's reserved service prefix.
const PREFIX: &str = "/bss-products/v1";

/// Mounts the products REST surface onto `host_router`.
///
/// Nests an empty router under [`PREFIX`] so an unconfigured or
/// not-yet-implemented boot answers `404` under the gear's own namespace
/// rather than leaving the prefix unclaimed. Phase 4 replaces the empty
/// `Router::new()` with the merged authoring routers and starts taking the
/// per-request state (the `PolicyEnforcer`, the repositories) they need.
pub fn router(host_router: Router) -> Router {
    host_router.nest(PREFIX, Router::new())
}
