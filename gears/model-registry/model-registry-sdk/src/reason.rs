// Created: 2026-10-26 by Constructor Tech
// Updated: 2026-11-05 by Constructor Tech
//! Reason codes carried in `CanonicalError` contexts (DESIGN §4 Error
//! Handling).

/// `PermissionDenied` reasons (`context.reason`).
pub mod permission {
    /// The model's provider is disabled (`resolve_model`).
    pub const PROVIDER_DISABLED: &str = "PROVIDER_DISABLED";
    /// The model is pending, rejected, or revoked (`resolve_model`).
    pub const MODEL_NOT_APPROVED: &str = "MODEL_NOT_APPROVED";
    /// The PDP denied the operation.
    pub const NOT_PERMITTED: &str = "NOT_PERMITTED";
    /// A write addressed an inherited provider or model.
    pub const INHERITED_READ_ONLY: &str = "INHERITED_READ_ONLY";
}

/// `Aborted` reasons (`context.reason`). Both are retriable after a re-read.
pub mod aborted {
    /// `expected_version` (or `If-Match`) does not match the stored
    /// `version`, the provider or its model set changed during a type
    /// migration, or a resent `provider_settings`, `secret_ref`, or provider
    /// `gts_type` changed concurrently. Takes precedence over
    /// [`PROVIDER_TYPE_CHANGED`].
    pub const OPTIMISTIC_LOCK_FAILURE: &str = "OPTIMISTIC_LOCK_FAILURE";
    /// The provider's `gts_type` changed between settings validation and the
    /// model write (`create_model`, unconditional `update_model`).
    pub const PROVIDER_TYPE_CHANGED: &str = "PROVIDER_TYPE_CHANGED";
}
