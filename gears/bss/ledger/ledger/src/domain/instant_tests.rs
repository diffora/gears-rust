//! Unit tests for the `chrono` → `time` instant helpers.
//!
//! The pricing twin has these; this module shipped without them, so the doc
//! claims here — the exact chrono bytes, the microsecond preimage integer, the
//! `YYYYMM` and civil-date derivations — were asserted nowhere.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::NaiveDate;
use time::OffsetDateTime;

use super::{
    format_rfc3339_offset, from_unix, rfc3339, timestamp_micros, timestamp_millis, to_naive_date,
    try_utc_ymd_hms, utc_ymd_hms, yyyymm,
};

fn at(secs: i64, nanos: u32) -> OffsetDateTime {
    from_unix(secs, nanos).expect("a representable instant")
}

// chrono's `AutoSi` rule: nothing, `.SSS`, `.SSSSSS` or nine digits — whichever
// first divides the nanoseconds evenly — and the `+00:00` designator
// `to_rfc3339()` wrote. Each arm is a case; a renderer that printed minimal
// digits (`time`'s `Rfc3339`) or a fixed width (`.SSS`) fails at least one.
#[test]
fn offset_form_reproduces_chrono_to_rfc3339_byte_for_byte() {
    assert_eq!(
        format_rfc3339_offset(at(1_757_000_000, 0)),
        "2025-09-04T15:33:20+00:00"
    );
    assert_eq!(
        format_rfc3339_offset(at(1_757_000_000, 100_000_000)),
        "2025-09-04T15:33:20.100+00:00",
        "a tenth of a second is `.100`, not `.1`"
    );
    assert_eq!(
        format_rfc3339_offset(at(1_757_000_000, 123_456_000)),
        "2025-09-04T15:33:20.123456+00:00"
    );
    assert_eq!(
        format_rfc3339_offset(at(1_757_000_000, 123_456_789)),
        "2025-09-04T15:33:20.123456789+00:00"
    );
}

// A non-UTC input renders in UTC: the offset is folded in, not printed.
#[test]
fn offset_form_normalises_to_utc_first() {
    let plus_two = at(1_757_000_000, 0).to_offset(time::UtcOffset::from_hms(2, 0, 0).unwrap());
    assert_eq!(format_rfc3339_offset(plus_two), "2025-09-04T15:33:20+00:00");
}

// The audit-chain preimage integer. The shipped vector uses whole seconds, so
// the sub-second arm — where a truncation or a rounding would silently change
// the hash — is pinned here.
#[test]
fn timestamp_micros_keeps_the_sub_second_digits() {
    assert_eq!(
        timestamp_micros(at(1_757_000_000, 0)),
        1_757_000_000_000_000
    );
    assert_eq!(
        timestamp_micros(at(1_757_000_000, 123_456_789)),
        1_757_000_000_123_456
    );
    assert_eq!(
        timestamp_millis(at(1_757_000_000, 123_456_789)),
        1_757_000_000_123
    );
}

// `yyyymm` is `u8::from(month)`, never the month's name: the defect this branch
// carried was `format!("{:02}", now.month())` printing `September`.
#[test]
fn yyyymm_is_six_digits_across_the_year_boundary() {
    assert_eq!(yyyymm(utc_ymd_hms(2026, 9, 8, 10, 0, 0)), "202609");
    assert_eq!(yyyymm(utc_ymd_hms(2026, 12, 31, 23, 59, 59)), "202612");
    assert_eq!(yyyymm(utc_ymd_hms(2027, 1, 1, 0, 0, 0)), "202701");
    let late_evening_west =
        utc_ymd_hms(2026, 1, 1, 1, 0, 0).to_offset(time::UtcOffset::from_hms(-3, 0, 0).unwrap());
    assert_eq!(
        yyyymm(late_evening_west),
        "202601",
        "the period is taken in UTC, not in the caller's offset"
    );
}

#[test]
fn to_naive_date_takes_the_utc_civil_date() {
    assert_eq!(
        to_naive_date(utc_ymd_hms(2026, 2, 28, 23, 59, 59)),
        NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()
    );
    let just_past_midnight_east =
        utc_ymd_hms(2026, 3, 1, 0, 30, 0).to_offset(time::UtcOffset::from_hms(5, 0, 0).unwrap());
    assert_eq!(
        to_naive_date(just_past_midnight_east),
        NaiveDate::from_ymd_opt(2026, 3, 1).unwrap(),
        "the civil date is UTC's, not the offset's"
    );
}

// An invalid calendar value falls back to the epoch rather than panicking —
// the documented contract of `utc_ymd_hms`.
#[test]
fn an_impossible_civil_date_is_none_not_the_epoch() {
    assert_eq!(try_utc_ymd_hms(2026, 2, 30, 0, 0, 0), None);
    assert_eq!(try_utc_ymd_hms(2026, 13, 1, 0, 0, 0), None);
    assert_eq!(try_utc_ymd_hms(2026, 1, 1, 24, 0, 0), None);
    assert_eq!(
        try_utc_ymd_hms(2026, 2, 28, 23, 59, 59).map(timestamp_millis),
        Some(1_772_323_199_000)
    );
}

/// A fixture that names a date the calendar does not have fails where it is
/// written, not later as an epoch instant that satisfies every `<` bound.
#[test]
#[should_panic(expected = "is not a valid civil date-time")]
fn a_fixture_with_an_impossible_date_fails_where_it_is_written() {
    let _unreached = utc_ymd_hms(2026, 2, 30, 0, 0, 0);
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

/// The premise, measured on `time` itself: its own `rfc3339` keeps the written
/// offset, and the UTC form of this instant is year 10000, which `time` cannot
/// hold — so `to_offset(UTC)` on it (`to_naive_date`'s first line) panics.
#[test]
fn time_alone_admits_an_instant_that_has_no_utc_form() {
    let parsed = OffsetDateTime::parse(
        "9999-12-31T23:00:00-05:00",
        &time::format_description::well_known::Rfc3339,
    )
    .expect("time's rfc3339 parses the local form");
    assert!(parsed.checked_to_offset(time::UtcOffset::UTC).is_none());
}

#[test]
fn a_request_instant_arrives_in_utc_whatever_offset_it_was_written_in() {
    let carrier: Carrier = serde_json::from_str(
        r#"{"at":"2026-03-01T05:00:00-05:00","maybe":"2026-03-01T05:00:00.25+02:00"}"#,
    )
    .expect("both instants are representable");
    assert_eq!(carrier.at.offset(), time::UtcOffset::UTC);
    assert_eq!(
        format_rfc3339_offset(carrier.at),
        "2026-03-01T10:00:00+00:00"
    );
    assert_eq!(
        carrier.maybe.map(format_rfc3339_offset),
        Some("2026-03-01T03:00:00.250+00:00".to_owned())
    );
    assert_eq!(yyyymm(carrier.at), "202603");
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
