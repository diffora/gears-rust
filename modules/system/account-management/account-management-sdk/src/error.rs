//! Account Management SDK public error type.
//!
//! Variant-per-failure shape: each enum case identifies what went wrong
//! semantically (mirrors the `mini-chat::DomainError` convention).
//! SDK consumers pattern-match on variants directly; AIP-193 / HTTP
//! mapping is performed at the canonical boundary in the impl crate
//! (`account-management::infra::sdk_error_mapping`).
//!
//! # Group-membership helpers
//!
//! Per-variant matching is precise but category-level handling (retry
//! on transient outages, surface as 404 regardless of which resource
//! was missing) is a recurring need. The `is_*` helpers below collapse
//! related variants into a single predicate so consumers do not need
//! to enumerate every variant — adding a new transient variant means
//! extending `is_unavailable()` in one place, not patching every
//! call site.

use thiserror::Error;

/// AM public error envelope.
#[derive(Error, Debug, Clone)]
#[non_exhaustive]
pub enum AccountManagementError {
    // ===================================================================
    // Tenant CRUD
    // ===================================================================
    /// Tenant with `tenant_id` does not exist (or is soft-deleted /
    /// provisioning — both surface as `NotFound` per AM contract).
    #[error("tenant {tenant_id} not found: {detail}")]
    TenantNotFound { tenant_id: String, detail: String },

    /// `IdP` user not found within the requested tenant scope. The
    /// `tenant_id` carried here is informational (the lookup scope);
    /// the discriminator for "the missing thing" is `user_id`.
    #[error("user {user_id} not found: {detail}")]
    UserNotFound { user_id: String, detail: String },

    /// Conversion request with `request_id` does not exist (or has
    /// been soft-deleted, or the caller's scope cannot reach it —
    /// existence-leak protection collapses all three into `NotFound`).
    #[error("conversion request {request_id} not found: {detail}")]
    ConversionRequestNotFound { request_id: String, detail: String },

    /// Unique-constraint violation when creating a tenant.
    #[error("tenant already exists: {detail}")]
    TenantAlreadyExists { detail: String },

    /// `tenant_type` reference is malformed or unknown.
    #[error("invalid tenant type: {detail}")]
    InvalidTenantType { detail: String },

    /// `tenant_type` is registered but not permitted for the requested
    /// placement (parent / depth / root constraint).
    #[error("tenant type not allowed for this placement: {detail}")]
    TenantTypeNotAllowed { detail: String },

    /// Hierarchy depth budget exceeded.
    #[error("tenant depth exceeded: {detail}")]
    TenantDepthExceeded { detail: String },

    /// Tenant still has child tenants; cannot be deleted/converted.
    #[error("tenant has child tenants")]
    TenantHasChildren,

    /// Tenant still owns active RG memberships; cannot be deleted.
    #[error("tenant still owns resources")]
    TenantHasResources,

    /// Root tenant cannot be deleted (delete operation refused).
    #[error("root tenant cannot be deleted")]
    RootTenantCannotDelete,

    /// Root tenant cannot be converted (conversion operation refused).
    #[error("root tenant cannot be converted")]
    RootTenantCannotConvert,

    /// Root tenant status is immutable — `suspend` / `unsuspend` refused.
    /// Symmetric with [`Self::RootTenantCannotDelete`]: the platform
    /// root is a singleton whose lifecycle state must not flip from
    /// the public API. Downstream modules that branch on
    /// `root.status` may take unexpected paths (read-only mode,
    /// refuse provisioning, etc.) without a documented recovery
    /// runbook.
    #[error("root tenant status cannot be changed")]
    RootTenantCannotChangeStatus,

    /// `IdP` plugin rejected the provisioning request shape BEFORE
    /// making any external call. Permanent client error — the
    /// `provisioning` row was compensated by the saga; nothing on
    /// the provider side to undo. Distinct from
    /// [`Self::InvalidRequest`] so the canonical envelope can carry
    /// the dotted-path `field` (e.g. `provisioning_metadata.realm_name`)
    /// the plugin localised the violation to, surfaced as
    /// `field_violations[0].field` with reason `IDP_INVALID_INPUT`.
    /// `field` is `None` when the plugin couldn't localise to a
    /// specific key; the canonical mapping then uses
    /// `"provisioning_metadata"` as the field key (the surface
    /// every `IdP` plugin shares).
    #[error("IdP provider rejected request shape: {detail}")]
    IdpInvalidInput {
        detail: String,
        field: Option<String>,
    },

