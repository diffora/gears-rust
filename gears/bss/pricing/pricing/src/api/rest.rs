//! REST surface.
//!
//! Every route is mounted under the gear's service prefix,
//! `/bss-pricing/v1/{resource}`, with actions as sub-resource segments (D-140),
//! and every collection surface paginates with an opaque cursor (D-125).

pub mod auth_context;
pub mod cursor;
pub mod error;
pub mod frontier;
pub mod plans;
pub mod preconditions;
pub mod prices;
pub mod state;
