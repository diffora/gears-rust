//! Typed repositories over the Foundation entities.
//!
//! Four land ahead of the paths that call them, and for the same reason: each
//! carries a storage invariant rather than a caller convention. The pin-frontier
//! repository's `advance` is forward-only in SQL; the plan repository's draft
//! edits are compare-and-swaps in SQL, with the row-version bump inside the same
//! statement that matches on the version the caller read; the idempotency gate's
//! at-most-once guarantee **is** an `INSERT ... ON CONFLICT DO NOTHING`; the
//! price repository's row and band set are one transaction, because a row whose
//! geometry can land a moment late is a row that is briefly wrong. None of those
//! guards survives being reimplemented per call site — that is what makes them
//! repositories and not helpers.
//!
//! The remaining tables get their repositories with the paths that write them —
//! a repository nothing calls is dead code, and dead code fails CI here.

pub mod idempotency_repo;
pub mod pin_frontier_repo;
pub mod plan_repo;
pub mod price_repo;

pub use idempotency_repo::{ClaimOutcome, IdempotencyGate};
pub use pin_frontier_repo::PinFrontierRepo;
pub use plan_repo::{NewPlanDraft, PlanRepo};
pub use price_repo::{NewPriceDraft, PriceRepo};
