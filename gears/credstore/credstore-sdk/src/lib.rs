//! credstore SDK — public API traits, models, errors, GTS schema, secret types.
pub mod api;
pub mod error;
pub mod gts;
pub mod models;
pub mod plugin_api;
pub mod types;

pub use api::CredStoreClientV1;
pub use error::CredStoreError;
pub use gts::{CredStorePluginSpecV1, SECRET_RESOURCE_TYPE, SecretTypeTraits, SecretV1};
pub use models::{
    GetSecretResponse, OwnerId, SecretRef, SecretValue, SharingMode, TenantId, WriteOptions,
};
pub use plugin_api::CredStorePluginClientV1;
pub use types::{SECRET_TYPE_CATALOG, SecretType, SecretTypeDescriptor, SecretTypeRef};
