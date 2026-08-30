//! BSS Product & SKU Registry gear.
//!
//! The authoring surface and System of Record for `Product` and `SKU`: what a
//! catalog entity is, what states it may occupy, and what has to be true before
//! it may be published. It computes no price and evaluates no plan — those are
//! the sibling pricing gear's.
//!
//! The crate is laid out as the design set is: a shared **Registry Foundation**
//! (the publish engine — entity model, draft/published state machine,
//! fail-closed validation pipeline, append-only history, idempotency, event
//! outbox and audit trail) that capability features author through. The
//! Foundation owns publish; capability features own capability policy and
//! register their rules into the pipeline.
//!
//! No crate-level lint allowances: the workspace bar is met as written rather
//! than paid down later.

#[doc(hidden)]
pub mod api;
#[doc(hidden)]
pub mod authz;
#[doc(hidden)]
pub mod config;
#[doc(hidden)]
pub mod domain;
#[doc(hidden)]
pub mod gear;
#[doc(hidden)]
pub mod gts;
#[doc(hidden)]
pub mod infra;

/// What more than one test module needs, written once. See its own doc for
/// why a test-support module earns its place in this crate in particular.
#[cfg(test)]
mod test_support;
