//! BSS Product & SKU Registry SDK.
//!
//! The infrastructure-free contract consumers bind to: the transport-agnostic
//! models a `Product` and a `SKU` are read as, and the client trait a consumer
//! resolves from `ClientHub`.
//!
//! No serde derives live here. The gear's REST DTOs own serde and map onto
//! these types, which is the sibling `bss-ledger-sdk`'s and `bss-pricing-sdk`'s
//! arrangement and keeps a wire concern out of the contract.
//!
//! No crate-level lint allowances: the workspace bar is met as written.
//!
//! # The schema pin rides beside this crate
//!
//! `schema-pin.toml` at this crate's root is the `SchemaPin` (C1, P-D-12): the
//! versioned, committed serialization of the joint fields both gears' CI
//! compares against — every member with its comparability flag (P-D-57), the
//! `status` entry spelled per side with its two-value wire vocabulary
//! (P-D-66), and `CatalogVersion` as a `surface` entry delegated to the
//! counterpart port trait (P-D-65). Registry-side changes to a pinned field
//! bump it through the ordinary review of both gears.
//!
//! @cpt-dod:cpt-cf-bss-products-dod-schema-pin:p1

#![forbid(unsafe_code)]

pub mod api;
pub mod increments;
pub mod models;

pub use api::ProductsClient;
pub use models::{EntityKind, LifecycleState, Product, Sku};
