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

use crate::infra::approval::ApprovalService;
use crate::infra::publish::PublishService;
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

/// The governance surface's dependencies — the approval routes and the publish
/// mount, shared via `Extension<Arc<GovernanceState>>`.
///
/// One struct for the two, for [`AuthoringState`]'s reason: they are one seam
/// rather than two. `POST /plans/{planId}/publish` reads the plan repository to
/// resolve the revision under the caller's `If-Match`, asks the approval
/// workflow whether that exact revision carries an approved unit, and then hands
/// the publish engine the authorization it minted from it — so a split would
/// mean three `Extension`s wired identically and a handler reaching for the
/// wrong one being a compile error nobody sees until a route is added.
///
/// It is deliberately **not** folded into [`AuthoringState`]. That one is the
/// draft plane: four repositories and an idempotency gate, all cheap clones over
/// one provider. This one carries the services that hold the catalog-version
/// registry handle, and the authoring routes must not be able to reach one: a
/// version request is what decides what this deployment may freeze.
///
/// **"The engine is the only requester of a `CatalogVersion`" stood here and is
/// now false**, corrected rather than left as a claim the code disproves. D-99
/// makes every window *mutation* a publish unit running the same engine path, so
/// [`GovernanceState::windows`] is a second requester — legitimately, and by a
/// decision rather than by drift. What the sentence was protecting is unchanged
/// and is restated at its real strength: the registry handle stays inside this
/// state, both requesters take the **same** `Arc`, and neither invents a version
/// locally. Two requesters of one registry is one incrementer; two *incrementers*
/// is what `pricing-sdk`'s registry contract refuses, and nothing here is one.
///
/// The two requesters are kept apart by their **request id** and not by hope:
/// `PublishUnitKind::request_token` gives a plan publish `plan-publish/...` and a
/// window mutation `window-mutation/<window_id>/...`, so a retry of either finds
/// its own pending handle and never the other's.
#[derive(Clone)]
pub struct GovernanceState {
    /// The provider the publish subject is assembled over. Its own read, not
    /// the engine's: `PublishService::precheck` answers a report and the
    /// materiality evaluator needs the shape.
    pub db: DBProvider<DbError>,
    /// The plan revision chain — the publish route resolves its subject here.
    pub plans: PlanRepo,
    /// Published price rows, for `inst-mat-newrow`'s baseline.
    pub prices: PriceRepo,
    /// The approval workflow: open a pinned unit, read it, decide it.
    pub approvals: ApprovalService,
    /// The publish engine. Its `commit` is the act an approval authorizes.
    pub publish: PublishService,
    /// The window mutation workflow — the three §5 surfaces, each a publish unit
    /// (D-99).
    ///
    /// Here and not on [`AuthoringState`] because it requests a
    /// `CatalogVersion`, which is the whole criterion this split is drawn on. It
    /// shares the registry `Arc` with [`GovernanceState::publish`]; the module doc
    /// says why that is one incrementer and not two.
    pub windows: crate::infra::window::WindowService,
    /// The at-most-once gate the `POST …/windows` claims under (D-191).
    ///
    /// Here as well as on [`AuthoringState`] rather than instead of it: the two planes
    /// claim under different `operation` tokens over the same table, so one gate value
    /// shared by both would still key them apart, and a second field is what keeps each
    /// state carrying the collaborators its own routes reach for. The window `POST` was
    /// the last mutating create in this gear with **no** gate — it parsed the required
    /// `Idempotency-Key` and dropped it.
    pub idempotency: IdempotencyGate,
    /// The approval-threshold policy: the effective version, and the proposal that
    /// opens a D-10 unit.
    ///
    /// Here rather than on [`AuthoringState`] because it is governance twice over:
    /// its `PUT` gates on `approval_policy × write`, which the authoring roles do
    /// not hold, and its proposal opens an approval unit through the same store
    /// `approvals` writes. It requests no `CatalogVersion`, so it does not bear on
    /// the split's own criterion either way.
    pub thresholds: crate::infra::threshold::ThresholdService,
}
