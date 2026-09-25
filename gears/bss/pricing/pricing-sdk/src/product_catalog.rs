//! The Product & SKU registry's **read** contract, as this gear needs it.
//!
//! # What the catalog actually needs from the registry, and nothing more
//!
//! A legacy price row is keyed on a **meter** and a plan may be **bound to a SKU**, and
//! both of those are strings and ids this gear takes as given: it owns no
//! registry, validates neither against one, and is not made correct by having
//! this port. What the port buys is the one thing missing without it — an
//! operator authoring a legacy price row or binding a plan has **no way to know what the
//! catalog sells**, so the pick-lists can only ever offer what the tenant has
//! already used, and a first legacy price row for a new SKU is typed from memory.
//!
//! SKU browsing is a suggestion source. `list_tax_categories` also exposes the
//! registry-owned category dictionary: pricing owns the assignment on a legacy price row,
//! not a category default on a SKU or region. The dictionary defines codes
//! and labels, not rates or tax calculation rules.
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
//! SKU validity does not depend on the browse list, so the fail-closed posture the version registry
//! takes would be wrong here: a publish must not proceed on an invented version,
//! but a pick-list with no suggestions is merely a pick-list with no
//! suggestions. What must **not** happen is the two being confused. An empty
//! catalog and an unreachable registry are opposite facts for an operator about
//! to type a SKU id from memory, and the surface over this port is required to
//! tell them apart — which is why the error is a typed absence rather than an
//! empty `Vec`.
//!
//! # Canonical at the boundary, typed projection beside it
//!
//! Per ADR `cpt-cf-errors-adr-sdk-canonical-projection` the trait returns
//! [`CanonicalError`]; [`ProductCatalogError`] is an **opt-in** view over it, not
//! the contract. Adding a variant to the projection is not an SDK break, and the
//! catch-all [`ProductCatalogError::Other`] keeps the conversion infallible so a
//! category this port does not emit today still arrives with full fidelity.
//!
//! The three dispositions and how they are carried:
//!
//! | disposition | canonical | projection |
//! |---|---|---|
//! | no registry wired | `Unimplemented` | [`ProductCatalogError::Unconfigured`] |
//! | configured, did not answer | `ServiceUnavailable` | [`ProductCatalogError::Unreachable`] |
//! | answered unusably | `Internal` | [`ProductCatalogError::Internal`] |
//!
//! `Unimplemented` carries "no registry wired" because it is the one category
//! that says *this deployment does not offer the capability* — which is the fact,
//! and which no reason string is needed to tell apart from an outage.

use async_trait::async_trait;
use toolkit_canonical_errors::{CanonicalError, resource_error};
use toolkit_contract::ir::binding::{
    HttpBindingIr, HttpFieldBinding, HttpMethod, HttpMethodBindingIr, StreamFraming,
};
use toolkit_contract::ir::contract::{
    ContractIr, FieldIr, FieldRole, Idempotency, InputShape, MethodIr, MethodKind, PrimitiveType,
    TypeRef,
};
use toolkit_security::SecurityContext;
use uuid::Uuid;

#[resource_error(gts_id!("cf.bss.pricing.config.v1~"))]
struct CatalogResource;

/// One SKU, as much of it as pricing has any business knowing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSku {
    /// The registry's id — what a plan's `sku_id` binds to.
    pub sku_id: Uuid,
    /// The operator-facing code, e.g. `COMP-VCPU-H`.
    pub sku_code: String,
    /// The human name.
    pub name: String,
    /// **The declared metering unit, and the whole reason a price picker wants
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
    /// `product` | `service` | `bundle`, verbatim — the registry owns this
    /// vocabulary as it owns `status`, and `dod-sdk-read-shape` names the
    /// member `type` on the wire. A fourth value must not become a parse
    /// failure here.
    pub sku_type: String,
    /// Whether the SKU may be sold on its own. `false` is a composition- or
    /// metering-only member — priced, never picked as a line of its own.
    pub sellable: bool,
    /// The usage collector's `UsageType` id a usage SKU's declaration carries
    /// (registry **P-D-05**); absent on a SKU priced per period. Present
    /// exactly when `metering_unit` is, on a well-formed registry row.
    pub usage_type_ref: Option<String>,
    /// Registry-owned withdrawal from new sale. Independent of `status`: the
    /// status word is display vocabulary, this flag is the fact a write path
    /// consults, and neither is derived from the other.
    pub deprecated: bool,
}

