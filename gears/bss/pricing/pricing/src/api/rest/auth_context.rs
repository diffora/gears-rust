//! Authentication-context extraction for the REST handlers.
//!
//! Every catalog surface requires an authenticated [`SecurityContext`]:
//! authoring is governed and audited by actor, and even the read surfaces are
//! tenant-scoped, so a request without one is refused with 401 rather than
//! silently treated as an anonymous identity.

use axum::extract::Extension;
use toolkit::api::canonical_prelude::CanonicalError;
use toolkit_security::SecurityContext;

use crate::api::rest::error::unauthenticated;

/// Extract the [`SecurityContext`] from the request extensions, returning 401
/// when it is missing, carries the anonymous all-zero placeholder, or lacks the
/// positive `subject_type` marker a real `AuthN` resolver always populates.
///
/// # Errors
/// [`CanonicalError`] (401) when no authenticated context is present.
pub(crate) fn require_authenticated(
    extension_ctx: Option<Extension<SecurityContext>>,
) -> Result<SecurityContext, CanonicalError> {
    let Some(Extension(ctx)) = extension_ctx else {
        return Err(unauthenticated());
    };
    if ctx.subject_id().is_nil() || ctx.subject_tenant_id().is_nil() {
        return Err(unauthenticated());
    }
    if ctx.subject_type().is_none() {
        return Err(unauthenticated());
    }
    Ok(ctx)
}

#[cfg(test)]
#[path = "auth_context_tests.rs"]
mod auth_context_tests;
