//! Instant helpers shared by the ledger's `time::OffsetDateTime` surface.
//!
//! Calendar dates (`due_date`, posting `effective_at`) stay `chrono::NaiveDate`.
//! Instants — posted-at, queued-at, policy `effective_from` — are UTC
//! [`time::OffsetDateTime`], matching AM and pricing.

use chrono::NaiveDate;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// RFC 3339 rendering at millisecond precision with a `Z` designator.
///
/// Matches chrono `to_rfc3339_opts(SecondsFormat::Millis, true)` so stored
/// keys and error detail stay byte-stable across the type change.
#[must_use]
pub fn format_rfc3339(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{millis:03}Z",
        year = at.year(),
        month = u8::from(at.month()),
        day = at.day(),
        hour = at.hour(),
        minute = at.minute(),
        second = at.second(),
        millis = at.nanosecond() / 1_000_000
    )
}

/// Microseconds since the Unix epoch. Same integer chrono produced via
/// `timestamp_micros`.
#[must_use]
pub fn timestamp_micros(at: OffsetDateTime) -> i64 {
    at.unix_timestamp()
        .saturating_mul(1_000_000)
        .saturating_add(i64::from(at.nanosecond() / 1_000))
}

/// Construct a UTC instant from a civil date-time.
///
/// Invalid calendar values fall back to the Unix epoch — every in-tree caller
/// passes a real date.
#[must_use]
pub fn utc_ymd_hms(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> OffsetDateTime {
    use time::{Date, Month, PrimitiveDateTime, Time};

    let Ok(month_num) = u8::try_from(month) else {
        return OffsetDateTime::UNIX_EPOCH;
    };
    let Ok(month) = Month::try_from(month_num) else {
        return OffsetDateTime::UNIX_EPOCH;
    };
    let Ok(day) = u8::try_from(day) else {
        return OffsetDateTime::UNIX_EPOCH;
    };
    let Ok(hour) = u8::try_from(hour) else {
        return OffsetDateTime::UNIX_EPOCH;
    };
    let Ok(minute) = u8::try_from(minute) else {
        return OffsetDateTime::UNIX_EPOCH;
    };
    let Ok(second) = u8::try_from(second) else {
        return OffsetDateTime::UNIX_EPOCH;
    };
    let Ok(date) = Date::from_calendar_date(year, month, day) else {
        return OffsetDateTime::UNIX_EPOCH;
    };
    let Ok(tod) = Time::from_hms(hour, minute, second) else {
        return OffsetDateTime::UNIX_EPOCH;
    };
    PrimitiveDateTime::new(date, tod).assume_utc()
}

/// Unix timestamp plus nanoseconds, or `None` when the pair is not a real
/// instant.
#[must_use]
pub fn from_unix(seconds: i64, nanos: u32) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()?
        .replace_nanosecond(nanos)
        .ok()
}

/// Unix timestamp in milliseconds, or `None` when the value is not a real
/// instant.
#[must_use]
pub fn from_unix_millis(millis: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000).ok()
}

/// Milliseconds since the Unix epoch.
#[must_use]
pub fn timestamp_millis(at: OffsetDateTime) -> i64 {
    at.unix_timestamp()
        .saturating_mul(1_000)
        .saturating_add(i64::from(at.nanosecond() / 1_000_000))
}

/// `chrono::Duration::try_days` — `None` when `days` does not fit a
/// `time::Duration`.
#[must_use]
pub fn try_days(days: i64) -> Option<time::Duration> {
    days.checked_mul(24)?
        .checked_mul(60)?
        .checked_mul(60)
        .map(time::Duration::seconds)
}

/// The civil UTC date of `at`, as the `NaiveDate` posting dates still use.
#[must_use]
pub fn to_naive_date(at: OffsetDateTime) -> NaiveDate {
    let at = at.to_offset(time::UtcOffset::UTC);
    NaiveDate::from_ymd_opt(
        at.year(),
        u32::from(u8::from(at.month())),
        u32::from(at.day()),
    )
    .unwrap_or(NaiveDate::MIN)
}

/// `YYYYMM` period id for a UTC instant. Replaces chrono `format("%Y%m")`.
#[must_use]
pub fn yyyymm(at: OffsetDateTime) -> String {
    let at = at.to_offset(time::UtcOffset::UTC);
    format!("{:04}{:02}", at.year(), u8::from(at.month()))
}

/// Parse an RFC 3339 instant.
///
/// # Errors
///
/// When `raw` is not RFC 3339.
pub fn parse_rfc3339(raw: &str) -> Result<OffsetDateTime, time::error::Parse> {
    OffsetDateTime::parse(raw, &Rfc3339)
}
