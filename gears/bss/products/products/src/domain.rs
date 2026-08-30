//! The gear's business logic: what a catalog entity is and what may be done to
//! it, with no knowledge of transport or storage.

pub mod concurrency;
pub mod containment;
pub mod error;
pub mod idempotency;
pub mod name;
pub mod rules;
pub mod validation;