/// One page of a prefix search over SKU names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogSkuPage {
    /// The matching SKUs in this page, in the registry's own order.
    pub items: Vec<CatalogSku>,
    /// Absent when there is no further page.
    pub next_cursor: Option<String>,
}

/// A tax-category definition owned by Product Catalog, not a tax rate.
///
/// Pricing assigns `code` to a legacy price row's `tax_category_ref`. Neither a SKU nor
/// a region supplies a second editable assignment or fallback. Labels are read
/// from the provider rather than stored alongside each assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogTaxCategory {
    /// Stable provider-owned code. Development providers visibly prefix demo codes.
    pub code: String,
    /// Human-readable label for the category picker.
    pub display_name: String,
}

/// Why the catalog could not be read — the typed view over [`CanonicalError`].
///
/// Opt-in: the trait contract is canonical, and a consumer that only propagates
/// with `?` never needs this type. Project at the call site with
/// `.map_err(ProductCatalogError::from)`.
#[derive(Debug, Clone)]
pub enum ProductCatalogError {
    /// No registry is wired. The ordinary state of this deployment, and not a
    /// fault: it is reported so a surface can say "not asked" instead of "none".
    Unconfigured,
    /// A registry is configured and did not answer.
    Unreachable(String),
    /// It answered, and the answer could not be used.
    Internal(String),
    /// A category this port does not emit today. Carries the canonical error
    /// whole, so a projection that has fallen behind loses nothing.
    Other(CanonicalError),
}

impl std::fmt::Display for ProductCatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unconfigured => f.write_str("no product catalog is configured"),
            Self::Unreachable(detail) => write!(f, "product catalog unreachable: {detail}"),
            Self::Internal(detail) => write!(f, "product catalog error: {detail}"),
            Self::Other(canonical) => write!(f, "product catalog error: {canonical}"),
        }
    }
}

impl std::error::Error for ProductCatalogError {}

impl From<CanonicalError> for ProductCatalogError {
    fn from(err: CanonicalError) -> Self {
        let detail = err.detail().to_owned();
        match &err {
            CanonicalError::Unimplemented { .. } => Self::Unconfigured,
            CanonicalError::ServiceUnavailable { .. } => Self::Unreachable(detail),
            CanonicalError::Internal { .. } => Self::Internal(detail),
            _ => Self::Other(err),
        }
    }
}

/// The canonical error a client raises when no registry is wired.
///
/// Public because every implementation of the port owes the same answer to the
/// same fact, and a second spelling of it would be a second thing
/// [`ProductCatalogError::Unconfigured`] has to recognise.
#[must_use]
pub fn unconfigured_catalog() -> CanonicalError {
    CatalogResource::unimplemented("no product catalog is configured").create()
}

/// The canonical error a client raises when a configured catalog did not answer.
#[must_use]
pub fn catalog_unreachable(detail: impl Into<String>) -> CanonicalError {
    CanonicalError::service_unavailable()
        .with_detail(detail)
        .create()
}

