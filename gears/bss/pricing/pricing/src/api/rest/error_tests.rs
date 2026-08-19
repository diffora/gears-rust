//! Tests for the REST-level error mapping.

use super::{authz_error_to_canonical, unauthenticated};
use crate::authz::{AuthzError, DeniedAttempt};

/// A denial carrying the operand set the funnel now emits.
fn denial(reason: &str) -> AuthzError {
    AuthzError::Denied(Box::new(DeniedAttempt {
        subject_principal_id: uuid::Uuid::from_u128(0x_5e_eb),
        subject_tenant_id: uuid::Uuid::from_u128(0x_7e_11),
        resource_type: crate::authz::labels::PLAN.to_owned(),
        action: "write".to_owned(),
        resource_id: None,
        owner_tenant_id: None,
        reason: reason.to_owned(),
    }))
}

#[test]
fn a_denial_is_403_and_keeps_the_deny_reason() {
    let err = authz_error_to_canonical(denial("no plan x write"));

    assert_eq!(err.status_code(), 403);
    assert!(format!("{err:?}").contains("no plan x write"));
}

#[test]
fn a_denial_tells_the_caller_the_reason_and_nothing_about_the_subject() {
    // The operand set exists for the log, not for the wire: a 403 that echoed the
    // principal id back would hand an unauthenticated prober a way to confirm one.
    let err = authz_error_to_canonical(denial("no plan x write"));
    let rendered = format!("{err:?}");

    assert!(!rendered.contains(&uuid::Uuid::from_u128(0x_5e_eb).to_string()));
    assert!(!rendered.contains(&uuid::Uuid::from_u128(0x_7e_11).to_string()));
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
