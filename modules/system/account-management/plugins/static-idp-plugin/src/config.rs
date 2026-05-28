//! Configuration for the static `IdP` plugin.

use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StaticIdpPluginConfig {
    /// Vendor name for GTS instance registration. Read by AM's
    /// `choose_plugin_instance` filter to decide whether this plugin
    /// matches the configured `idp.vendor`. Defaults to `"cf"` so a
    /// stock deploy with `IdpConfig::default()` (which uses the
    /// same `"cf"` default) resolves this plugin out-of-the-box.
    pub vendor: String,

    /// Plugin priority — lower wins on tie-breaks within the same
    /// vendor. Defaults to `100` to leave headroom for higher-priority
    /// vendor-specific deploys (e.g. a real `IdP` plugin with
    /// `priority < 100`) to outrank the static echo if both happen
    /// to publish under `vendor = "cf"`.
    pub priority: i16,
}

impl Default for StaticIdpPluginConfig {
    fn default() -> Self {
        Self {
            // Matches `account_management::config::IdpConfig::default().vendor`
            // so the static echo plugin is the out-of-the-box answer
            // when AM is deployed without an external IdP plugin.
            vendor: "cf".to_owned(),
            priority: 100,
        }
    }
}
