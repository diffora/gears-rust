//! Local registry vocabulary and reservation/fence eligibility (decision 17).
use crate::domain::error::DomainError;
use bss_products_sdk::models::Lifecycle;
use toolkit_macros::domain_model;
/// The object in an owner gear that depends on a SKU.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Price,
    PlanItem,
    SoldAs,
}
impl RefKind {
    /// Stable storage token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Price => "price",
            Self::PlanItem => "plan_item",
            Self::SoldAs => "sold_as",
        }
    }
}
/// Released attempts remain as history and no longer block a fence.
#[domain_model]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefState {
    Reserved,
    Confirmed,
    Released,
}
impl RefState {
    /// Stable storage token.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reserved => "reserved",
            Self::Confirmed => "confirmed",
            Self::Released => "released",
        }
    }
}
/// Live counts by owner object, with the reserved subset reported separately.
/// `prices` and `plans` include reserved and confirmed rows; `plans` includes
/// both plan items and sold-as references. `reserved` is not an extra total.
#[domain_model]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReferenceSummary {
    pub prices: u32,
    pub plans: u32,
    pub reserved: u32,
}
/// Check local eligibility before a reservation is written in the transaction.
/// @cpt-cf-bss-products-fr-reference-registry
///
/// # Errors
/// `SKU_RETIRING` while retiring; `SKU_FENCED` for other fences or inactive heads.
pub fn reservation_allowed(lifecycle: Lifecycle, fenced: bool) -> Result<(), DomainError> {
    if fenced {
        return Err(DomainError::Conflict {
            code: "SKU_FENCED",
            detail: "the SKU is fenced".into(),
        });
    }
    if lifecycle == Lifecycle::Retiring {
        return Err(DomainError::Conflict {
            code: "SKU_RETIRING",
            detail: "the SKU is retiring".into(),
        });
    }
    if fenced || !matches!(lifecycle, Lifecycle::Published | Lifecycle::Deprecated) {
        return Err(DomainError::Conflict {
            code: "SKU_FENCED",
            detail: "the SKU does not admit new references".into(),
        });
    }
    Ok(())
}
/// A reserved row counts until explicitly confirmed or released; time never expires it.
/// @cpt-cf-bss-products-fr-reference-registry
///
/// # Errors
/// Returns the caller's fence refusal code when any live reference remains.
pub fn fence_allowed(live_references: u32, code: &'static str) -> Result<(), DomainError> {
    if live_references > 0 {
        return Err(DomainError::Conflict {
            code,
            detail: format!("{live_references} live reference(s) block this fence"),
        });
    }
    Ok(())
}
#[cfg(test)]
#[path = "references_tests.rs"]
mod references_tests;
