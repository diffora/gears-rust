//! Tests for [`crate::config`].

use std::path::PathBuf;

use super::{BssPricingConfig, ConfigError, FixturesConfig, JobsConfig, LimitsConfig};

#[test]
fn defaults_are_the_ratified_launch_values() {
    let cfg = BssPricingConfig::default();

    assert!(!cfg.events_enabled, "no broker is wired in this repository");
    assert_eq!(cfg.jobs.readmodel_warm_tick_secs, 5);
    assert_eq!(cfg.jobs.catalog_version_overdue_secs, 300);
    assert_eq!(cfg.limits.max_tier_bands_per_row, 100);
    assert_eq!(cfg.limits.max_price_rows_per_plan, 500);
    // PRD 14: `customEveryNDays <= 366`, `customEveryNMonths <= 24`, ratified
    // 2026-07-28. Unlike the two above these are hard caps -- P1 rejects an
    // over-cap interval at authoring rather than clamping it.
    assert_eq!(cfg.limits.max_custom_interval_days, 366);
    assert_eq!(cfg.limits.max_custom_interval_months, 24);
    assert_eq!(cfg.limits.idempotency_key_ttl_hours, 24);
    assert_eq!(
        cfg.fixtures.registry_path,
        PathBuf::from("gears/bss/fixtures/corpus/registry.toml")
    );
    assert!(cfg.validate().is_ok());
}

#[test]
fn the_fixtures_section_carries_only_a_path() {
    // The gate has no off switch, so the section has no boolean. Anything that
    // looked like `enabled: false` would be a way to publish an ungated
    // `modelKind`, which is the one thing the corpus exists to prevent.
    let err = serde_json::from_str::<FixturesConfig>("{\"enabled\": false}")
        .expect_err("unknown fixture keys are denied");

    assert!(
        err.to_string().contains("enabled"),
        "the error names the offending key: {err}"
    );
}

#[test]
fn an_overridden_registry_path_is_taken_verbatim() {
    let cfg: BssPricingConfig =
        serde_json::from_str("{\"fixtures\": {\"registry_path\": \"/opt/bss/registry.toml\"}}")
            .expect("a fixtures override is a valid section");

    assert_eq!(
        cfg.fixtures.registry_path,
        PathBuf::from("/opt/bss/registry.toml")
    );
    assert!(cfg.validate().is_ok());
}

#[test]
fn an_empty_registry_path_is_rejected_at_boot() {
    // Left unchecked this boots a gate that is closed for every kind and logs
    // "file not found" for a path the operator never wrote, so it is caught
    // where the mistake is legible.
    let fixtures = FixturesConfig {
        registry_path: PathBuf::new(),
    };

    assert_eq!(
        fixtures.validate(),
        Err(ConfigError::EmptyPath {
            field: "fixtures.registry_path"
        })
    );
}

#[test]
fn an_empty_section_deserializes_to_the_defaults() {
    let cfg: BssPricingConfig = serde_json::from_str("{}").expect("empty map is a valid section");

    assert_eq!(
        cfg.jobs.readmodel_warm_tick_secs,
        JobsConfig::default().readmodel_warm_tick_secs
    );
    assert_eq!(
        cfg.limits.max_price_rows_per_plan,
        LimitsConfig::default().max_price_rows_per_plan
    );
}

#[test]
fn an_unknown_field_is_rejected_rather_than_ignored() {
    // A typo in a limit must not silently leave the launch default in force:
    // the operator believes they raised a cap that never moved.
    let err = serde_json::from_str::<BssPricingConfig>("{\"max_price_rows\": 900}")
        .expect_err("unknown fields are denied");

    assert!(
        err.to_string().contains("max_price_rows"),
        "the error names the offending key: {err}"
    );
}

#[test]
fn a_zero_cadence_is_rejected_by_field() {
    let jobs = JobsConfig {
        readmodel_warm_tick_secs: 0,
        ..JobsConfig::default()
    };

    assert_eq!(
        jobs.validate(),
        Err(ConfigError::ZeroInterval {
            field: "jobs.readmodel_warm_tick_secs"
        })
    );
}

#[test]
fn a_zero_overdue_threshold_is_rejected() {
    let jobs = JobsConfig {
        catalog_version_overdue_secs: 0,
        ..JobsConfig::default()
    };

    assert_eq!(
        jobs.validate(),
        Err(ConfigError::ZeroInterval {
            field: "jobs.catalog_version_overdue_secs"
        })
    );
}

#[test]
fn zero_limits_are_rejected_by_field() {
    for (limits, field) in [
        (
            LimitsConfig {
                max_tier_bands_per_row: 0,
                ..LimitsConfig::default()
            },
            "limits.max_tier_bands_per_row",
        ),
        (
            LimitsConfig {
                max_price_rows_per_plan: 0,
                ..LimitsConfig::default()
            },
            "limits.max_price_rows_per_plan",
        ),
        // A zero interval cap is not a permissive one: P1 also requires
        // `n > 0`, so no `n` at all could satisfy both bounds and every custom
        // frequency would be unpublishable.
        (
            LimitsConfig {
                max_custom_interval_days: 0,
                ..LimitsConfig::default()
            },
            "limits.max_custom_interval_days",
        ),
        (
            LimitsConfig {
                max_custom_interval_months: 0,
                ..LimitsConfig::default()
            },
            "limits.max_custom_interval_months",
        ),
        (
            LimitsConfig {
                idempotency_key_ttl_hours: 0,
                ..LimitsConfig::default()
            },
            "limits.idempotency_key_ttl_hours",
        ),
    ] {
        assert_eq!(limits.validate(), Err(ConfigError::ZeroLimit { field }));
    }
}

#[test]
fn validate_rejects_the_whole_config_when_a_sub_section_is_invalid() {
    let cfg = BssPricingConfig {
        limits: LimitsConfig {
            idempotency_key_ttl_hours: 0,
            ..LimitsConfig::default()
        },
        ..BssPricingConfig::default()
    };

    assert!(cfg.validate().is_err());
}

#[test]
fn the_idempotency_ttl_reads_as_a_duration() {
    let ttl = LimitsConfig::default().idempotency_key_ttl();

    assert_eq!(ttl.as_secs(), 24 * 3_600);
}
