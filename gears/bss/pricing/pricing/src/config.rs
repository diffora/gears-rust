//! Default-tolerant deployment configuration during the model rebuild.

/// Retired deployment keys are accepted while pricing has no configurable behavior.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
#[allow(
    clippy::empty_structs_with_brackets,
    reason = "serde must accept a configuration map, not a unit value"
)]
pub struct BssPricingConfig {}
