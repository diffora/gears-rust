//! REST DTOs for the credstore module.

use credstore_sdk::{GetSecretResponse, SharingMode};
use uuid::Uuid;

use crate::domain::error::DomainError;

/// Sharing mode for the REST transport layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[modkit_macros::api_dto(response, request)]
pub enum SharingModeDto {
    /// Only the owner can access the secret.
    Private,
    /// Any actor inside the owning tenant can access the secret.
    #[default]
    Tenant,
    /// Descendant tenants can inherit the secret.
    Shared,
}

impl From<SharingMode> for SharingModeDto {
    fn from(value: SharingMode) -> Self {
        match value {
            SharingMode::Private => Self::Private,
            SharingMode::Tenant => Self::Tenant,
            SharingMode::Shared => Self::Shared,
        }
    }
}

impl From<SharingModeDto> for SharingMode {
    fn from(value: SharingModeDto) -> Self {
        match value {
            SharingModeDto::Private => Self::Private,
            SharingModeDto::Tenant => Self::Tenant,
            SharingModeDto::Shared => Self::Shared,
        }
    }
}

/// Request body for `POST /credstore/v1/secrets`.
///
/// `Debug` is hand-written to redact `value` — a derived `Debug` would expose
/// the plaintext secret if this DTO is ever `{:?}`-logged by a future layer.
#[derive(Clone, PartialEq, Eq)]
#[modkit_macros::api_dto(request)]
#[serde(deny_unknown_fields)]
pub struct CreateSecretRequestDto {
    /// Secret reference key — `[a-zA-Z0-9_-]+`, max 255 characters.
    pub reference: String,
    /// Secret value as a UTF-8 string.
    pub value: String,
    /// Sharing mode for the secret.
    #[serde(default)]
    pub sharing: SharingModeDto,
}

impl std::fmt::Debug for CreateSecretRequestDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreateSecretRequestDto")
            .field("reference", &self.reference)
            .field("value", &"[REDACTED]")
            .field("sharing", &self.sharing)
            .finish()
    }
}

/// Request body for `PUT /credstore/v1/secrets/{ref}`.
///
/// `Debug` is hand-written to redact `value` (see [`CreateSecretRequestDto`]).
#[derive(Clone, PartialEq, Eq)]
#[modkit_macros::api_dto(request)]
#[serde(deny_unknown_fields)]
pub struct UpdateSecretRequestDto {
    /// Secret value as a UTF-8 string.
    pub value: String,
    /// Sharing mode for the secret.
    #[serde(default)]
    pub sharing: SharingModeDto,
}

impl std::fmt::Debug for UpdateSecretRequestDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UpdateSecretRequestDto")
            .field("value", &"[REDACTED]")
            .field("sharing", &self.sharing)
            .finish()
    }
}

/// Access metadata returned alongside the secret value.
#[derive(Debug, Clone, PartialEq, Eq)]
#[modkit_macros::api_dto(response)]
pub struct SecretMetadataDto {
    /// The tenant that owns this secret.
    pub owner_tenant_id: Uuid,
    /// The sharing mode that governed the lookup result.
    pub sharing: SharingModeDto,
    /// Whether the secret came from an ancestor tenant.
    pub is_inherited: bool,
    /// Monotonic version of the resolved secret (also returned as `ETag`).
    pub version: i64,
}

/// Response body for `GET /credstore/v1/secrets/{ref}`.
///
/// `Debug` is hand-written to redact `value` (see [`CreateSecretRequestDto`]).
#[derive(Clone, PartialEq, Eq)]
#[modkit_macros::api_dto(response)]
pub struct GetSecretResponseDto {
    /// Secret value as a UTF-8 string.
    pub value: String,
    /// Access metadata for the resolved secret.
    pub metadata: SecretMetadataDto,
}

impl std::fmt::Debug for GetSecretResponseDto {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GetSecretResponseDto")
            .field("value", &"[REDACTED]")
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl GetSecretResponseDto {
    /// Convert the domain [`GetSecretResponse`] into the REST DTO shape.
    ///
    /// # Errors
    ///
    /// Returns [`DomainError::Internal`] when the secret value is not valid
    /// UTF-8 (e.g. binary written via the SDK). The JSON/string transport cannot
    /// represent it, and lossy decoding would silently corrupt the secret, so we
    /// reject rather than mangle.
    pub fn try_from_response(resp: &GetSecretResponse) -> Result<Self, DomainError> {
        let value = String::from_utf8(resp.value.as_bytes().to_vec()).map_err(|_| {
            DomainError::internal(
                "secret value is not valid UTF-8 and cannot be encoded for the REST transport",
            )
        })?;
        Ok(Self {
            value,
            metadata: SecretMetadataDto {
                owner_tenant_id: resp.owner_tenant_id.0,
                sharing: resp.sharing.into(),
                is_inherited: resp.is_inherited,
                version: resp.version,
            },
        })
    }
}

#[cfg(test)]
#[path = "dto_tests.rs"]
mod tests;
