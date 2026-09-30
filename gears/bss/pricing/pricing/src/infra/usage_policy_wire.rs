//! Closed policy wire/storage adapters; SDK value types remain infrastructure-free.
use bss_pricing_sdk::terms as sdk;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum Timezone {
    #[serde(rename = "UTC")]
    Utc,
}
impl From<Timezone> for sdk::Timezone {
    fn from(value: Timezone) -> Self {
        match value {
            Timezone::Utc => Self::Utc,
        }
    }
}
impl From<sdk::Timezone> for Timezone {
    fn from(value: sdk::Timezone) -> Self {
        match value {
            sdk::Timezone::Utc => Self::Utc,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum AggregationScope {
    #[serde(rename = "subscription_line")]
    SubscriptionLine,
    #[serde(rename = "resource")]
    Resource,
}
impl From<AggregationScope> for sdk::AggregationScope {
    fn from(value: AggregationScope) -> Self {
        match value {
            AggregationScope::SubscriptionLine => Self::SubscriptionLine,
            AggregationScope::Resource => Self::Resource,
        }
    }
}
impl From<sdk::AggregationScope> for AggregationScope {
    fn from(value: sdk::AggregationScope) -> Self {
        match value {
            sdk::AggregationScope::SubscriptionLine => Self::SubscriptionLine,
            sdk::AggregationScope::Resource => Self::Resource,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum Reset {
    #[serde(rename = "rating_window_start")]
    RatingWindowStart,
}
impl From<Reset> for sdk::Reset {
    fn from(value: Reset) -> Self {
        match value {
            Reset::RatingWindowStart => Self::RatingWindowStart,
        }
    }
}
impl From<sdk::Reset> for Reset {
    fn from(value: sdk::Reset) -> Self {
        match value {
            sdk::Reset::RatingWindowStart => Self::RatingWindowStart,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum Fold {
    #[serde(rename = "SUM")]
    Sum,
}
impl From<Fold> for sdk::Fold {
    fn from(value: Fold) -> Self {
        match value {
            Fold::Sum => Self::Sum,
        }
    }
}
impl From<sdk::Fold> for Fold {
    fn from(value: sdk::Fold) -> Self {
        match value {
            sdk::Fold::Sum => Self::Sum,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub enum PartialWindow {
    #[serde(rename = "actual_quantity_full_thresholds")]
    ActualQuantityFullThresholds,
}
impl From<PartialWindow> for sdk::PartialWindow {
    fn from(value: PartialWindow) -> Self {
        match value {
            PartialWindow::ActualQuantityFullThresholds => Self::ActualQuantityFullThresholds,
        }
    }
}
impl From<sdk::PartialWindow> for PartialWindow {
    fn from(value: sdk::PartialWindow) -> Self {
        match value {
            sdk::PartialWindow::ActualQuantityFullThresholds => Self::ActualQuantityFullThresholds,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RatingWindow {
    BillingCycle,
    CalendarHour { timezone: Timezone },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MeterRef {
    pub usage_type_id: String,
    pub version: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct QuantitySemantics {
    pub meter: MeterRef,
    pub unit: String,
    pub fold: Fold,
    pub accrual_policy_version: String,
}
/// Author input contains no server-assigned policy identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UsageRatingPolicyInput {
    pub rating_window: RatingWindow,
    pub aggregation_scope: AggregationScope,
    pub reset: Reset,
    pub quantity_semantics: QuantitySemantics,
    pub partial_window: PartialWindow,
}
impl From<&UsageRatingPolicyInput> for sdk::UsageRatingPolicyInput {
    fn from(p: &UsageRatingPolicyInput) -> Self {
        let q = &p.quantity_semantics;
        Self {
            rating_window: match p.rating_window.clone() {
                RatingWindow::BillingCycle => sdk::RatingWindow::BillingCycle,
                RatingWindow::CalendarHour { timezone } => sdk::RatingWindow::CalendarHour {
                    timezone: timezone.into(),
                },
            },
            aggregation_scope: p.aggregation_scope.into(),
            reset: p.reset.into(),
            quantity_semantics: sdk::QuantitySemantics {
                meter: sdk::MeterRef {
                    usage_type_id: q.meter.usage_type_id.clone(),
                    version: q.meter.version.clone(),
                },
                unit: q.unit.clone(),
                fold: q.fold.into(),
                accrual_policy_version: q.accrual_policy_version.clone(),
            },
            partial_window: p.partial_window.into(),
        }
    }
}
impl From<&sdk::UsageRatingPolicyInput> for UsageRatingPolicyInput {
    fn from(p: &sdk::UsageRatingPolicyInput) -> Self {
        let q = &p.quantity_semantics;
        Self {
            rating_window: match p.rating_window.clone() {
                sdk::RatingWindow::BillingCycle => RatingWindow::BillingCycle,
                sdk::RatingWindow::CalendarHour { timezone } => RatingWindow::CalendarHour {
                    timezone: timezone.into(),
                },
            },
            aggregation_scope: p.aggregation_scope.clone().into(),
            reset: p.reset.clone().into(),
            quantity_semantics: QuantitySemantics {
                meter: MeterRef {
                    usage_type_id: q.meter.usage_type_id.clone(),
                    version: q.meter.version.clone(),
                },
                unit: q.unit.clone(),
                fold: q.fold.clone().into(),
                accrual_policy_version: q.accrual_policy_version.clone(),
            },
            partial_window: p.partial_window.clone().into(),
        }
    }
}
/// Materialized immutable policy. Version is an exact decimal string on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UsageRatingPolicy {
    pub policy_id: Uuid,
    pub version: String,
    pub digest: String,
    pub content: UsageRatingPolicyInput,
}
/// Lowercase canonical SHA-256 storage text.
#[must_use]
pub fn digest_text(digest: bss_pricing_sdk::Digest) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in digest {
        text.push(char::from(HEX[usize::from(byte >> 4)]));
        text.push(char::from(HEX[usize::from(byte & 15)]));
    }
    text
}
