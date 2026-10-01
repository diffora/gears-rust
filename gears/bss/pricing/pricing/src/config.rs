//! Default-tolerant deployment configuration and versioned seller hold policy.

/// Retired deployment keys remain accepted; commercial policy is validated at startup.
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct BssPricingConfig {
    /// Versioned duration observed when issuing an acceptance.
    pub seller_hold_policy: SellerHoldPolicy,
}

/// A deployment's seller policy; changing it never rewrites issued receipts.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SellerHoldPolicy {
    /// Positive immutable policy version named by new-sale requests.
    pub version: u64,
    /// Positive duration from the server-issued acceptance instant.
    pub duration_seconds: u32,
}
impl Default for SellerHoldPolicy {
    fn default() -> Self {
        Self {
            version: 1,
            duration_seconds: 86_400,
        }
    }
}
impl SellerHoldPolicy {
    /// Refuse invalid policy before registering any commercial provider.
    /// # Errors
    /// Zero version or duration is not a usable seller policy.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.version > 0,
            "seller_hold_policy.version must be positive"
        );
        anyhow::ensure!(
            self.duration_seconds > 0,
            "seller_hold_policy.duration_seconds must be positive"
        );
        Ok(())
    }
}
