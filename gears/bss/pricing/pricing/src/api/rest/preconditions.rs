//! Required strong row tags, idempotency keys and canonical request parsing.

use crate::infra::error_mapping::DomainError;
use axum::http::{HeaderMap, header::IF_MATCH};
use serde::Serialize;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Optimistic version carried by a strong decimal entity tag.
pub struct RowVersion(u64);

impl RowVersion {
    #[must_use]
    /// Wrap a row version.
    pub const fn new(version: u64) -> Self {
        Self(version)
    }

    #[must_use]
    /// Read the version number.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Read a stored version.
    ///
    /// # Errors
    /// Rejects negative database values.
    pub fn from_stored(stored: i64) -> Result<Self, DomainError> {
        u64::try_from(stored).map(Self).map_err(|_| {
            DomainError::Internal(format!(
                "row_version column holds a negative value: {stored}"
            ))
        })
    }

    /// Convert to the database integer.
    ///
    /// # Errors
    /// Rejects versions exceeding the database range.
    pub fn to_stored(self) -> Result<i64, DomainError> {
        i64::try_from(self.0).map_err(|_| {
            DomainError::Internal(format!(
                "row_version {} exceeds the bigint column range",
                self.0
            ))
        })
    }

    #[must_use]
    /// Render one strong decimal entity tag.
    pub fn to_etag(self) -> String {
        format!("\"{}\"", self.0)
    }

    /// Parse one strong decimal entity tag.
    ///
    /// # Errors
    /// Rejects weak, wildcard, list and nondecimal tags.
    pub fn from_etag(raw: &str) -> Result<Self, DomainError> {
        let refuse = |why: &str| DomainError::InvalidRequest(format!("If-Match {raw}: {why}"));
        let digits = strong_tag_body(raw)?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(refuse("the tag must quote one or more ASCII digits"));
        }
        digits
            .parse::<u64>()
            .map(Self)
            .map_err(|_| refuse("the version is past the representable range"))
    }
}

/// Unquote one strong entity tag.
///
/// # Errors
/// Rejects weak, wildcard, list and unquoted tags.
pub fn strong_tag_body(raw: &str) -> Result<&str, DomainError> {
    let refuse = |why: &str| DomainError::InvalidRequest(format!("If-Match {raw}: {why}"));
    let tag = raw.trim();

    if tag == "*" {
        return Err(refuse(
            "the wildcard matches any version and would overwrite whichever one is current",
        ));
    }
    if tag.starts_with("W/") {
        return Err(refuse(
            "RFC 9110 forbids a weak validator here; a weak comparison cannot decide whether a write is safe",
        ));
    }
    if tag.contains(',') {
        return Err(refuse(
            "one entity tag is expected, and a list does not say which version was read",
        ));
    }
    tag.strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .ok_or_else(|| refuse("a strong entity tag is wrapped in double quotes"))
}

impl fmt::Display for RowVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Canonical header name for the required client key.
pub const IDEMPOTENCY_KEY: &str = "idempotency-key";

const MAX_KEY_LEN: usize = 255;

#[must_use]
/// Render the row version for an `ETag` response header.
pub fn etag(version: RowVersion) -> String {
    version.to_etag()
}

/// Read the required strong row-version header.
///
/// # Errors
/// Rejects missing, malformed or unsupported validators.
pub fn if_match(headers: &HeaderMap) -> Result<RowVersion, DomainError> {
    let Some(raw) = headers.get(IF_MATCH) else {
        return Err(DomainError::InvalidRequest(
            "If-Match is required on this verb: a mutation of a draft row \
             asserts the version it was authored against (D-141), and an \
             unconditional write would overwrite a concurrent editor's work"
                .to_owned(),
        ));
    };
    let raw = raw.to_str().map_err(|_| {
        DomainError::InvalidRequest("If-Match: the header value is not valid UTF-8".to_owned())
    })?;
    RowVersion::from_etag(raw)
}

/// Read a bounded, printable client idempotency key.
///
/// # Errors
/// Rejects missing, empty, oversized or non-ASCII keys.
pub fn idempotency_key(headers: &HeaderMap) -> Result<String, DomainError> {
    let refuse = |why: &str| DomainError::InvalidRequest(format!("Idempotency-Key: {why}"));
    let Some(raw) = headers.get(IDEMPOTENCY_KEY) else {
        return Err(refuse(
            "this operation is guarded at-most-once and requires the header; \
             an unguarded create on a governed authoring plane is the retry \
             hazard the gate exists for",
        ));
    };
    let key = raw
        .to_str()
        .map_err(|_| refuse("the header value is not valid UTF-8"))?;
    if key.is_empty() {
        return Err(refuse("an empty key names nothing"));
    }
    if key.len() > MAX_KEY_LEN {
        return Err(refuse(
            "the key is longer than the 255 characters this surface stores",
        ));
    }
    if !key.bytes().all(|b| b.is_ascii_graphic() || b == b' ') {
        return Err(refuse(
            "the key must be printable ASCII; it is stored and echoed in refusals",
        ));
    }
    Ok(key.to_owned())
}

/// Parse JSON while keeping malformed bodies in the canonical 400 envelope.
///
/// # Errors
/// Returns an invalid-request error for empty or malformed bodies.
pub fn parse_body<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, DomainError> {
    if body.is_empty() {
        return Err(DomainError::InvalidRequest(
            "the request body is empty; send a JSON object, `{}` for an empty one".to_owned(),
        ));
    }
    serde_json::from_slice(body)
        .map_err(|e| DomainError::InvalidRequest(format!("the request body is not readable: {e}")))
}

/// Hash the parsed request's serialization, using ordered maps for stable object order.
///
/// # Errors
/// Returns an internal error when serialization fails.
pub fn request_digest<T: Serialize>(request: &T) -> Result<Vec<u8>, DomainError> {
    let canonical = serde_json::to_string(request).map_err(|e| {
        DomainError::Internal(format!("cannot render the request for its digest: {e}"))
    })?;
    Ok(payload_hash(&canonical))
}

/// Hash canonical payload bytes with the existing SHA-256 provider.
#[must_use]
pub fn payload_hash(canonical: &str) -> Vec<u8> {
    aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, canonical.as_bytes())
        .as_ref()
        .to_vec()
}

#[cfg(test)]
#[path = "preconditions_tests.rs"]
mod preconditions_tests;