    // ===================================================================
    // Conversion request
    // ===================================================================
    /// Another conversion request for the same tenant is still pending.
    #[error("a pending conversion request already exists: {request_id}")]
    PendingConversionExists { request_id: String },

    /// Approver/rejecter side does not match the conversion's target
    /// transition.
    #[error(
        "invalid actor for conversion transition: attempted={attempted_status} caller_side={caller_side}"
    )]
    InvalidActorForConversionTransition {
        attempted_status: String,
        caller_side: String,
    },

    /// Conversion request is already in a terminal state (approved or
    /// rejected); cannot be transitioned again.
    #[error("conversion request already resolved")]
    ConversionAlreadyResolved,

    // ===================================================================
    // Tenant Metadata
    // ===================================================================
    /// Metadata entry not found. Surfaces uniformly for both
    /// "`schema_id` is unknown to the types-registry" and "schema is
    /// registered but no row exists at `(tenant_id, schema_uuid)`" —
    /// AM intentionally does not distinguish the two on the wire so
    /// clients see a single `not_found` shape for every metadata
    /// lookup miss. `entry` carries the chained `schema_id` string
    /// the caller supplied (or, on rare orphan-row paths, the bare
    /// `schema_uuid`).
    #[error("metadata entry {entry} not found: {detail}")]
    MetadataEntryNotFound { entry: String, detail: String },

    /// Optimistic-lock precondition on
    /// [`crate::UpsertMetadataRequest::expected_version`] did not
    /// match the stored row's [`crate::MetadataEntry::version`]. The
    /// caller MUST re-read the entry, decide how to merge with the
    /// concurrent change, and re-issue the upsert with the updated
    /// `expected_version`. HTTP 409 (AIP-193 `Aborted`).
    #[error("metadata version mismatch for {entry}: expected v{expected}, stored v{current}")]
    MetadataVersionMismatch {
        entry: String,
        expected: i64,
        current: i64,
    },

    // ===================================================================
    // Generic validation / precondition (fallbacks)
    // ===================================================================
    /// Request shape rejected by validator (no typed variant).
    #[error("invalid request: {detail}")]
    InvalidRequest { detail: String },

    /// Metadata-payload validation rejected. Distinct from
    /// [`Self::InvalidRequest`] so the canonical envelope can route
    /// to `gts.cf.core.am.tenant_metadata.v1~` instead of the tenant
    /// default — both still map to AIP-193 `InvalidArgument` (HTTP
    /// 400). Producers raise this when the metadata payload itself or
    /// its `schema_id` is malformed (chain-shape, null body, GTS body
    /// validation), keeping [`Self::InvalidRequest`] for tenant-state
    /// guards.
    #[error("metadata validation failed: {detail}")]
    MetadataInvalidRequest { detail: String },

    /// State precondition violation not covered by a more specific
    /// variant (tenant deleted, type immutable, etc.).
    #[error("precondition failed: {detail}")]
    PreconditionFailed { detail: String },

    /// Deployment-level feature gate refused the operation. Distinct
    /// from [`Self::UnsupportedOperation`] (`IdP` capability gap) so
    /// callers can distinguish a configuration switch from a vendor
    /// gap without string matching.
    #[error("feature disabled: {detail}")]
    FeatureDisabled { detail: String },

    // ===================================================================
    // Authorization
    // ===================================================================
    /// PEP denied cross-tenant access (or AM-side ancestry walk
    /// rejected the call). HTTP 403.
    #[error("cross-tenant access denied")]
    CrossTenantDenied,

    // ===================================================================
    // Transactional
    // ===================================================================
    /// Storage retry budget exhausted for a serializable transaction.
    /// Treated as 409 per AIP-193 `Aborted`.
    #[error("serialization conflict: {detail}")]
    SerializationConflict { detail: String },

    // ===================================================================
    // IdP plugin contract
    // ===================================================================
    /// `IdP` plugin transport / availability failure. Surfaces as 503;
    /// distinct from generic [`Self::ServiceUnavailable`] so the
    /// bootstrap saga retry loop can pattern-match this specifically
    /// without losing the AIP-193 status mapping.
    #[error("IdP plugin unavailable")]
    IdpUnavailable,

    /// `IdP` plugin declared the operation unsupported in its current
    /// profile. HTTP 501.
    #[error("operation not supported by IdP provider")]
    UnsupportedOperation,

    // ===================================================================
    // Generic infra / fallback
    // ===================================================================
    /// Non-IdP transient infrastructure failure (DB transport, PDP
    /// evaluation, types-registry). HTTP 503 with optional
    /// `retry_after_seconds` hint.
    #[error("service unavailable: {detail}")]
    ServiceUnavailable {
        detail: String,
        retry_after_seconds: Option<u32>,
    },

    /// Unclassified internal failure. `detail` MUST be redacted at the
    /// construction site — the impl crate's classifier produces only
    /// DSN-free strings.
    #[error("internal error: {detail}")]
    Internal { detail: String },
}

