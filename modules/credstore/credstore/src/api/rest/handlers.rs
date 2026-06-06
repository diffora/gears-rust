//! REST handlers for the credstore module.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Extension, Path};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use credstore_sdk::{SecretRef, SecretValue};
use modkit::api::canonical_prelude::*;
use modkit_security::SecurityContext;

use super::dto::{CreateSecretRequestDto, GetSecretResponseDto, UpdateSecretRequestDto};
use crate::domain::error::DomainError;
use crate::domain::secret::model::WritePrecondition;
use crate::domain::secret::service::Service;

/// Concrete service alias for the handlers.
pub(crate) type ConcreteService = Service;

/// Parse an optional `If-Match` precondition (RFC 7232). The GET handler emits
/// a strong `"<version>"` validator, so we accept `*` (target must exist) or a
/// single quoted version integer; anything else is a typed 400.
fn parse_if_match(
    headers: &axum::http::HeaderMap,
) -> Result<Option<WritePrecondition>, DomainError> {
    let Some(raw) = headers.get(axum::http::header::IF_MATCH) else {
        return Ok(None);
    };
    let value = raw
        .to_str()
        .map_err(|_| DomainError::InvalidPrecondition {
            detail: "If-Match header is not valid ASCII".to_owned(),
        })?
        .trim();
    if value == "*" {
        return Ok(Some(WritePrecondition::Exists));
    }
    // Strong validator only: `"<version>"`. Weak (`W/"..."`) is not valid for writes.
    let version = value
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .and_then(|inner| inner.parse::<i64>().ok())
        .ok_or_else(|| DomainError::InvalidPrecondition {
            detail: "If-Match must be `*` or a quoted version ETag".to_owned(),
        })?;
    Ok(Some(WritePrecondition::Version(version)))
}

/// `POST /credstore/v1/secrets`
///
/// # Errors
///
/// Returns a canonical `Problem` envelope on invalid reference (400), access denied (403),
/// conflict (409), or service unavailable (503).
pub async fn create_secret(
    uri: axum::http::Uri,
    Extension(ctx): Extension<SecurityContext>,
    Extension(svc): Extension<Arc<ConcreteService>>,
    Json(body): Json<CreateSecretRequestDto>,
) -> ApiResult<impl IntoResponse> {
    let key = SecretRef::new(body.reference).map_err(|e| {
        CanonicalError::from(DomainError::InvalidSecretRef {
            detail: e.to_string(),
        })
    })?;
    svc.put(
        &ctx,
        &key,
        SecretValue::from(body.value),
        body.sharing.into(),
        true,
        None,
    )
    .await?;
    let location = format!("{}/{}", uri.path().trim_end_matches('/'), key.as_ref());
    Ok((
        StatusCode::CREATED,
        [(axum::http::header::LOCATION, location)],
    )
        .into_response())
}

/// `PUT /credstore/v1/secrets/{ref}`
///
/// Honours an optional `If-Match` precondition: a stale version yields a
/// canonical `Aborted` (409, `OPTIMISTIC_LOCK_FAILURE`).
///
/// # Errors
///
/// Returns a canonical `Problem` envelope on invalid reference / malformed
/// `If-Match` (400), access denied (403), unsupported transition (400),
/// version precondition failure (409), or service unavailable (503).
pub async fn put_secret(
    Extension(ctx): Extension<SecurityContext>,
    Extension(svc): Extension<Arc<ConcreteService>>,
    Path(reference): Path<String>,
    headers: axum::http::HeaderMap,
    Json(body): Json<UpdateSecretRequestDto>,
) -> ApiResult<impl IntoResponse> {
    let precondition = parse_if_match(&headers)?;
    let key = SecretRef::new(reference).map_err(|e| {
        CanonicalError::from(DomainError::InvalidSecretRef {
            detail: e.to_string(),
        })
    })?;
    svc.put(
        &ctx,
        &key,
        SecretValue::from(body.value),
        body.sharing.into(),
        false,
        precondition,
    )
    .await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `GET /credstore/v1/secrets/{ref}`
///
/// # Errors
///
/// Returns a canonical `Problem` envelope on invalid reference (400), access denied (403),
/// not found (404), or service unavailable (503).
pub async fn get_secret(
    Extension(ctx): Extension<SecurityContext>,
    Extension(svc): Extension<Arc<ConcreteService>>,
    Path(reference): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let key = SecretRef::new(reference).map_err(|e| {
        CanonicalError::from(DomainError::InvalidSecretRef {
            detail: e.to_string(),
        })
    })?;
    match svc.get(&ctx, &key).await? {
        Some(ref resp) => {
            // Reject (don't lossily decode) a non-UTF-8 value before building headers.
            let dto = GetSecretResponseDto::try_from_response(resp)?;
            // Strong validator; `version` is the per-row monotonic counter.
            let etag = format!("\"{}\"", resp.version);
            Ok((
                StatusCode::OK,
                [
                    (axum::http::header::ETAG, etag),
                    // Secret material must never be cached by intermediaries.
                    (axum::http::header::CACHE_CONTROL, "no-store".to_owned()),
                ],
                Json(dto),
            )
                .into_response())
        }
        None => Err(CanonicalError::from(DomainError::NotFound)),
    }
}

/// `DELETE /credstore/v1/secrets/{ref}`
///
/// Honours an optional `If-Match` precondition: a stale version yields a
/// canonical `Aborted` (409, `OPTIMISTIC_LOCK_FAILURE`).
///
/// # Errors
///
/// Returns a canonical `Problem` envelope on invalid reference / malformed
/// `If-Match` (400), access denied (403), not found (404), version precondition
/// failure (409), or service unavailable (503).
pub async fn delete_secret(
    Extension(ctx): Extension<SecurityContext>,
    Extension(svc): Extension<Arc<ConcreteService>>,
    Path(reference): Path<String>,
    headers: axum::http::HeaderMap,
) -> ApiResult<impl IntoResponse> {
    let precondition = parse_if_match(&headers)?;
    let key = SecretRef::new(reference).map_err(|e| {
        CanonicalError::from(DomainError::InvalidSecretRef {
            detail: e.to_string(),
        })
    })?;
    svc.delete(&ctx, &key, precondition).await?;
    Ok(StatusCode::NO_CONTENT.into_response())
}
