//! REST surface.
//!
//! Every route is mounted under the gear's service prefix,
//! `/bss-pricing/v1/{resource}`, with actions as sub-resource segments (D-140),
//! and every collection surface paginates with an opaque cursor (D-125).
//!
//! # The read shape, and the five families that deviate from it
//!
//! **The pattern is the approvals pair**: a record family offers a
//! cursor-paginated `GET` on its collection *and* a `GET` on one member by id, so a
//! caller who holds an id never pages a tenant's whole set to reach one row, and a
//! caller who holds none can find what the server minted. It is recorded here
//! rather than left implicit because the alternative is what review finding Z13-9
//! found: nine families, each missing a *different* half, and no statement anywhere
//! of which half a tenth family owes.
//!
//! Complete pairs: **approvals**, **plans**, **migrations**, **bundles**. The
//! deviations, each with the reason it is one:
//!
//! * **price rows** — `GET /plans/{planId}/prices` lists; there is no
//!   `GET …/prices/{priceId}`, though `PATCH` and `DELETE` address exactly that
//!   path. A row is authored, read back and mutated inside one plan's page, and its
//!   entity tag comes from the page; a by-id read is owed the day a client holds a
//!   `priceId` across sessions.
//! * **price overlays** — `GET /price-overlays` lists;
//!   `/price-overlays/{overlayId}` is `PATCH`-only. This is the sharpest one and
//!   the only deviation with a cost today: the list narrows on `scope_class` and
//!   nothing else, so reading one known overlay means paging the tenant's overlay
//!   set at D-125's default.
//! * **price windows** — `GET /price-windows` lists (narrowable to one
//!   `price_id`); `/price-windows/{windowId}` is `PATCH` + `DELETE`. Same shape as
//!   price rows, mitigated the same way: the `price_id` filter is the practical
//!   by-id read.
//! * **repricing runs** and **bulk imports** — the mirror image: each has a by-id
//!   `GET` and no collection read. Deliberate rather than pending. Both are
//!   addressed by a **client-supplied** key (`run_id`, the import's client key), so
//!   a caller that opened a run already holds the id it needs, and neither is a
//!   catalog object an operator browses.
//!
//! Two statements a later surface should be held to: a family's list read owes
//! `limit`, `cursor` and its filters as **declared** query parameters — bound by
//! `module_test`'s `every_query_reading_route_declares_the_parameters_it_reads`,
//! which was written because a paginated route declaring none of its parameters
//! cannot be paged by a generated client at all (Z13-10) — and a by-id read is not
//! satisfied by a mutating route at the same path, which is how three of the five
//! deviations above came to look like pairs in a route census.

pub mod approvals;
pub mod audit;
pub mod auth_context;
pub mod bulk_imports;
pub mod bundles;
pub mod catalog_skus;
pub mod correlation;
pub mod cursor;
pub mod customer_groups;
pub mod cutovers;
pub mod error;
pub mod frontier;
pub mod history;
pub mod migrated_origin_snapshots;
pub mod migrations;
pub mod overlays;
pub mod plans;
pub mod preconditions;
pub mod preview;
pub mod prices;
pub mod publish;
pub mod repricing_runs;
pub mod retirement;
pub mod rounding_policies;
pub mod rounding_policy;
pub mod state;
pub mod supersessions;
pub mod tax_display_policy;
pub mod taxonomies;
pub mod threshold_policy;
pub mod windows;
