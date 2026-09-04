//! The **millisecond quantum** every authored instant is expressed at (D-144).
//!
//! The gear's treatment of time fixed the zone and left the resolution open —
//! "all effective dating, window boundaries, `grandfatherUntil`,
//! `availableFrom`/`availableTo` and anchor math are UTC" — and an unquantized
//! axis is not a stylistic gap. `cohort` **is** a cutover instant and is matched
//! for **equality** across a gear boundary against a window bound another gear
//! produced (`design/07-pricewindow-linkage.md` §5, D-126): two instants denoting
//! the same moment at different resolutions are not equal, so the generation
//! becomes unfindable by exactly the subscribers grandfathering exists to
//! protect.
//!
//! A finer instant is therefore **refused, never truncated**. Truncation is what
//! an unstated quantum degenerates into, and it moves the instant a scope-key
//! axis, a window bound and an approval-time floor are all derived from — a
//! truncating producer and a non-truncating consumer agree until the day they do
//! not, with no failure in between.
//!
//! The rule is over instants the gear **authors, carries in a contract field,
//! publishes or compares**. Storage bookkeeping an operator never authors —
//! `created_at`, audit-chain and outbox timestamps — is outside it: none of it
//! enters a contract field and none of it is compared with anything.
//!
//! This is [`crate::domain::money`]'s temporal sibling and is shaped like it on
//! purpose: a predicate for callers that only have to decide, and a checked form
//! that names the code for callers that have to refuse.

use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

use crate::domain::error::DomainError;

/// Nanoseconds in the quantum. One millisecond.
const QUANTUM_NANOS: u32 = 1_000_000;

/// Is `at` expressible at the quantum — i.e. does it carry no precision below
/// one millisecond?
///
/// `time::OffsetDateTime` does not represent leap-second nanoseconds (the
/// nanosecond field is always in `0..1_000_000_000`), so the remainder is
/// taken over that field alone. A leap second authored at whole milliseconds
/// is therefore expressible the same way any other instant is.
#[must_use]
pub fn is_quantized(at: OffsetDateTime) -> bool {
    at.nanosecond().is_multiple_of(QUANTUM_NANOS)
}

/// The authoring-time form of [`is_quantized`].
///
/// `field` is the authored field the instant arrived on (`grandfatherUntil`,
/// `availableFrom`, `cohort`), so the author corrects one value instead of
/// resubmitting a request and guessing which of its instants was refused.
///
/// # Errors
///
/// [`DomainError::TimestampPrecisionExceeded`] when `at` carries precision finer
/// than one millisecond.
pub fn check_quantum(field: &str, at: OffsetDateTime) -> Result<(), DomainError> {
    if is_quantized(at) {
        return Ok(());
    }
    Err(DomainError::TimestampPrecisionExceeded(format!(
        "{field} {} is finer than the millisecond quantum",
        format_rfc3339(at)
    )))
}

/// RFC 3339 rendering at millisecond precision with a `Z` designator.
///
/// Matches chrono `to_rfc3339_opts(SecondsFormat::Millis, true)`, which is
/// what stored `subject_ref` values, approval-unit keys and error detail
/// already used. `time`'s well-known `Rfc3339` drops the fractional second
/// when it is zero, which would re-key every unit already in the register.
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

/// Microseconds since the Unix epoch. Same integer [`chrono`] produced via
/// `timestamp_micros`, so the audit-chain preimage stays byte-stable.
#[must_use]
pub fn timestamp_micros(at: OffsetDateTime) -> i64 {
    at.unix_timestamp()
        .saturating_mul(1_000_000)
        .saturating_add(i64::from(at.nanosecond() / 1_000))
}

/// Construct a UTC instant from a civil date-time.
///
/// Invalid calendar values fall back to the Unix epoch — every in-tree caller
/// passes a real date, the same ones `chrono`'s `with_ymd_and_hms` accepted.
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
/// instant. Replaces `chrono::DateTime::from_timestamp`.
#[must_use]
pub fn from_unix(seconds: i64, nanos: u32) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp(seconds)
        .ok()?
        .replace_nanosecond(nanos)
        .ok()
}

/// Unix timestamp in milliseconds, or `None` when the value is not a real
/// instant. Replaces `chrono::DateTime::from_timestamp_millis`.
#[must_use]
pub fn from_unix_millis(millis: i64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(millis) * 1_000_000).ok()
}

/// Milliseconds since the Unix epoch. Replaces `chrono`'s `timestamp_millis`.
#[must_use]
pub fn timestamp_millis(at: OffsetDateTime) -> i64 {
    at.unix_timestamp()
        .saturating_mul(1_000)
        .saturating_add(i64::from(at.nanosecond() / 1_000_000))
}

/// Drop sub-millisecond nanos so the instant sits on the D-144 quantum.
#[must_use]
pub fn truncate_millis(at: OffsetDateTime) -> OffsetDateTime {
    let nanos = at.nanosecond() / QUANTUM_NANOS * QUANTUM_NANOS;
    at.replace_nanosecond(nanos).unwrap_or(at)
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

/// The latest civil instant `time` can represent. Stands in for chrono's
/// `DateTime::<Utc>::MAX_UTC` in overflow cases.
#[must_use]
pub fn max_utc() -> OffsetDateTime {
    utc_ymd_hms(9999, 12, 31, 23, 59, 59)
}

/// Parse an RFC 3339 instant. Replaces `DateTime::parse_from_rfc3339`.
///
/// # Errors
///
/// When `raw` is not RFC 3339.
pub fn parse_rfc3339(raw: &str) -> Result<OffsetDateTime, time::error::Parse> {
    OffsetDateTime::parse(raw, &Rfc3339)
}

#[cfg(test)]
#[path = "instant_tests.rs"]
mod instant_tests;
