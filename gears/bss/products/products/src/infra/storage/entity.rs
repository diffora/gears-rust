//! One module per table, named for the table without its `products_` prefix.

pub mod audit_log;
pub mod idempotency;
pub mod identity_ref;
pub mod product;
pub mod sku;
