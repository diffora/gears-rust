//! Typed gear configuration.

use serde::Deserialize;

/// The gear's boot configuration.
///
/// @cpt-cf-bss-products-fr-idempotent-authoring
///
/// Every field has a default, so a boot that configures the gear at all gets a
/// working one; `deny_unknown_fields` is what turns a typo in the operator's
/// file into a boot failure rather than a silently ignored setting.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, default)]
pub struct ProductsConfig {
    /// How long an idempotency key is retained, in hours.
    ///
    /// The floor the design pins is 24 hours **and** at least the maximum
    /// freeze timeout, which the catalog-version feature exports. Until that
    /// feature exists the second half has no source, so this carries the first.
    pub idempotency_retention_hours: u32,
}

impl Default for ProductsConfig {
    fn default() -> Self {
        Self {
            idempotency_retention_hours: 24,
        }
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod config_tests;
