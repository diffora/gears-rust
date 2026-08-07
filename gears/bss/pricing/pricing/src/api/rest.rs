//! REST surface.
//!
//! Every route is mounted under the gear's service prefix,
//! `/bss-pricing/v1/{resource}`, with actions as sub-resource segments (D-140),
//! and every collection surface paginates with an opaque cursor (D-125).

pub mod approvals;
pub mod auth_context;
pub mod bundles;
pub mod correlation;
pub mod cursor;
pub mod cutovers;
pub mod error;
pub mod frontier;
pub mod migrated_origin_snapshots;
pub mod migrations;
pub mod overlays;
pub mod plans;
pub mod preconditions;
pub mod preview;
pub mod prices;
pub mod publish;
pub mod retirement;
pub mod state;
pub mod supersessions;
pub mod tax_display_policy;
pub mod taxonomies;
pub mod threshold_policy;
pub mod windows;
