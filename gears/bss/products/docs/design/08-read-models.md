<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./06-catalog-version.md | Owners: BSS Product Catalog team -->

# DESIGN — Read Models & Browse (Slice 8)

<!-- toc -->

- [1. Context](#1-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
  - [1.5 Scope](#15-scope)
  - [1.6 Constraints & Assumptions](#16-constraints--assumptions)
  - [1.7 Naming & Design-Introduced Names](#17-naming--design-introduced-names)
  - [1.8 Context & Dependencies](#18-context--dependencies)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Browse / search / filter](#browse--search--filter)
  - [Read the version-history timeline](#read-the-version-history-timeline)
  - [Project (the write→read pipeline)](#project-the-writeread-pipeline)
  - [Degrade gracefully](#degrade-gracefully)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 Projection shape & partitioning](#31-projection-shape--partitioning)
  - [3.2 Error taxonomy (slice-owned codes)](#32-error-taxonomy-slice-owned-codes)
  - [3.3 NFR measurement](#33-nfr-measurement)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice owns every consumer-facing **read** surface that is not a frozen snapshot: the
cache-first browse/search projections (Products, SKUs, categories), the **per-state visibility
contract**, the **`asOfCatalogVersion` staleness signal**, graceful degradation under overload,
the faceted search/filter surface (p2), and the projections other slices delegated here
(04's `DeferredRetireIntent`, the approval queue stays 05's). It is the slice the §7
show-stopper NFRs are premised on: read p95 < 100 ms at ≥ 2 000 QPS/tenant partition,
convergence p99 < 2 s, read availability 99.9 % decoupled from the 99.5 % write path.

### 1.2 Purpose

Portals and sales tooling browse the catalog orders of magnitude more often than anyone writes
it. The write path must never carry that load, and the read path must never lie in the
dangerous direction: stale is acceptable and **labeled**; unpublished, cross-scope, or
mislabeled content is never acceptable at any load.

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-presentation` | The browse/search consumer; cache warming |
| `cpt-cf-bss-products-actor-marketplace` | Listing reads over published SKUs |
| `cpt-cf-bss-products-actor-auditor` | Version-history timeline reads (served from 01's frozen rows, projected here) |
| `cpt-cf-bss-products-actor-catalog-admin` | Deferred-intent and freeze-status dashboards |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.8 (`fr-cache-first-browse`), §5.1 (advanced search p2), §7
  (NFR #1/#2/#7/#10 + convergence); AC #32; glossary (Read model)
- [`./01-foundation.md`](./01-foundation.md) §4.3 — frozen versions as the **only**
  consumer-read surface (the projector's sole entity-content source)
- [`./06-catalog-version.md`](./06-catalog-version.md) — `CatalogVersionPublished` (the
  staleness anchor), the capture store (category/definition content)
- [`./04-lifecycle.md`](./04-lifecycle.md) M6 — 04 owns the deferred-intent query surface;
  this slice only projects it

### 1.5 Scope

**In**:
- the projector (event-driven, from 01/04/06 events over frozen content)
- the projection schemas
- per-state visibility
- staleness signaling
- scoping enforcement on the read path
- degradation behavior
- facets/filters (p2)
- history-timeline projection
- the convergence budget and its measurement.

**Out**:
- the write path and its availability (01)
- frozen-snapshot resolution (06's `IntentfulResolver` — a different surface with different guarantees)
- the approval queue (05)
- external search infrastructure choices (implementation detail behind the projection contract).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Cache-first: browse/search never touches write-path tables at request time; projections are the serving store | PRD `fr-cache-first-browse` |
| C2 | Per-state visibility: `published` browsable; `deprecated` browsable **with a machine-readable flag** and excludable by filter; `retired` excluded from default browse, retrievable only via explicit history query; `draft`/`discarded` never served | PRD AC #32 |
| C3 | Every read response carries `asOfCatalogVersion` — the one staleness signal, in degraded mode too (no silently-stale response) | PRD `fr-cache-first-browse`, NFR #7 |
| C4 | Stale reads are safe: never unpublished, never cross-scope, at any load; shedding/queuing over leaking | NFR #7 |
| C5 | Convergence: projection reflects a write within p99 < 2 s **of write commit** — the PRD's thrice-stated clock (M1 fix: the earlier re-basing to outbox acceptance collapsed budgets NFR #3 keeps distinct); decomposed as commit→durable-acceptance (01's outbox meter) + acceptance→projected (this slice's meter) | PRD §17.1, NFR #3 |
| C6 | **Product/SKU** entity content projects **only** from frozen `products_entity_version` rows — never from head rows (01's rule; what makes stale-but-safe structural). The frozen set **excludes** `lifecycle_state`, `deprecation_provenance` and `replaced_by_sku_id` (**P-D-24**) — those move on transitions that write no version row, so this slice reads them from the head row, which is what this slice's own row listing — "identity, state + flags" — already assumed. The events it projects from carry 01's common body core, `*Published` additionally carrying **`publishedVersion`** (**P-D-27**), which is the projector's key. **Governed live entities** (categories + display values, definitions, recognized sets/tier labels) are read from their **live tables** (H3 fix: they have no frozen versions and no draft state to leak — their mutations are governed-and-applied, so a live read is already-published content; the per-CV captures are 06's snapshot concern, not this projector's source) | 01 §4.3, H3 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `ReadProjector` | The single event-driven consumer building all projections (per-tenant ordered by the outbox `(tenant, aggregate)` keys) |
| `BrowseProjection` | The denormalized per-tenant serving rows: entity content + display attributes (resolved per `LocaleResolver`) + category paths + state + flags |
| `StalenessStamp` | The per-tenant high-water mark `(asOfCatalogVersion, projectedAt)` every response carries |
| `VisibilityFilter` | The per-state contract applied at query build time — not post-filtering |

### 1.8 Context & Dependencies

**Consumed**: 01 events (publishes, discards), 04 events (deprecation/retirement flips —
**not** deferred intents: those sources emit none, and their dashboards are **polled
projections** from 04's own table per `inst-ps-shape`, item 35 of the 2026-08-26 review), 02 events (`Category*`, `CategoryDisplayUpdated`,
`AttributeDefinitionUpdated`), 06 (`CatalogVersionPublished` — advances the `StalenessStamp`),
03 vocabulary events (tier labels for display). **Produced**: the browse/search API, the
history timeline, the dashboards; the convergence and staleness metrics.

## 2. Actor Flows (CDSL)

### Browse / search / filter

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-browse`

1. [ ] - `p1` - `GET /bss-products/v1/browse…` (`product|sku × read`): tenant/brand/region scope is resolved from claims and applied **inside the query** (`VisibilityFilter` + scope predicates at query build — post-filtering is forbidden because a shed row must never have been fetched) - `inst-rb-query`
2. [ ] - `p1` - Per-state contract (C2): `deprecated` rows carry the machine-readable flag and an `excludeDeprecated` filter; `retired` appears only through the explicit history surface - `inst-rb-visibility`
3. [ ] - `p1` - Every response carries the `StalenessStamp` (C3) — including error and degraded responses that carry any content at all - `inst-rb-stamp`
4. [ ] - `p2` - Facets (category tree, type, tier label, sellable, unit) build from the same projection; filterable under **every** assigned category (primary + secondary — the 02 contract) - `inst-rb-facets`

### Read the version-history timeline

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-history`

1. [ ] - `p2` - `GET …/{entity}/versions` (`audit × read` for cross-entity trails; `product|sku × read` for the own-entity timeline): projected from 01's frozen rows — version list, per-version diff (computed between frozen rows), approval refs, actor pseudonyms; `retired` entities are reachable here (the C2 carve-out) - `inst-rh-timeline`

### Project (the write→read pipeline)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-project`

1. [ ] - `p1` - `ReadProjector` consumes the broker per `(tenant, aggregate)` ordering; each event is projected idempotently (the sequence is a **consumer checkpoint per aggregate**, not a row version — L1); Product/SKU content is fetched from frozen versions by the ids the event carries, live-entity content from its live tables (C6), never from heads - `inst-rp-consume`
2. [ ] - `p1` - **The stamp is a floor (P-D-07)**: every catalog version ≤ the stamp is fully reflected, and later entity events may **add, change, or remove** content relative to the stamped version (H1 fix — the earlier "strictly additive" premise was false: a retirement flip removes without an increment); `projectedAt` is the fine-grained coordinate; a tenant with zero catalog versions stamps `asOfCatalogVersion = null` + `projectedAt` (M6). The stamp **advances only after the event's own changed-entity list is projected from frozen rows in the same step** (H2 fix: the stamp never claims a version whose content it is missing, regardless of cross-aggregate arrival order) - `inst-rp-stamp`
3. [ ] - `p1` - A projector checkpoint that predates the available event tail **fails loudly** and rebuilds from the bootstrap path (latest `CatalogVersion` + tail — the slice-12 replay contract); the rebuild serves the old projection until cutover (read availability through rebuilds) - `inst-rp-bootstrap`
4. [ ] - `p2` - A category **re-parent** re-files every descendant's browse path: the projector recomputes the affected subtree from the event (the 02 risk, owned here); the subtree recompute is bounded by the taxonomy depth/children limits - `inst-rp-reparent`

### Degrade gracefully

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-degrade`

1. [ ] - `p1` - Above the throughput ceiling: shed or queue with `429`/`503` + `Retry-After` — never serve from the write path, never widen scope, never drop the stamp (C3/C4); shedding is per tenant partition so one tenant's burst cannot starve another - `inst-dg-shed`
2. [ ] - `p1` - Under projector lag past the convergence budget: keep serving (stale-but-labeled), raise `read_model_lag` naming the tenant and the lag; the stamp makes the staleness machine-readable to every caller - `inst-dg-lag`

## 3. Processes / Business Logic

### 3.1 Projection shape & partitioning

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-projection`

1. [ ] - `p1` - `products_read_entity` — per-tenant denormalized rows: identity, state + flags (`deprecated`, `compositionPending`, `sellable`), **brand/region scope columns** (the operands `VisibilityFilter`'s scope predicates filter on — M2), type/tier/unit display fields, resolved display attributes per locale coordinate (materialized for the tenant's active locales), category paths, `published_version`. Partitioned/indexed per `(tenant_id, …)` — the NFR #1/#2 unit is the tenant partition - `inst-ps-shape`
2. [ ] - `p1` - `products_read_deferred_intent`, `products_read_freeze_status`, `products_read_delivery_state` — the operator dashboards. These are **polled projections from their owning surfaces** (04's deferred table, 06's `FreezeLedger`, the broker's per-consumer delivery/DLQ state — the `fr-event-delivery-resilience` projection clause, M5), NOT broker consumers: their sources deliberately emit no events (H4 fix — the earlier "refreshed by the same projector" claimed sources that don't exist) - `inst-ps-dashboards`
3. [ ] - `p2` - **Join mechanics (L1)**: a browse row joins ≥ 3 aggregates; the join is *convergent* — every join-relevant event recomputes the affected rows from projection-local state, and a row whose join target has not yet projected is **parked (withheld from browse) and re-attempted**, the parking bounded by the convergence monitoring (never a placeholder render). The metadata map is **excluded from search by construction** (C: never projected into any searchable field; PRD glossary) — it is retrievable on the single-entity read only - `inst-ps-metadata`

### 3.2 Error taxonomy (slice-owned codes)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-contract-read-errors`

`READ_MODEL_OVERLOADED` (shed; carries `Retry-After`) — raised by the **single per-tenant-partition limiter component in front of every read endpoint** (browse, history, facets, dashboards): one door (L4). Everything else on this surface is standard not-found/validation via 01's envelope — reads introduce no new failure semantics.

**Problem responses (RFC 9457):** `READ_MODEL_OVERLOADED` (503).

*Statuses added 2026-08-26, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, 2026-08-02, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"* — the citation was right the first time;
a 2026-08-26 pass re-pointed it at D-186 and was wrong to, D-186 being a later amendment scoped to
one config route) and where an earlier pass here wrongly wrote
412 and called that pricing's convention — **403** where the caller may not perform the act at
all, **404** only where a path segment names a resource this tenant has none of. **503** where retry
is the remedy is this gear's own addition — pricing's set carries no 503 at all, so that one
class is not "checked against it". **The 422s here are architectural, not wire** — see 01 §3.3, which quotes the sibling
plan-price gear's rule (the `MUST NOT` being this gear's own choice, 01 §3.3): no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for an error **carrying a registry code** in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.*

### 3.3 NFR measurement

- [ ] `p2` - **ID**: `cpt-cf-bss-products-algo-read-nfrs`

p95 latency and QPS per tenant partition (NFR #1/#2) measured at the API edge; convergence
(C5) measured **write-commit → projection-visible** per event class, decomposed into the two
meters C5 names (commit→durable-acceptance, 01's outbox meter; acceptance→projected, this
slice's) — **not** outbox-acceptance → projection-visible, which is the re-basing C5's M1 fix
explicitly struck for collapsing budgets NFR #3 keeps distinct (item 35 of the 2026-08-26
review); availability split (NFR
#10) — the read path's health is independent of write-path health by construction (separate
serving store), and the probe that proves it is a read served during a simulated write-path
outage.

## 4. Data / Storage (normative shape; DDL in migrations)

§3.1's projection tables — **rebuildable state, not records**: no append-only guards, no
audit rows of their own (the audited truth lives upstream); dropped and rebuilt from the
bootstrap path at any time without loss — **with the zero-version tenant stated rather than
assumed** (item 35 of the 2026-08-26 review): the bootstrap initializes from the latest
`CatalogVersion` (12 `inst-rc-bootstrap`), and a tenant that has published none has no such
anchor, so its rebuild starts from the **empty catalog plus the full event tail**, which is
lossless precisely because there is no pre-tail content to lose. "Without loss" is therefore
total, and the anchorless case is a distinct code path, not an edge case the sentence glosses. This family is exempt from the **published/history
append-only guard** (L2 — softened: the idempotency store's sweep and guarded category deletes
are other non-append surfaces; the exemption is recorded at 01 C5 too), and the exemption is
the point.

## 5. Testing posture (slice-local)

- Visibility matrix: one fixture asserting all five states across default browse, filtered
  browse, and history — `draft`/`discarded` absent everywhere, `retired` only in history,
  `deprecated` flagged + excludable (each refusal with its positive control).
- Scope probe, layer-split per AC #30 (L3): an out-of-claim request is **denied and audited at
  the gateway**; the SecureORM emptiness beneath it is defense-in-depth (the probe asserts
  both layers, so a door silently absorbing auditable cross-scope attempts fails it); a shed
  response leaks neither content nor counts (C4 under simulated overload).
- Convergence probe: publish → projection visible within budget, **measured from write
  commit** and decomposed into the two meters C5 names — never from outbox acceptance, the
  re-basing C5's M1 fix struck for collapsing budgets NFR #3 keeps distinct (2026-08-26 branch
  review: the M1 fix landed at the constraint and at `algo-read-nfrs` and missed the probe, so
  the one artefact that would actually be written could not fail the case the fix exists to
  catch — a slow outbox eating the whole convergence budget invisibly); lag alarm fires past
  budget while serving continues stale-but-stamped.
- Rebuild probe: checkpoint-behind-tail → loud failure → bootstrap rebuild → cutover with the
  old projection serving throughout.
- Re-parent probe: a subtree re-files completely; no orphan paths.
- Stamp probe: every response shape (success, empty, degraded) carries `asOfCatalogVersion`.

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-usecase-catalog-browser-history` (§10 use case, claimed by id here 2026-08-26 — all seven were in lint 1's universe and none was claimed); **§9.1 by id** — `cpt-cf-bss-products-interface-read-model` (the browse/search surface this slice serves; claimed by id here for the first time, 2026-08-26 branch review). `cpt-cf-bss-products-fr-cache-first-browse`; AC #32; **NFRs by id** (#1 `cpt-cf-bss-products-nfr-read-latency`,
**#2** `cpt-cf-bss-products-nfr-read-throughput`, **#7** `cpt-cf-bss-products-nfr-graceful-degradation`,
#10 `cpt-cf-bss-products-nfr-availability-audit` — positional numbers alone left `inst-cc-fr` reporting zero claims
for all ten NFRs, item 30 of the 2026-08-26 review) + the convergence interim (§17.1);
**AC #39** (the registry-side obligations: durable acceptance before reported success, the
per-consumer delivery/DLQ projection, no state mutation on delivery failure — previously cited
by nothing); `cpt-cf-bss-products-fr-event-delivery-resilience` (the per-consumer delivery/DLQ **projection**
clause — M5); the §5.1 p2 rows "Advanced search, filter & faceting" and the read half of
"Catalog read models"; 04-M6 (deferred-intent projection), 02 re-parent invalidation; P-D-07
(the stamp-floor semantics).

**Risks & open items**:
- **P-D-07 (floor + gated advance) — CONFIRMED by the product owner 2026-08-26, conditionally**
  (was: flagged). The original "strictly additive" premise was refuted by this slice's own
  review (H1) and replaced, not patched. The confirmation is conditional on there **being** a
  projection: the floor is a property of a serving store that lags, so it has no subject
  without one.
- **Open above this slice: does browse need a separate serving store at all?** Raised
  2026-08-26 and now a PRD §15 question for the NFR workshop. `fr-cache-first-browse`'s
  rationale rested on two uncalibrated numbers — NFR #1's 10K SKUs/tenant is a scale a direct
  multi-way query plausibly serves, and NFR #2's ≥ 2,000 read QPS/tenant partition is not a
  portal number. The FR's rationale has been re-derived onto the two properties that survive
  recalibration and that this slice actually supplies: the **availability split** (C1 + §3.3's
  write-path-outage probe) and **structural stale-but-safe** (C6 — projecting only from frozen
  rows, never heads). If the workshop retires the projection, this slice collapses to a query
  layer and P-D-07 is deleted with it; `products_read_delivery_state` survives regardless (it
  polls the broker's delivery/DLQ state, which is not in this gear's database).
- **Locale materialization** (per active locale) trades storage for the p95 budget; the
  active-locale set per tenant needs a config home — implementation note.
- Search-engine choice (LIKE/FTS vs external) is deliberately behind the projection contract;
  the NFR #2 load test decides, not this document.
