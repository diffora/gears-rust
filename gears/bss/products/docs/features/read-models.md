# Feature: Read Models & Browse

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-read-models-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-read-models`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Browse, search and filter](#browse-search-and-filter)
  - [Read the version-history timeline](#read-the-version-history-timeline)
  - [Project the write-to-read pipeline](#project-the-write-to-read-pipeline)
  - [Degrade gracefully](#degrade-gracefully)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Projection shape and partitioning](#projection-shape-and-partitioning)
  - [Error taxonomy](#error-taxonomy)
  - [Non-functional measurement](#non-functional-measurement)
- [4. States (CDSL)](#4-states-cdsl)
- [5. Definitions of Done](#5-definitions-of-done)
  - [The projection table](#the-projection-table)
  - [The projector, whose key already ships](#the-projector-whose-key-already-ships)
  - [The three-source read path, and the reader it is blocked on](#the-three-source-read-path-and-the-reader-it-is-blocked-on)
  - [The browse door](#the-browse-door)
  - [The per-state visibility contract](#the-per-state-visibility-contract)
  - [The staleness stamp, as a floor](#the-staleness-stamp-as-a-floor)
  - [The history timeline](#the-history-timeline)
  - [Degradation, and the limiter that does not exist](#degradation-and-the-limiter-that-does-not-exist)
  - [The three polled dashboards, which are not the projector's](#the-three-polled-dashboards-which-are-not-the-projectors)
  - [The convergence and availability meters](#the-convergence-and-availability-meters)
  - [Facets and filters](#facets-and-filters)
  - [The re-parent subtree recompute](#the-re-parent-subtree-recompute)
  - [The four design-introduced names exist as named seams](#the-four-design-introduced-names-exist-as-named-seams)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Carried verbatim from `design/08` §6](#carried-verbatim-from-design08-6)
  - [Raised here rather than carried](#raised-here-rather-than-carried)
  - [Owed to other documents, recorded and deliberately not edited](#owed-to-other-documents-recorded-and-deliberately-not-edited)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

This feature owns every consumer-facing **read** surface that is not a frozen snapshot: the
cache-first browse, search and filter projections, the **per-state visibility contract**, the
**`asOfCatalogVersion` staleness signal**, graceful degradation under overload, the faceted search
surface, the version-history timeline, and the operator dashboards other features delegated here —
the approval queue staying `05-governance`'s.

It is the feature the show-stopper non-functional requirements are premised on — read p95 under
100 ms at **≥** 2 000 QPS per tenant partition, convergence p99 under 2 seconds, and a read
availability
target decoupled from the write path's.

### 1.2 Purpose

Portals and sales tooling browse the catalog orders of magnitude more often than anyone writes it.
Two things follow, and they are the whole design: **the write path must never carry that load**, and
**the read path must never lie in the dangerous direction.**

Stale is acceptable and **labelled**. Unpublished, cross-scope or mislabelled content is not
acceptable at any load — which is why shedding and queuing are the admitted failure modes and
serving from the write path is not.

### 1.3 Actors

| Actor | Role in this feature |
|-------|------|
| `cpt-cf-bss-products-actor-presentation` | The browse and search consumer; cache warming |
| `cpt-cf-bss-products-actor-marketplace` | Listing reads over published SKUs |
| `cpt-cf-bss-products-actor-auditor` | Version-history timeline reads |
| `cpt-cf-bss-products-actor-catalog-admin` | The deferred-intent, freeze-status and delivery/DLQ dashboards — **no flow is declared for this surface**; its component, cadence and door are §7 row 10's open item |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.8 (`cpt-cf-bss-products-fr-cache-first-browse`), §5.1's advanced-search
  row, §7's non-functional requirements and the convergence interim of §17.1, §12 AC #30, #32 and
  #39, and the glossary's *Read model*.
- [`../DECISIONS.md`](../DECISIONS.md) **P-D-07** (the stamp is a **floor**, not a high-water claim of
  completeness), **P-D-24** and **P-D-35** (the four columns the frozen set excludes), **P-D-27**
  (the event body core, and `publishedVersion` on `*Published` — this feature's projector key),
  **P-D-39** (the empty scope set means unrestricted).
- [`../design/08-read-models.md`](../design/08-read-models.md) — the slice. Its §2 carries the
  **normative steps** of all four flows, §3.1 and §3.3 the normative process detail, and §3.2 the
  normative error roster. This document declares their ids and carries the actor, the scenarios, the
  Input/Output and the boundary.
- Sibling slices: [`../design/01-foundation.md`](../design/01-foundation.md) §4.3 (frozen versions as
  the only consumer-read surface for Product/SKU content),
  [`../design/06-catalog-version.md`](../design/06-catalog-version.md)
  (`CatalogVersionPublished`, the staleness anchor; the capture store),
  [`../design/04-lifecycle.md`](../design/04-lifecycle.md) (the deferred-intent query surface, which
  `04` owns and this feature only projects),
  [`../design/12-consumer-contracts.md`](../design/12-consumer-contracts.md) (the replay and
  bootstrap contract this feature's projector is the first consumer of).

**Requirements**: `cpt-cf-bss-products-fr-cache-first-browse`,
`cpt-cf-bss-products-fr-event-delivery-resilience` (the per-consumer delivery and dead-letter
**projection** clause only; durable acceptance is `01-foundation`'s),
`cpt-cf-bss-products-nfr-read-latency`, `cpt-cf-bss-products-nfr-read-throughput`,
`cpt-cf-bss-products-nfr-graceful-degradation`, `cpt-cf-bss-products-nfr-availability-audit`,
`cpt-cf-bss-products-usecase-catalog-browser-history`,
`cpt-cf-bss-products-interface-read-model`

**Principles**: `cpt-cf-bss-products-principle-publish-through-engine`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Components**: `cpt-cf-bss-products-component-capability-handlers`

**Sequences**: **none** — DECOMPOSITION §2.8 states it: the projector consumes the events
`cpt-cf-bss-products-seq-authoring-publish` emits.

**All eight requirement, use-case and interface ids above are claimed by id, not by position.** The four non-functional requirements
were cited as *"NFR #1, #2, #7, #10"* across this design set until the branch review, and
`design/12`'s lint 1 keys on `nfr-*` ids — so the positional form reported **zero** claims for all
ten. The ids are re-measured against `PRD.md` at `a135631b8`: all eight exist with these exact
spellings, and all eight are DECOMPOSITION §2.8's **Requirements Covered** list, entry for entry.

**The code surface this feature is written against, measured at `a135631b8`.** Every DoD in §5 names
what exists — which on this feature is almost nothing, and saying so precisely is the point.

- **Not one of the four projection tables ships.** `products_read_entity`,
  `products_read_deferred_intent`, `products_read_freeze_status` and `products_read_delivery_state`
  occur **zero** times across `products/src`. The seven shipped migrations are the schema, the two
  entity tables, the audit log, the identity-reference map, the idempotency store and
  `products_entity_version`.
- **No browse route ships**, and there is **no projector** — no `ReadProjector`, no `fn project*`.
- **`READ_MODEL_OVERLOADED` ships nowhere**, and neither does any limiter: no rate-limit component,
  no shed path, no `Retry-After` anywhere in the crate.
- **Two of the three head-read columns have no column on either head table.**
  `deprecation_provenance` and `replaced_by_sku_id` occur **zero** times in the entity structs, and
  the crate says so where it matters: *"§4.3's other two, which have **no column on this table at
  this commit**"* and *"they arrive with `04-lifecycle`"*. Only `lifecycle_state` is readable today.
- **The staleness stamp's anchor event ships nowhere.** `CatalogVersionPublished` — which
  `design/08` §1.8 makes the sole advancer of the stamp — occurs **zero** times in `products/src`;
  the shipped roster is `01-foundation`'s eight.
- **Ten shipped doors register a 503, and that registration covers two paths and names no code.**
  Eight are write doors answering `AUDIT_UNAVAILABLE`; the two `GET` doors write no audit row at all,
  so their only 503 is `api/rest.rs`'s codeless fail-closed authorization arm — *"`Unavailable`
  becomes a fail-closed 503 whose diagnostic stays server-side"*. And the toolkit's `error_503`
  registers a status and a title, not a code.
- **`READ_MODEL_OVERLOADED` is the third of the gear's three 503 *codes*, not a second class.**
  `domain/error.rs` says so of `AUDIT_UNAVAILABLE` in as many words — *"One of the gear's three
  503s"* — and **P-D-25** and `design/12`'s lint 2 both name this feature's code beside it and
  `03-sku-classification`'s `USAGE_TYPE_UNAVAILABLE`.

**The one thing the crate has already done for this feature is guarantee its projector's key.** The
crate names slice 08 exactly once, in a test doc comment in `api/rest/products_tests.rs`:

> §4.5: every one of the eight Foundation events carries the same body core, and
> `ProductPublished`/`SkuPublished` **additionally** carry `publishedVersion` — which slice 06 reads
> as content and slice 08's projector keys on. A body without it is a body those two consumers
> cannot use.
>
> The **value** is asserted, not merely the key's presence. …

So **`inst-rp-consume`'s** operand is armed, by a test that checks the value. That is a DoD not to
re-specify — and the guarantee lives in a **test**, not on a production path, which is worth stating
rather than rounding up.

**`inst-rp-stamp`'s host is armed; its event anchor is not.** The advance rule
(`domain::read_model::advance_stamp` + `repo::apply_read_stamp`) encodes the floor, the
ordering refusal and the null-anchor arm. Its *event* operand
`CatalogVersionPublished` still ships nowhere — the projector that would feed the host is
`dod-projector`. The host being green does not make that consumer build-ready.

**A contradiction the naive reading finds, and measurement withdraws.**
`GET /bss-products/v1/products/{id}` ships and returns head content, while C6 says Product/SKU
entity content projects *"**only** from frozen `products_entity_version` rows — never from head
rows"*.
That is **not** a conflict: `api/rest/products.rs`'s module doc names that door by
`features/foundation.md`'s own heading, *"Authoring head read"* — the crate's own label for it is
`.summary("Read a Product head")` — and calls the head *"the authoring surface in every non-terminal
state"*. It is the authoring plane. C6 governs
the **consumer** plane, which has no shipped door at all. Recorded so the withdrawal is not
re-litigated.

**And one formerly-absent pair that two features depended on.** `domain::canonical` now ships
`decode_rendering` (P-D-77 closed `features/clone.md` §7 row 23), and `repo::latest_entity_version`
reads the newest frozen row. A by-key production reader is still only a test helper. This feature's
remaining frozen-read-path blockers are its own §7 rows 9 and 19, not the closed decoder question.

## 2. Actor Flows (CDSL)

Each flow below is **declared here and stepped in
[`../design/08-read-models.md`](../design/08-read-models.md) §2**, whose steps are the normative
ones. What this section carries is the triggering actor, the scenarios and the boundary.

### Browse, search and filter

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-browse`

