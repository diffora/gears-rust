//! Fixed tenant-name constraints shared by command DTOs and bootstrap.

use crate::{CreateTenantRequest, UpdateTenantRequest};

/// Minimum tenant-name length in Unicode scalar values.
pub const TENANT_NAME_MIN_CHARS: usize = 1;
/// Maximum tenant-name length in Unicode scalar values.
pub const TENANT_NAME_MAX_CHARS: usize = 255;

/// Invalid tenant command name; diagnostics never include the supplied value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TenantInputValidationError {
    #[error("name must contain {min}..={max} characters")]
    NameLength { min: usize, max: usize },
}

/// Check the supplied tenant name contains 1–255 Unicode scalars.
///
/// Does not trim or mutate the value. AM normalizes request names before this
/// check; bootstrap checks its raw configured name because it is stored unchanged.
/// No registry, tenant-type or provider-metadata validation is performed.
///
/// # Errors
/// [`TenantInputValidationError`] when the length is outside the fixed bounds.
pub fn validate_tenant_name(name: &str) -> Result<(), TenantInputValidationError> {
    if !(TENANT_NAME_MIN_CHARS..=TENANT_NAME_MAX_CHARS).contains(&name.chars().count()) {
        return Err(TenantInputValidationError::NameLength {
            min: TENANT_NAME_MIN_CHARS,
            max: TENANT_NAME_MAX_CHARS,
        });
    }
    Ok(())
}

impl CreateTenantRequest {
    /// Validate the fixed name bounds without changing the input.
    ///
    /// See [`validate_tenant_name`]. Whitespace normalization, authorization,
    /// hierarchy/type checks and the opaque metadata size cap belong to the service.
    ///
    /// # Errors
    /// [`TenantInputValidationError`] for an invalid name length.
    pub fn validate(&self) -> Result<(), TenantInputValidationError> {
        validate_tenant_name(&self.name)
    }
}

impl UpdateTenantRequest {
    /// Validate a supplied name without changing the input.
    ///
    /// An omitted name is skipped; the service rejects empty patches separately.
    /// See [`validate_tenant_name`] for bounds and normalization policy.
    ///
    /// # Errors
    /// [`TenantInputValidationError`] for an invalid supplied name length.
    pub fn validate(&self) -> Result<(), TenantInputValidationError> {
        if let Some(name) = &self.name {
            validate_tenant_name(name)?;
        }
        Ok(())
    }
}
