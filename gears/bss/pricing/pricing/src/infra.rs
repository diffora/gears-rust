//! Runtime persistence and error plumbing.

pub mod approval_kinds;
pub mod book_stats;
pub mod broker;
pub mod error_mapping;
pub mod events;
pub mod plan_revisions;
pub mod prices;
pub mod storage;
pub mod usage;

pub mod reference_registry;

pub mod reference_work;

pub mod reference_ticker;

pub mod reference_events;

pub(crate) mod pricing_reads;
