//! The quantum every authored instant is held to, and the refusal that keeps a
//! finer one from being silently moved.

use time::OffsetDateTime;

use super::{check_quantum, format_rfc3339, is_quantized, utc_ymd_hms};
use crate::domain::error::DomainError;

/// The cutover instant, on the quantum.
fn cutover() -> OffsetDateTime {
    utc_ymd_hms(2026, 8, 2, 12, 0, 0)
}

#[test]
fn whole_milliseconds_are_expressible_and_finer_ones_are_not() {
    assert!(is_quantized(cutover()));
    assert!(is_quantized(
        cutover()
            .replace_nanosecond(123_000_000)
            .expect("valid nanos")
    ));
    assert!(
        !is_quantized(
            cutover()
                .replace_nanosecond(123_400_000)
                .expect("valid nanos")
        ),
        "a microsecond below the quantum is precision the catalog cannot compare"
    );
    assert!(!is_quantized(
        cutover().replace_nanosecond(1).expect("valid nanos")
    ));
}

#[test]
fn a_finer_instant_is_refused_rather_than_truncated() {
    // The whole point of the code: the value the author submitted is not quietly
    // moved to the quantum. A truncating producer and a non-truncating consumer
    // agree until the day they do not, and `cohort` is matched for equality
    // across a gear boundary, so the divergence surfaces as a generation nobody
    // can find rather than as an error.
    let authored = cutover()
        .replace_nanosecond(500_001_000)
        .expect("valid nanos");

    let Err(DomainError::TimestampPrecisionExceeded(detail)) = check_quantum("cohort", authored)
    else {
        panic!("a sub-millisecond instant must be refused");
    };
    assert!(
        detail.contains("cohort"),
        "the refusal names the field the author has to fix, got: {detail}"
    );
    assert!(
        detail.contains("2026-08-02"),
        "and the instant it refused, got: {detail}"
    );
}

#[test]
fn format_rfc3339_keeps_the_millisecond_z_form_stored_keys_use() {
    assert_eq!(
        format_rfc3339(utc_ymd_hms(2099, 4, 1, 0, 0, 0)),
        "2099-04-01T00:00:00.000Z"
    );
}

#[test]
fn an_instant_on_the_quantum_passes_every_authored_field() {
    for field in ["cohort", "grandfatherUntil", "availableFrom", "availableTo"] {
        assert!(check_quantum(field, cutover()).is_ok(), "field: {field}");
    }
}
