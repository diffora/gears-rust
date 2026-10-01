//! The inbox's boot configuration.

use serde::Deserialize;

/// Which gears the inbox asks, in the order it asks them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalsConfig {
    /// Stable source names (`pricing`, `products`). A name with no registration is unavailable.
    pub sources: Vec<String>,
}
