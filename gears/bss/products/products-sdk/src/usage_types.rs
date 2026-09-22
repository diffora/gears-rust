//! The usage-type catalog as this registry sees it — one narrow port for one
//! external dependency (**P-D-05**).
//!
//! # Why the port is here and not bound to the collector's own client
//!
//! The gear used to take `dyn UsageCollectorClientV1` from `ClientHub`, so a
//! deployment whose usage types come from something other than that collector
//! had to implement the whole of it — records, batch writes, aggregation,
//! deactivation, deletion — to supply a catalog of type ids. This trait is the
//! part products actually needs, and a module that registers one is preferred
//! over the collector adapter.
//!
//! # Why both methods sit on one trait
//!
//! A supplier that fills the authoring pick-list also answers the publish gate.
//! Split them and a deployment lists from one catalog while gating against
//! another; the disagreement reaches an operator as *"I picked it from your
//! list and publish says it does not exist"*.
//!
//! # No serde here
//!
//! This crate's own rule (see the crate doc): the gear's REST DTOs own serde
//! and map onto these types, so a wire concern stays out of the contract.

use async_trait::async_trait;
use toolkit_canonical_errors::{CanonicalError, resource_error};
use toolkit_security::SecurityContext;

#[resource_error(gts_id!("cf.bss.products.recognized_set.v1~"))]
struct UsageTypeCatalogResource;

/// The canonical error every implementation owes when no catalog is configured
/// — a **501**, never an empty page.
///
/// Public, and public for pricing's reason one gear over: every implementation
/// of the port owes the same answer to the same fact, and a second spelling of
/// it is a second thing a caller has to recognise.
#[must_use]
pub fn unconfigured_usage_type_catalog() -> CanonicalError {
    UsageTypeCatalogResource::unimplemented("no usage-type catalog is configured").create()
}

/// The canonical error when a configured catalog did not answer — a **503**,
/// and distinct from the 501 above because the operator's next act differs:
/// configure one, versus retry or go and look at the one that is configured.
#[must_use]
pub fn usage_type_catalog_unreachable(detail: impl Into<String>) -> CanonicalError {
    CanonicalError::service_unavailable()
        .with_detail(detail)
        .create()
}

/// One usage type, in the three fields this registry reads.
///
/// Nothing else the catalog knows is carried: **P-D-05 is resolvability
/// only**, so a lifecycle state or a dimension set would be a fact this gear
/// promises to judge and does not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageTypeBinding {
    /// The GTS id a meter declaration names — a derived type of the usage
    /// record base, `…usage_record.v1~…`.
    pub gts_id: String,
    /// `counter` or `gauge`, as the catalog spells it.
    pub kind: String,
    /// The metadata keys the catalog declares for this type, in its own order.
    pub metadata_fields: Vec<String>,
}

/// What the catalog answers about one id.
///
/// **Three values, not two**, and the third is the point: a catalog that says
/// *no* and a catalog that says *nothing* are different facts, and the publish
/// gate treats them differently — the first is a refusal the author can fix,
/// the second is fail-closed and retryable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UsageTypeAnswer {
    /// The catalog knows this id.
    Resolved(UsageTypeBinding),
    /// The catalog answered, and the answer is no — **or** the id is not a
    /// valid GTS id at all, which cannot name anything anywhere and so needs
    /// no round trip to refuse.
    Unresolved,
    /// The catalog did not answer: absent, unreachable, or past the caller's
    /// deadline.
    Unavailable,
}

/// One page of the authoring pick-list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageTypePage {
    /// The page's types, in the catalog's own order.
    pub items: Vec<UsageTypeBinding>,
    /// The cursor that walks forward, absent on the last page.
    pub next_cursor: Option<String>,
    /// The cursor that walks back, absent on the first.
    pub prev_cursor: Option<String>,
}

/// The catalog a deployment binds this registry to.
///
/// Registered on `ClientHub` as `dyn UsageTypeCatalog`; the gear prefers a
/// registered implementation over its own adapter over the usage collector,
/// and over any configured fallback.
#[async_trait]
pub trait UsageTypeCatalog: Send + Sync + 'static {
    /// Does this id name a usage type?
    ///
    /// **Resolvability and nothing else** (P-D-05): no lifecycle check, no
    /// dimension check. An implementation that cannot answer must say
    /// [`UsageTypeAnswer::Unavailable`] rather than guess, because the publish
    /// gate fails closed on it and would otherwise freeze a meter nobody
    /// confirmed.
    async fn resolve(&self, ctx: &SecurityContext, usage_type_ref: &str) -> UsageTypeAnswer;

    /// The authoring pick-list, paged.
    ///
    /// `q` narrows by substring of the id, `kind` by equality. An
    /// implementation that cannot narrow may ignore either, but must not
    /// answer a page it did not narrow as though it had.
    ///
    /// # Errors
    ///
    /// [`CanonicalError`] when the catalog is unconfigured, unreachable or
    /// unusable. **None of those is an empty page.** A caller must be able to
    /// tell *"this deployment has no usage types"* from *"nobody could be
    /// asked"*, and an implementation that collapses them hands an operator
    /// silence to read as a clean answer.
    async fn list(
        &self,
        ctx: &SecurityContext,
        q: Option<&str>,
        kind: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<UsageTypePage, CanonicalError>;
}
