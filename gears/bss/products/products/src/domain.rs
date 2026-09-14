//! The gear's business logic: what a catalog entity is and what may be done to
//! it, with no knowledge of transport or storage.

pub mod activation;
pub mod approval;
pub mod batch;
pub mod bucket;
pub mod canonical;
pub mod cascade;
pub mod concurrency;
pub mod containment;
pub mod deprecation;
pub mod disposition;
pub mod error;
pub mod governance;
pub mod idempotency;
pub mod lifecycle;
pub mod live_op;
pub mod materiality;
pub mod name;
pub mod read_model;
pub mod recognized;
pub mod retention;
pub mod retirement;
pub mod rules;
pub mod states;
pub mod taxonomy;
pub mod transition;
pub mod undeprecation;
pub mod validation;
