//! Typed pricing invoice and entry policy projections required by reads.

use uuid::Uuid;

use crate::Digest;

/// `BillingCycle` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BillingCycle {
    Month,
    Year,
}

/// `Timezone` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Timezone {
    Utc,
}

/// `RatingWindow` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RatingWindow {
    BillingCycle,
    CalendarHour { timezone: Timezone },
}

/// `AggregationScope` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AggregationScope {
    SubscriptionLine,
    Resource,
}

/// `Reset` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reset {
    RatingWindowStart,
}

/// `Fold` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fold {
    Sum,
}

/// `PartialWindow` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartialWindow {
    ActualQuantityFullThresholds,
}

/// `MeterRef` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeterRef {
    /// Usage type id.
    pub usage_type_id: String,
    /// Version.
    pub version: String,
}

/// `QuantitySemantics` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuantitySemantics {
    /// Meter.
    pub meter: MeterRef,

    /// Unit.
    pub unit: String,

    /// Fold.
    pub fold: Fold,

    /// Accrual policy version.
    pub accrual_policy_version: String,
}

/// `UsageRatingPolicyInput` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRatingPolicyInput {
    /// Rating window.
    pub rating_window: RatingWindow,

    /// Aggregation scope.
    pub aggregation_scope: AggregationScope,

    /// Reset.
    pub reset: Reset,

    /// Quantity semantics.
    pub quantity_semantics: QuantitySemantics,

    /// Partial window.
    pub partial_window: PartialWindow,
}

/// `UsageRatingPolicy` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageRatingPolicy {
    /// Policy id.
    pub policy_id: Uuid,

    /// Version.
    pub version: u64,

    /// Digest.
    pub digest: Digest,

    /// Content.
    pub content: UsageRatingPolicyInput,
}

/// `BillingTiming` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BillingTiming {
    Advance,
    Arrears,
}

/// `InputSource` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSource {
    Entry,
    SkuVersion,
    SellerSettings,
}

/// `Rounding` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rounding {
    HalfEven,
}

/// `InvoiceInputs` value in the versioned pricing read contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvoiceInputs {
    /// Template.
    pub template: String,

    /// Template digest.
    pub template_digest: Digest,

    /// Template source.
    pub template_source: InputSource,

    /// Gl code.
    pub gl_code: String,

    /// Tax category.
    pub tax_category: String,

    /// Timing.
    pub timing: BillingTiming,

    /// Currency scale.
    pub currency_scale: u32,

    /// Rounding.
    pub rounding: Rounding,
}

/// Invoice-period origin, resolved before Pricing is called.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BillingAnchor {
    /// First day of the calendar month or year at midnight UTC.
    Calendar,
    /// Explicit subscription anniversary; Pricing never rounds it.
    SubscriptionStart,
}
/// Provenance of the resolved invoice terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermsSource {
    /// Explicit order intent.
    ExplicitOrder,
    /// Immutable seller policy version.
    SellerPolicy {
        /// Policy identity.
        id: Uuid,
        /// Positive policy version.
        version: u64,
    },
}
/// Versioned consumer projection of Subscriptions-owned invoice terms.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BillingTerms {
    /// Supported new-sale snapshot version is 1.
    pub schema_version: u32,
    /// Invoice frequency, independent of usage rating windows.
    pub cycle: BillingCycle,
    /// Resolved anchor convention.
    pub anchor: BillingAnchor,
    /// Resolved anchor instant, preserved without normalization.
    pub anchor_at: time::OffsetDateTime,
    /// Commercial timezone.
    pub timezone: Timezone,
    /// Explicit provenance.
    pub source: TermsSource,
    /// Canonical snapshot digest, excluding this field itself.
    pub digest: Digest,
}

impl std::str::FromStr for BillingCycle {
    type Err = crate::acceptance::UnsupportedCommercialValue;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "month" => Ok(Self::Month),
            "year" => Ok(Self::Year),
            _ => Err(crate::acceptance::UnsupportedCommercialValue {
                field: "cycle",
                value: value.into(),
                reason: crate::acceptance::CommercialReason::UnsupportedTerms,
            }),
        }
    }
}

impl std::str::FromStr for BillingAnchor {
    type Err = crate::acceptance::UnsupportedCommercialValue;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "calendar" => Ok(Self::Calendar),
            "subscription_start" => Ok(Self::SubscriptionStart),
            _ => Err(crate::acceptance::UnsupportedCommercialValue {
                field: "anchor",
                value: value.into(),
                reason: crate::acceptance::CommercialReason::UnsupportedTerms,
            }),
        }
    }
}

impl std::str::FromStr for Timezone {
    type Err = crate::acceptance::UnsupportedCommercialValue;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "UTC" => Ok(Self::Utc),
            _ => Err(crate::acceptance::UnsupportedCommercialValue {
                field: "timezone",
                value: value.into(),
                reason: crate::acceptance::CommercialReason::UnsupportedWindow,
            }),
        }
    }
}

impl std::str::FromStr for Rounding {
    type Err = crate::acceptance::UnsupportedCommercialValue;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "half_even" => Ok(Self::HalfEven),
            _ => Err(crate::acceptance::UnsupportedCommercialValue {
                field: "rounding",
                value: value.into(),
                reason: crate::acceptance::CommercialReason::UnsupportedTerms,
            }),
        }
    }
}

impl std::str::FromStr for AggregationScope {
    type Err = crate::acceptance::UnsupportedCommercialValue;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "subscription_line" => Ok(Self::SubscriptionLine),
            "resource" => Ok(Self::Resource),
            _ => Err(crate::acceptance::UnsupportedCommercialValue {
                field: "aggregation_scope",
                value: value.into(),
                reason: crate::acceptance::CommercialReason::UnsupportedScope,
            }),
        }
    }
}
