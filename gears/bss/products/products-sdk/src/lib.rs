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

#![forbid(unsafe_code)]

pub mod api;
pub mod models;

pub use api::ProductsClient;
pub use models::{EntityKind, LifecycleState, Product, Sku};
