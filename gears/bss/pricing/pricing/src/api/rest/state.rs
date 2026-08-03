//! Per-request state for the authoring surface, built once in `init()`.
//!
//! One struct for the plan plane and the price plane together, rather than one
//! per module, because the two share a transaction seam and a gate: a
//! `POST …/plans/{planId}/prices` reads the plan repository to check the row's
//! parent and writes through the price repository under the idempotency gate, so
//! splitting the state would mean two `Extension`s wired identically and a
//! handler picking the wrong one being a compile error nobody sees until a route
//! is added.
//!
//! The repositories are cheap clones over the same provider (each holds only a
//! `DBProvider`), so carrying four of them costs nothing beyond the handle. `db`
//! is here for [`crate::infra::idempotent`], which is the one thing on this
//! surface that opens a transaction of its own.

use toolkit_db::{DBProvider, DbError};

use crate::infra::storage::repo::{IdempotencyGate, PlanRepo, PlanShapeRepo, PriceRepo};

/// The authoring surface's dependencies, shared via
/// `Extension<Arc<AuthoringState>>` exactly as
/// [`frontier::ApiState`](crate::api::rest::frontier::ApiState) is.
#[derive(Clone)]
pub struct AuthoringState {
    /// The provider the transaction seam opens its one transaction on.
    pub db: DBProvider<DbError>,
    /// The plan revision chain.
    pub plans: PlanRepo,
    /// The revision-scoped child shape tables (phases, add-on rules, the
    /// descriptor set).
    pub shapes: PlanShapeRepo,
    /// Draft price rows and their bands.
    pub prices: PriceRepo,
    /// The at-most-once gate, holding the configured retention window.
    pub idempotency: IdempotencyGate,
}
