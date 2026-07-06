//! Domain types for multipart upload sessions and parts.
//!
//! @cpt-cf-file-storage-fr-multipart-upload

use time::OffsetDateTime;
use toolkit_macros::domain_model;
use uuid::Uuid;

use crate::domain::error::DomainError;

/// State of a multipart upload session.
#[domain_model]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MultipartUploadState {
    InProgress,
    Completed,
    Aborted,
}

impl MultipartUploadState {
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Aborted => "aborted",
        }
    }

    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "aborted" => Some(Self::Aborted),
            _ => None,
        }
    }
}

/// An in-flight multipart upload session.
///
/// `declared_size` and `part_size` were added by the
/// `multipart-coordinator` server-authoritative feature (§6).
#[domain_model]
#[derive(Debug, Clone)]
pub struct MultipartUploadSession {
    pub upload_id: Uuid,
    pub file_id: Uuid,
    pub version_id: Uuid,
    pub backend_upload_handle: String,
    pub state: MultipartUploadState,
    pub declared_mime: String,
    pub mime_validated: bool,
    /// Total file size declared at initiate time (bytes).
    pub declared_size: u64,
    /// Server-chosen plan unit (bytes, uniform except the final part).
    pub part_size: u64,
    pub created_at: OffsetDateTime,
    pub expires_at: OffsetDateTime,
}

/// One uploaded part of a multipart session.
#[domain_model]
#[derive(Debug, Clone)]
pub struct MultipartPart {
    pub upload_id: Uuid,
    pub part_number: u32,
    pub backend_etag: String,
    pub part_hash: Vec<u8>,
    pub size: i64,
    pub uploaded_at: OffsetDateTime,
}

// ── Server-authoritative parts plan (multipart-coordinator feature) ────────────

/// One planned part as returned to the client in the initiate response.
///
/// The `upload_url` is a sidecar signed URL containing the exact `size` claim.
/// The client must `PUT` exactly `size` bytes to `upload_url`.
///
/// @cpt-cf-file-storage-fr-multipart-upload
#[domain_model]
#[derive(Debug, Clone)]
pub struct MultipartPartPlan {
    /// 1-based part number (S3 convention).
    pub part_number: u32,
    /// Byte offset of this part within the final assembled object.
    pub offset: u64,
    /// Exact byte length of this part.
    pub size: u64,
    /// Sidecar signed URL the client `PUT`s this part's bytes to.
    pub upload_url: String,
}

/// The server-authoritative parts plan returned by `POST /files/{id}/multipart`.
///
/// @cpt-cf-file-storage-fr-multipart-upload
#[domain_model]
#[derive(Debug, Clone)]
pub struct MultipartPlan {
    pub upload_id: Uuid,
    pub version_id: Uuid,
    /// The hash algorithm used for per-part hashes (`"SHA-256"` in P2).
    pub part_hash_algorithm: String,
    /// Uniform part size (bytes); the final part may be smaller.
    pub part_size: u64,
    /// One entry per part, in ascending `part_number` order.
    pub parts: Vec<MultipartPartPlan>,
    /// Token expiry; all per-part URLs share this expiry.
    pub expires_at: OffsetDateTime,
}

/// Minimum part size used when the backend does not declare a minimum.
///
/// 5 MiB is the S3 minimum for all parts except the last. This value also
/// doubles as the lower bound of the sane range that a client-supplied
/// `preferred_part_size` is validated against at the service boundary
/// (`MultipartService::initiate_multipart_upload`, P2 remediation 2.11).
pub const DEFAULT_MIN_PART_SIZE: u64 = 5 * 1024 * 1024;

/// Maximum accepted `preferred_part_size` client hint (P2 remediation 2.11).
///
/// 5 GiB is S3's absolute maximum part size. Values above this cannot be a
/// legitimate part-size preference; they are rejected at the service
/// boundary before ever reaching [`compute_plan`]. The checked arithmetic in
/// [`compute_plan`]/[`round_up_to`] below is kept regardless, as
/// defense-in-depth for callers that bypass that boundary.
pub const MAX_PART_SIZE: u64 = 5 * 1024 * 1024 * 1024;

