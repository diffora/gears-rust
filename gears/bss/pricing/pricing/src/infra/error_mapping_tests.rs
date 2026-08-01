//! Exhaustive `DomainError` → `CanonicalError` ladder check: every variant maps
//! to its expected canonical category, and the fail-closed rejections carry the
//! agreed wire code. This locks the machine-readable error contract — the codes
//! are what a consumer branches on once the coarse category is known.

use toolkit::api::canonical_prelude::CanonicalError;

use crate::domain::error::DomainError;
use crate::domain::validation::ValidationReport;

fn status(err: DomainError) -> u16 {
    CanonicalError::from(err).status_code()
}

fn rendered(err: DomainError) -> String {
    // The wire code lives in the problem body, not the status line; rendering
    // through Debug is enough to assert it survives the ladder.
    format!("{:?}", CanonicalError::from(err))
}

#[test]
fn conflicts_are_409() {
    // The three Foundation-owned conflicts. A caller resolves them by refetching
    // and retrying, which is exactly what 409 tells it to do.
    let detail = || "detail".to_owned();

    assert_eq!(status(DomainError::DuplicateScopeKey(detail())), 409);
    assert_eq!(status(DomainError::StaleVersion(detail())), 409);
    assert_eq!(
        status(DomainError::IdempotencyPayloadMismatch(detail())),
        409
    );
}

#[test]
fn fail_closed_publish_rejections_render_400_not_the_documented_422() {
    // The design set's 422s are architectural: the platform's canonical family
    // has no 422 (FailedPrecondition renders 400), so the code — not the status
    // — is the discriminator (Foundation 3.3). Pinned so the rendering is a
    // checked fact rather than a surprise at the first consumer.
    let detail = || "detail".to_owned();

    assert_eq!(status(DomainError::RoundingPolicyUnresolved(detail())), 400);
    assert_eq!(status(DomainError::PrecisionExceeded(detail())), 400);
    assert_eq!(status(DomainError::AmountNegative(detail())), 400);
    assert_eq!(status(DomainError::CurrencyInvalid(detail())), 400);
    assert_eq!(status(DomainError::FixtureMissing(detail())), 400);
    assert_eq!(status(DomainError::LifecycleForbidden(detail())), 400);
}

#[test]
fn the_wire_codes_survive_the_ladder() {
    let detail = || "detail".to_owned();

    assert!(
        rendered(DomainError::RoundingPolicyUnresolved(detail()))
            .contains("ROUNDING_POLICY_UNRESOLVED")
    );
    assert!(rendered(DomainError::PrecisionExceeded(detail())).contains("PRECISION_EXCEEDED"));
    assert!(rendered(DomainError::AmountNegative(detail())).contains("AMOUNT_NEGATIVE"));
    assert!(rendered(DomainError::CurrencyInvalid(detail())).contains("CURRENCY_INVALID"));
    assert!(rendered(DomainError::FixtureMissing(detail())).contains("FIXTURE_MISSING"));
    assert!(rendered(DomainError::DuplicateScopeKey(detail())).contains("DUPLICATE_SCOPE_KEY"));
    assert!(rendered(DomainError::StaleVersion(detail())).contains("STALE_VERSION"));
    assert!(
        rendered(DomainError::IdempotencyPayloadMismatch(detail()))
            .contains("IDEMPOTENCY_PAYLOAD_MISMATCH")
    );
}

#[test]
fn the_validation_envelope_carries_every_blocking_violation() {
    // One round trip must be enough to remediate a plan: if the envelope
    // truncated, an author would publish N times to discover N problems.
    let mut report = ValidationReport::default();
    report.violate("TIER_BANDS_OVERLAP", "price-1", "bands 2 and 3 overlap");
    report.violate("TIER_TOP_BAND_CLOSED", "price-1", "top band must be open");
    report.warn("ADVISORY", "price-1", "not blocking");

    let body = rendered(DomainError::ValidationFailed(report));

    assert!(body.contains("TIER_BANDS_OVERLAP"));
    assert!(body.contains("TIER_TOP_BAND_CLOSED"));
    // The advisory rides the success path's report, never the error envelope.
    assert!(!body.contains("ADVISORY"));
}

#[test]
fn unavailability_is_503_and_keeps_its_diagnostic_server_side() {
    // An operator needs to know which dependency is down; a caller only needs
    // to know to retry, and the registry's internals are not its business.
    let body = rendered(DomainError::CatalogVersionUnavailable(
        "registry refused connection at 10.0.0.7".to_owned(),
    ));

    assert_eq!(
        status(DomainError::CatalogVersionUnavailable("x".to_owned())),
        503
    );
    assert_eq!(
        status(DomainError::ReadModelUnavailable("x".to_owned())),
        503
    );
    assert!(!body.contains("10.0.0.7"));
}

#[test]
fn absent_and_scoped_out_share_one_answer() {
    assert_eq!(
        status(DomainError::NotFound {
            subject: "plan".to_owned(),
            id: "3f2a".to_owned(),
        }),
        404
    );
}

#[test]
fn a_bad_request_is_400_and_an_internal_fault_is_500() {
    assert_eq!(status(DomainError::InvalidRequest("x".to_owned())), 400);
    assert_eq!(status(DomainError::Internal("x".to_owned())), 500);
}

#[test]
fn a_registry_failure_becomes_the_fail_closed_domain_variant() {
    use crate::domain::ports::{CatalogVersionRegistryError, registry_failure};

    // Every registry failure lands on the same answer: no version, no publish.
    // Inventing one locally would make this gear a second incrementer.
    for err in [
        CatalogVersionRegistryError::Unconfigured,
        CatalogVersionRegistryError::Unreachable("down".to_owned()),
        CatalogVersionRegistryError::Rejected("unknown sku".to_owned()),
        CatalogVersionRegistryError::Internal("boom".to_owned()),
    ] {
        assert!(matches!(
            registry_failure(&err),
            DomainError::CatalogVersionUnavailable(_)
        ));
    }
}
