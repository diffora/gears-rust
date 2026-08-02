//! `bss-pricing` configuration section.
//!
//! Every field has a launch default, so a `gears:` entry with no `config:`
//! block is a valid deployment. The numbers are the ratified NFR values
//! (`PRD.md` §14/§15, ratified 2026-07-28), not invented ones; no field here can
//! turn a fail-closed check off.
//!
//! **This section is per deployment, and four of its values are per tenant**
//! (D-152). The four §14 caps in [`LimitsConfig`] are the **default** a tenant
//! with no `pricing_policy_object` entry takes; the tenant's own value, when
//! there is one, is resolved by
//! [`PolicyObjectRepo`](crate::infra::storage::repo::PolicyObjectRepo) and is
//! what the authoring rules are built from. Reading a cap straight off this
//! struct on an authoring path is therefore the defect D-152 closed — every
//! tenant of a deployment sharing one limit — and the reason the caps are not
//! handed to the domain from here.

use std::path::PathBuf;
use std::time::Duration;

use serde::Deserialize;

/// Root of the `bss-pricing` config section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BssPricingConfig {
    /// Emit the frozen event set to the broker. Default `false`: there is no
    /// event broker in this repository yet, and the publish path must not fail
    /// because a fan-out target is absent (the outbox row is still written —
    /// fan-out, not the transaction, is what this gates).
    pub events_enabled: bool,
    /// Background-job cadences.
    pub jobs: JobsConfig,
    /// Publish-time size and lifetime limits.
    pub limits: LimitsConfig,
    /// Where the joint conformance-fixture registry is read from.
    pub fixtures: FixturesConfig,
}

impl BssPricingConfig {
    /// Validate every sub-section.
    ///
    /// # Errors
    /// [`ConfigError`] on the first invalid value; `init()` aborts loudly
    /// rather than booting a gear whose ticker would panic or whose caps would
    /// admit an unbounded plan.
    pub fn validate(&self) -> Result<(), ConfigError> {
        self.jobs.validate()?;
        self.limits.validate()?;
        self.fixtures.validate()
    }
}

/// Where the generated joint conformance-fixture registry lives
/// (`gears/bss/fixtures/corpus/registry.toml`), read once at init by
/// [`crate::infra::fixture_gate::FixtureGate`].
///
/// There is deliberately **no** field here that disables the gate. The only
/// thing a deployment may state is where the artifact is; whether a `modelKind`
/// is publishable is decided by the corpus, and a path that resolves to nothing
/// leaves the gate closed for every kind rather than open for any.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FixturesConfig {
    /// Path to the generated `registry.toml`. Relative paths resolve against
    /// the process working directory; the default is the in-repository location,
    /// which is what the workspace-root e2e deployment sees.
    pub registry_path: PathBuf,
}

impl Default for FixturesConfig {
    fn default() -> Self {
        Self {
            registry_path: PathBuf::from("gears/bss/fixtures/corpus/registry.toml"),
        }
    }
}

impl FixturesConfig {
    /// # Errors
    /// [`ConfigError::EmptyPath`] when the path is blank. An empty string would
    /// otherwise be accepted by `PathBuf` and fail at load, producing a
    /// permanently closed gate whose cause reads as a missing file rather than
    /// as the configuration mistake it is.
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.registry_path.as_os_str().is_empty() {
            return Err(ConfigError::EmptyPath {
                field: "fixtures.registry_path",
            });
        }
        Ok(())
    }
}

/// Cadences for the gear's background work.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JobsConfig {
    /// How often the read-model warm re-drive sweeps for publishes whose
    /// projection has not completed. The publish→read-model propagation target
    /// is p95 ≤ 5s, and a degraded publish's re-drive continues past it, so the
    /// sweep runs at that order.
    pub readmodel_warm_tick_secs: u64,
    /// How long a `pricing_catalog_version_ref` may stay `pending` before
    /// `pricing.catalogversion.commit_overdue` raises Critical. Default 300s =
    /// the ratified max batching delay (D-47: p95 ≤ 60s, max 5 min).
    pub catalog_version_overdue_secs: u64,
}

impl Default for JobsConfig {
    fn default() -> Self {
        Self {
            readmodel_warm_tick_secs: 5,
            catalog_version_overdue_secs: 300,
        }
    }
}

impl JobsConfig {
    /// The read-model warm re-drive cadence.
    #[must_use]
    pub const fn readmodel_warm_interval(&self) -> Duration {
        Duration::from_secs(self.readmodel_warm_tick_secs)
    }

    /// The pending-`CatalogVersion` alarm threshold.
    #[must_use]
    pub const fn catalog_version_overdue_after(&self) -> Duration {
        Duration::from_secs(self.catalog_version_overdue_secs)
    }