**Actor**: `cpt-cf-bss-products-actor-presentation`, and
`cpt-cf-bss-products-actor-marketplace` for listing reads

**Success Scenarios**:

- **The scope is resolved from claims and applied inside the query.** `GET /bss-products/v1/browse…`
  spends `product|sku × read`, plus `category × read` for the category and facet half. Tenant, brand
  and region predicates are built into the query alongside the visibility filter.
- **Per-state visibility is the contract, not a convention.** `published` is browsable;
  `deprecated` is browsable **carrying a machine-readable flag** and excludable by an
  `excludeDeprecated` filter; `retired` appears only through the explicit history surface;
  `draft` and `discarded` are never served.
- **Every response carries the staleness stamp** — `(asOfCatalogVersion, projectedAt)` — including
  empty, error and degraded responses.
- **Facets build from the same projection** and filter under **every** assigned category, primary
  and secondary alike.

**Error Scenarios**:

- **Above the throughput ceiling the door sheds or queues** with `503 READ_MODEL_OVERLOADED` and a
  `Retry-After`, per tenant partition so one tenant's burst cannot starve another.
- **A shed response leaks neither content nor counts.**
- An out-of-claim request is **denied and audited at the gateway**, with the storage layer's
  emptiness beneath it as defence in depth rather than as the control.

**Boundary, and what this flow deliberately does not do.**

**Post-filtering is forbidden.** The visibility and scope predicates are applied at query build
**because a shed row must never have been fetched** — a filter applied after the fetch has already
spent the budget the ceiling exists to protect, and has already read content the caller may not see.

**It never serves from the write path**, never widens scope under load, and never drops the stamp.
Those three are the shape of C4: stale is safe, and the unsafe directions are closed by construction
rather than by care.

**The metadata map is excluded from search by construction** — never projected into any searchable
field, retrievable on the single-entity read alone.

**Frozen-snapshot resolution is not this flow.** `06-catalog-version`'s resolver is a different
surface with different guarantees, and a caller that needs a pinned answer uses it.

### Read the version-history timeline

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-history`

**Actor**: `cpt-cf-bss-products-actor-auditor`, and any entity owner for its own timeline

**Success Scenarios**:

- `GET /bss-products/v1/{products|skus}/{id}/versions` returns the version list, the per-version
  diff computed between frozen rows, the approval references and the actor pseudonyms. It spends
  `audit × read` for cross-entity trails and `product|sku × read` for an entity's own timeline.
- **`retired` entities are reachable here**, and only here. This is the single carve-out from the
  default-browse exclusion.

**Error Scenarios**:

- The same shed behaviour as browse: the limiter sits in front of **every** read endpoint, this one
  included.

**Boundary.** The diff is **computed between frozen rows**, not stored — so it is exactly as
truthful as the frozen content and no more. Actor identities are rendered as **pseudonyms** — the normative step's own words — and this flow
caches none of them. **Whether a timeline render may resolve one through the identity map, and under
which grant, is not this document's to say**: `design/10` §6 registers that *"the two slices disagree
about what 08 does"* and names the owner as *"05's RBAC catalog owner with this slice and 08"*. §7
carries it.

### Project the write-to-read pipeline

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-project`

**Actor**: the `ReadProjector` (system), driven by the broker

**Success Scenarios**:

- **The projector consumes per `(tenant, aggregate)` ordering and projects each event
  idempotently.** The sequence is a **consumer checkpoint per aggregate**, not a row version.
- **Content comes from three places and the split is normative.** Product and SKU entity content
  from **frozen version rows** by the ids the event carries; live-entity content — categories,
  display values, definitions, recognized sets — from its **live tables**; and
  `lifecycle_state`, `deprecation_provenance` and `replaced_by_sku_id` from the **head row**,
  because the frozen set excludes them (**P-D-24**, extended by **P-D-35**). **No other Product or
  SKU content is read from a head.**
- **The stamp is a floor** (**P-D-07**): every catalog version at or below the stamp is fully
  reflected, and later entity events may **add, change or remove** content relative to it.
  `projectedAt` is the fine-grained coordinate. A tenant with zero catalog versions stamps
  `asOfCatalogVersion = null` with a `projectedAt`.
- **The stamp advances only after the event's own changed-entity list is projected from frozen rows
  in the same step**, so it never claims a version whose content it is missing — whatever the
  cross-aggregate arrival order.
- **A category re-parent re-files every descendant's browse path**, the subtree recompute bounded by
  the taxonomy's own depth and children limits.

**Error Scenarios**:

- **A checkpoint that predates the available event tail fails loudly** and rebuilds from the
  bootstrap path, which is `12-consumer-contracts`' replay contract. **The old projection serves
  throughout the rebuild**, and the new one is cut over — never dropped in place.
- **A tenant that has published no catalog version has no anchor to rebuild from**, so its rebuild
  starts from the **empty catalog plus the full retained tail**. That arm is lossless precisely
  because there is no pre-tail content to lose, and it is a distinct code path rather than an edge
  case of the anchored one.
- **A row whose join target has not yet projected is parked** — withheld from browse and
  re-attempted — never rendered as a placeholder.

**Boundary, and the two things this flow is not.**

**It is not the only projector in the feature.** The three operator dashboards are **polled
projections** from their owning surfaces, not broker consumers: `04`'s deferred table and the
broker's delivery and DLQ state emit no events at all, and `06`'s acknowledgements are audit-plane.
The `ReadProjector` is the **event-driven** consumer and the dashboards are outside its subject.

**The projection is rebuildable state, not records.** No append-only guard, no audit rows of its
own — the audited truth lives upstream, and the exemption from the published-and-history append-only
guard is the point rather than an oversight.

### Degrade gracefully

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-degrade`

**Actor**: the read-path limiter (system)

**Success Scenarios**:

- **Above the ceiling: shed or queue** with `503 READ_MODEL_OVERLOADED` and `Retry-After`, per
  tenant partition.
- **Under projector lag past the convergence budget: keep serving.** Stale-but-labelled, with a
  `read_model_lag` alarm naming the tenant and the lag. The stamp is what makes the staleness
  machine-readable to every caller, so a lagging read is honest rather than silent.

**Error Scenarios**:

- There is no third behaviour. **Never the write path, never a widened scope, never a dropped
  stamp** — the three refusals C3 and C4 make structural.

**Boundary.** Shedding is **per tenant partition** because the throughput unit is the tenant
partition; a global limiter would let one tenant's burst shed another's traffic, which is the
starvation this rule exists to prevent.

## 3. Processes / Business Logic (CDSL)

Each process below is **declared here and specified in
[`../design/08-read-models.md`](../design/08-read-models.md) §3**.

### Projection shape and partitioning

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-projection`

**Input**: the events of `01`, `02`, `03`, `04` and `06`; frozen `products_entity_version` rows for
Product and SKU content; head rows for the **three** excluded columns the projection carries —
`lifecycle_state`, `deprecation_provenance`, `replaced_by_sku_id`; `internal_revision` is excluded
too and is projected nowhere; the live tables of
the governed entities; and, for the dashboards, `04`'s deferred table, `06`'s freeze ledger and the
broker's per-consumer delivery state.

**Output**: `products_read_entity`'s denormalized per-tenant rows, the three dashboard tables, and
the staleness stamp every read response carries.

**Boundary**: the row shape is normative in `design/08` §3.1 and is not restated here. What this
document owns is the obligation that every column have a source, that the search surface exclude the
metadata map, and that the join be **convergent** rather than eventually-correct-by-luck.

**The one property that decides whether the rest is buildable.** A browse row joins at least three
aggregates, and the join is convergent because **every join-relevant event recomputes the affected
rows from projection-local state**. A row whose target has not projected is parked and re-attempted.
The alternative — rendering a placeholder and filling it later — would put unpublished or
mislabelled content on a consumer surface, which C4 forbids at any load.

