//! Infrastructure layer — persistence, transports and the outward-facing
//! mappings the domain deliberately knows nothing about.

pub mod approval;
pub mod error_mapping;
pub mod fixture_gate;
pub mod idempotent;
pub mod jobs;
pub mod publish;
pub mod read_model;
pub mod storage;
pub mod supersession;
pub mod threshold;
pub mod window;
