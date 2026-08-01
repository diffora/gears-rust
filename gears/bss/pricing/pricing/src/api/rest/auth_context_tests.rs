//! Tests for the authentication-context extractor.

use super::require_authenticated;

#[test]
fn a_request_without_a_context_is_refused() {
    let err = require_authenticated(None).expect_err("no context is a 401");

    assert_eq!(err.status_code(), 401);
}