### Error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-read-errors`

**Input**: a read request at any of the four endpoints — browse, history, facets, dashboards.

**Output**: either the response, or a single refusal: `READ_MODEL_OVERLOADED`, **503**, carrying
`Retry-After`.

**Boundary**: **this feature declares exactly one error code**, and everything else on the read
surface is standard not-found or validation through `01-foundation`'s envelope. Reads introduce no
new failure semantics, which is the reason the roster is one row long rather than an oversight.

**The code is raised by one component, not by four doors.** A **single per-tenant-partition
limiter** sits in front of every read endpoint. One door, one code — so a caller cannot receive the
shed refusal from one surface and a silent truncation from another.

**Its 503 is the third of the three codes P-D-25 names, on a different plane.** The other two are
`AUDIT_UNAVAILABLE`, a write refusal whose audit row could not be written, and
`03-sku-classification`'s `USAGE_TYPE_UNAVAILABLE`. All three share the status and nothing else:
retry is the remedy in each, and that is the whole of what they have in common. A fourth, codeless
503 already answers on both shipped `GET` doors — `authz_error_to_canonical`'s fail-closed arm — and
is not a registry code at all.

**The status itself is not settled, and this document does not settle it.** `design/08` §3.2 records
the **503** as this gear's own addition, *"Proposed per row and open to correction"*, pricing's set
carrying no 503 at all. The **code** is fixed; the status is the part still open, and §6's criterion
says which it pins.

### Non-functional measurement

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-read-nfrs`

**Input**: request timings at the API edge, per tenant partition; write-commit timestamps; the
projector's own visibility timestamps.

**Output**: the p95 latency and QPS meters, the convergence meter, and the availability split.

**Boundary**: **the convergence clock starts at write commit**, not at outbox acceptance, and is
decomposed into the two meters C5 names — commit to durable acceptance, which is
`01-foundation`'s, and acceptance to projected, which is this feature's.

**Re-basing the clock is the specific error this document must not make.** Measuring from outbox
acceptance collapses two budgets the non-functional requirements keep distinct, and it lets **a slow
outbox eat the whole convergence budget invisibly**. The slice records that its own earlier fix
landed at the constraint and at this process and **missed the probe** — so the one artefact that
would actually be written could not fail the case the fix exists to catch. §6's convergence
criterion is therefore written from commit, explicitly.

**The availability claim is proven by one probe, not argued.** The read path's health is independent
of the write path's by construction — a separate serving store — and the probe that proves it is a
browse read served during a simulated write-path outage.

## 4. States (CDSL)

**No state machine is declared by this feature.**

`design/08` declares no `state-` id, and the thing this feature owns is explicitly **not** stateful
in the lifecycle sense: `design/08` §4 calls the projection *"rebuildable state, not records"*,
rebuilt from the bootstrap path at any time without loss.

The five lifecycle states this feature reads — `published`, `deprecated`, `retired`, `draft`,
`discarded`, the order §5's table uses — are `01-foundation`'s and `04-lifecycle`'s. This feature **observes** them through the
visibility contract and moves none of them.

The one thing that does advance monotonically is the **staleness stamp**, and **P-D-07** makes it a
**floor rather than a state**: it asserts that everything at or below it is reflected, and asserts
nothing about what lies above. A later event may add, change or remove content relative to the
stamped version, which is why the earlier "strictly additive" reading was struck — a retirement flip
removes content without incrementing anything.

Because §4 declares nothing, **this feature mints no `inst-` id at all** — as on **all ten**
FEATUREs written before it, measured: none of them declares one. The fourteen instruction ids its flows run on are `design/08` §2's and §3's and are
stepped there.

## 5. Definitions of Done

Every DoD below names what exists at `a135631b8`. On this feature that is the shortest list in the
set: **no table, no route, no code and no projector of this feature ships**, so all but two DoDs
create — `dod-frozen-read-path` reads three shipped tables and `dod-read-seams` creates nothing of
its own.
Where something adjacent already exists — the projector's key, the frozen table, the 503 — the DoD
says so rather than restating it as new work.

### The projection table

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-projection-table`

`products_read_entity` exists, per-tenant and denormalized, carrying: identity; state with the
`deprecated`, `compositionPending` and `sellable` flags; `deprecation_provenance` and
`replaced_by_sku_id`; the **brand and region scope columns** the query-build predicates filter on;
the type, tier and unit display fields; the resolved display attributes per locale coordinate,
materialized for the tenant's active locales; the category paths; and `published_version`.

**It ships** as `m20260901_000023` on both dialect arms, with SeaORM entity
`infra::storage::entity::read_entity`, schema oracles in `migrations_tests`, and the P-D-39
scope predicate in `infra::storage::repo::scope_condition` (moved there from `domain::read_model` by P-D-163, so the domain names no ORM type). An earlier draft of this body claimed
*"It does not exist today — zero occurrences across `products/src`"*; that claim was true before
the migration landed and is false at `d6cce574b`. The open §7 rows 2, 11 and 12 still name
adjacent questions (locale config home, metadata field, parked-row exit) and do **not** retract
the table itself.

**The scope columns carry P-D-39's empty-set rule**, and it inverts the obvious predicate: **an
empty set means unrestricted**, so a scope predicate matches a row whose set is empty **or**
contains the claim. A predicate written as containment alone hides every unrestricted row.

**Partitioned/indexed per `(tenant_id, …)`** — the slice's own form, kept because the conjunction is
a strengthening this document must not make: the gear *"ships two dialects and refuses every other
one"*, and physical partitioning is not expressible on the SQLite arm. The obligation is the index,
and the reason is that an index not leading with `tenant_id` cannot serve a per-partition budget.

**Implements**: `cpt-cf-bss-products-algo-projection`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_read_entity`
- Entities: `ReadEntity`, `BrowseProjection`
- Modules: `infra::storage::entity::read_entity`, `domain::read_model`

### The projector, whose key already ships

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-projector`

`ReadProjector` exists as the single event-driven consumer, ordered per `(tenant, aggregate)`,
projecting each event idempotently against a **consumer checkpoint per aggregate**.

**Its stamp-advance step reads `products_catalog_version_entry`** (**P-D-70**): the
`CatalogVersionPublished` changed-entity list selects, the manifest supplies each entity's frozen
version reference, and the head's `published_version` is refused as the source — it may run ahead of
the catalog version and reading it breaches the three-column carve-out. **A Product's retirement
reaches it through the Product analogue of `SkuRetirementEffective`**, whose mint is `04`'s
already-registered item, now load-bearing (P-D-70).

**Its key is already guaranteed and this DoD does not re-specify it.**
`api/rest/products_tests.rs` asserts that `ProductPublished` and `SkuPublished` carry
`publishedVersion` **by value**, and says why in as many words: *"which slice 06 reads as content and
slice 08's projector keys on. A body without it is a body those two consumers cannot use."* The
guarantee is a test rather than a production invariant, which is worth knowing when the projector is
built against it.

**The checkpoint is per aggregate, not per row.** A row version would make a re-delivered event
either skipped or double-applied depending on which aggregate moved last; a per-aggregate checkpoint
makes idempotence a property of the consumer rather than of the row.

