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
