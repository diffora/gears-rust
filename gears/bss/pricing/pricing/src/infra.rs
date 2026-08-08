//! Infrastructure layer — persistence, transports and the outward-facing
//! mappings the domain deliberately knows nothing about.

pub mod approval;
pub mod bundle;
pub mod change_graph;
pub mod clone;
pub mod currency_binding;
pub mod cutover;
pub mod error_mapping;
pub mod fixture_gate;
pub mod idempotent;
pub mod jobs;
pub mod metrics;
pub mod migration;
pub mod overlay_publish;
pub mod publish;
pub mod read_model;
pub mod retirement;
pub mod storage;
pub mod supersession;
pub mod synthesis;
pub mod threshold;
pub mod window;
