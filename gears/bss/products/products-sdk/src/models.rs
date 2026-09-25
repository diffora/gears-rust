//! Wire types of the SKU registry. Spec §4 and §2.2 (`SkuVersion`).
use serde::{Deserialize, Serialize};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkuType {
    Recurring,
    Usage,
    OneTime,
    Bundle,
}
impl SkuType {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Recurring => "recurring",
            Self::Usage => "usage",
            Self::OneTime => "one_time",
            Self::Bundle => "bundle",
        }
    }
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "recurring" => Some(Self::Recurring),
            "usage" => Some(Self::Usage),
            "one_time" => Some(Self::OneTime),
            "bundle" => Some(Self::Bundle),
            _ => None,
        }
    }
}

/// `retiring` is the committed fence of spec §4: set before pricing is asked for references.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    Draft,
    Published,
    Deprecated,
    Retiring,
    Retired,
}
impl Lifecycle {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Deprecated => "deprecated",
            Self::Retiring => "retiring",
            Self::Retired => "retired",
        }
    }
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "draft" => Some(Self::Draft),
            "published" => Some(Self::Published),
            "deprecated" => Some(Self::Deprecated),
            "retiring" => Some(Self::Retiring),
            "retired" => Some(Self::Retired),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingTiming {
    Advance,
    Arrears,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Category {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub code: String,
    pub name: String,
    pub is_default: bool,
    pub sort_order: i32,
    pub status: String,
    pub version: i64,
}

/// The registry's SKU as the doors return it. Field names are `snake_case` on the wire (the
/// `api_dto` macro's rule, conv §4); consumers read them as such.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct Sku {
    pub id: Uuid,
    pub tenant_id: Uuid,
    pub code: String,
    pub name: String,
    pub r#type: SkuType,
    pub category_id: Uuid,
    pub description: String,
    pub sellable: bool,
    pub lifecycle: Lifecycle,
    pub revision: i64,
    pub published_version: i64,
    pub gl_code: Option<String>,
    pub tax_category: Option<String>,
    pub invoice_line_template: Option<String>,
    pub billing_timing: Option<BillingTiming>,
    pub usage_type_ref: Option<String>,
    pub unit: Option<String>,
    pub type_change_pending: bool,
    pub pending_unit_id: Option<Uuid>,
    pub approved_by_unit_id: Option<Uuid>,
    pub created_by: Uuid,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

/// The business content of a SKU — what an approval unit fingerprints (spec §6: never lock,
/// version, revision or lifecycle fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SkuContent {
    pub code: String,
    pub name: String,
    pub r#type: SkuType,
    pub category_id: Uuid,
    pub description: String,
    pub sellable: bool,
    pub gl_code: Option<String>,
    pub tax_category: Option<String>,
    pub invoice_line_template: Option<String>,
    pub billing_timing: Option<BillingTiming>,
    pub usage_type_ref: Option<String>,
    pub unit: Option<String>,
}
impl From<&Sku> for SkuContent {
    fn from(s: &Sku) -> Self {
        Self {
            code: s.code.clone(),
            name: s.name.clone(),
            r#type: s.r#type,
            category_id: s.category_id,
            description: s.description.clone(),
            sellable: s.sellable,
            gl_code: s.gl_code.clone(),
            tax_category: s.tax_category.clone(),
            invoice_line_template: s.invoice_line_template.clone(),
            billing_timing: s.billing_timing,
            usage_type_ref: s.usage_type_ref.clone(),
            unit: s.unit.clone(),
        }
    }
}

/// One published version of a SKU, appended on publish and on every applied change (spec §2.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SkuVersion {
    pub sku_id: Uuid,
    pub published_version: i64,
    pub effective_from: Date,
    pub content: SkuContent,
    pub created_at: OffsetDateTime,
}

/// Payload of `SkuChanged`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SkuChangedPayload {
    pub sku_id: Uuid,
    pub tenant_id: Uuid,
    pub changed: Vec<String>,
    pub effective_from: Date,
    pub published_version: i64,
}

#[cfg(test)]
mod registry_tests {
    #[test]
    fn sku_type_uses_the_pricebook_vocabulary() {
        for token in ["recurring", "usage", "one_time", "bundle"] {
            assert_eq!(
                super::SkuType::parse(token).map(super::SkuType::as_str),
                Some(token)
            );
        }
        for token in ["offer", "component", "product", "service", "", "Usage"] {
            assert_eq!(super::SkuType::parse(token), None);
        }
    }
}

/// Kind of owner object protected by a reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceKind {
    Price,
    PlanItem,
    SoldAs,
}
/// Released attempts remain tombstones and no longer block a fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReferenceState {
    Reserved,
    Confirmed,
    Released,
}
/// The attempt handle used for confirmation, release and reconciliation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct ReservationReceipt {
    pub reservation_id: Uuid,
    pub state: ReferenceState,
}
