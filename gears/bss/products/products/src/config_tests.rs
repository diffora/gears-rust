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

/// P-D-84 arm 5: the freeze timeout floors the idempotency retention —
/// `max(24h, max_freeze_timeout)` finally has both operands — and the
/// default contributes nothing beyond the shipped constant.
#[test]
fn the_freeze_timeout_floors_the_retention() {
    let mut cfg = ProductsConfig::default();
    assert_eq!(
        cfg.resolved_idempotency_retention_hours(),
        IDEMPOTENCY_RETENTION_FLOOR_HOURS,
        "the default timeout leaves the floor at the constant"
    );
    cfg.freeze_timeout_hours = 100;
    assert_eq!(
        cfg.resolved_idempotency_retention_hours(),
        100,
        "a configured timeout above 24h raises the floor with it"
    );
}

/// P-D-84 arm 6: a timeout above the ten-year ceiling is refused at boot,
/// which is what keeps the clamp's `min <= max` precondition an invariant
/// rather than a panic on the first keyed request.
#[test]
fn a_timeout_above_the_ceiling_is_a_boot_refusal() {
    let mut cfg = ProductsConfig::default();
    assert!(cfg.validate().is_ok());
    cfg.freeze_timeout_hours = IDEMPOTENCY_RETENTION_CEILING_HOURS + 1;
    let refused = cfg.validate().expect_err("the ceiling refuses");
    assert!(refused.contains("freeze_timeout_hours"));
}

/// An unset `default_locale` is admitted, and that is the property `Gear::init`
/// depends on: it calls `config_or_default()` and then `validate()`, so a
/// refusal on the default would make the gear un-bootable in every deployment
/// that ships no config file — the measurement `require_broker`'s own doc
/// records for its default.
///
/// **P-D-101**: the field shortens step 3 of the fallback chain and does not
/// decide whether resolution succeeds — step 4's global fallback is what makes
/// the chain total — so "absent" is a working value and not a hole.
#[test]
fn an_unset_default_locale_is_admitted_at_boot() {
    let cfg = ProductsConfig::default();
    assert_eq!(cfg.default_locale, "", "the default is absent, not a guess");
    assert!(
        cfg.validate().is_ok(),
        "a config that ships no default_locale must still boot"
    );
}

/// A non-empty but untrimmed value is refused, because no stored coordinate can
/// ever match it: `products_attribute_value.locale` holds the token as written,
/// so ` en` would make step 3 miss on every row while looking configured.
///
/// The locale's **shape** is deliberately not validated — no document in the
/// set names a locale grammar — so this case pins the one failure the config
/// layer can prove without inventing a vocabulary.
#[test]
fn an_untrimmed_default_locale_is_refused_at_boot() {
    let mut cfg = ProductsConfig {
        default_locale: " en".to_owned(),
        ..ProductsConfig::default()
    };
    let refusal = cfg.validate().expect_err("an untrimmed locale is refused");
    assert!(
        refusal.contains("default_locale"),
        "the refusal names the field: {refusal}"
    );

    cfg.default_locale = "en".to_owned();
    assert!(
        cfg.validate().is_ok(),
        "the trimmed form of the same value is admitted"
    );
}
