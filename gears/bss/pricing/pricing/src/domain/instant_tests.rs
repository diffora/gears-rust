//! The quantum every authored instant is held to, and the refusal that keeps a
//! finer one from being silently moved.

use time::OffsetDateTime;

use super::{
    check_quantum, format_rfc3339, format_rfc3339_offset, format_rfc3339_serde, is_quantized,
    rfc3339, try_utc_ymd_hms, utc_ymd_hms,
};
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

// chrono's `Serialize` rule, byte for byte: nothing on a whole second, else
// `.SSS` / `.SSSSSS` / nine digits — whichever first divides the nanoseconds —
// and `Z`. A renderer printing minimal digits (`time`'s `Rfc3339`: `.1`) or a
// fixed width (`format_rfc3339`: `.000`) fails at least one arm. The first case
// is the frozen-payload expectation `rest_customer_groups.rs` carries.
#[test]
fn serde_form_reproduces_chrono_serialize_byte_for_byte() {
    let at = |nanos: i128| {
        OffsetDateTime::from_unix_timestamp_nanos(1_757_000_000 * 1_000_000_000 + nanos)
            .expect("a representable instant")
    };
    assert_eq!(format_rfc3339_serde(at(0)), "2025-09-04T15:33:20Z");
    assert_eq!(
        format_rfc3339_serde(at(100_000_000)),
        "2025-09-04T15:33:20.100Z",
        "a tenth of a second is `.100`, not `.1`"
    );
    assert_eq!(
        format_rfc3339_serde(at(123_456_000)),
        "2025-09-04T15:33:20.123456Z"
    );
    assert_eq!(
        format_rfc3339_serde(at(123_456_789)),
        "2025-09-04T15:33:20.123456789Z"
    );
    // and it is not the Millis form the keys use
    assert_ne!(format_rfc3339_serde(at(0)), format_rfc3339(at(0)));
}

// chrono's `to_rfc3339()`: the serde grouping with `+00:00`. This is the form
// the grandfather-until subject and `pending_ref` were stored under on main, so
// a retry across the migration must reproduce it byte for byte.
#[test]
fn offset_form_reproduces_chrono_to_rfc3339_byte_for_byte() {
    let at = |nanos: i128| {
        OffsetDateTime::from_unix_timestamp_nanos(1_757_000_000 * 1_000_000_000 + nanos)
            .expect("a representable instant")
    };
    assert_eq!(format_rfc3339_offset(at(0)), "2025-09-04T15:33:20+00:00");
    assert_eq!(
        format_rfc3339_offset(at(100_000_000)),
        "2025-09-04T15:33:20.100+00:00"
    );
    assert_eq!(
        format_rfc3339_offset(at(123_456_000)),
        "2025-09-04T15:33:20.123456+00:00"
    );
    assert_eq!(
        format_rfc3339_offset(at(123_456_789)),
        "2025-09-04T15:33:20.123456789+00:00"
    );
    // three forms, three contracts, none interchangeable
    let whole = at(0);
    assert_ne!(format_rfc3339_offset(whole), format_rfc3339_serde(whole));
    assert_ne!(format_rfc3339_offset(whole), format_rfc3339(whole));
}

// ---------------------------------------------------------------------------
// The serde boundary: every instant that enters is a UTC one.
// ---------------------------------------------------------------------------

/// A request shape with both field forms the DTOs use.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct Carrier {
    #[serde(with = "rfc3339")]
    at: OffsetDateTime,
    #[serde(default, with = "rfc3339::option")]
    maybe: Option<OffsetDateTime>,
}

/// The premise, measured on `time` itself rather than asserted: its own
/// `rfc3339` keeps the written offset, and the UTC form of this instant is
/// year 10000, which `time` cannot hold — so `to_offset(UTC)` on it panics.
#[test]
fn time_alone_admits_an_instant_that_has_no_utc_form() {
    let parsed = OffsetDateTime::parse(
        "9999-12-31T23:00:00-05:00",
        &time::format_description::well_known::Rfc3339,
    )
    .expect("time's rfc3339 parses the local form");
    assert_eq!(parsed.offset().whole_hours(), -5);
    assert!(parsed.checked_to_offset(time::UtcOffset::UTC).is_none());
}

#[test]
fn a_request_instant_arrives_in_utc_whatever_offset_it_was_written_in() {
    let carrier: Carrier = serde_json::from_str(
        r#"{"at":"2026-03-01T05:00:00-05:00","maybe":"2026-03-01T05:00:00.25+02:00"}"#,
    )
    .expect("both instants are representable");
    assert_eq!(carrier.at.offset(), time::UtcOffset::UTC);
    assert_eq!(format_rfc3339(carrier.at), "2026-03-01T10:00:00.000Z");
    assert_eq!(
        carrier.maybe.map(format_rfc3339),
        Some("2026-03-01T03:00:00.250Z".to_owned())
    );
}

#[test]
fn an_instant_with_no_utc_form_is_refused_at_the_boundary_not_at_the_first_render() {
    let err = serde_json::from_str::<Carrier>(r#"{"at":"9999-12-31T23:00:00-05:00"}"#)
        .expect_err("no UTC form");
    assert!(err.to_string().contains("no UTC representation"), "{err}");

    let err = serde_json::from_str::<Carrier>(
        r#"{"at":"2026-01-01T00:00:00Z","maybe":"9999-12-31T23:00:00-05:00"}"#,
    )
    .expect_err("the option form refuses it too");
    assert!(err.to_string().contains("no UTC representation"), "{err}");
}

#[test]
fn the_wire_form_stays_times_own_and_an_absent_option_reads_as_none() {
    let carrier: Carrier =
        serde_json::from_str(r#"{"at":"2026-01-01T00:00:00Z"}"#).expect("a bare instant");
    assert_eq!(carrier.maybe, None);
    assert_eq!(
        serde_json::to_string(&carrier).expect("serializes"),
        r#"{"at":"2026-01-01T00:00:00Z","maybe":null}"#
    );
}

// ---------------------------------------------------------------------------
// utc_ymd_hms: a fixture literal is checked where it is written.
// ---------------------------------------------------------------------------

#[test]
fn an_impossible_civil_date_is_none_not_the_epoch() {
    assert_eq!(try_utc_ymd_hms(2026, 2, 30, 0, 0, 0), None);
    assert_eq!(try_utc_ymd_hms(2026, 13, 1, 0, 0, 0), None);
    assert_eq!(try_utc_ymd_hms(2026, 1, 1, 24, 0, 0), None);
    assert_eq!(
        try_utc_ymd_hms(2026, 2, 28, 23, 59, 59).map(format_rfc3339),
        Some("2026-02-28T23:59:59.000Z".to_owned())
    );
}

/// A fixture that names a date the calendar does not have fails where it is
/// written, not later as an epoch instant that satisfies every `<` bound.
#[test]
#[should_panic(expected = "is not a valid civil date-time")]
fn a_fixture_with_an_impossible_date_fails_where_it_is_written() {
    let _unreached = utc_ymd_hms(2026, 2, 30, 0, 0, 0);
}
