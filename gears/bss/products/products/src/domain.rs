//! The gear's business logic: what a catalog entity is and what may be done to
//! it, with no knowledge of transport or storage.

pub mod batch;
pub mod bucket;
pub mod canonical;
pub mod concurrency;
pub mod containment;
pub mod deprecation;
pub mod disposition;
pub mod error;
pub mod governance;
pub mod idempotency;
pub mod live_op;
pub mod name;
pub mod recognized;
pub mod retention;
pub mod rules;
pub mod states;
pub mod transition;
pub mod validation;
