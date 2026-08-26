<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md | Owners: BSS Product Catalog team -->

# DESIGN — Taxonomy & Attributes (Slice 2)

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
  - [Manage the taxonomy (create / rename / re-parent / retire / delete)](#manage-the-taxonomy-create--rename--re-parent--retire--delete)
  - [Assign categories to a Product](#assign-categories-to-a-product)
  - [Manage attribute definitions (governed live entities)](#manage-attribute-definitions-governed-live-entities)
  - [Author localized attribute values](#author-localized-attribute-values)
  - [Write the metadata map](#write-the-metadata-map)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 The governed-live-entity pattern](#31-the-governed-live-entity-pattern)
  - [3.2 Taxonomy integrity mechanics](#32-taxonomy-integrity-mechanics)
  - [3.3 Error taxonomy (slice-owned codes)](#33-error-taxonomy-slice-owned-codes)
  - [3.4 Concurrency](#34-concurrency)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
  - [4.1 Tables](#41-tables)
  - [4.2 Well-known seeds (`WellKnownSeed`)](#42-well-known-seeds-wellknownseed)
  - [4.3 Events](#43-events)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice owns the two **describing** surfaces of the registry: the **Category taxonomy**
(hierarchical, cycle-free, browse/curation backbone) and the **attribute system** — governed
attribute **definitions**, localized attribute **values** with the brand/region fallback chain,
the seeded **well-known display attributes**, and the ungoverned **metadata map**. It introduces
the design pattern the PRD calls a **governed live entity**: an object that is *not*
draft/publish-versioned (no lifecycle state machine, no published version) but whose material
mutations still pass the slice-05 two-person gate and land atomically, in place, audited and
evented.

### 1.2 Purpose

Give Products/SKUs their classification and their sales-facing description without ever minting
a second identity: the taxonomy is for browse and curation (never pricing), attribute values are
entity content riding the Foundation's revisions, and the display names repeat freely precisely
because the canonical internal name is a quasi-code (P-D-04).

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-product-manager` | Authors attribute values, assigns categories |
| `cpt-cf-bss-products-actor-catalog-admin` | Taxonomy ops (create/rename/re-parent/retire/delete), attribute-definition lifecycle |
| `cpt-cf-bss-products-actor-presentation` | Consumes localized resolution + category filters via read models (slice 08) |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.2 (`fr-manage-taxonomy`, the category half of `fr-create-product`),
  §6.4 (`fr-localized-attributes`), §6.11 (the content-PII write block this slice hosts);
  AC #5 (category part), #6, #12, #35 (write-block clause)
- [`./01-foundation.md`](./01-foundation.md) — the pipeline this slice registers into
- [`../DECISIONS.md`](../DECISIONS.md) P-D-04 (display names repeat), P-D-06 (metadata map
  placement — introduced by this slice)

### 1.5 Scope

**In**: category tree + its invariants and ops; attribute definitions (typed, localized flag,
brand/region visibility, deprecate-then-remove); attribute values + locale fallback resolution;
well-known seeds; metadata map; the governed-live-entity mutation pattern (op envelope handed to
slice 05); the content-PII write-block **hook** (detector policy owned by slice 10).

**Out**: approval machinery itself (05); read-model/search projections and faceting (08);
category read-model warming (08); erasure execution (10); `PlanTier` and recognized sets — also
governed live entities, but owned by slice 03 which reuses this slice's pattern.

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Categories and attribute definitions are **live entities**: no draft state, no published version — in-place mutation under the slice-05 gate for material ops | PRD `fr-manage-taxonomy` |
| C2 | **Product/SKU** attribute values are entity content: they ride the owning entity's internal revision, freeze into its published versions, and never mutate a frozen version. **Category** values are live-entity content — categories have no revisions (H2 fix; see `inst-av-category-branch`) | PRD `fr-revision-vs-version` |
| C3 | Taxonomy limits (max depth, max children/node) are configured policies with interim defaults (PRD §17.1 extensibility limits) | PRD §7 `nfr-scale-extensibility` |
| C4 | The metadata map is ungoverned, size-bounded, non-localized, search-excluded, PII-prohibited | PRD glossary + `fr-localized-attributes` |
| C5 | No binary assets; `imageUri` and friends carry reference URIs only | PRD §5.2 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `GovernedLiveOp` | The pinned operation envelope for live-entity mutations: `(op kind, target, payload, expected target state)` — what slice 05 approves and what the apply step executes atomically |
| `TaxonomyWalk` | The in-transaction ancestor walk validating acyclicity + depth on create/re-parent |
| `LocaleResolver` | The fallback-chain evaluator `(locale, region, brand) → (locale, brand) → (default-locale, brand) → global` |
| `WellKnownSeed` | The migration-seeded attribute-definition set (see §4.2) |

### 1.8 Context & Dependencies

**Consumed**: Foundation doors + pipeline (01); slice-05 gate for material live-entity ops;
config store (depth/children/size limits, tenant default locale). **Produced**: `Category*`
events, `AttributeDefinitionUpdated`, `MetadataUpdated`; the category-assignment and
attribute-value validators registered on Product/SKU save and publish; the localized-resolution
contract slice 08 projects.

## 2. Actor Flows (CDSL)

### Manage the taxonomy (create / rename / re-parent / retire / delete)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-manage-taxonomy`

1. [ ] - `p1` - Authorize `category × write`; wrap the request as a `GovernedLiveOp`; **every taxonomy op is material** (PRD `fr-materiality-gated-publish` enumerates category create/rename/re-parent/retire/delete), so the op queues through the slice-05 two-person gate before anything mutates - `inst-tx-governed-op`
2. [ ] - `p1` - On apply, re-validate against the **live** tree (the gate pinned the op, not the world): name uniqueness within the parent on `(tenant_id, parent_id, normalized(name))` — re-checked on rename **and** re-parent; violation fails `DUPLICATE_CATEGORY_NAME` - `inst-tx-name-in-parent`
3. [ ] - `p1` - `TaxonomyWalk` inside the write transaction, under the per-tenant taxonomy writer lock (§3.4): a re-parent whose new ancestor chain contains the node itself fails `TAXONOMY_CYCLE`; a create/re-parent exceeding configured max depth or max children fails `TAXONOMY_LIMIT` naming the limit - `inst-tx-walk`
4. [ ] - `p1` - Retire/delete **MUST** be refused while any **non-terminal** Product (`draft`/`published`/`deprecated` — the PRD's operand is "active", and `retired` *and* `discarded` are both terminal) references the category (primary or secondary) or any active child exists. **The guard reads the referencing Product's lifecycle state, never the presence of a `products_product_category` row** (item 17 of the 2026-08-26 review: discard releases the code and name reservations but leaves the category link, so on the old "non-`retired`" operand one discarded draft blocked the category permanently) — `CATEGORY_REFERENCED`, with a sample of holders named; retire marks the node closed to new assignment, delete is admitted only on a retired, empty, unreferenced node - `inst-tx-retire-guard`
5. [ ] - `p1` - Each applied op writes audit + emits its event (`CategoryCreated`/`CategoryRenamed`/`CategoryReparented`/`CategoryRetired`/`CategoryDeleted`) in the same transaction; the op envelope id rides the event for approval traceability - `inst-tx-event`

### Assign categories to a Product

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-assign-categories`

1. [ ] - `p1` - A Product carries **exactly one primary + zero or more secondary** categories; assignment is ordinary draft content (rides `inst-fd-save-txn`); this slice registers the validators: target category exists, is not retired, duplicates between primary/secondary rejected - `inst-tx-assign`
2. [ ] - `p1` - At publish, the registered `→ published` validator requires the primary category present (`PRIMARY_CATEGORY_REQUIRED` — its own code per 01's one-door rule, L8 fix; the PRD's "optional at draft, required at publish") - `inst-tx-primary-at-publish`

### Manage attribute definitions (governed live entities)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-attribute-definitions`

1. [ ] - `p1` - A definition = `(key, value type, localized?, brand/region visibility scope, state)`; create and **material** changes (type change, visibility narrowing, deprecation) ride `GovernedLiveOp` through the slice-05 gate; display-label edits are non-material single-approver ops per the §17.1 interim materiality default - `inst-ad-governed`
2. [ ] - `p1` - Changes **MUST** be backward-compatible: a type change on a definition with live values is refused (`DEFINITION_IN_USE`) — the path is deprecate-then-remove: `deprecated` blocks new values, removal is admitted once no **non-terminal head** (published/deprecated Product or SKU, active category) carries a value — frozen versions are **self-contained copies**: they stay renderable after removal, and they neither block it nor are touched by it (operand narrowed to the PRD's live-reference condition — M2 fix, 2026-08-25 review) - `inst-ad-deprecate-then-remove`
3. [ ] - `p1` - Every applied change emits `AttributeDefinitionUpdated` + audit - `inst-ad-event`

### Author localized attribute values

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-attribute-values`

1. [ ] - `p1` - Values are entity content (C2): writes ride the entity draft-save door; this slice registers validators — definition exists and is not `deprecated` (`ATTRIBUTE_DEFINITION_UNKNOWN`/`_DEPRECATED`), value matches the declared type, `(locale, region, brand)` coordinates lie within the definition's visibility scope **and** the entity's own scope - `inst-av-validate`
2. [ ] - `p1` - The **content-PII write block** runs here for attribute/description free text: hard prohibition, fail-closed on uncertainty, curated allow-list for legitimate person-named products; the detector policy + allow-list are slice 10's, this door only invokes them (`CONTENT_PII_BLOCKED`) - `inst-av-pii-block`
2a. [ ] - `p1` - **The same block runs on every operator free-text `reason`** (item 24 of the 2026-08-26 review), enumerated so no door is left out: audit rows (01 §4), approval rejections and break-glass session reasons (05), correction-override and break-glass-correction reasons (07), the retirement reason carried into the `SkuRetired` broker payload (PRD §12), and bulk/promotion row reasons (09). These records are **never edited** and erasure is a map-only tombstone (10 C1), so PII typed into one of them is unreachable by erasure **forever** and, for `SkuRetired`, has already left the gear. Fail-closed at the door is therefore the only reach erasure can have over them — the same detector, the same allow-list, the same `CONTENT_PII_BLOCKED`, invoked by the owning door - `inst-av-pii-reason`
3. [ ] - `p1` - The registered `→ published` validator requires the **default-locale value at the global (brand-less) coordinate** for every localized definition the entity carries values for (rejected at publish, not at draft save); per-brand default-locale values are optional overrides — the global one is what makes the fallback chain total for **every** brand (M5 fix, 2026-08-25 review) - `inst-av-default-locale`
4. [ ] - `p1` - Read-side resolution (consumed by slice 08): `LocaleResolver` walks `(locale, region, brand) → (locale, brand) → (default-locale, brand) → global`; default-locale resolves per brand, falling back to the tenant default — the chain is total for every brand by step 3's **global** default-locale guarantee. **Totality is anchored on the resolution path, not on the config value** (item 37 of the 2026-08-26 review): the tenant default locale is ungoverned config with no re-validation, so anchoring on it would un-total the chain for every already-published entity the moment it changed. So the final step is the **global** fallback and the tenant default is only a *preference* consulted before it; a tenant-default change is therefore non-retroactive by construction — the same posture `inst-ti-limits` states for depth limits - `inst-av-resolve`
5. [ ] - `p1` - **Category branch (H2 fix, 2026-08-25 review)**: categories have no revisions or publishes, so their display values are **live-entity content**: written through a category live-value door (`If-Match` on the category row-version token; non-material single-approver per the §17.1 default — a display edit is not a rename of the canonical name), audited and emitted as `CategoryDisplayUpdated`; the **global default-locale value is required at the first write** of a definition for that category (the write-time analogue of step 3); a `CatalogVersion` captures current category values as of its snapshot instant — they have no frozen versions of their own - `inst-av-category-branch`

### Write the metadata map

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-metadata`

*Door: `PATCH /bss-products/v1/{products|skus}/{id}/metadata`, grant **`metadata × write`** (05's
catalog — named here 2026-08-26; the flow had named neither a path nor a pair, so slice 12's
lint 3 could not see it).*

1. [ ] - `p2` - Per-entity string→string map; size-bounded (configured caps on key count, key and value byte length — `METADATA_LIMIT`); non-localized; PII-prohibited (the same `inst-av-pii-block` hook, no carve-out); excluded from read-model search by construction (08 never projects it) - `inst-md-write`
2. [ ] - `p2` - **Placement (P-D-06)**: the map lives **beside** the entity, outside the frozen published-version content — mutable in place on any non-terminal entity without a version bump, audited + `MetadataUpdated`-evented per write; a `CatalogVersion` captures the map **as of its own snapshot instant**, and that copy is frozen with the snapshot (old snapshots never move — byte-identity holds) - `inst-md-placement`

## 3. Processes / Business Logic

### 3.1 The governed-live-entity pattern

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-governed-live`

1. [ ] - `p1` - A `GovernedLiveOp` pins the **operation** (kind + target + payload + the target's expected current state), not an entity revision; slice 05 approves the envelope; the apply step re-validates the expected state against the live row and fails `STALE_LIVE_OP` if the world moved (the live-entity analogue of the Foundation's pinned-revision publish) - `inst-gl-envelope`
2. [ ] - `p1` - Apply is atomic: mutation + audit + event in one transaction; there is no partially-applied taxonomy op - `inst-gl-atomic`
3. [ ] - `p1` - The pattern is exported: slice 03 reuses it verbatim for `PlanTier` and the recognized code/unit sets - `inst-gl-export`

### 3.2 Taxonomy integrity mechanics

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-taxonomy-integrity`

1. [ ] - `p1` - Acyclicity is validated by `TaxonomyWalk` (ancestor chain of the new parent must not contain the node) executed inside the write transaction; correctness rests on §3.4's single-writer discipline, not on the walk alone — two concurrent re-parents could otherwise each pass and jointly close a cycle - `inst-ti-acyclic`
2. [ ] - `p1` - Uniqueness-in-parent is also an index (`UNIQUE (tenant_id, parent_id, name_normalized)` over non-deleted rows), so the read-then-write race is decided by the store exactly as the Foundation's `ReservationIndex` decides `skuCode` - `inst-ti-unique-index`
3. [ ] - `p2` - Depth/children limits are validated on the mutation path only — a later limit **decrease** never invalidates existing structure (config change is not retroactive; the lint reports over-limit subtrees informationally) - `inst-ti-limits`

### 3.3 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-taxonomy-errors`

`DUPLICATE_CATEGORY_NAME`, `TAXONOMY_CYCLE`, `TAXONOMY_LIMIT`, `CATEGORY_REFERENCED`,
`CATEGORY_RETIRED` (assignment to a retired node), `ATTRIBUTE_DEFINITION_UNKNOWN`,
`ATTRIBUTE_DEFINITION_DEPRECATED`, `DEFINITION_IN_USE`, `ATTRIBUTE_TYPE_MISMATCH` (the attribute-value write, when a value does not match its definition's declared type — named 2026-08-26),
`ATTRIBUTE_SCOPE_VIOLATION`, `DEFAULT_LOCALE_MISSING` (publish-time), `PRIMARY_CATEGORY_REQUIRED`, `CONTENT_PII_BLOCKED` (verdict policy owned by slice 10 — L1),
`METADATA_LIMIT`, `STALE_LIVE_OP`. Registered into the Foundation taxonomy (01 §3.3); the
AC #38 row "taxonomy cycle" maps here; the PII write-block is **AC #35's** clause (L1 fix —
misattributed to #38 until the 2026-08-25 review).

**Problem responses (RFC 9457):** `DUPLICATE_CATEGORY_NAME`, `CATEGORY_REFERENCED`, `DEFINITION_IN_USE`, `STALE_LIVE_OP` (409); `TAXONOMY_CYCLE`, `TAXONOMY_LIMIT`, `CATEGORY_RETIRED`, `ATTRIBUTE_DEFINITION_UNKNOWN`, `ATTRIBUTE_DEFINITION_DEPRECATED`, `ATTRIBUTE_TYPE_MISMATCH`, `ATTRIBUTE_SCOPE_VIOLATION`, `DEFAULT_LOCALE_MISSING`, `PRIMARY_CATEGORY_REQUIRED`, `CONTENT_PII_BLOCKED`, `METADATA_LIMIT` (422).

*Statuses added 2026-08-26. The gear declared its codes with no HTTP status and no
problem-response block in any slice, against `guidelines/DNA/README.md`'s RFC 9457 rule and
`.cf-studio/config/rules/api-contracts.md`. The mapping follows pricing's convention — 422 for
content the door cannot process, 409 where the current state refuses the act, 403 where the
caller may not perform it at all, 404 for a path naming a resource this tenant has none of,
412 for the `If-Match` precondition, 503 where retry is the remedy. Proposed per row and open
to correction; the requirement is that every code carries one.*

### 3.4 Concurrency

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-taxonomy-concurrency`

1. [ ] - `p1` - Taxonomy mutations serialize **per tenant** behind a taxonomy writer lock (advisory lock on Postgres, the write transaction on SQLite): taxonomy ops are rare and human-paced, and single-writer is what makes `TaxonomyWalk`'s verdict trustworthy - `inst-tc-writer-lock`
2. [ ] - `p1` - Attribute-value and metadata writes need no extra machinery: they ride the entity row's `If-Match` (01) - `inst-tc-etag`

## 4. Data / Storage (normative shape; DDL in migrations)

### 4.1 Tables

- **`products_category`** — `category_id` (PK, uuid) · `tenant_id` · `parent_id` (nullable FK,
  self) · `name` / `name_normalized` · `state` (`active|retired`) · timestamps. Indexes:
  `UNIQUE (tenant_id, parent_id, name_normalized)`; FK children guard on delete. Deletion is
  physical **only** through `inst-tx-retire-guard` (retired + empty + unreferenced); everything
  else is state flips, audited.
- **`products_product_category`** — the **single source of truth** for category assignments
  (01 §4.1 carries no inline category columns) — `(tenant_id, product_id, category_id, role)` with
  `role ∈ {primary, secondary}`; `UNIQUE (tenant_id, product_id, category_id)`; partial
  `UNIQUE (tenant_id, product_id) WHERE role = 'primary'` — exactly-one-primary is an index,
  not a convention.
- **`products_attribute_definition`** — `definition_id` (PK) · `tenant_id` · `key` (unique per
  tenant) · `value_type` · `localized` (bool) · visibility scope (brand/region sets) · `state`
  (`active|deprecated`) · `seeded_by` (nullable — well-known marker) · timestamps.
- **`products_attribute_value`** — owned entity coordinates `(tenant_id, entity_kind,
  entity_id)` + `definition_id` + locale coordinates `(locale?, region?, brand?)` + `value`;
  `UNIQUE` over the full coordinate tuple. For Product/SKU rows: at publish the values are **copied into the frozen
  `products_entity_version` content** (01 §4.3) — the table always holds the current head
  state, history lives in the version rows. For **category** rows the table IS the live state —
  no freeze-copy (H2 fix).
- **`products_metadata`** — `(tenant_id, entity_kind, entity_id, key)` PK · `value` ·
  timestamps; caps enforced at the door (`METADATA_LIMIT`). Outside version content (P-D-06).

### 4.2 Well-known seeds (`WellKnownSeed`)

Seeded by migration, per tenant bootstrap, marked `seeded_by = 'registry'` (a seeded definition
is deprecatable but not deletable): `displayName` (localized, per Product/SKU/Category),
`description` (localized), `imageUri` (URI string, non-localized), `unitDisplayLabel`
(localized — the sales-facing unit label, display only, never the metering-unit identity),
`marketingFeatures` (localized string list). PRD `fr-localized-attributes` + the 2026-08-25
industry-parity widening.

### 4.3 Events

`CategoryCreated` / `CategoryRenamed` / `CategoryReparented` / `CategoryRetired` /
`CategoryDeleted`, `CategoryDisplayUpdated`, `AttributeDefinitionUpdated`, `MetadataUpdated` — broker-native envelope,
ordering key `(tenant, category tree)` for taxonomy (one aggregate: the tree, matching the
single-writer discipline) and `(tenant, entity)` for metadata. Attribute-**value** writes emit
no event of their own: they are entity content and ride `ProductDraftSaved`/`SkuDraftSaved`
(explicit "no event" record per 01 §4.5 rule).

## 5. Testing posture (slice-local)

- Cycle probe: two concurrent re-parents that would jointly close a cycle — one must fail; the
  writer lock is the mechanism under test, not the walk.
- Exactly-one-primary and uniqueness-in-parent probed on the index (concurrent inserts), with
  positive controls.
- Locale fallback: a resolution matrix fixture covering every chain step incl. brand-default
  and tenant-default fallbacks, plus the M5 case — a brand-B reader against a value present
  only at `(default-locale, brand A)`, total only via the global coordinate; publish-time `DEFAULT_LOCALE_MISSING` paired with its green
  control.
- P-D-06 byte-identity probe: metadata mutated after a `CatalogVersion` snapshot; the old
  snapshot's checksum must not move.
- Deprecate-then-remove, probed **both ways** against the M2-narrowed operand (`inst-ad-deprecate-then-remove`:
  the operand is the **non-terminal head**, and frozen versions neither block a removal nor are
  touched by it): removal **refused** while a `published`/`deprecated` head carries a value, and
  **admitted** while only a *frozen version* carries one. The negative arm is the whole point —
  the probe previously asserted the pre-M2 behaviour ("removal refused while a frozen version
  carries a value") and would have gone **green on the defect** (item 13 of the 2026-08-26
  review; the sibling probe in slice 03 was swept at the time, this one was not).

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-fr-manage-taxonomy`, `cpt-cf-bss-products-fr-create-product` (category clauses),
`cpt-cf-bss-products-fr-localized-attributes`, `cpt-cf-bss-products-fr-retention-erasure` (write-block clause, hook only); AC #5
(category part), #6, #12, #35 (write-block), #38 (taxonomy-cycle row).

**Risks & open items**:
- **P-D-06 — CONFIRMED by the product owner 2026-08-26** (was: to ratify). The
  metadata-outside-version-content placement stands as designed: the map is design-introduced
  reading of a PRD that says both "ungoverned" and "captured in CatalogVersion snapshots", and
  it keeps old snapshots byte-identical while letting the map move without version churn. The
  accepted cost, stated at confirmation: the map carries **no history between snapshots** — an
  intermediate value overwritten before the next `CatalogVersion` survives only as the audit
  row recording the write. A key needing version history is an **Attribute**, not metadata.
- **Definition removal candidates**: the guard reads **non-terminal heads** only (M2 —
  no frozen-content scan is involved, and the earlier wording of this item described exactly the
  scan M2 removed), so it is an index-scale check; the open item that survives is
  presentational — the lint should surface removable definitions rather than operators
  discovering the guard by refusal.
- **Category re-parent vs read models**: a re-parent re-files every descendant's browse path;
  slice 08 owes the invalidation contract (noted for its design).
- The PII detector's false-positive posture (fail-closed on uncertainty) will generate operator
  friction; the allow-list governance loop (slice 10 + Legal sign-off, PRD AC #35) must exist
  before GA, not after the first blocked legitimate product name.
