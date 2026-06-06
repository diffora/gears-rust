//! credstore SDK — public API traits, models, errors, GTS schema.
pub mod api;
pub mod error;
pub mod gts;
pub mod models;
pub mod plugin_api;

pub use api::CredStoreClientV1;
pub use error::CredStoreError;
pub use gts::{CredStorePluginSpecV1, SECRET_RESOURCE_TYPE, SecretV1};
pub use models::{GetSecretResponse, OwnerId, SecretRef, SecretValue, SharingMode, TenantId};
pub use plugin_api::CredStorePluginClientV1;