impl AccountManagementError {
    // -------------------------------------------------------------------
    // Group-membership helpers
    //
    // These collapse related variants into a single predicate so
    // callers do not have to enumerate every variant for
    // category-level handling (backoff, retry, surface-as-404). Adding
    // a new variant to one of these categories means extending the
    // helper here in one place — call sites stay untouched.
    // -------------------------------------------------------------------

    /// `true` for any not-found shape (tenant, user, conversion
    /// request, metadata entry, metadata schema, …). Use when the
    /// caller wants to treat any missing resource uniformly (cache
    /// invalidation, "go back to list").
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::TenantNotFound { .. }
                | Self::UserNotFound { .. }
                | Self::ConversionRequestNotFound { .. }
                | Self::MetadataEntryNotFound { .. }
        )
    }

    /// `true` for any transient infrastructure outage where retry is
    /// the appropriate response. Covers `ServiceUnavailable` (generic
    /// infra) and `IdpUnavailable` (vendor plugin transport).
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::ServiceUnavailable { .. } | Self::IdpUnavailable)
    }

    /// `true` if the operation MAY succeed on a future retry: any
    /// transient outage plus serializable-retry exhaustion.
    /// `IntegrityCheckInProgress` is NOT included — caller is expected
    /// to back off until the in-flight check completes, not retry
    /// immediately.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        self.is_unavailable() || matches!(self, Self::SerializationConflict { .. })
    }

    /// `true` for request-shape rejections (HTTP 400 `InvalidArgument`).
    #[must_use]
    pub fn is_validation_error(&self) -> bool {
        matches!(
            self,
            Self::InvalidTenantType { .. }
                | Self::InvalidRequest { .. }
                | Self::MetadataInvalidRequest { .. }
                | Self::RootTenantCannotDelete
                | Self::RootTenantCannotConvert
                | Self::RootTenantCannotChangeStatus
                | Self::IdpInvalidInput { .. }
        )
    }

    /// `true` for state-precondition failures (HTTP 400
    /// `FailedPrecondition` per AIP-193).
    #[must_use]
    pub fn is_precondition_failed(&self) -> bool {
        matches!(
            self,
            Self::TenantTypeNotAllowed { .. }
                | Self::TenantDepthExceeded { .. }
                | Self::TenantHasChildren
                | Self::TenantHasResources
                | Self::InvalidActorForConversionTransition { .. }
                | Self::ConversionAlreadyResolved
                | Self::PreconditionFailed { .. }
                | Self::FeatureDisabled { .. }
        )
    }

    /// `true` for duplicate-on-create failures (HTTP 409
    /// `AlreadyExists` per AIP-193). Distinct from
    /// [`Self::is_precondition_failed`] because the duplicate-resource
    /// category carries the existing resource id as part of its
    /// envelope and is retryable only after the caller resolves the
    /// existing row.
    #[must_use]
    pub fn is_already_exists(&self) -> bool {
        matches!(
            self,
            Self::TenantAlreadyExists { .. } | Self::PendingConversionExists { .. }
        )
    }

    /// `true` for authorization denials (HTTP 403).
    #[must_use]
    pub fn is_permission_denied(&self) -> bool {
        matches!(self, Self::CrossTenantDenied)
    }

    // -------------------------------------------------------------------
    // Field accessors
    // -------------------------------------------------------------------

    /// Retry-after hint (seconds) for transient outages that carry
    /// one. Currently only [`Self::ServiceUnavailable`] populates it;
    /// other variants return `None`.
    #[must_use]
    pub fn retry_after_seconds(&self) -> Option<u32> {
        match self {
            Self::ServiceUnavailable {
                retry_after_seconds,
                ..
            } => *retry_after_seconds,
            _ => None,
        }
    }
}

#[cfg(test)]
#[cfg_attr(coverage_nightly, coverage(off))]
#[path = "error_tests.rs"]
mod error_tests;
