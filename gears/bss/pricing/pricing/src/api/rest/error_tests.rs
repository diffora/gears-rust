//! Tests for the REST-level error mapping.

use super::{authz_error_to_canonical, unauthenticated};
use crate::authz::AuthzError;

#[test]
fn a_denial_is_403_and_keeps_the_deny_reason() {
    let err = authz_error_to_canonical(AuthzError::Denied("no plan x write".to_owned()));

    assert_eq!(err.status_code(), 403);
    assert!(format!("{err:?}").contains("no plan x write"));
}

#[test]
fn a_pdp_outage_fails_closed_as_503_and_leaks_no_diagnostic() {
    // Never an allow, and never an explanation of the policy engine's internals
    // to an unauthenticated-for-all-we-know caller.
    let err = authz_error_to_canonical(AuthzError::Unavailable(
        "pdp connect timeout to 10.0.0.9".to_owned(),
    ));

    assert_eq!(err.status_code(), 503);
    assert!(!format!("{err:?}").contains("10.0.0.9"));
}

#[test]
fn a_missing_identity_is_401_not_403() {
    // 403 would tell an anonymous caller that the resource exists and that it
    // is merely not allowed to it.
    assert_eq!(unauthenticated().status_code(), 401);
}