/// Compute the server-chosen `part_size` and generate the plan skeleton
/// (without URLs — those are injected by `MultipartService`).
///
/// Rules (FEATURE §3):
/// - `part_size = max(preferred, backend_min)` rounded up to the nearest
///   multiple of `DEFAULT_MIN_PART_SIZE` (BLAKE3-friendly alignment deferred,
///   SHA-256 is used in P2).
/// - `parts = ceil(declared_size / part_size)`.
/// - The last part's `size` is `declared_size - (parts - 1) * part_size`.
///
/// One raw part entry from `compute_plan`: `(part_number, offset, size)`.
pub type RawPartEntry = (u32, u64, u64);

/// Returns `(part_size, parts_count)` ready to be used by the caller.
///
/// # Errors
/// Returns [`DomainError::Validation`] if the part-size arithmetic would
/// overflow `u64`. Callers are expected to have already validated
/// `preferred_part_size` against a sane range (P2 remediation 2.11); this is
/// a defense-in-depth guard against a huge/adversarial value reaching this
/// function by another path, rather than panicking or silently wrapping.
pub fn compute_plan(
    declared_size: u64,
    preferred_part_size: Option<u64>,
    backend_min_part_size: Option<u64>,
) -> Result<(u64, Vec<RawPartEntry>), DomainError> {
    let min = backend_min_part_size.unwrap_or(DEFAULT_MIN_PART_SIZE);
    let preferred = preferred_part_size.unwrap_or(min);
    // Part size = max(preferred, backend_min), rounded up to the nearest `min`.
    let raw = preferred.max(min);
    let part_size = round_up_to(raw, min).ok_or_else(|| {
        DomainError::validation(
            "preferred_part_size",
            format!("part-size computation overflowed for preferred={preferred}, min={min}"),
        )
    })?;

    if declared_size == 0 {
        return Ok((part_size, vec![(1, 0, 0)]));
    }

    let n_parts = declared_size.div_ceil(part_size);
    let capacity = usize::try_from(n_parts).unwrap_or(usize::MAX);
    let mut parts = Vec::with_capacity(capacity);
    for i in 0..n_parts {
        let offset = i.checked_mul(part_size).ok_or_else(|| {
            DomainError::validation(
                "preferred_part_size",
                format!("part offset overflowed at part {}", i + 1),
            )
        })?;
        let size = if i + 1 == n_parts {
            declared_size - offset
        } else {
            part_size
        };
        let part_number = u32::try_from(i + 1).unwrap_or(u32::MAX);
        parts.push((part_number, offset, size));
    }
    Ok((part_size, parts))
}

/// Round `value` up to the next multiple of `align` (≥ 1).
///
/// Uses checked arithmetic: returns `None` on overflow instead of
/// panicking (under overflow-checks) or silently wrapping to a tiny value
/// (P2 remediation 2.11).
fn round_up_to(value: u64, align: u64) -> Option<u64> {
    if align == 0 {
        return Some(value);
    }
    value.div_ceil(align).checked_mul(align)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P2 remediation 2.11: a near-`u64::MAX` value must not panic (under
    /// overflow-checks) or silently wrap to a tiny `part_size` — it must be
    /// reported as `None` so the caller can turn it into a domain error.
    /// `round_up_to` is private, so this is a same-module unit test rather
    /// than an integration test in `tests/multipart_test.rs`.
    #[test]
    fn round_up_to_does_not_overflow_on_max_input() {
        assert_eq!(round_up_to(u64::MAX, DEFAULT_MIN_PART_SIZE), None);
        assert_eq!(round_up_to(u64::MAX, u64::MAX), Some(u64::MAX));
        assert_eq!(round_up_to(1, u64::MAX), Some(u64::MAX));
        // Sanity: ordinary inputs still round up correctly.
        assert_eq!(round_up_to(7, 5), Some(10));
        assert_eq!(round_up_to(10, 5), Some(10));
    }

    /// `compute_plan` must surface the overflow as a domain error instead of
    /// panicking, even when called directly with an adversarial
    /// `preferred_part_size` that bypasses the service-boundary validation.
    #[test]
    fn compute_plan_returns_validation_error_on_overflowing_preferred_part_size() {
        let err = compute_plan(u64::MAX, Some(u64::MAX), None).unwrap_err();
        assert!(matches!(err, DomainError::Validation { .. }));
    }
}
