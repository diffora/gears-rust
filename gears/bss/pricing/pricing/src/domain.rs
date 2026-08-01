//! Domain layer — the publish engine's vocabulary and rules.
//!
//! Carries no infrastructure types (enforced by DE0301): the repositories,
//! entities and transports live in [`crate::infra`] and [`crate::api`], and the
//! domain is what stays true regardless of where the rows are stored.

pub mod error;
pub mod events;
pub mod lifecycle;
pub mod money;
pub mod ports;
pub mod read_model;
pub mod scope_key;
pub mod snapshot;
pub mod validation;
