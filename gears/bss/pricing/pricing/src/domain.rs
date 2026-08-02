//! Domain layer — the publish engine's vocabulary and rules.
//!
//! Carries no infrastructure types (enforced by DE0301): the repositories,
//! entities and transports live in [`crate::infra`] and [`crate::api`], and the
//! domain is what stays true regardless of where the rows are stored.

pub mod concurrency;
pub mod error;
pub mod events;
pub mod instant;
pub mod lifecycle;
pub mod money;
pub mod plan;
pub mod plan_rules;
pub mod plan_shape;
pub mod ports;
pub mod price_record;
pub mod price_row;
pub mod read_model;
pub mod rules;
pub mod scope_key;
pub mod snapshot;
pub mod validation;
