//! credstore — `CredStore` module: tenant-scoped secrets over pluggable backends.
pub mod api;
pub mod client;
pub mod config;
pub mod domain;
pub mod infra;
pub mod module;

pub use module::CredStoreModule;
