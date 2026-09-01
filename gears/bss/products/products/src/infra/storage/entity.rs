//! One module per table, named for the table without its `products_` prefix.

pub mod approval;
pub mod approval_decision;
pub mod audit_log;
pub mod breakglass_session;
pub mod bulk_batch;
pub mod bulk_row;
pub mod catalog_version;
pub mod catalog_version_capture;
pub mod catalog_version_counter;
pub mod catalog_version_entry;
pub mod catalog_version_request;
pub mod category;
pub mod entity_version;
pub mod freeze_ack;
pub mod freeze_participant;
pub mod idempotency;
pub mod identity_ref;
pub mod product;
pub mod product_category;
pub mod reference_member;
pub mod reference_producer;
pub mod reference_watermark;
pub mod sku;