/// The registry's read contract
/// (`cpt-cf-bss-products-interface-read-model`, the browse half).
///
/// `#[toolkit::provides]` validates this trait through
/// [`product_catalog_client_v1_ir`] and
/// [`product_catalog_client_v1_rest_http_binding`]. The contract-kind suffix
/// rule (`Api` / `Backend` / `Embedded` / `Extension`) refuses
/// `#[toolkit::contract]` on this existing name; the IR below is the same
/// shape the macro would emit (`SafeRead` on every method, tenant
/// `SecurityContext` off the wire). Do not add a second trait to satisfy
/// the suffix.
#[async_trait]
pub trait ProductCatalogClientV1: Send + Sync {
    /// The SKUs a write names. Ids the registry does not know come back absent,
    /// not as an error: `SKU_NOT_PUBLISHED` is a rule's finding, not a transport's.
    ///
    /// # Errors
    /// [`CanonicalError`] when no registry is wired, it cannot be reached, or its
    /// answer is unusable.
    async fn get_skus(
        &self,
        ctx: &SecurityContext,
        ids: &[Uuid],
    ) -> Result<Vec<CatalogSku>, CanonicalError>;

    /// The human's selector: a prefix query over the name, paged.
    ///
    /// # Errors
    /// [`CanonicalError`] when no registry is wired, it cannot be reached, or its
    /// answer is unusable.
    async fn search_skus(
        &self,
        ctx: &SecurityContext,
        q: Option<&str>,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<CatalogSkuPage, CanonicalError>;

    /// Tax-category definitions available to this tenant, in provider order.
    ///
    /// No CRUD or rate data. Implementations must not fall back to fabricated
    /// definitions when the real provider is unavailable. A validator can read
    /// this list once per operation, not once per price.
    ///
    /// # Errors
    /// [`CanonicalError`] when unconfigured, unreachable or unusable. These
    /// cases must not be interpreted as an empty valid dictionary.
    async fn list_tax_categories(
        &self,
        ctx: &SecurityContext,
    ) -> Result<Vec<CatalogTaxCategory>, CanonicalError>;
}

/// The default until the registry gear exists: it says so, every time.
///
/// Not an empty list. The distinction is the whole point of the error type —
/// see the module doc.
#[derive(Debug, Default, Clone, Copy)]
pub struct UnconfiguredProductCatalogClientV1;

#[async_trait]
impl ProductCatalogClientV1 for UnconfiguredProductCatalogClientV1 {
    async fn get_skus(
        &self,
        _ctx: &SecurityContext,
        _ids: &[Uuid],
    ) -> Result<Vec<CatalogSku>, CanonicalError> {
        Err(unconfigured_catalog())
    }

    async fn search_skus(
        &self,
        _ctx: &SecurityContext,
        _q: Option<&str>,
        _limit: u32,
        _cursor: Option<&str>,
    ) -> Result<CatalogSkuPage, CanonicalError> {
        Err(unconfigured_catalog())
    }