    /// # Errors
    /// [`ConfigError::ZeroInterval`] for a zero cadence: `tokio`'s interval
    /// panics on a zero period, and a zero alarm threshold would fire on every
    /// publish before the registry could possibly have answered.
    pub const fn validate(&self) -> Result<(), ConfigError> {
        if self.readmodel_warm_tick_secs == 0 {
            return Err(ConfigError::ZeroInterval {
                field: "jobs.readmodel_warm_tick_secs",
            });
        }
        if self.catalog_version_overdue_secs == 0 {
            return Err(ConfigError::ZeroInterval {
                field: "jobs.catalog_version_overdue_secs",
            });
        }
        Ok(())
    }
}

/// Publish-time size and lifetime limits (ratified 2026-07-28).
///
/// **The four caps are deployment *defaults*, not the values in force**
/// (D-152). Each is what a tenant with no `pricing_policy_object` entry is
/// governed by, which is what keeps the ratified launch numbers from moving; the
/// value an authoring run actually enforces comes from
/// [`PolicyObjectRepo::authoring_policy`](crate::infra::storage::repo::PolicyObjectRepo::authoring_policy).
/// The TTL below is not one of them — an idempotency window is a property of the
/// deployment's dedup store, not of a tenant's catalog policy.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsConfig {
    /// Default soft cap on tier bands per price row.
    pub max_tier_bands_per_row: u32,
    /// Default soft cap on price rows per plan.
    pub max_price_rows_per_plan: u32,
    /// Default largest `n` a `customEveryN Days(n)` frequency may carry
    /// (`INVALID_CUSTOM_INTERVAL`, PRD §14 / AC #84).
    ///
    /// Unlike the two soft caps above this one is **hard**: P1 says an over-cap
    /// interval is rejected at authoring with no silent clamp, because a
    /// clamped interval is a billing period the operator did not author and
    /// would never see.
    pub max_custom_interval_days: u32,
    /// Default largest `n` a `customEveryN Months(n)` frequency may carry. The
    /// months cap is separate from the days cap because the two units bound
    /// different things; see [`LimitsConfig::max_custom_interval_days`] for the
    /// hard-cap note.
    pub max_custom_interval_months: u32,
    /// Client idempotency-key retention. A replay inside the window returns the
    /// stored response; outside it the key is forgotten and the call executes
    /// again, so this is a correctness-relevant duration, not a cache knob.
    pub idempotency_key_ttl_hours: u64,
}

impl Default for LimitsConfig {
    fn default() -> Self {
        Self {
            max_tier_bands_per_row: 100,
            max_price_rows_per_plan: 500,
            max_custom_interval_days: 366,
            max_custom_interval_months: 24,
            idempotency_key_ttl_hours: 24,
        }
    }
}

impl LimitsConfig {
    /// The idempotency-key retention window.
    #[must_use]
    pub const fn idempotency_key_ttl(&self) -> Duration {
        Duration::from_secs(self.idempotency_key_ttl_hours * 3_600)
    }

    /// # Errors
    /// [`ConfigError::ZeroLimit`] for a zero cap: a zero band or row cap makes
    /// every plan unpublishable, a zero interval cap makes every custom
    /// frequency unpublishable (P1 requires `n > 0`, so no `n` could satisfy
    /// both bounds), and a zero TTL disables idempotency replay silently — all
    /// fail loudly at boot instead.
    pub const fn validate(&self) -> Result<(), ConfigError> {
        if self.max_tier_bands_per_row == 0 {
            return Err(ConfigError::ZeroLimit {
                field: "limits.max_tier_bands_per_row",
            });
        }
        if self.max_price_rows_per_plan == 0 {
            return Err(ConfigError::ZeroLimit {
                field: "limits.max_price_rows_per_plan",
            });
        }
        if self.max_custom_interval_days == 0 {
            return Err(ConfigError::ZeroLimit {
                field: "limits.max_custom_interval_days",
            });
        }
        if self.max_custom_interval_months == 0 {
            return Err(ConfigError::ZeroLimit {
                field: "limits.max_custom_interval_months",
            });
        }
        if self.idempotency_key_ttl_hours == 0 {
            return Err(ConfigError::ZeroLimit {
                field: "limits.idempotency_key_ttl_hours",
            });
        }
        Ok(())
    }
}

/// A rejected configuration value. Carries the dotted field path so the boot
/// log names what to fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// A cadence or threshold was zero.
    #[error("`{field}` must be greater than zero")]
    ZeroInterval {
        /// Dotted path of the offending field.
        field: &'static str,
    },
    /// A size or lifetime cap was zero.
    #[error("`{field}` must be greater than zero")]
    ZeroLimit {
        /// Dotted path of the offending field.
        field: &'static str,
    },
    /// A path was configured as the empty string.
    #[error("`{field}` must not be empty")]
    EmptyPath {
        /// Dotted path of the offending field.
        field: &'static str,
    },
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
