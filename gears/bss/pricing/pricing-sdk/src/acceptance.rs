//! Commercial query values. Acceptance methods and persistence are separate capabilities.
use crate::{Digest, read::BindingSelection, terms::BillingTerms};
use rust_decimal::Decimal;
use time::OffsetDateTime;
use uuid::Uuid;

/// Independently authorized tenant axes supplied by Orders.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_field_names)] // Contract names distinguish all three tenant axes.
pub struct TenantAxes {
    /// Catalog owner.
    pub seller_tenant_id: Uuid,
    /// Payer.
    pub payer_tenant_id: Uuid,
    /// Resource owner.
    pub resource_tenant_id: Uuid,
}
/// Market accepted by the buyer; no foreign exchange is performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Market {
    /// Exact currency code.
    pub currency: String,
    /// Requested market region, when present.
    pub region: Option<String>,
}
/// Duration in invoice periods, independent of usage windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Term {
    /// Continues until ended.
    Rolling,
    /// Positive number of invoice periods.
    FixedPeriods {
        /// Count of periods.
        count: u32,
    },
}
/// Complete commercial intent; live eligibility facts are never caller supplied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSaleQuery {
    /// Seller, payer and resource ownership axes.
    pub tenant_axes: TenantAxes,
    /// Ordering identity.
    pub order_id: Uuid,
    /// Positive order version.
    pub order_version: u64,
    /// Order line identity.
    pub line_id: Uuid,
    /// Selected plan.
    pub plan_id: Uuid,
    /// Selected published revision.
    pub plan_revision_id: Uuid,
    /// Exactly one requested cell per item.
    pub selections: Vec<BindingSelection>,
    /// Positive exact plan quantity.
    pub quantity: Decimal,
    /// Accepted currency and region.
    pub market: Market,
    /// Earliest activation instant, independent of the invoice anchor.
    pub start_at: OffsetDateTime,
    /// Accepted duration.
    pub term: Term,
    /// Resolved Subscriptions-owned snapshot; Pricing never chooses a default.
    pub billing_terms: BillingTerms,
    /// Caller preview, compared with independently resolved bindings by the command layer.
    pub resolved_bindings_digest: Digest,
    /// Positive version of the seller's hold policy.
    pub hold_policy_version: u64,
}

