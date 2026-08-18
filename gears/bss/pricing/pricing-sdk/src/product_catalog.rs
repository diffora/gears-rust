//! The Product & SKU registry's **read** contract, as this gear needs it.
//!
//! # What the catalog actually needs from the registry, and nothing more
//!
//! A price row is keyed on a **meter** and a plan may be **bound to a SKU**, and
//! both of those are strings and ids this gear takes as given: it owns no
//! registry, validates neither against one, and is not made correct by having
//! this port. What the port buys is the one thing missing without it — an
//! operator authoring a row or binding a plan has **no way to know what the
//! catalog sells**, so the pick-lists can only ever offer what the tenant has
//! already used, and a first row for a new SKU is typed from memory.
//!
//! So this is a suggestion source, deliberately: `list_skus` and nothing else.
//! No create, no lifecycle, no resolution — those belong to the registry's own
//! authoring surface (`cpt-cf-bss-products-interface-authoring-publish`), and a
//! gear that could write the catalog it prices would be the second author of it.
//!
//! # Why it lives in the SDK
//!
//! [`CatalogVersionRegistryV1`](crate::catalog_version_registry)'s reason,
//! unchanged: the contract belongs where the registry gear can implement it
//! without depending on `bss-pricing`. The two ports are the same relationship
//! seen from two sides — that one asks the registry to increment, this one asks
//! it what it holds.
//!
//! # Failing is not the same as answering "none"
//!
//! Nothing depends on this list, so the fail-closed posture the version registry
//! takes would be wrong here: a publish must not proceed on an invented version,
//! but a pick-list with no suggestions is merely a pick-list with no
//! suggestions. What must **not** happen is the two being confused. An empty
//! catalog and an unreachable registry are opposite facts for an operator about
//! to type a SKU id from memory, and the surface over this port is required to
//! tell them apart — which is why the error is a typed absence rather than an
//! empty `Vec`.

use async_trait::async_trait;
use toolkit_security::SecurityContext;
use uuid::Uuid;

/// One SKU, as much of it as pricing has any business knowing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSku {
    /// The registry's id — what a plan's `sku_id` binds to.
    pub sku_id: Uuid,
    /// The operator-facing code, e.g. `COMP-VCPU-H`.
    pub sku_code: String,
    /// The human name.
    pub name: String,
    /// **The declared metering unit, and the whole reason a row picker wants
    /// this list.** A SKU that declares one is a usage SKU; one that does not is
    /// priced per period. There is no separate flag — the presence of the unit
    /// *is* the fact, which is how the registry PRD models it too.
    pub metering_unit: Option<String>,
    /// `draft` | `published` | `deprecated`, verbatim. Not an enum: the registry
    /// owns this vocabulary and a fifth state must not become a parse failure
    /// in the gear that merely displays it.
    pub status: String,
    /// The registry-owned tier, which a plan's own tier is checked against.
    pub plan_tier: Option<String>,
}

/// Why the catalog could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProductCatalogError {
    /// No registry is wired. The ordinary state of this deployment, and not a
    /// fault: it is reported so a surface can say "not asked" instead of "none".
    Unconfigured,
    /// A registry is configured and did not answer.
    Unreachable(String),
    /// It answered, and the answer could not be used.
    Internal(String),
}

impl std::fmt::Display for ProductCatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unconfigured => f.write_str("no product catalog is configured"),
            Self::Unreachable(detail) => write!(f, "product catalog unreachable: {detail}"),
            Self::Internal(detail) => write!(f, "product catalog error: {detail}"),
        }
    }
}

impl std::error::Error for ProductCatalogError {}

/// The registry's read contract
/// (`cpt-cf-bss-products-interface-read-model`, the browse half).
#[async_trait]
pub trait ProductCatalogClientV1: Send + Sync {
    /// Every SKU this tenant may price, in the registry's own order.
    ///
    /// Unpaginated on purpose. A catalog large enough to need paging is a
    /// catalog whose pick-list needed a search box instead, and that is a
    /// decision for the surface that has one — not something to pretend to
    /// support with a cursor nobody passes.
    ///
    /// # Errors
    /// [`ProductCatalogError`] when no registry is wired, it cannot be reached,
    /// or its answer is unusable. **None of these is an empty catalog**, and a
    /// caller that renders them as one is telling an operator the tenant sells
    /// nothing.
    async fn list_skus(
        &self,
        ctx: &SecurityContext,
    ) -> Result<Vec<CatalogSku>, ProductCatalogError>;
}

/// The default until the registry gear exists: it says so, every time.
///
/// Not an empty list. The distinction is the whole point of the error type —
/// see the module doc.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredProductCatalogClientV1;

#[async_trait]
impl ProductCatalogClientV1 for UnconfiguredProductCatalogClientV1 {
    async fn list_skus(
        &self,
        _ctx: &SecurityContext,
    ) -> Result<Vec<CatalogSku>, ProductCatalogError> {
        Err(ProductCatalogError::Unconfigured)
    }
}