**Ticked (P-D-150).** `infra::projector` is the consumer. Its source is `products_read_inbox`:
every consumed family writes its event there **in the transaction that wrote the outbox row**
(`infra::events::record_inbox`), because the gear can read neither the toolkit's outbox rows nor a
broker stream — so `created_at` is the commit instant (P-D-124's origin) and per-tenant order is the
row id. The checkpoint is `products_read_checkpoint(tenant_id) → inbox_id` — "per partition" read as
per tenant, every inbox row being one tenant's — and it carries the serving **generation**: a
checkpoint the swept tail has run past rebuilds from the latest catalog version's manifest into
generation N+1 and swaps, the old rows dropped after (`inst-rp-bootstrap`, P-D-126 row 8); a tenant
with no version rebuilds anchorless. A row that cannot apply — a publish whose frozen row is gone, a
payload that does not decode — is **parked** in `products_read_poison`, retried each pass up to
`read_poison_retry_ceiling`, then skipped with `read_model_poison` raised (rows 9 and 12). The
stamp-advance step reads the version event's changed-entity list against the projected rows, never
the head. Probes: `a_publish_reaches_the_inbox_and_projects_from_the_frozen_row`,
`a_poison_row_is_parked_retried_then_skipped_and_surfaced`,
`a_checkpoint_behind_the_swept_tail_rebuilds_and_swaps`, `the_stamp_is_a_floor_over_projected_entities`.

**Implements**: `cpt-cf-bss-products-flow-project`, `cpt-cf-bss-products-algo-projection`

**Touches**:
- Modules: `infra::events`, `infra::broker`
- Entities: `ReadProjector`

### The three-source read path, and the reader it is blocked on

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-frozen-read-path`

Product and SKU entity content is projected **only** from frozen `products_entity_version` rows.
Live-entity content is read from its live tables. `lifecycle_state`, `deprecation_provenance` and
`replaced_by_sku_id` are read from the **head row**.

**The head carve-out is not a violation of the frozen-only rule; it is what makes it stateable.**
`01-foundation` §4.3 **excludes** those three columns and `internal_revision` from frozen content
(**P-D-24**, extended by **P-D-35**), because they move on transitions that write no version row. A
projector that expected them in the frozen row would find them absent; one that read *other* content
from the head would leak in-flight authoring. The rule is: those three from the head, everything
else frozen, nothing else from a head at all.

**The governed live entities are read live for a stated reason**, not for convenience: categories,
display values, definitions and recognized sets **have no frozen versions and no draft state to
leak**, their mutations being governed-and-applied. A live read there is already-published content.

**Earlier body text claimed this DoD was blocked on a reader and a parser that did not exist**,
citing `features/clone.md` §7 row 23. **That claim is false at `d6cce574b`**: `domain::canonical`
ships `decode_rendering` (P-D-77 closed row 23), and `repo::latest_entity_version` reads the newest
frozen row. A by-`(tenant, entity_kind, entity_id, published_version)` production reader is still
only a test helper (`find_frozen_version` in `repo_tests`), not a public repository function.

**This DoD remains blocked on open register rows, not on inventing a substitute reader here:**
§7 **row 9** (projector posture when a `*Published` event's frozen row has been collected — owner:
this slice with 10 and 12) and §7 **row 19** (published entity rescope without a version row —
owner: P-D-24/P-D-35 with `01-foundation`). Building a local stub around either would hide the gap.

**Ticked (P-D-150).** Both rows are answered (P-D-126: row 9 a parked poison message, row 19 not a
defect) and the by-version reader exists: `repo::entity_version_at`. `projector::project_entity`
renders a row from the frozen version the event names — name, code, scopes, type, unit, tier label,
`composition_pending` and `sellable` from the frozen content — reads `lifecycle_state`,
`deprecation_provenance` and `replaced_by_sku_id` from the head row and nothing else from it, and
the governed live entities live: category paths from the tree, display attributes from the
definitions and values for the active locales, the tier label from the recognized set. A head edited
after its publish is not read (probed: the row keeps the frozen name).

**Implements**: `cpt-cf-bss-products-algo-projection`

**Touches**:
- DB Table: `products_entity_version`, `products_product`, `products_sku`
- Modules: `infra::storage::repo`, `domain::canonical`

### The browse door

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-browse-door`

`GET /bss-products/v1/browse…` exists, spending `product|sku × read` plus `category × read` for the
category and facet half. **No browse route ships today.**

**Scope and visibility predicates are built into the query.** Post-filtering is refused by this DoD,
not merely discouraged: a shed row must never have been fetched, so a design that fetches and then
filters has already spent the budget and already read what the caller may not see.

**The response never omits the stamp** — success, empty, error and degraded alike.

**Ticked (P-D-150).** `GET /bss-products/v1/browse` (`read::browse`) on `product × read` and
`sku × read` (both when `kind` is absent). The tenant predicate is the PEP's scope, the per-state
contract `repo::visibility_condition(VisibilityFilter::for_surface(...))` (default browse, or the filtered surface
under `excludeDeprecated`), brand and region claims `scope_condition`s — all inside the one statement
`repo::browse_read_entities` runs; nothing is fetched and dropped. Filters: name prefix, category
path (any assigned category), SKU type, tier label, sellable, unit; `includeFacets` adds the facets;
`limit` at most 500. Every answer — rows or none — carries the `StampView`, the anchorless tenant
reading `asOfCatalogVersion = null` with a `projectedAt`. Probe:
`browse_serves_the_projection_under_the_visibility_contract_with_the_stamp`.

**Implements**: `cpt-cf-bss-products-flow-browse`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- API: `GET /bss-products/v1/browse…`
- Modules: `api::rest`

### The per-state visibility contract

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-visibility`

Five states, three surfaces, one rule each:

| state | default browse | filtered browse | history |
|---|---|---|---|
| `published` | served | served | served |
| `deprecated` | served **with the flag** | excludable by `excludeDeprecated` | served |
| `retired` | **never** | **never — the by-id read serves it under an explicit state opt-in, P-D-70** | **served — the one carve-out** |
| `draft` | **never** | **never** | **never** |
| `discarded` | **never** | **never** | **never** |

**The contract is applied at query build.** A row a caller may not see is not fetched.

**`retired`'s only *browse-family* surface is the history flow, which is `p2` while this contract
is `p1`** — and the `p1` cut is not therefore empty of it: **P-D-70 arm 4** (§7 row 14, resolved)
makes `retired` retrievable at `p1` through the **by-id read under an explicit state opt-in**, never
the default. The priority disagreement is real and §7 row 17 carries it, but the earlier reading of
this paragraph — that at `p1` alone `retired` is *"reachable nowhere"* — was retired by that
decision and contradicted this DoD's own `retired`/filtered-browse cell.

**Implements**: `cpt-cf-bss-products-flow-browse`

**Touches**:
- Entities: `VisibilityFilter`
- DB Table: `products_read_entity`

### The staleness stamp, as a floor

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-staleness-stamp`

Every read response carries `(asOfCatalogVersion, projectedAt)` — **persisted as one per-tenant
stamp row** (**P-D-70**: a column duplicated per projection row cannot answer an *empty* projection,
the anchorless rebuild's own arm, and deriving from the consumer checkpoint ties response metadata to
broker internals). **`projectedAt` advances on every projector apply, version or none** — a
zero-version tenant's bootstrap is an apply and stamps it — and every polled surface carries the
stamp of its own table's last apply, which is what C3's every-response rule means for
`products_read_delivery_state`.

**The stamp is a floor** (**P-D-07**), and the DoD is that the implementation encode the floor
reading rather than the completeness reading: everything at or below the stamp is reflected, and
**later entity events may add, change or remove** content relative to it. The strictly-additive
premise was false — a retirement flip removes content and increments no version — and a projector
built on it would treat a legitimate removal as corruption.

**The stamp advances only after the event's own changed-entity list is projected from frozen rows in
the same step.** This is the ordering obligation: without it, a `CatalogVersionPublished` arriving
before the entity events of that version would stamp a version whose content is missing, and the
stamp would be a claim rather than a floor.

**A tenant with zero catalog versions stamps `asOfCatalogVersion = null` with a `projectedAt`.** The
null is a stated value, not an absence — a response without the field is indistinguishable from a
dropped stamp.

**It ships** as table `products_read_stamp` (`m20260901_000024`), entity
`infra::storage::entity::read_stamp`, domain host `domain::read_model::advance_stamp`, and
persistence host `infra::storage::repo::apply_read_stamp`. The projector that *drives* those hosts
remains `dod-projector`. An earlier Touches line named `products_read_entity`; the body's
per-tenant-row semantics require `products_read_stamp`, and that is the table that exists.

**Implements**: `cpt-cf-bss-products-flow-project`

**Touches**:
- Entities: `StalenessStamp`
- DB Table: `products_read_stamp`
- Modules: `domain::read_model`, `infra::storage::repo::read_models`

### The history timeline

- [x] `p2` - **ID**: `cpt-cf-bss-products-dod-history-timeline`

`GET /bss-products/v1/{products|skus}/{id}/versions` returns the version list, the per-version diff
**computed between frozen rows**, the approval references and the actor pseudonyms.

**It is no longer the only surface that reaches a `retired` entity** (**P-D-70**): the by-id read
serves `retired` under an explicit state opt-in — never the default — which is what keeps the FR's
`p1` promise while this flow stays `p2`. **And it is a request-time read over frozen rows, settled**:
frozen rows are not write-path for C1's purpose, C1 keeping browse and search off the *head* tables,
and `products_entity_version` being append-only history a read contends with nothing on.

**Whether it is a materialized projection or a request-time read is not settled**, and this DoD does
not settle it: `design/08` §1.5 puts it in scope and `design/08` §4 calls the projection tables
rebuildable state, while `design/08` §3.1 declares no history table. §7 carries it, because the
answer decides whether the convergence budget applies to this surface at all.

**Ticked (P-D-150), as a request-time read.** `GET /bss-products/v1/{products|skus}/{id}/versions`
(`read::product_history` / `read::sku_history`) reads `products_entity_version` through
`repo::entity_versions_of` at request time — no materialised history table, so the convergence
budget does not apply to this surface (the open question above, answered by the build: the frozen
rows are append-only history a read contends with nothing on). Each version carries its
`publishedVersion`, `publishedAt`, `approvalRef`, the actor's pseudonym and the keys whose values
differ from the previous frozen row; a `retired` head is served (the C2 carve-out), a draft or
discarded one is the miss. Behind the limiter, with the stamp. Probe:
`the_timeline_renders_frozen_versions_and_their_diffs`.

**Implements**: `cpt-cf-bss-products-flow-history`

**Touches**:
- API: `GET /bss-products/v1/products/{id}/versions`, `GET /bss-products/v1/skus/{id}/versions`
- DB Table: `products_entity_version`

### Degradation, and the limiter that does not exist

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-degradation`

A **single per-tenant-partition limiter** sits in front of every read endpoint and answers
`503 READ_MODEL_OVERLOADED` with `Retry-After` above the ceiling.

**Nothing of this ships.** No rate-limit component, no shed path, no `Retry-After` anywhere in the
crate, and the code itself occurs nowhere.

**Per tenant partition, because a global limiter starves.** The throughput requirement is stated per
partition, so a shared bucket lets one tenant's burst shed another's traffic — the failure this rule
exists to prevent, and one that a global limiter passes every aggregate load test.

**Under lag rather than overload the answer is different and must not be conflated**: keep serving,
stale-but-labelled, and raise `read_model_lag` naming the tenant and the lag. Shedding a lagging
read would trade a labelled staleness — which C3 makes safe — for an outage.

**A shed response leaks neither content nor counts.**

**The row's `sellable` is the head's own flag, not a derived one (P-D-164).** The projection
computed `published && !composition_pending` until the benidorm wave measured it: the row already
carries `lifecycle_state` and `compositionPending` as members of its own, so deriving said nothing
new and dropped `inst-cl-sellable`'s bucket-iii flag — pricing's operand for predicate 6. A SKU
saved `sellable = false` served `true` on browse and `?sellable=false` could not find it.

**Ticked (P-D-150).** `read::ReadPathLimiter` — the fifth name `design/08` §1.7 minted (P-D-126) —
is one process-wide component, a token bucket **per tenant** at `read_path_qps_ceiling` (interim
200/s), installed at boot and consulted first by all six read doors; above the ceiling a door answers
`503 READ_MODEL_OVERLOADED` (`DomainError::ReadModelOverloaded`, the code on the audit channel) with
`Retry-After` and no body content. One tenant's burst sheds that tenant alone. Under **lag** nothing
sheds: the projector raises `read_model_lag` past `read_convergence_budget_secs` and the doors keep
serving, stale-but-stamped. Probe: `the_limiter_sheds_one_tenant_with_retry_after_and_spares_another`.
The bucket map is bounded (P-D-163): past 4 096 entries an acquire drops every bucket idle for a
second — lossless, an idle bucket being a full one — so the map holds the tenants active in the
last second, not every tenant ever seen. Probe: `idle_limiter_buckets_are_evicted_past_the_high_water_mark`.

**Implements**: `cpt-cf-bss-products-flow-degrade`, `cpt-cf-bss-products-algo-read-errors`

**Touches**:
- Modules: `api::rest`

*(No entity is named: `design/08` §1.7 introduces four names and none is a limiter, and
`VisibilityFilter` is the query-build filter this DoD must not be wired into. §7 carries whether a
fifth name is minted, which would be `design/08`'s edit rather than this one's.)*

### The three polled dashboards, which are not the projector's

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-dashboards`

`products_read_deferred_intent`, `products_read_freeze_status` and `products_read_delivery_state`
exist. **None ships.**

**They are polled projections, not broker consumers**, and the reason is a measurement rather than a
preference: their sources **emit no events**. `04`'s deferred table and the broker's per-consumer
delivery and DLQ state emit none at all, and `06`'s acknowledgements and re-triggers are
audit-plane. An earlier reading had them *"refreshed by the same projector"*, which claimed sources
that do not exist.

**So the `ReadProjector`'s subject excludes them**, and a build that wires them into the broker
consumer will find nothing to consume.

**What this DoD cannot specify**: no document names the polling component, the interval, the route
or a staleness bound for these three. §7 carries it; the DoD is met by the tables and their stated
sources.

**Ticked (P-D-150).** The three tables exist (`m20260901_000029`) and are **polled projections**:
`projector::poll_dashboards` runs on the runtime loop every `read_dashboard_poll_secs` (interim 30;
P-D-126 row 10) over 04's deferred-retirement table, 06's ledger (per version, the participant counts
by state) and the projector's own inbox and poison park, each row stamped `polled_at`. Their doors —
`GET /bss-products/v1/read/deferred-intents` (`scheduled_transition × read`), `/read/freeze-status`
(`catalog_version × read`), `/read/delivery-state` (`audit × read`) — sit behind the limiter and carry
the stamp; the broker consumer is not involved (probed with no projector pass at all). Probe:
`the_three_dashboards_answer_from_their_polled_tables`.

**Implements**: `cpt-cf-bss-products-algo-projection`

**Touches**:
- DB Table: `products_read_deferred_intent`, `products_read_freeze_status`,
  `products_read_delivery_state`
- Entities: `ReadDeferredIntent`, `ReadFreezeStatus`, `ReadDeliveryState`

### The convergence and availability meters

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-nfr-meters`

Four meters: p95 latency and QPS **per tenant partition** at the API edge; convergence
**from write commit**, decomposed into `01-foundation`'s commit-to-durable-acceptance meter and this
feature's acceptance-to-projected meter; and the availability split.

**The clock's origin is the DoD's substance.** Measuring convergence from outbox acceptance
collapses budgets the requirements keep distinct and hides a slow outbox entirely. The slice records
that its own fix for this landed at the constraint and at the process and **missed the probe** —
which is why §6's criterion states the origin explicitly rather than inheriting it.

**One of the two meters is not this feature's and is declared by nobody.**
`01-foundation` declares no observability surface and records its own probe as owed, so the
commit-to-acceptance half has no owner. §7 carries it; this feature builds its own half and cannot
close the composed budget alone.

**Ticked (P-D-150), as tracing events.** `read_edge_latency` per served read with the door and the
tenant (p95 and QPS per tenant partition are the metrics backend's aggregation over it);
`read_model_convergence` per projected event, **measured from write commit** — the inbox row's
`created_at`, written in the mutating transaction, is P-D-124's origin — decomposed from `01`'s
commit-to-durable-acceptance half exactly as C5 names; `read_model_lag` past the budget while
serving continues. The availability split is by construction (the serving store is the projection),
and its probe stays owed to a write-path-outage fixture.

**Implements**: `cpt-cf-bss-products-algo-read-nfrs`

**Touches**:
- Modules: `api::rest`, `infra::events`

*(The commit-instant operand this meter needs exists on no event the projector receives — §7 row 23.)*

### Facets and filters

- [x] `p2` - **ID**: `cpt-cf-bss-products-dod-facets`

Facets over the category tree, type, tier label, `sellable` and unit build from the **same**
projection — no second store — and filter under **every** assigned category, primary and secondary
alike.

**The every-category rule is `design/08` §2's `inst-rb-facets`, over the assignment model
`02-taxonomy-attributes` states** — at most one primary plus zero or more secondary categories.
`design/02` itself puts *"read-model/search projections and faceting (08)"* **Out** of scope and
states no facet rule at all, so the rule is this feature's and the model is 02's.

A facet that filtered on the primary assignment alone would hide a Product from a category it is
genuinely assigned to, which is a wrong answer rather than a partial one.

**Ticked (P-D-150).** `includeFacets=true` on the browse door renders the facets from the **same**
serving rows the query admitted — category paths (every path in the row's `category_paths`, primary
and secondary alike), SKU type, tier label, `sellable`, unit — as value/count buckets; the `category`
filter matches any assigned category's path. No second store. Probe: the browse probe's facet
assertion.

**Implements**: `cpt-cf-bss-products-flow-browse`

**Touches**:
- DB Table: `products_read_entity`

### The re-parent subtree recompute

- [x] `p2` - **ID**: `cpt-cf-bss-products-dod-reparent`

A category re-parent re-files **every** descendant's browse path, the projector recomputing the
affected subtree from the event.

**Termination rests on `02`'s depth and children limits, and those limits have no value anywhere.**
`design/02` §6 records it — *"The taxonomy and metadata limits have no interim default anywhere"* —
and names *"08's bounded subtree recompute"* as one of four rules that read them. **Its owner is the
§17.1 policy owner**, not this feature. So the recompute is bounded in principle and unbounded in
practice until those rows exist, and §7 carries it.

**Ticked (P-D-150).** A `CategoryReparented` (and a rename, retirement, deletion or display
update) re-files every Product row's paths from the live tree (`projector::refresh_category_paths`):
each row's assignments are read and its paths re-rendered, rows whose paths did not move untouched.
Recomputing the tenant's rows rather than diffing the subtree is bounded by 02's depth and children
caps, which `ProductsConfig` carries since P-D-107, so the recompute terminates by configuration.

**Implements**: `cpt-cf-bss-products-flow-project`

**Touches**:
- DB Table: `products_read_entity`

### The four design-introduced names exist as named seams

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-read-seams`

`design/08` §1.7 introduces four names and each is addressable:

- **`ReadProjector`** — the single event-driven consumer. **Its consumer-visible order is the
  broker's**, not the outbox's: `infra/events.rs` records that the `(tenant, aggregate)` hash is *"a
  *pipeline* invariant that P-D-47 supersedes for the guarantee a consumer actually reads"*, and that
  the broker's ordering — a read-side `sequence` per `(topic, partition)`, one partition per tenant
  in publish order — is **stronger** than the key the envelope promises. The polled dashboard family
  is **not** its subject.
- **`BrowseProjection`** — the denormalized serving rows: content, resolved display attributes,
  category paths, state and flags.
- **`StalenessStamp`** — the per-tenant floor every response carries. (`design/08` §1.7 words it
  *"high-water mark"*; **P-D-07** makes it a floor, which is the reading §4 states.)
- **`VisibilityFilter`** — the per-state contract applied **at query build**, not as
  post-filtering.

**DECOMPOSITION §2.8's entity list is a different set** — `ReadEntity`, `ReadDeliveryState`,
`ReadDeferredIntent`, `ReadFreezeStatus`, the four **tables**. Both sets belong in the entry, on the rule
**§2.7** follows — its list carries four table-derived names beside all four of `design/07` §1.7's —
and this pass swept the four §1.7 names in beside the four table names. **§2.10 is not a second
precedent**: its list is exactly `design/10` §1.7's four names with no table-derived set.

**And the rule itself is registered as unsettled.** `features/clone.md` §7 **row 24** asks whether
DECOMPOSITION's `Domain Model Entities` field is a listing convention or an ontological claim, and
assigns it to the design-set owner. This document cites that row and raises no second one; if the
answer is *ontological*, this sweep and this DoD's rationale both reverse.

**Implements**: `cpt-cf-bss-products-algo-projection`, `cpt-cf-bss-products-flow-browse`

**Touches**:
- Entities: `ReadProjector`, `BrowseProjection`, `StalenessStamp`, `VisibilityFilter`

## 6. Acceptance Criteria

*Ticks measured clause by clause at **P-D-158** (2026-09-05); the criterion-to-probe map is in that
entry. A box left open names a clause no probe asserts yet.*

**The visibility matrix — fifteen cases, each with its positive control**

- [x] All five states across default browse, filtered browse and history: `draft` and `discarded`
      absent everywhere; `retired` present in history and absent from both browse surfaces;
      `deprecated` flagged in default browse, excluded under `excludeDeprecated` **and served in
      history**; `published` present on all three. **Fifteen cases** — the enumeration above names
      all fifteen, and each absence is asserted beside a presence that proves the fixture could
      reach the serving path. **The `retired`-in-history arm's own route is `p2`** — row 14 is
      resolved and freed this contract, so what remains is the timeline flow's priority (row 17),
      not a blocker on the matrix. The matrix is asserted in
      `domain::read_model::read_model_tests` over all four surfaces including P-D-70 arm 4's by-id
      read, which is where the `p1` retrievability now lands.

**Scope, at both layers**

- [ ] An out-of-claim request is **denied and audited at the gateway**, and the storage layer beneath
      it returns empty. **Both layers are asserted**, so a door that silently absorbs an auditable
      cross-scope attempt fails.
- [x] A row whose scope set is **empty** is served to every claim — P-D-39's rule, and the case a
      containment-only predicate fails.

**The stamp**

- [x] Every response shape — success, empty, error, degraded — carries both
      `asOfCatalogVersion` and `projectedAt`.
- [x] A zero-version tenant's response carries `asOfCatalogVersion = null` **and** a `projectedAt`.
      A response omitting the field fails: absence is indistinguishable from a dropped stamp.
- [x] **A retirement flip removes a row's content without advancing any catalog version, and the
      stamp does not regress.** The floor's negative case — a projector built on the
      strictly-additive premise passes every additive test and fails this one.
- [x] `CatalogVersionPublished` arriving **before** that version's entity events does not advance the
      stamp until the changed-entity list is projected.

**Convergence, measured from commit**

- [ ] Publish to projection-visible within budget, **measured from write commit** and reported as
      the two meters separately. A probe measuring from outbox acceptance fails this criterion —
      that re-basing is the defect the slice's own fix missed at the probe.
- [ ] The lag alarm fires past budget **while serving continues**, stale-but-stamped.

**Degradation**

- [x] Under simulated overload the door answers `503 READ_MODEL_OVERLOADED` with `Retry-After`, and
      the response leaks neither content nor counts.
- [x] Shedding one tenant partition does not shed another's traffic — the criterion a global
      limiter passes on aggregate load and fails here.
- [ ] The limiter answers on **all four** read endpoints: browse, history, facets, dashboards — the
      facets and dashboards arms **blocked on §7 rows 10 and 22**, neither surface having a declared
      route.

**Availability and rebuild**

- [ ] A browse read is served during a simulated write-path outage.
- [x] A checkpoint behind the tail fails **loudly**, rebuilds from the bootstrap path, and cuts over
      — with the old projection serving throughout.
- [x] The anchorless arm: a tenant with zero published versions rebuilds from the empty catalog plus
      the whole retained tail.

**The projection's own rules**

- [ ] The metadata map is absent from **every** searchable field. **The single-entity-read half is
      blocked on §7 row 11**: no flow in this document declares that door and the row shape carries
      no metadata field.
- [x] The three dashboard tables refresh **with the broker consumer stopped** — the probe that
      proves they are polled rather than consumed.
- [ ] A category re-parent re-files a subtree completely, leaving no orphan path.
- [x] A parked row is withheld from browse and never rendered as a placeholder.
- [ ] A facet filters a Product under a **secondary** category assignment, not the primary alone —
      the case a primary-only build passes every aggregate test and fails here.
- [x] A **re-delivered** event applies once against the per-aggregate checkpoint: neither skipped nor
      double-applied. **Blocked on §7 row 24** for what that checkpoint is at the broker.
- [x] Product and SKU content in a projected row matches the **frozen** version, and
      `lifecycle_state`, `deprecation_provenance` and `replaced_by_sku_id` match the **head** — both
      halves in one probe, since either alone passes a build that got the other wrong.

## 7. Known unknowns

**The arithmetic of this section.** Twenty-five rows: **sixteen carried verbatim** from
[`../design/08-read-models.md`](../design/08-read-models.md) §6 — the slice's full count, not a
selection — and **nine raised by the three-lens review of this document**. Of the twenty-five,
**nine block no DoD** (rows 1, 3 and 17, plus rows 6, 7, 13, 14, 20 and 21, which **P-D-70 resolved
on 2026-09-01** — kept in place rather than struck); the other sixteen each name the DoD they block.
Rows 10 and 25 are **parked for the owner** by the same decision — the dashboards' door and the
identity-map render — so `dod-dashboards` and `dod-history-timeline` wait on them. A final subsection carries defects owed to other documents; those are not rows.

**Sixteen, not fifteen, and the correction is worth stating.** The first transcription split the
slice's bullets on a bold-led marker and so fused two items — the locale-materialization note and the
search-engine choice — into one row, the plain bullet having no bold lead. The fidelity check used
the same pattern on both sides and reported fifteen against fifteen. The count above is now built
from an independent primitive: the number of `- ` line starts in `design/08` §6, which is sixteen,
and it agrees with both the split and the carried rows.

**Carried, not answered**, and registered against **its owner's** register. **Four departures from
verbatim, declared so the claim is checkable.** First, the slice's inline `Owner:` sentence and any
provenance marker become this document's `**Owner**:` field — and **row 4's owner is stated in the
row's own prose rather than with that token**, so it is set from those words and the departure is
noted in the row itself. Second, every bare `§N` inside a
carried row is **`design/08`'s numbering, not this document's**; every `§15` in a carried row is
already written `PRD` §15 and needs no exception. Third, each row gains a `**Blocks**:` field, which `design/08` §6 does not have. Apart
from those three, nothing is altered.

**One question is deliberately NOT raised here because another document already owns it**, and is
cited instead: **the frozen-row reader and the canonical-rendering decoder** were registered at
`features/clone.md` §7 row 23. That row is **closed by P-D-77** (`decode_rendering` ships beside
the renderer). `cpt-cf-bss-products-dod-frozen-read-path` remains open on this feature's own §7
rows 9 and 19 — collected-row posture and published rescope without a version — not on the
closed decoder question. The reciprocal note below is retained as historical record of the
citation discipline, not as a live blocker.

### Carried verbatim from `design/08` §6

1. **Open above this slice: does browse need a separate serving store at all?** Raised in review
    and now a PRD §15 question for the NFR workshop. `fr-cache-first-browse`'s rationale rested on
    two uncalibrated numbers — NFR #1's 10K SKUs/tenant is a scale a direct multi-way query
    plausibly serves, and NFR #2's ≥ 2,000 read QPS/tenant partition is not a portal number. The
    FR's rationale has been re-derived onto the two properties that survive recalibration and that
    this slice actually supplies: the **availability split** (C1 + §3.3's write-path-outage probe)
    and **structural stale-but-safe** (C6 — projecting only from frozen rows, never heads). If the
    workshop retires the projection, this slice collapses to a query layer and P-D-07 is deleted
    with it; `products_read_delivery_state` survives regardless (it polls the broker's
    delivery/DLQ state, which is not in this gear's database).
    **Blocks**: no DoD — a `PRD` §15 question above this feature, on whether the serving store is
    needed at all.
    **Owner**: *(P-D-132, 2026-09-03: the Program Lead names the workshop's DRI and date; until it is held the interim numbers are binding design targets.)* the NFR workshop (`PRD` §15) — assigned by P-D-126, 2026-09-03.

2. ~~**Locale materialization**~~ **Answered (P-D-126, 2026-09-03): a `ProductsConfig` field on the P-D-107 idiom** when `08` builds, an empty set refused at boot. *The item's text stood as:* (per active locale) trades storage for the p95 budget; the
    active-locale set per tenant needs a config home — implementation note.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-projection-table`.)*
    **Owner**: not stated in the slice; carried unassigned.

3. Search-engine choice (LIKE/FTS vs external) is deliberately behind the projection contract; the
    NFR #2 load test decides, not this document.
    **Blocks**: no DoD — the row's own text says the NFR #2 load test decides, not a document.
    **Owner**: *(P-D-132, 2026-09-03: the workshop's, as row 1.)* the NFR workshop (`PRD` §15) — assigned by P-D-126, 2026-09-03.

4. ~~**Open above this slice: who measures the < 3 s propagation budget, and against which meter?**~~ **Answered (P-D-124, 2026-09-03): `08` asserts the one meter against the < 3 s threshold; no second probe is owed** — the Program Lead's routing is answered by the arm that reads the budget sentence jointly. *The item's text stood as:*
    `PRD` §15 names this slice with 01 and 06 — §5's convergence probe instruments the
    commit→durable-acceptance segment and asserts it against this slice's own budget; whether one
    meter may be asserted against two thresholds, or a second probe is owed, is the Program
    Lead's. Both siblings register it and this slice did not.
    **Blocks**: no DoD — **resolved by P-D-124** *(was: `cpt-cf-bss-products-dod-nfr-meters`.)*
    **Owner**: the Program Lead — stated in the row's own words (*"is the Program Lead's"*) rather
    than with an `Owner:` token, so the mechanical carry did not pick it up. *(Two lenses raised
    it independently.)*

5. ~~**The `commit → durable-acceptance` meter C5 attributes to slice 01 is declared by no slice.**~~ **Answered (P-D-124, 2026-09-03): declared by `01`**, instrumented by `08`'s convergence probe, composed by `06`; the build is the lead's. *The item's text stood as:*
    01 declares no observability surface and records its NFR #3 probe as owed; 06 §6 registers the
    same gap and names this slice. Without it the p99 < 2 s show-stopper budget has no measurement
    point for its first segment.
    **Blocks**: no DoD — **resolved by P-D-124** *(was: `cpt-cf-bss-products-dod-nfr-meters`.)*
    **Owner**: the Program Lead with 01 and 06. *(Raised by the slice-08 first lens pass.)*

6. ~~**Is the history timeline a materialized projection or a request-time read?**~~
    **Answered in the slice (owner call, 2026-09-01 — P-D-70 arm 1): a request-time read over frozen
   rows, and frozen rows are not write-path for C1's purpose** — C1 keeps browse and search off the
   *head* tables, and append-only history contends with nothing. §3.1 declaring no history table was
   the choice already made.
    Original text: §1.5 puts it In
    scope and §4 calls the projection tables rebuildable state, while §3.1 declares no history
    table and `inst-rh-timeline` describes a computation over 01's frozen rows. If it runs at
    request time it meets C1's "browse/search never touches write-path tables at request time"
    head-on, and nothing says whether frozen rows count as write-path for that purpose.
    **Blocks**: no DoD — **resolved by P-D-70**.
    **Owner**: was this slice with 01. *(Two lenses raised it independently.)*; **closed**.

7. ~~**What tells the projector a Product has been retired?**~~
    **Answered in the slice (owner call, 2026-09-01 — P-D-70 arm 2): the Product analogue of
   `SkuRetirementEffective`**, whose mint is `04`'s already-registered §6 item, now load-bearing —
   nothing else can carry a flip that may trail `effectiveAt`. Until `04` mints it, a retired Product
   stays browsable, pinned on the owning item.
    Original text: C2 requires `retired` out of default
    browse and `inst-rp-stamp` rests its floor semantics on the flip, while 04's Events list names
    a SKU-only `SkuRetirementEffective` and 04 §6 registers that it has no Product analogue and no
    explicit "no event". As it stands a retired Product stays browsable forever.
    **Blocks**: no DoD — **resolved by P-D-70**; `cpt-cf-bss-products-dod-projector` and `dod-visibility` carry the mechanism.
    **Owner**: was the lifecycle owner with the events consumer set — this slice is the surface that
    fails. *(Raised by the slice-08 first lens pass.)*; **closed**.

8. ~~**What happens to live events during a bootstrap rebuild, and what checkpoint does cutover
    install?**~~ **Answered (P-D-126, 2026-09-03): shadow-then-swap** — rebuild into a shadow from the replay start, tail live events into it, swap atomically, checkpoint at the last consumed `(topic, partition)`; the `StalenessStamp` is rebuilt with the rows. *The item's text stood as:* `inst-rp-bootstrap` says the rebuild serves the old projection until cutover and
    12's replay contract defines only the starting point; neither states the concurrency model
    (shadow-then-swap vs live-tail-follow), the cutover checkpoint, or whether the
    `StalenessStamp` is rebuilt with the rows.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-projector`.)*
    **Owner**: this slice with 12. *(Raised by the slice-08 first lens pass.)*

9. ~~**What does the projector do when a `*Published` event's frozen row has been collected?**~~ **Answered (P-D-126, 2026-09-03): a poison message** — parked with a configured bound and alarmed through `products_read_delivery_state`; the retention invariant that prevents it is `12`'s number, routed there. *The item's text stood as:*
    Version rows are retained only while a manifest references them, and under the anchorless
    rebuild arm no manifest exists — so every frozen row is collectable while the events naming
    them are still in the retained tail.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-frozen-read-path`.)*
    **Owner**: this slice with 10 and 12 — skip, fail the rebuild, or bound event-log retention by
    version-row retention. *(Raised by the slice-08 first lens pass.)*

10. ~~**Who runs the polled dashboards, at what cadence, behind which door?**~~ **Answered (P-D-126, 2026-09-03): ticks in `gear.rs`'s lifecycle loop at a configured cadence** (P-D-113's precedent); the dashboards are `08`'s read endpoints behind the limiter, each under its source table's `× read` grant. *The item's text stood as:* `inst-ps-dashboards`
    names three tables and their sources and no component, no interval, no route and no staleness
    bound, while §3.2 fronts them with the limiter and 04 states it owns the deferred-intent query
    surface. 05 already records `scheduled_transition × write|cancel|read` as pairs no slice names
    on a door.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-dashboards`.)*
    **Owner**: this slice with 04, 06 and 05. *(Raised by the slice-08 first lens pass.)*

11. ~~**Where does the single-entity metadata read come from?**~~ **Answered (P-D-126, 2026-09-03): a live join on `products_metadata`** — P-D-06 places the map beside the entity; nothing is projected and `MetadataUpdated` need not be consumed. *The item's text stood as:* `inst-ps-metadata` makes the map
    retrievable "on the single-entity read only" and 02 books that read against this slice; §2
    declares no such flow, the row shape carries no metadata field, and `MetadataUpdated` is
    absent from §1.8's Consumed list.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-projection-table`.)*
    **Owner**: this slice with 02. *(Raised by the slice-08 first lens pass.)*

12. ~~**A parked browse row has no bound and no exit.**~~ **Answered (P-D-126, 2026-09-03): parked with a bound and alarmed, never silent** — the same posture as row 9, on the P-D-107 idiom for the ceiling. *The item's text stood as:* A row whose join target has not projected is
    withheld and re-attempted, "bounded by the convergence monitoring" — but the only lag rule is
    keyed to the projector, so a caught-up projector holding a row whose target was dead-lettered
    trips no alarm and withholds it indefinitely. Nothing defines the projector's poison-message
    posture.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-projection-table`.)*
    **Owner**: this slice with the events consumer owner. *(Raised by the slice-08 first lens
    pass.)*

13. ~~**When does `projectedAt` advance, and do polled surfaces carry the stamp?**~~
    **Answered in the slice (owner call, 2026-09-01 — P-D-70 arm 3): `projectedAt` advances on every
    projector apply, version or none** — a zero-version tenant's bootstrap is an apply and stamps
    it — and every polled surface carries the stamp of its own table's last apply.
    Original text: The advance rule
    covers the version half only, and for a zero-version tenant `projectedAt` is the sole
    freshness signal with no rule writing it. Separately §3.2 makes the dashboards read endpoints,
    so C3's every-response rule reaches `products_read_delivery_state`, whose content bears no
    relation to a catalog version.
    **Blocks**: no DoD — **resolved by P-D-70**; `cpt-cf-bss-products-dod-staleness-stamp` and `dod-dashboards` carry the rule.
    **Owner**: was this slice with P-D-07's owner. *(Raised by the slice-08 first lens pass.)*; **closed**.

14. ~~**Is `retired` retrievable in the p1 cut?**~~
    **Answered in the slice (owner call, 2026-09-01 — P-D-70 arm 4): yes — through the by-id read
    under an explicit state opt-in**, never the default; browse stays exclusionary and the timeline
    stays `p2`, the FR's `p1` met by the smallest surface that can carry it.
    Original text: C2 and `inst-rb-query` state it at `p1` through
    "the explicit history surface", and that surface is the `p2` timeline flow.
    **Blocks**: no DoD — **resolved by P-D-70**; `cpt-cf-bss-products-dod-visibility` is freed and `dod-history-timeline` carries the widened access note.
    **Owner**: was this slice with the PRD owner, the FR's priority being the PRD's. *(Raised by the
    slice-08 first lens pass.)*; **closed**.

15. ~~**Under which aggregate key are 02's two display events ordered?**~~ **Answered (P-D-116 row 15 and P-D-122, 2026-09-03): their own entity's id** — a display write serializes on `products_category.mutation_seq`, `CategoryDisplayUpdated` carries the token it spent (`mutationSeq`) so the projector can order on it, and `AttributeDefinitionUpdated` orders on the definition's id. *The item's text stood as:* The projector's idempotence
    rests on a per-`(tenant, aggregate)` checkpoint, and 02 §6 registers that
    `CategoryDisplayUpdated` and `AttributeDefinitionUpdated` fall under neither of its two stated
    ordering keys. Without one a rename and a display edit on the same category can land in either
    order.
    **Blocks**: no DoD — **resolved by P-D-116 / P-D-122**.
    **Owner**: was 02 with 12 — this slice is the only consumer of both. *(Raised by the slice-08
    first lens pass.)*

16. ~~**Does the composition-clear re-publish reach this projector?**~~ **Answered (P-D-125, 2026-09-03): yes** — the clear runs through `01`'s publish door and emits `SkuPublished` beside `SkuCompositionCleared`. *The item's text stood as:* The browse row carries
    `compositionPending` and the projector keys on `*Published`; 06 §6 registers that neither
    slice says whether the clear emits `SkuPublished` beside `SkuCompositionCleared`. If only the
    latter fires, every composed bundle stays flagged in browse.
    **Blocks**: no DoD — **resolved by P-D-125** *(was: `cpt-cf-bss-products-dod-projector`.)*
    **Owner**: as 06 states it, with this slice. *(Raised by the slice-08 first lens pass.)*

### Raised here rather than carried

17. ~~**Which register sets a feature's priority when the PRD and DECOMPOSITION disagree?**~~ **Answered (P-D-126, 2026-09-03): `DECOMPOSITION` prices features and a feature's items carry the feature's priority; the PRD prices requirements** — under P-D-125's importance reading nothing is misassigned. *The item's text stood as:* `PRD.md`
    prices `nfr-graceful-degradation`, `fr-event-delivery-resilience` and
    `usecase-catalog-browser-history` at **`p2`**; DECOMPOSITION §2.8 lists all eight of its
    Requirements Covered at **`p1`**. This document then splits — the degradation flow, its DoD, the
    error algo and the dashboards are `p1`, the history flow and its DoD `p2` — and no document
    states a precedence.
    **Blocks**: no DoD — every DoD builds the same thing under either answer.
    **Owner**: the PRD owner with the DECOMPOSITION owner.

18. ~~**Does the read-path limiter get a name?**~~ **Answered (P-D-126, 2026-09-03): `ReadPathLimiter`**, added to `design/08` §1.7. *The item's text stood as:* `design/08` §1.7 introduces four design-introduced
    names and none is a limiter; §3.2 calls it only *"the **single per-tenant-partition limiter
    component in front of every read endpoint**"*. So
    `cpt-cf-bss-products-dod-degradation` names no entity, and a fifth §1.7 name would be
    `design/08`'s edit rather than this document's.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-degradation`.)*
    **Owner**: `design/08`'s owner.

19. ~~**Can a published entity be rescoped without the projection noticing?**~~ **Answered (P-D-126, 2026-09-03): not a defect** — C6 projects the published version; a head edit is unpublished until published and narrowing takes effect at publish, which is the stale-but-safe property. *The item's text stood as:* `brand_scope` and
    `region_scope` are **inside** the frozen roster, and the shipped head door admits them *"on any
    non-terminal head, published or not"* while writing **no version row**. The head carve-out is
    exactly three columns and neither scope column is among them, so a narrowing rescope leaves the
    browse row matching the **old, wider** scope until the next publish. That is staleness in the
    direction §1.2 closes and C4 calls structural.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-frozen-read-path`,)*
    `cpt-cf-bss-products-dod-browse-door`.
    **Owner**: **P-D-24**'s and **P-D-35**'s owner with `01-foundation` — it is that enumeration
    that is at stake.

20. ~~**Which frozen row does the stamp-advance step read?**~~
    **Answered (owner call, 2026-09-01 — P-D-70 arm 5): the stamp-advance reads
    `products_catalog_version_entry`** — the event's list selects, the manifest supplies the frozen
    version references (the table P-D-60 made exactly this shape), and the head's
    `published_version` is refused as the source.
    Original text: `CatalogVersionPublished` carries a
    changed-entity **list**, not per-entity versions, and this feature's own Input names frozen
    version rows without naming `06`'s manifest — which is where each entity's current published
    version reference lives. The head's `published_version` is the only other candidate and may be
    ahead of the catalog version, and reading it would breach the three-column carve-out.
    **Blocks**: no DoD — **resolved by P-D-70**; `cpt-cf-bss-products-dod-projector` carries the feed.
    **Owner**: was this feature with `06-catalog-version`; **closed**.

21. ~~**Where is the `StalenessStamp` persisted?**~~
    **Answered (owner call, 2026-09-01 — P-D-70 arm 6): one per-tenant stamp row** carrying the last
    `catalog_version_id` and `projectedAt` — the only arm that answers an **empty** projection, the
    anchorless rebuild's own case.
    Original text: Every read response carries both halves, and
    **neither is in the projection row's normative shape** — `design/08` §3.1 ends at
    `published_version`. A per-tenant stamp row, a duplicated column on every projection row, and
    derivation from the consumer checkpoint are all admissible and none is stated; the answer is
    load-bearing for the anchorless rebuild arm, which has no version to restate the stamp from.
    **Blocks**: no DoD — **resolved by P-D-70**; `cpt-cf-bss-products-dod-staleness-stamp` is freed.
    **Owner**: was this feature with **P-D-07**'s owner — the pairing carried row 13 already names; **closed**.

22. ~~**Is faceting a route or a browse parameter, and are the dashboards endpoints at all?**~~ **Answered (P-D-126, 2026-09-03): facets are a browse parameter, dashboards are endpoints** — browse, the entity read, the history timeline and one per dashboard; `DECOMPOSITION` §2.8's *"two"* is the projection pair and is owed the dashboards. *The item's text stood as:*
    `design/08` §3.2 and this document both count **four** read endpoints, while DECOMPOSITION
    §2.8's API field lists **two** and `cpt-cf-bss-products-dod-browse-door` folds the facet half
    into the browse route. `cpt-cf-bss-products-dod-facets` names no API. Until the door count is
    fixed, the limiter's coverage criterion cannot be written.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-degradation`, `cpt-cf-bss-products-dod-facets`.)*
    **Owner**: `design/08`'s owner, with `04`, `05` and `06` for the dashboard door of row 10.