/// Stable commercial refusal metadata, independent of transport and provider failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommercialReason {
    /// `UNSUPPORTED_TERMS` refusal.
    UnsupportedTerms,
    /// `UNSUPPORTED_MODEL` refusal.
    UnsupportedModel,
    /// `UNSUPPORTED_WINDOW` refusal.
    UnsupportedWindow,
    /// `UNSUPPORTED_SCOPE` refusal.
    UnsupportedScope,
    /// `MISSING_BILLING_TERMS` refusal.
    MissingBillingTerms,
    /// `MISSING_RATING_POLICY` refusal.
    MissingRatingPolicy,
    /// `METER_POLICY_MISMATCH` refusal.
    MeterPolicyMismatch,
    /// `UNALIGNED_BILLING_ANCHOR` refusal.
    UnalignedBillingAnchor,
    /// `BILLING_CYCLE_MISMATCH` refusal.
    BillingCycleMismatch,
    /// `QUANTITY_INVALID` refusal.
    InvalidQuantity,
    /// `TERM_COUNT_INVALID` refusal.
    InvalidTermCount,
    /// `SELECTION_INCOMPLETE` refusal.
    IncompleteSelection,
    /// `BINDING_ENTRY_MISMATCH` refusal.
    BindingEntryMismatch,
    /// `CURRENCY_MISMATCH` refusal.
    CurrencyMismatch,
    /// `MARKET_MISMATCH` refusal.
    MarketMismatch,
    /// `DIMENSION_MISMATCH` refusal.
    DimensionMismatch,
    /// `INCOMPLETE_COMMERCIAL_INPUTS` refusal.
    IncompleteCommercialInputs,
    /// `BILLING_TERMS_DIGEST_MISMATCH` refusal.
    BillingTermsDigestMismatch,
    /// `MONEY_DIGEST_MISMATCH` refusal.
    MoneyDigestMismatch,
    /// `TEMPLATE_DIGEST_MISMATCH` refusal.
    TemplateDigestMismatch,
    /// `AMOUNT_INVALID` refusal.
    InvalidMoney,
    /// `INVALID_TIERS` refusal.
    InvalidTiers,
    /// `NOT_SELLABLE` refusal.
    NotSellable,
    /// `RESOLUTION_CHANGED` refusal.
    ResolutionChanged,
    /// `ACCEPTANCE_MISMATCH` refusal.
    AcceptanceMismatch,
    /// `HOLD_EXPIRED` refusal.
    HoldExpired,
    /// `PRICE_CLOSED` refusal.
    PriceClosed,
    /// `SKU_RETIRED` refusal.
    SkuRetired,
    /// `MARKET_CHANGED` refusal.
    MarketChanged,
    /// `PERMISSION_DENIED` refusal.
    PermissionDenied,
    /// `RECEIPT_NOT_FOUND` refusal.
    ReceiptNotFound,
}
impl CommercialReason {
    /// Stable domain code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedTerms => "UNSUPPORTED_TERMS",
            Self::UnsupportedModel => "UNSUPPORTED_MODEL",
            Self::UnsupportedWindow => "UNSUPPORTED_WINDOW",
            Self::UnsupportedScope => "UNSUPPORTED_SCOPE",
            Self::MissingBillingTerms => "MISSING_BILLING_TERMS",
            Self::MissingRatingPolicy => "MISSING_RATING_POLICY",
            Self::MeterPolicyMismatch => "METER_POLICY_MISMATCH",
            Self::UnalignedBillingAnchor => "UNALIGNED_BILLING_ANCHOR",
            Self::BillingCycleMismatch => "BILLING_CYCLE_MISMATCH",
            Self::InvalidQuantity => "QUANTITY_INVALID",
            Self::InvalidTermCount => "TERM_COUNT_INVALID",
            Self::IncompleteSelection => "SELECTION_INCOMPLETE",
            Self::BindingEntryMismatch => "BINDING_ENTRY_MISMATCH",
            Self::CurrencyMismatch => "CURRENCY_MISMATCH",
            Self::MarketMismatch => "MARKET_MISMATCH",
            Self::DimensionMismatch => "DIMENSION_MISMATCH",
            Self::IncompleteCommercialInputs => "INCOMPLETE_COMMERCIAL_INPUTS",
            Self::BillingTermsDigestMismatch => "BILLING_TERMS_DIGEST_MISMATCH",
            Self::MoneyDigestMismatch => "MONEY_DIGEST_MISMATCH",
            Self::TemplateDigestMismatch => "TEMPLATE_DIGEST_MISMATCH",
            Self::InvalidMoney => "AMOUNT_INVALID",
            Self::InvalidTiers => "INVALID_TIERS",
            Self::NotSellable => "NOT_SELLABLE",
            Self::ResolutionChanged => "RESOLUTION_CHANGED",
            Self::AcceptanceMismatch => "ACCEPTANCE_MISMATCH",
            Self::HoldExpired => "HOLD_EXPIRED",
            Self::PriceClosed => "PRICE_CLOSED",
            Self::SkuRetired => "SKU_RETIRED",
            Self::MarketChanged => "MARKET_CHANGED",
            Self::PermissionDenied => "PERMISSION_DENIED",
            Self::ReceiptNotFound => "RECEIPT_NOT_FOUND",
        }
    }
    /// Concrete reason retained in canonical error metadata.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnsupportedTerms => "UnsupportedTerms",
            Self::UnsupportedModel => "UnsupportedModel",
            Self::UnsupportedWindow => "UnsupportedWindow",
            Self::UnsupportedScope => "UnsupportedScope",
            Self::MissingBillingTerms => "MissingBillingTerms",
            Self::MissingRatingPolicy => "MissingRatingPolicy",
            Self::MeterPolicyMismatch => "MeterPolicyMismatch",
            Self::UnalignedBillingAnchor => "UnalignedBillingAnchor",
            Self::BillingCycleMismatch => "BillingCycleMismatch",
            Self::InvalidQuantity => "InvalidQuantity",
            Self::InvalidTermCount => "InvalidTermCount",
            Self::IncompleteSelection => "IncompleteSelection",
            Self::BindingEntryMismatch => "BindingEntryMismatch",
            Self::CurrencyMismatch => "CurrencyMismatch",
            Self::MarketMismatch => "MarketMismatch",
            Self::DimensionMismatch => "DimensionMismatch",
            Self::IncompleteCommercialInputs => "IncompleteCommercialInputs",
            Self::BillingTermsDigestMismatch => "BillingTermsDigestMismatch",
            Self::MoneyDigestMismatch => "MoneyDigestMismatch",
            Self::TemplateDigestMismatch => "TemplateDigestMismatch",
            Self::InvalidMoney => "InvalidMoney",
            Self::InvalidTiers => "InvalidTiers",
            Self::NotSellable => "NotSellable",
            Self::ResolutionChanged => "ResolutionChanged",
            Self::AcceptanceMismatch => "AcceptanceMismatch",
            Self::HoldExpired => "HoldExpired",
            Self::PriceClosed => "PriceClosed",
            Self::SkuRetired => "SkuRetired",
            Self::MarketChanged => "MarketChanged",
            Self::PermissionDenied => "PermissionDenied",
            Self::ReceiptNotFound => "ReceiptNotFound",
        }
    }
}
#[toolkit_canonical_errors::resource_error("gts.cf.bss.pricing.plan.v1~")]
struct CommercialResource;
impl From<CommercialReason> for toolkit_canonical_errors::CanonicalError {
    fn from(reason: CommercialReason) -> Self {
        use CommercialReason as R;
        match reason {
            R::NotSellable
            | R::ResolutionChanged
            | R::AcceptanceMismatch
            | R::HoldExpired
            | R::PriceClosed
            | R::SkuRetired
            | R::MarketChanged => CommercialResource::aborted(reason.code())
                .with_reason(reason.as_str())
                .create(),
            R::PermissionDenied => CommercialResource::permission_denied()
                .with_reason(reason.as_str())
                .create(),
            R::ReceiptNotFound => CommercialResource::not_found(reason.as_str())
                .with_resource("acceptance")
                .create(),
            _ => CommercialResource::invalid_argument()
                .with_field_violation("commercial_terms", reason.code(), reason.as_str())
                .create(),
        }
    }
}

/// An unsupported wire scalar; retain its original value instead of supplying a default.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unsupported {field}: {value}")]
pub struct UnsupportedCommercialValue {
    /// Input field.
    pub field: &'static str,
    /// Exact unrecognized wire value.
    pub value: String,
    /// Stable refusal classification.
    pub reason: CommercialReason,
}
impl From<UnsupportedCommercialValue> for toolkit_canonical_errors::CanonicalError {
    fn from(value: UnsupportedCommercialValue) -> Self {
        CommercialResource::invalid_argument()
            .with_field_violation(value.field, value.to_string(), value.reason.as_str())
            .create()
    }
}
