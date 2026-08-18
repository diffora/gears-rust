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
//! * **customer-group memberships** — `GET /customer-groups/{group}/members`
//!   lists; `PATCH …/members/{id}` and `POST …/members/{payerId}/move` address a
//!   member by id and there is no `GET …/members/{id}`. The price-row deviation's
//!   exact shape, and until 2026-08-18 it had **no mitigation at all**: the price
//!   window's stated mitigation is its `price_id` filter and this list had no
//!   filter and no pagination, against this module's own opening sentence. It has
//!   both now (D-125's pair plus `payer_id`), so the filtered page is the
//!   practical by-id read, exactly as it is for price rows and price windows. The
//!   sentence saying so was already written — inside an `If-Match` parameter doc
//!   at `customer_groups.rs`, where no reader of this statement would find it.
//!
//! Two statements a later surface should be held to: a family's list read owes
//! `limit`, `cursor` and its filters as **declared** query parameters — bound by
//! `module_test`'s `every_query_reading_route_declares_the_parameters_it_reads`,
//! which was written because a paginated route declaring none of its parameters
//! cannot be paged by a generated client at all (Z13-10) — and a by-id read is not
//! satisfied by a mutating route at the same path, which is how three of the six
//! deviations above came to look like pairs in a route census.
//!
//! # Records this surface does not serve, and the shape the list above has no
//! entry for
//!
//! The deviation list is organised entirely around *which read half a family
//! owes*, so four findings from the 2026-08-17 CRUD census had nowhere to be
//! written down. They are here rather than nowhere, because a reader who greps this
//! statement for a table and finds it absent concludes the surface is complete.
//!
//! * **`pricing_catalog_version_ref` is posted by five routes and read by none.**
//!   The publish, the bundle publish, the overlay submit, the retirement and the
//!   three membership writes all hand a caller a `pendingVersionRef`, and
//!   `grep -rn "catalog_version_ref" src/api/` returns zero hits: no route answers
//!   whether a handle committed or which version number it became.
//!   `GET /catalog-version/frontier` is not that read — it serves the tenant-level
//!   pin watermark and says nothing about one publish's outcome. **The intended
//!   reader is named in the design set and is not this gear's**:
//!   `01-foundation.md` §4.4 has an overdue pending ref *"surface on the publish
//!   status API"*, which is not among these 67 routes. Owed as a route, not as a
//!   sentence.
//! * **`GET /migrated-origin-snapshots/{subscriptionRef}` reads a table no route
//!   writes.** `pricing_snapshot_provenance`'s only writers are
//!   `SynthesisService::synthesize`/`synthesize_in`, and a crate-wide grep for
//!   `.synthesize(` finds callers in **one test file** — no route, and none of the
//!   three mounted jobs (`gated_markets`, `readmodel_warm`, `window_activation`).
//!   The route's own description says *"synthesis then runs as a separate audited
//!   step"* without drawing the conclusion: that step is mounted nowhere, so on a
//!   production stand this read can only answer 404. This is the **inverse** of
//!   every entry above — a read whose record nothing produces — and the list had no
//!   shape for it.
//! * **`pricing_operator_flag` is a migrated table with no code at all**: no
//!   repository, no reader, no writer, and `operator_flag::` appears outside its
//!   entity and its migration registration nowhere. Not a REST defect; recorded
//!   here because a CRUD census is the pass that finds it and this is the document
//!   that claims to say which halves are owed.
//! * **Six of `pricing_policy_object`'s eight configurables have no route in either
//!   direction.** Two are served (`tax-display-policy`, `rounding-policy`); the six
//!   that are not are `enforced_migration_notice_days`, `max_tier_bands_per_row`,
//!   `max_price_rows_per_plan`, `max_custom_interval_days`,
//!   `max_custom_interval_months` and `additional_required_descriptors`. Four of the
//!   six are limits `config.rs`'s `LimitsConfig` also carries, so the operator-visible
//!   source of truth for a tenant limit is ambiguous between a config file and a
//!   column nothing can read.
//!
//! # Gate before parse, and the one stated exception
//!
//! **A handler asks its PDP question before it parses anything the caller sent**,
//! so a caller outside the scope learns they are denied rather than that their
//! body, header or path segment is malformed. Thirty-four of the thirty-seven
//! mutating handlers already did; two window mutations did not, and this paragraph
//! exists because the gear stated the rule **twice in two modules and in opposite
//! directions**, each citing itself as the standard — `bulk_imports.rs` called
//! gate-first *"this directory's stated discipline"*, `taxonomies.rs` named the
//! census property that depends on it, and `windows.rs` argued parse-first as *"the
//! shape `schedule_window` reads its idempotency key in"*. Three prose statements
//! where one was owed, and the two outliers answered 400 where both written rules
//! say 403, so the census's recorded question sequence for those routes was a claim
//! about the well-formed subset of requests only.
//!
//! Nothing was exploitable — the direction is fail-earlier — and it is stated here
//! rather than left to a fourth module doc because that is what the last three
//! attempts produced.
//!
//! **The one exception is `GET /plans/{planId}/sellability`**, which parses its
//! three market parameters first and says why at the call site: the route is
//! `plan × read` over a *market*, and a caller who omitted `currency` has asked a
//! question that names no subject for a gate to be about. It is an exception
//! because it is argued, not because it is a read.

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