    async fn list_tax_categories(
        &self,
        _ctx: &SecurityContext,
    ) -> Result<Vec<CatalogTaxCategory>, CanonicalError> {
        Err(unconfigured_catalog())
    }
}

fn secctx_field() -> FieldIr {
    FieldIr {
        name: "ctx".to_owned(),
        ty: TypeRef::Named("SecurityContext".to_owned()),
        optional: false,
        role: FieldRole::SecurityContext,
    }
}

fn query_binding(field: &str) -> HttpFieldBinding {
    HttpFieldBinding::Query {
        field: field.to_owned(),
        param: field.to_owned(),
    }
}

fn unary(
    name: &str,
    fields: Vec<FieldIr>,
    output: TypeRef,
    field_bindings: Vec<HttpFieldBinding>,
) -> (MethodIr, HttpMethodBindingIr) {
    let method = MethodIr {
        name: name.to_owned(),
        kind: MethodKind::Unary,
        input: InputShape { fields },
        output,
        error: Some(TypeRef::Named("CanonicalError".to_owned())),
        idempotency: Idempotency::SafeRead,
        optional: false,
    };
    let binding = HttpMethodBindingIr {
        method_name: name.to_owned(),
        http_method: HttpMethod::Get,
        path_template: "/browse".to_owned(),
        field_bindings,
        retryable: false,
        streaming: false,
        stream_framing: StreamFraming::ServerSentEvents,
        optional: false,
    };
    (method, binding)
}

/// Contract IR for [`ProductCatalogClientV1`].
///
/// `#[toolkit::provides]` calls this as `{contract}_ir()` and runs
/// [`toolkit_contract::ir::validate_contract`].
#[must_use]
pub fn product_catalog_client_v1_ir() -> ContractIr {
    let (get_skus, _) = get_skus_ir();
    let (search_skus, _) = search_skus_ir();
    let (list_tax_categories, _) = list_tax_categories_ir();
    ContractIr {
        name: "ProductCatalogClientV1".to_owned(),
        gear: "bss-products".to_owned(),
        version: "v1".to_owned(),
        methods: vec![get_skus, search_skus, list_tax_categories],
    }
}

/// HTTP binding IR for [`ProductCatalogClientV1`].
///
/// The REST arm of products' provider calls the existing browse door
/// (`GET /bss-products/v1/browse?kind=sku`); this binding is the IR
/// `validate_http_binding` checks, not a second contract surface.
#[must_use]
pub fn product_catalog_client_v1_rest_http_binding() -> HttpBindingIr {
    let (_, get_skus) = get_skus_ir();
    let (_, search_skus) = search_skus_ir();
    let (_, list_tax_categories) = list_tax_categories_ir();
    HttpBindingIr {
        base_path: "/bss-products/v1".to_owned(),
        methods: vec![get_skus, search_skus, list_tax_categories],
    }
}

fn get_skus_ir() -> (MethodIr, HttpMethodBindingIr) {
    unary(
        "get_skus",
        vec![
            secctx_field(),
            FieldIr {
                name: "ids".to_owned(),
                ty: TypeRef::List(Box::new(TypeRef::Primitive(PrimitiveType::Uuid))),
                optional: false,
                role: FieldRole::Wire,
            },
        ],
        TypeRef::List(Box::new(TypeRef::Named("CatalogSku".to_owned()))),
        vec![query_binding("ids")],
    )
}

fn search_skus_ir() -> (MethodIr, HttpMethodBindingIr) {
    unary(
        "search_skus",
        vec![
            secctx_field(),
            FieldIr {
                name: "q".to_owned(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::String))),
                optional: true,
                role: FieldRole::Wire,
            },
            FieldIr {
                name: "limit".to_owned(),
                ty: TypeRef::Primitive(PrimitiveType::I32),
                optional: false,
                role: FieldRole::Wire,
            },
            FieldIr {
                name: "cursor".to_owned(),
                ty: TypeRef::Optional(Box::new(TypeRef::Primitive(PrimitiveType::String))),
                optional: true,
                role: FieldRole::Wire,
            },
        ],
        TypeRef::Named("CatalogSkuPage".to_owned()),
        vec![
            query_binding("q"),
            query_binding("limit"),
            query_binding("cursor"),
        ],
    )
}

fn list_tax_categories_ir() -> (MethodIr, HttpMethodBindingIr) {
    unary(
        "list_tax_categories",
        vec![secctx_field()],
        TypeRef::List(Box::new(TypeRef::Named("CatalogTaxCategory".to_owned()))),
        Vec::new(),
    )
}

#[cfg(test)]
mod contract_ir_tests {
    use toolkit_contract::ir::{validate_contract, validate_http_binding};

    use super::{product_catalog_client_v1_ir, product_catalog_client_v1_rest_http_binding};

    #[test]
    fn the_contract_ir_and_browse_http_binding_validate() {
        let ir = product_catalog_client_v1_ir();
        validate_contract(&ir).expect("contract IR");
        validate_http_binding(&ir, &product_catalog_client_v1_rest_http_binding())
            .expect("HTTP binding IR");
    }
}
