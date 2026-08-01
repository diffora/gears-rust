//! REST-level error mapping: the authz gate and missing-authentication cases.
//!
//! Domain rejections reach the wire through the single
//! `From<DomainError> for CanonicalError` ladder
//! ([`crate::infra::error_mapping`]), never from here — this module handles only
//! what happens before a handler reaches the domain at all.

use toolkit::api::canonical_prelude::{CanonicalError, resource_error};

use crate::authz::AuthzError;

/// Stamps `context.resource_type` on the canonical error.
#[resource_error(gts_id!("cf.bss.pricing.plan.v1~"))]
struct PricingResourceError;

/// Map an [`AuthzError`] from the PEP gate to a [`CanonicalError`].
///
/// `Denied` becomes a 403 carrying the deny reason. `Unavailable` becomes a
/// fail-closed 503 whose diagnostic stays server-side: a PDP outage must not
/// degrade into an allow, and it must not tell the caller why the policy engine
/// is unhappy either.
pub(crate) fn authz_error_to_canonical(err: AuthzError) -> CanonicalError {
    match err {
        AuthzError::Denied(reason) => PricingResourceError::permission_denied()
            .with_reason(reason)
            .create(),
        AuthzError::Unavailable(detail) => {
            tracing::error!(detail, "authorization service unavailable");
            CanonicalError::service_unavailable().create()
        }
    }
}

/// Build a 401 for a request without an authenticated `SecurityContext` —
/// distinct from a permission denial (403), which means the caller is known and
/// not allowed.
pub(crate) fn unauthenticated() -> CanonicalError {
    CanonicalError::unauthenticated()
        .with_reason("AUTHENTICATION_REQUIRED")
        .create()
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod error_tests;