23. ~~**The convergence meter's stated origin has no operand on anything the projector receives.**~~ **Answered (P-D-124, 2026-09-03): the outbox body row's `created_at`**, written inside the mutating transaction by the toolkit's own migration — every event class has it, not only publishes. *The item's text stood as:*
    The clock starts at **write commit**, and neither the interim envelope nor the broker's event
    core carries a timestamp. The only commit-adjacent stamp in the crate is
    `products_entity_version.published_at`, which exists on the publish path alone — and a head save
    writes no version row at all, so the non-publish event classes have no origin.
    **Blocks**: no DoD — **resolved by P-D-124** *(was: `cpt-cf-bss-products-dod-nfr-meters`.)*
    **Owner**: this feature with `01-foundation`, whose outbox owns the first segment.

24. ~~**Is a "consumer checkpoint per aggregate" a resume position or a dedup marker?**~~ **Answered (P-D-126, 2026-09-03): a resume position per `(topic, partition)`**, the platform's shape; per-aggregate order holds within the partition. *The item's text stood as:* Only a resume
    position can *"predate the available event tail"* and trigger a rebuild, and the platform's only
    checkpoint shape is per `(topic, partition)` — with one partition per tenant. So a per-aggregate
    coordinate exists in this gear's outbox and not at the broker the projector consumes. Carried
    rows 8 and 15 both presume one.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-projector`.)*
    **Owner**: this feature with `12-consumer-contracts`, which owns the replay contract the rebuild
    starts from.

25. ~~**May a timeline render resolve an identity through the map, and under which grant?**~~ **Answered by P-D-117 (recorded by P-D-126, 2026-09-03): only the compliance export resolves through the map**; the timeline renders pseudonyms. *The item's text stood as:*
    `design/10` §6 registers that *"the two slices disagree about what 08 does"* — `compliance ×
    export` being the only grant any document attaches to a map read, while `inst-im-render` has 08's
    projections resolving at render time and `design/08` says only *"actor pseudonyms"*. The owner it
    names is *"05's RBAC catalog owner with this slice and 08"*, so this feature is a co-owner and
    the question is cited rather than answered.
    **Blocks**: no DoD — **resolved by P-D-126** *(was: `cpt-cf-bss-products-dod-history-timeline`.)*
    **Owner**: `05-governance`'s RBAC catalog owner with `10-retention-erasure`'s and this feature.

### Owed to other documents, recorded and deliberately not edited

- **`design/02` §6 records that the taxonomy and metadata limits have no interim default anywhere**,
  and names *"08's bounded subtree recompute"* as one of four rules that read them, with the owner as
  the §17.1 policy owner. `cpt-cf-bss-products-dod-reparent`'s termination argument rests on those
  values. Recorded as the DoD's own stated gap; the register entry is `design/02`'s.
- **`features/clone.md` §7 row 23** named only `cpt-cf-bss-products-dod-clone-read-surface` as
  blocked and is now **closed by P-D-77**. `cpt-cf-bss-products-dod-frozen-read-path` was a second
  consumer of the same absent pair; the decoder half is no longer absent. The reciprocal is kept
  here as the citation record, not as a live blocker — live blockers are this feature's §7 rows 9
  and 19.
- **`DECOMPOSITION.md` §2.8 Data** lists `products_read_entity` and the three polled dashboard
  tables, and **omits `products_read_stamp`**. P-D-70 arm 6 settled the per-tenant stamp row after
  that Data list was written. This feature cannot edit `DECOMPOSITION.md` (strand boundary); the
  repair is owed to the design-set owner.
- **`design/10` §6's identity-map question names this feature as a co-owner and `design/08` does not
  carry it.** Recorded as row 25 above; the entry is `design/10`'s.

