//! The serde contract itself — defaults fill in, unknown keys are denied —
//! and the retention resolution that serde cannot express: a well-typed value
//! that would switch idempotency off.

use super::{
    IDEMPOTENCY_RETENTION_CEILING_HOURS, IDEMPOTENCY_RETENTION_FLOOR_HOURS, ProductsConfig,
};

#[test]
fn defaults_fill_in_for_an_empty_table() {
    let cfg: ProductsConfig =
        serde_json::from_str("{}").expect("an empty table is a valid configuration");
    assert_eq!(cfg, ProductsConfig::default());
    assert_eq!(cfg.idempotency_retention_hours, 24);
}

#[test]
fn an_unknown_key_is_refused_rather_than_ignored() {
    let parsed: Result<ProductsConfig, _> =
        serde_json::from_str(r#"{"idempotency_retention_hous": 48}"#);
    assert!(
        parsed.is_err(),
        "a misspelled key must fail the boot, not be dropped"
    );
}

/// A configured `0` resolves to the floor rather than to a zero window.
///
/// This is the case the resolution exists for. `idempotency_expiry` stamps
/// `expires_at = now + window`, so a zero window stamps `expires_at == now`
/// and the very next request on that key reads it as expired, takes it over
/// and re-executes the guarded mutation: at-most-once off, with no boot
/// failure and no log. `deny_unknown_fields` cannot see this — the key is
/// spelled correctly and the value parses.
#[test]
fn a_configured_zero_resolves_to_the_retention_floor() {
    let cfg: ProductsConfig = serde_json::from_str(r#"{"idempotency_retention_hours": 0}"#)
        .expect("zero is a well-typed u32 and parses");
    assert_eq!(
        cfg.idempotency_retention_hours, 0,
        "the field still reports what the operator wrote, so init can log the raise"
    );
    assert_eq!(
        cfg.resolved_idempotency_retention_hours(),
        IDEMPOTENCY_RETENTION_FLOOR_HOURS,
        "a window below the floor must resolve to the floor, never to itself"
    );
}

/// Any value below the floor resolves to the floor, not only `0`.
///
/// `0` is the extreme; the property is the floor itself, and a resolution
/// that special-cased zero would leave `1` stamping a window that expires
/// while its client is still retrying.
#[test]
fn a_configured_value_below_the_floor_resolves_to_the_floor() {
    let cfg: ProductsConfig = serde_json::from_str(r#"{"idempotency_retention_hours": 1}"#)
        .expect("one hour is a well-typed u32 and parses");
    assert_eq!(
        cfg.resolved_idempotency_retention_hours(),
        IDEMPOTENCY_RETENTION_FLOOR_HOURS
    );
}

/// `u32::MAX` resolves to the ceiling rather than to an unrepresentable
/// window.
///
/// Roughly 490 000 years of hours: `DateTime::checked_add_signed` has no
/// answer for it, and an expiry stamp with no answer is the second way to
/// reach a window that is not the operator's — the first being the `0`
/// above. Clamping here is what keeps the resolution total, so nothing
/// downstream has to decide what an overflowing window means.
#[test]
fn the_largest_configurable_value_resolves_to_the_retention_ceiling() {
    let cfg: ProductsConfig = serde_json::from_str(&format!(
        r#"{{"idempotency_retention_hours": {}}}"#,
        u32::MAX
    ))
    .expect("u32::MAX is a well-typed u32 and parses");
    assert_eq!(cfg.idempotency_retention_hours, u32::MAX);
    assert_eq!(
        cfg.resolved_idempotency_retention_hours(),
        IDEMPOTENCY_RETENTION_CEILING_HOURS,
        "an unrepresentable window must resolve to the ceiling, never overflow into a \
         stamp of its own"
    );
}

/// A legitimate value above the floor is carried through untouched.
///
/// The pair to the two clamps above, and the reason they are clamps and not
/// a replacement: an operator who asks for a week gets a week. An earlier
/// defect of exactly this shape — `idempotency_expiry` reading
/// `ProductsConfig::default()` rather than the operator's value — gave every
/// deployment the 24-hour floor however it was configured, so the
/// pass-through is asserted rather than assumed.
#[test]
fn a_configured_value_above_the_floor_is_carried_through_unchanged() {
    let cfg: ProductsConfig = serde_json::from_str(r#"{"idempotency_retention_hours": 168}"#)
        .expect("a week of hours parses");
    assert_eq!(cfg.resolved_idempotency_retention_hours(), 168);
}

/// The default configuration already sits on the floor, so an unconfigured
/// boot is never clamped.
#[test]
fn the_default_configuration_resolves_to_itself() {
    let cfg = ProductsConfig::default();
    assert_eq!(
        cfg.resolved_idempotency_retention_hours(),
        cfg.idempotency_retention_hours
    );
    assert_eq!(
        cfg.idempotency_retention_hours,
        IDEMPOTENCY_RETENTION_FLOOR_HOURS
    );
}
