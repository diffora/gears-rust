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
  AC #5 (category part), #6, #12, #35 (write-block clause), #38 (taxonomy-cycle row)
- [`./01-foundation.md`](./01-foundation.md) — the pipeline this slice registers into
- [`../DECISIONS.md`](../DECISIONS.md) P-D-04 (display names repeat), P-D-06 (metadata map
  placement — introduced by this slice)

### 1.5 Scope

**In**:
- category tree + its invariants and ops
- attribute definitions (typed, localized flag, brand/region visibility, deprecate-then-remove)
- attribute values + locale fallback resolution
- well-known seeds
- metadata map
- the governed-live-entity mutation pattern (op envelope handed to slice 05)
- the content-PII write-block **hook** (detector policy owned by slice 10).

**Out**:
- approval machinery itself (05)
- read-model/search projections and faceting (08)
- category read-model warming (08)
- erasure execution (10)
- `PlanTier` and recognized sets — also governed live entities, but owned by slice 03 which reuses this slice's pattern.

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Categories and attribute definitions are **live entities**: no draft state, no published version — in-place mutation under the slice-05 gate for material ops | PRD `fr-manage-taxonomy` + `fr-localized-attributes` |
| C2 | **Product/SKU** attribute values are entity content: they ride the owning entity's internal revision, freeze into its published versions, and never mutate a frozen version. **Category** values are live-entity content — categories have no revisions and no versions (H2 fix; see `inst-av-category-branch`). **P-D-50** gives the row a `mutation_seq` **act counter** for the live-value door's precondition: it is an act tag, not a revision — nothing freezes it, snapshots it or reads it as version content | PRD `fr-revision-vs-version` |
| C3 | Taxonomy limits (max depth, max children/node) are configured policies whose values PRD §7 `nfr-scale-extensibility` defers to the NFR workshop; §17.1 carries no interim default for them (§6) | PRD §7 `nfr-scale-extensibility` |
| C4 | The metadata map is ungoverned, size-bounded, non-localized, search-excluded, PII-prohibited | PRD glossary + `fr-localized-attributes` |
| C5 | No binary assets; `imageUri` and friends carry reference URIs only | PRD §5.2 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `GovernedLiveOp` | The pinned operation envelope for live-entity mutations: `(op kind, target, payload, expected target state)` — what slice 05 approves and what the apply step executes atomically |
| `TaxonomyWalk` | The in-transaction ancestor walk validating acyclicity + depth + children-per-node on create/re-parent |
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

Declared by [`../features/taxonomy-attributes.md`](../features/taxonomy-attributes.md) §2 as `cpt-cf-bss-products-flow-manage-taxonomy`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Authorize `category × write`; wrap the request as a `GovernedLiveOp`; **every one of these five taxonomy ops is material** (PRD `fr-materiality-gated-publish` enumerates category create/rename/re-parent/retire/delete), so the op queues through the slice-05 two-person gate before anything mutates - `inst-tx-governed-op`
2. [ ] - `p1` - On apply, re-validate against the **live** tree (the gate pinned the op, not the world): name uniqueness within the parent on `(tenant_id, parent_id, normalized(name))` — re-checked on rename **and** re-parent; violation fails `DUPLICATE_CATEGORY_NAME` - `inst-tx-name-in-parent`
3. [ ] - `p1` - `TaxonomyWalk` inside the write transaction, under the per-tenant taxonomy writer lock (§3.4): a re-parent whose new ancestor chain contains the node itself fails `TAXONOMY_CYCLE`; a create/re-parent exceeding configured max depth or max children fails `TAXONOMY_LIMIT` naming the limit - `inst-tx-walk`
4. [ ] - `p1` - Retire/delete **MUST** be refused while any **non-terminal** Product (`draft`/`published`/`deprecated` — the PRD's operand is "active", and `retired` *and* `discarded` are both terminal) references the category (primary or secondary) or any active child exists. **The guard reads the referencing Product's lifecycle state, never the presence of a `products_product_category` row** (item 17 of the review: discard releases the code and name reservations but leaves the category link, so on the old "non-`retired`" operand one discarded draft blocked the category permanently) — `CATEGORY_REFERENCED`, with a sample of holders named; retire marks the node closed to new assignment, delete is admitted only on a retired, empty, unreferenced node - `inst-tx-retire-guard`
5. [ ] - `p1` - Each applied op emits its event (`CategoryCreated`/`CategoryRenamed`/`CategoryReparented`/`CategoryRetired`/`CategoryDeleted`) in the same transaction (**P-D-21**: the event is the success-path audit record); the op envelope id rides the event for approval traceability - `inst-tx-event`

### Assign categories to a Product

Declared by [`../features/taxonomy-attributes.md`](../features/taxonomy-attributes.md) §2 as `cpt-cf-bss-products-flow-assign-categories`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - A Product carries **at most one primary + zero or more secondary** categories (the primary becomes required at publish); assignment is ordinary draft content (rides `inst-fd-save-txn`); this slice registers the validators: target category exists, is not retired (`CATEGORY_RETIRED`), duplicates between primary/secondary rejected - `inst-tx-assign`
2. [ ] - `p1` - At publish, the registered `→ published` validator requires the primary category present (`PRIMARY_CATEGORY_REQUIRED` — its own code, declared by this slice per 01 §3.3's code → declaring-slice rule (**P-D-36**), L8 fix; the PRD's "optional at draft, required at publish") - `inst-tx-primary-at-publish`

### Manage attribute definitions (governed live entities)

Declared by [`../features/taxonomy-attributes.md`](../features/taxonomy-attributes.md) §2 as `cpt-cf-bss-products-flow-attribute-definitions`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - A definition = `(key, value type, localized?, brand/region visibility scope, state)`; create and **material** changes (type change, visibility narrowing, deprecation) ride `GovernedLiveOp` through the slice-05 gate under grant **`attribute_definition × write`** (05's catalog); display-label edits are non-material ops, whose effective count is `min(N, 1)` per the §17.1 interim materiality default - `inst-ad-governed`
2. [ ] - `p1` - Changes **MUST** be backward-compatible: a type change on a definition with live values is refused (`DEFINITION_IN_USE`) — the path is deprecate-then-remove: `deprecated` blocks new values, removal is never admitted for a seeded definition (§4.2), and otherwise is admitted once no **non-terminal head** (`draft`/`published`/`deprecated` Product or SKU, active category) carries a value — **and a removal is the definition's `removed` state, never a DELETE** (**P-D-47**, the rule 03 §3.1 states for every `RecognizedSet`): the row survives as a tombstone outside the set, so a value on a terminal head keeps resolving and no `products_attribute_value` row is ever orphaned, and `removed → active` (as `deprecated → active`) re-lists the same identity through the same `GovernedLiveOp`, the identity never having changed — frozen versions are **self-contained copies**: they stay renderable after removal, and they neither block it nor are touched by it (operand narrowed — M2 fix; the PRD attribution it carried is struck, §6 — and it is **not** uniform with 03 `inst-rs-removal-operand`, whose operand is non-terminal *published* heads: 03 §6 registers the divergence) - `inst-ad-deprecate-then-remove`
3. [ ] - `p1` - Every applied change emits `AttributeDefinitionUpdated` (P-D-21: the event is the success-path audit record) - `inst-ad-event`

### Author localized attribute values

Declared by [`../features/taxonomy-attributes.md`](../features/taxonomy-attributes.md) §2 as `cpt-cf-bss-products-flow-attribute-values`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - **Product/SKU** values are entity content (C2): writes ride the entity draft-save door; this slice registers validators — definition exists and is not `deprecated` (`ATTRIBUTE_DEFINITION_UNKNOWN`/`_DEPRECATED`), value matches the declared type (`ATTRIBUTE_TYPE_MISMATCH`), `(locale, region, brand)` coordinates lie within the definition's visibility scope **and** the entity's own scope (`ATTRIBUTE_SCOPE_VIOLATION`) - `inst-av-validate`
2. [ ] - `p1` - The **content-PII write block** runs here for attribute/description free text: hard prohibition, fail-closed on uncertainty, curated allow-list for legitimate person-named products; the detector policy + allow-list are slice 10's, this door only invokes them (`CONTENT_PII_BLOCKED`) - `inst-av-pii-block`
3. [ ] - `p1` - **The same block runs on every operator free-text `reason`** (item 24 of the review), enumerated so no door is left out: audit rows (01 §4), approval rejections and break-glass session reasons (05), correction-override, break-glass-correction and producer-retirement reasons (07), the retirement reason carried into the `SkuRetired` broker payload (owned by slice 04 `inst-rt-initiate`; PRD §12 only restates it), and — struck by **P-D-50** — "bulk/promotion row reasons (09)", which are not 09's: its batch reason lives on 05's `ApprovalRecord` and its mass-retire reason on 04's `inst-rt-initiate`, both already enumerated here, and its only other stored reason is the literal `batch-abandoned` constant. These records are **never edited** and erasure is a map-only tombstone (10 C1), so PII typed into one of them is unreachable by erasure **forever** and, for `SkuRetired`, has already left the gear. Fail-closed at the door is therefore the only reach erasure can have over them — the same detector, the same allow-list, the same `CONTENT_PII_BLOCKED`, invoked by the owning door. **The hook is the single raiser and this slice is the single declaration**, which is what 01 §3.3 records — a code raised outside the pipeline needs no special status and gets none. **What is still owed** (third review pass, same day): of the doors enumerated above 01 and 04 (`inst-rt-initiate`) cite the hook and the code at the door; 05 and 07 wired theirs at the door under **P-D-50**, so every enumerated door now cites the block and none carries it as an owed item. A slice that adds a free-text `reason` field adds itself to the enumeration above; that is the whole registration - `inst-av-pii-reason`
4. [ ] - `p1` - The registered `→ published` validator requires the **default-locale value at the global (brand-less) coordinate** for every localized definition the entity carries values for (rejected at publish, not at draft save); per-brand default-locale values are optional overrides — the global one is what makes the fallback chain total for **every** brand (M5 fix) - `inst-av-default-locale`
5. [ ] - `p1` - Read-side resolution (consumed by slice 08): `LocaleResolver` walks `(locale, region, brand) → (locale, brand) → (default-locale, brand) → global`; default-locale resolves per brand, falling back to the tenant default — the chain is total for every brand by step 4's **global** default-locale guarantee. **Totality is anchored on the resolution path, not on the config value** (item 37 of the review): the tenant default locale is ungoverned config with no re-validation, so anchoring on it would un-total the chain for every already-published entity the moment it changed. So the final step is the **global** fallback and the tenant default is only a *preference* consulted before it; a tenant-default change is therefore non-retroactive by construction — the same posture `inst-ti-limits` states for depth limits - `inst-av-resolve`
6. [ ] - `p1` - **Category branch (H2 fix)**: categories have no revisions or publishes, so their display values are **live-entity content**: written through a category live-value door (`If-Match` on **`products_category.mutation_seq`**, the row's act counter, and a mismatch raises **`STALE_CATEGORY_TOKEN`** (409) — **P-D-50**, this slice's own code: `STALE_REVISION` is 01's entity-head code and `STALE_LIVE_OP` is the `GovernedLiveOp` envelope's, neither of which this door's precondition is; non-material, effective count `min(N, 1)` per the §17.1 default — a display edit is not a rename of the canonical name), emitting `CategoryDisplayUpdated` in the same transaction (**P-D-21**: the event is the success-path audit record); the **global default-locale value is required at the first write** of a definition for that category (the write-time analogue of step 4); a `CatalogVersion` captures current category values as of its snapshot instant — they have no frozen versions of their own - `inst-av-category-branch`

### Write the metadata map

Declared by [`../features/taxonomy-attributes.md`](../features/taxonomy-attributes.md) §2 as `cpt-cf-bss-products-flow-metadata`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

*Door: `PATCH /bss-products/v1/{products|skus}/{id}/metadata`, grant **`metadata × write`** (05's
catalog — named here; the flow had named neither a path nor a pair, so slice 12's
lint 3 could not see it).*

1. [ ] - `p2` - Per-entity string→string map; size-bounded (configured caps on key count, key and value byte length — `METADATA_LIMIT`); non-localized; PII-prohibited (the same `inst-av-pii-block` hook, no carve-out); excluded from read-model search by construction (08 never projects it into any searchable field; it is retrievable on 08's single-entity read only). **The PATCH is a per-key merge and a `null` value removes the key** (**P-D-50**): absent keys are untouched, so a map standing at the configured cap has an exit, which it did not before - `inst-md-write`
2. [ ] - `p2` - **Placement (P-D-06)**: the map lives **beside** the entity, outside the frozen published-version content — mutable in place on any non-terminal entity without a version bump, emitting `MetadataUpdated` in the same transaction (**P-D-21**: the event is the success-path audit record); a `CatalogVersion` captures the map **as of its own snapshot instant**, and that copy is frozen with the snapshot (old snapshots never move — byte-identity holds); a write to a **terminal** entity is refused **`ENTITY_TERMINAL`** (**P-D-50** — the code stays 01's and is raised here: P-D-06 puts the map outside the head's *version content*, which governs what a snapshot freezes and not what the terminal guard refuses, and P-D-32 already widened the code to any head write on a `retired`/`discarded` row) - `inst-md-placement`

## 3. Processes / Business Logic

### 3.1 The governed-live-entity pattern

Declared by [`../features/taxonomy-attributes.md`](../features/taxonomy-attributes.md) §3 as `cpt-cf-bss-products-algo-governed-live`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - A `GovernedLiveOp` pins the **operation** (kind + target + payload + the target's expected current state), not an entity revision; slice 05 approves the envelope; the apply step re-validates the expected state against the live row and fails `STALE_LIVE_OP` if the world moved (the live-entity analogue of the Foundation's pinned-revision publish) - `inst-gl-envelope`
2. [ ] - `p1` - Apply is atomic: mutation + event in one transaction (P-D-21); there is no partially-applied taxonomy op - `inst-gl-atomic`
3. [ ] - `p1` - The pattern is exported: slice 03 reuses it verbatim for `PlanTier` and the recognized code/unit sets - `inst-gl-export`

### 3.2 Taxonomy integrity mechanics

Declared by [`../features/taxonomy-attributes.md`](../features/taxonomy-attributes.md) §3 as `cpt-cf-bss-products-algo-taxonomy-integrity`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Acyclicity is validated by `TaxonomyWalk` (ancestor chain of the new parent must not contain the node) executed inside the write transaction; correctness rests on §3.4's single-writer discipline, not on the walk alone — two concurrent re-parents could otherwise each pass and jointly close a cycle - `inst-ti-acyclic`
2. [ ] - `p1` - Uniqueness-in-parent is also an index (`UNIQUE (tenant_id, parent_id, name_normalized)`, §4.1), so the read-then-write race is decided by the store exactly as the Foundation's `ReservationIndex` decides `skuCode` - `inst-ti-unique-index`
3. [ ] - `p2` - Depth/children limits are validated on the mutation path only — a later limit **decrease** never invalidates existing structure (config change is not retroactive; the lint reports over-limit subtrees informationally) - `inst-ti-limits`

### 3.3 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-taxonomy-errors`

`DUPLICATE_CATEGORY_NAME`, `TAXONOMY_CYCLE`, `TAXONOMY_LIMIT`, `CATEGORY_REFERENCED`,
`CATEGORY_RETIRED` (assignment to a retired node), `ATTRIBUTE_DEFINITION_UNKNOWN`,
`ATTRIBUTE_DEFINITION_DEPRECATED`, `DEFINITION_IN_USE`, `ATTRIBUTE_TYPE_MISMATCH` (raised by `inst-av-validate`, the attribute-value write, when a value does not match its definition's declared type — named),
`ATTRIBUTE_SCOPE_VIOLATION`, `DEFAULT_LOCALE_MISSING` (publish-time for Product/SKU; at the first category display-value write per `inst-av-category-branch`), `PRIMARY_CATEGORY_REQUIRED`, `STALE_CATEGORY_TOKEN` (**P-D-50** — the category live-value door's precondition mismatch), `CONTENT_PII_BLOCKED` (verdict policy owned by slice 10 — L1),
`METADATA_LIMIT`, `STALE_LIVE_OP`. Registered into the Foundation taxonomy (01 §3.3); the
AC #38 row "taxonomy cycle" maps here; the PII write-block is **AC #35's** clause (L1 fix —
misattributed to #38 until the review).

**Problem responses (RFC 9457):** `DUPLICATE_CATEGORY_NAME`, `CATEGORY_REFERENCED`, `DEFINITION_IN_USE`, `STALE_LIVE_OP` (409); `TAXONOMY_CYCLE`, `TAXONOMY_LIMIT`, `CATEGORY_RETIRED`, `ATTRIBUTE_DEFINITION_UNKNOWN`, `ATTRIBUTE_DEFINITION_DEPRECATED`, `ATTRIBUTE_TYPE_MISMATCH`, `ATTRIBUTE_SCOPE_VIOLATION`, `DEFAULT_LOCALE_MISSING`, `PRIMARY_CATEGORY_REQUIRED`, `CONTENT_PII_BLOCKED`, `METADATA_LIMIT` (422 architectural — each reaches the wire as 400; see the note below); `STALE_CATEGORY_TOKEN` (409).

*Statuses added, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"*) — **403** where the caller may not
perform the act at all, **404** only where a path segment names a resource this tenant has none
of. **503** where retry is the remedy is this gear's own addition — pricing's set carries no 503
at all, so that one
class is not "checked against it". **The 422s here are architectural, not wire** — see 01 §3.3, which quotes the sibling
plan-price gear's rule (the `MUST NOT` being this gear's own choice, 01 §3.3): no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for an error **carrying a registry code** in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.*

### 3.4 Concurrency

Declared by [`../features/taxonomy-attributes.md`](../features/taxonomy-attributes.md) §3 as `cpt-cf-bss-products-algo-taxonomy-concurrency`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Taxonomy mutations serialize **per tenant** behind a taxonomy writer lock (advisory lock on Postgres, the write transaction on SQLite): taxonomy ops are rare and human-paced, and single-writer is what makes `TaxonomyWalk`'s verdict trustworthy - `inst-tc-writer-lock`
2. [ ] - `p1` - **Product/SKU** attribute-value and metadata writes need no extra machinery: they ride the entity row's `If-Match` (01); the category live-value door carries its own token — `products_category.mutation_seq`, refusing a mismatch `STALE_CATEGORY_TOKEN` (**P-D-50**, `inst-av-category-branch`) - `inst-tc-etag`

## 4. Data / Storage (normative shape; DDL in migrations)

### 4.1 Tables

- **`products_category`** — `category_id` (PK, uuid) · `tenant_id` · `parent_id` (nullable FK,
  self) · `name` / `name_normalized` (the operand 01 §4.1 pins — NFKC → full casefold →
  trim + collapse, computed application-side) · `state` (`active|retired`) · **`mutation_seq`**
  (bigint, the live-value door's `If-Match` operand — **P-D-50**: the door demanded a token and
  this roster carried none. It counts **acts, not row writes**, following the donor's
  `pricing_price_window.mutation_seq` (D-190/D-191): the category door spends a `GovernedLiveOp`,
  and an approval subject built from an act identity has to render the same subject on the
  approved retry, which a counter advanced by non-operator writes would break) · timestamps. Indexes:
  `UNIQUE (tenant_id, parent_id, name_normalized)`; FK children guard on delete. Deletion is
  physical **only** through `inst-tx-retire-guard` (retired + empty + unreferenced); everything
  else is state flips, audited.
- **`products_product_category`** — the **single source of truth** for category assignments
  (01 §4.1 carries no inline category columns) — `(tenant_id, product_id, category_id, role)` with
  `role ∈ {primary, secondary}`; `UNIQUE (tenant_id, product_id, category_id)`; partial
  `UNIQUE (tenant_id, product_id) WHERE role = 'primary'` — at-most-one-primary is an index, not a
  convention; the *required* half is the publish validator of `inst-tx-primary-at-publish`, a draft
  carrying none being legal (AC #5's "optional at draft").
- **`products_attribute_definition`** — `definition_id` (PK) · `tenant_id` · `key` (unique per
  tenant) · `value_type` · `localized` (bool) · visibility scope (brand/region sets) · `state`
  (`active|deprecated|removed` — a removal is a state flip to the tombstone, never a DELETE, **P-D-47**) · `seeded_by` (nullable — well-known marker) · timestamps.
- **`products_attribute_value`** — owned entity coordinates `(tenant_id, entity_kind,
  entity_id)` + `definition_id` + locale coordinates `(locale?, region?, brand?)` + `value`;
  `UNIQUE` over the full coordinate tuple. For Product/SKU rows: at publish the values are **copied into the frozen
  `products_entity_version` content** (01 §4.3) — the table always holds the current head
  state, history lives in the version rows. For **category** rows the table IS the live state —
  no freeze-copy (H2 fix).
- **`products_metadata`** — `(tenant_id, entity_kind, entity_id, key)` PK · `value` ·
  timestamps; caps enforced at the door (`METADATA_LIMIT`). Outside version content (P-D-06).

*Inside a frozen entity version, this slice's two row collections — the category-assignment set and
the attribute-value set — are rendered as **JSON arrays sorted by the collection's own identifier**
(the category id, the attribute id), each element by 01 §4.3's field rule (**P-D-29**). 01's rule
orders fields, not rows; without this both engines could serialize one content in two orders and
10's restore drill compares those digests byte-for-byte.*

### 4.2 Well-known seeds (`WellKnownSeed`)

Seeded by migration, per tenant bootstrap, marked `seeded_by = 'registry'` (a seeded definition
is deprecatable but not removable): `displayName` (localized, per Product/SKU/Category),
`description` (localized), `imageUri` (URI string, non-localized), `unitDisplayLabel`
(localized — the sales-facing unit label, display only, never the metering-unit identity),
`marketingFeatures` (localized string list). PRD `fr-localized-attributes` + the industry-parity widening.

### 4.3 Events

`CategoryCreated` / `CategoryRenamed` / `CategoryReparented` / `CategoryRetired` /
`CategoryDeleted`, `CategoryDisplayUpdated`, `AttributeDefinitionUpdated`, `MetadataUpdated` — broker-native envelope,
ordering key `(tenant, category tree)` for taxonomy (one aggregate: the tree, matching the
single-writer discipline) and `(tenant, entity)` for metadata. **Product/SKU** attribute-value writes emit
no event of their own (category display values emit `CategoryDisplayUpdated`, above): they are entity content and ride `ProductHeadSaved`/`SkuHeadSaved` (**P-D-27**: a save lands on the head in every non-terminal state, not only on a draft)
(explicit "no event" record per 01 §4.5 rule).

## 5. Testing posture (slice-local)

- Cycle probe: two concurrent re-parents that would jointly close a cycle — one must fail; the
  writer lock is the mechanism under test, not the walk.
- At-most-one-primary and uniqueness-in-parent probed on the index (concurrent inserts), with
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
  carries a value") and would have gone **green on the defect** (item 13 of the review; the sibling probe in slice 03 was swept at the time, this one was not).

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-fr-manage-taxonomy`, `cpt-cf-bss-products-fr-create-product` (category clauses),
`cpt-cf-bss-products-fr-localized-attributes`, `cpt-cf-bss-products-fr-retention-erasure` (write-block clause, hook only); AC #5
(category part), #6, #12, #35 (write-block), #38 (taxonomy-cycle row); **NFR #6** `cpt-cf-bss-products-nfr-scale-extensibility` (the extensibility-limits half: max taxonomy depth, max children/node).

**Risks & open items**:
- **Definition removal candidates**: the guard reads **non-terminal heads** only (M2 —
  no frozen-content scan is involved, and the earlier wording of this item described exactly the
  scan M2 removed), so it is an index-scale check; the open item that survives is
  presentational — the lint should surface removable definitions rather than operators
  discovering the guard by refusal.
- The PII detector's false-positive posture (fail-closed on uncertainty) will generate operator
  friction; the allow-list governance loop (slice 10 + Legal sign-off, PRD AC #35) must exist
  before GA, not after the first blocked legitimate product name.
- **The taxonomy and metadata limits have no interim default anywhere.** C3 pointed at `PRD` §17.1
  (corrected this pass — that table has no such row), and `nfr-scale-extensibility` defers the values
  to the NFR workshop. Four rules read them: `inst-tx-walk`, `inst-ti-limits`, `inst-md-write`, and
  08's bounded subtree recompute. Owner: the §17.1 policy owner — a taxonomy-limits row and a
  metadata-caps row. **NFR #6's third limit, `max attributes/entity`, is claimed by no slice at
  all** — 01 takes the entity-count half, 06 the `CatalogVersion`-growth half, and the trace above
  claims depth and children/node only, so no rule reads it and §3.3 declares no code for it.
  *(Two lenses raised it independently.)*
- **What is the `global` coordinate's key?** `inst-av-default-locale` requires "the default-locale
  value at the global (brand-less) coordinate", while the same rule argues totality must **not**
  anchor on the tenant default because that config can change under published entities. If the row
  is keyed on the default locale it is anchored on exactly that value; if `global` means
  `(locale NULL, region NULL, brand NULL)` — which §4.1's `(locale?, region?, brand?)` admits — then
  the phrase names the wrong coordinate. Owner: this slice. *(Two lenses raised it independently.)*
- **The coordinate model admits combinations the resolver never visits.** Eight presence
  combinations are storable and the chain walks four steps: a value at `(locale, region, brand
  absent)` is unreachable to any branded reader, and a value with no locale is matched by no step.
  Separately "default-locale resolves per brand" presumes a per-brand default locale, while the only
  store named is the tenant default. Owner: this slice — the admitted coordinate roster per
  `localized` flag, the refusal outside it, and where a brand-scoped default lives. *(Raised by the slice-02 first lens pass.)*
- **Does a brand-less global value survive the scope check on a brand-scoped entity?**
  `inst-av-validate` requires coordinates within the entity's own scope; under the gear's stated
  containment reading an unrestricted coordinate under a restricted entity is **not** contained — so
  the write the publish validator demands is the write the save validator refuses. P-D-39's
  propagation named 01 and 04 only, and §4.1 gives the definition's visibility scope neither
  nullability nor an empty-set meaning. Owner: this slice with 05, whose `inst-gv-scope` reads the
  same rule. *(Raised by the slice-02 first lens pass.)*
- **Both uniqueness guarantees are UNIQUE over nullable columns.** `(tenant_id, parent_id,
  name_normalized)` over a nullable `parent_id` does not constrain **root** categories, and the
  attribute-value coordinate tuple does not constrain the **global** coordinate — the one
  `inst-av-default-locale` makes mandatory — because both engines treat NULLs as distinct. The
  gear's answer elsewhere is a NOT NULL column with a stated absence value (P-D-39). Owner: this
  slice with the schema owner — sentinels, `NULLS NOT DISTINCT`, or extra partial indexes. *(Raised by the slice-02 first lens pass.)*
- **The frozen-content sort key is not total for attribute values.** §4.1's ordering note sorts by
  "the attribute id", while row identity is the full coordinate tuple — so the key orders groups,
  not rows, and two engines can serialize one content two ways, which is the failure the note exists
  to prevent. Amending it is a register change **and a 01 change**: **P-D-29** and 01 §4.3 state
  the same rule in the same words. Owner: P-D-29's owner. *(Raised by the slice-02 first lens pass.)*
- **What does a category rename or delete do to entity versions already frozen against it?** The
  frozen assignment set holds category **ids**, not copies, so a delete leaves an id resolving to
  nothing and a rename silently changes what an old version renders. The sibling case is answered
  explicitly for attribute definitions ("frozen versions are self-contained copies"); for categories
  it is not. Owner: this slice with 06 and 08 — copy the name into the frozen set, or tombstone
  category rows. *(Raised by the slice-02 first lens pass.)*
- **Is definition removal a material op?** Removal is absent from `inst-ad-governed`'s material-op
  enumeration while deprecation, the step before it, is in it. *(The other half of this item —
  physical DELETE or a third state — is closed by **P-D-47**: a removal is the `removed` state, §4.1.)*
  Owner: this slice. *(Raised by the slice-02 first lens pass.)*
- **Where does a definition's display label live?** Label edits are a named non-material op, and
  §4.1's definition roster carries no label column, while the attribute-value table's `entity_kind`
  does not admit a definition as a value-bearing entity — so the op has no target. 03 solves the
  sibling case with a `display_label?` column on its own table. Owner: this slice. *(Raised by the slice-02 first lens pass.)*
- **Two concurrent metadata writes both pass their precondition.** Metadata writes ride the entity
  row's `If-Match` and, by P-D-06, bump no version — so the token never moves, both writers pass and
  the second silently overwrites the first, on a map that keeps no history between snapshots. Owner:
  this slice — does the metadata PATCH bump `internal_revision` (and what does that do to P-D-06's
  "no version bump"), or carry its own per-map token? *(Raised by the slice-02 first lens pass.)*
- **Which aggregate orders `CategoryDisplayUpdated` and `AttributeDefinitionUpdated`?** §4.3 gives
  ordering keys for taxonomy ops and for metadata, and neither of these two falls under either.
  It is not a free choice: display writes do not take the taxonomy writer lock, so putting them on
  the tree key claims a serialization the door does not provide, while a per-category key leaves a
  rename and a display edit on one category mutually unordered. Owner: this slice with 12.
  *(Two lenses raised it independently.)*
- **Three doors name no REST path, and one names no grant pair.** The taxonomy-op door, the
  attribute-definition door and the category live-value door all go unnamed, against this slice's
  own precedent — the metadata door
  carries its path and pair explicitly because without them 12's lint could not see it. The pair is
  a real choice, not an omission: 05 minted a separate `metadata × write` precisely because that map
  is mutable on a published entity, and the category display value has the same property. Owner:
  this slice with 05. *(Two lenses raised it independently.)*
- **Four refusals in this slice have no code.** "target category exists" and "duplicates between
  primary/secondary" in `inst-tx-assign`, the seeded-definition removal refusal, and the removal
  refusal on a non-terminal head carrying a value all lack one (the type-change arm of that same
  row already carries `DEFINITION_IN_USE`), and none appears in AC #38's enumeration, so no lint
  will report them. Owner: this slice with the error-contract owner — slice-owned codes, 01's
  `VALIDATION`, or a 404 path-segment case. *(Two lenses raised it independently.)*
- **Are `CATEGORY_RETIRED` and `ATTRIBUTE_DEFINITION_DEPRECATED` 422 or 409?** Both are the target's
  *current state* refusing the act — the shape §3.3's own convention puts at 409, and where the
  sibling `DEFINITION_IN_USE` already sits. The note calls the mapping "Proposed per row and open to correction". Owner: the API-contract owner. *(Raised by the slice-02 first lens pass.)*
- **Does the type-change operand mean the same thing as the removal operand?** One row states two:
  the undefined "live values" for the type change and the defined "non-terminal head" for removal.
  The stated reasoning for the removal operand — frozen versions are self-contained copies — applies
  equally to a type change, but the file never says the operand was narrowed. Owner: this slice.
  *(Raised by the slice-02 first lens pass.)*
- **Does slice 09 have an operator free-text `reason` at all?** This slice's PII enumeration names
  "bulk/promotion row reasons (09)", while 09's only two `reason`s are a system-produced failure
  reason and a fixed literal — and 09's own owed item quotes this enumeration as its evidence, so
  nothing independent establishes the door. Slice 10 has an interest: its "only table in the gear where PII may live" guarantee rests on this enumeration being complete. Owner: 09's owner with this slice. *(Raised by the slice-02 first lens pass.)*
- **Who writes the well-known seeds for a tenant created after the migration?** §4.2 says "Seeded by migration, per tenant bootstrap" — two different code paths, and a migration cannot create rows
  for tenants that do not yet exist. The rows are load-bearing: the publish validator refuses every
  localized definition whose default-locale value is absent, and AC #12 requires the seeds. 03
  registers the identical question with no writer either. Owner: this slice with 01.
  *(Two lenses raised it independently.)*
- **Which of `inst-av-validate`'s validators run at the category live-value door?** Step 1 registers
  them on the entity draft-save door, and `inst-av-category-branch` writes through a different one.
  §3.3 shows one of them reaching it (`DEFAULT_LOCALE_MISSING`, "at the first category display-value
  write") and says nothing about the definition-exists, type and scope checks — so a category value
  against a `deprecated` definition is admitted today, while `inst-ad-deprecate-then-remove` counts
  an active category as a value-carrying head. Owner: this slice with 01, whose registered-validator
  phase is keyed by `(entity kind, transition, target state, field set)`, which this door does not
  pass through. *(Two lenses raised it independently.)*
- **What `entity_kind` values does each table admit, and does a definition scope to entity kinds?**
  §4.1 gives `products_attribute_value` and `products_metadata` an `entity_kind` column and the set
  enumerates its values nowhere; the attribute-value table demonstrably admits `category`, while the
  only metadata door named admits `{products|skus}`. §4.2's `displayName` is "per Product/SKU/Category"
  while `key` is unique per tenant, so that parenthetical is either an applicability set the definition
  roster carries no column for, or a constraint on nothing. Owner: this slice — the admitted roster per
  table, and whether a category may carry a metadata map. *(Two lenses raised it independently.)*
- **What happens to `products_product_category` rows when a category is physically deleted?**
  `inst-tx-retire-guard` admits a physical delete on a retired, empty, unreferenced node and reads
  "unreferenced" from the referencing Product's lifecycle state, never from the link row — so discarded
  and retired Products still hold rows in the table §4.1 calls the single source of truth. §4.1 states
  an FK guard for `parent_id` only and gives `products_product_category.category_id` no referential
  action. Owner: this slice with the schema owner — the action on delete, or a narrowing of
  "unreferenced". *(Raised by the slice-02 second lens pass.)*
- **Does the PRD carry a live-reference condition for attribute definitions?**
  `inst-ad-deprecate-then-remove` credited its "non-terminal head" operand to the PRD, and that
  attribution is struck this pass: the PRD's definition clauses (`fr-localized-attributes`, AC #12)
  carry only "backward-compatible … deprecate-then-remove", and §17.1's "de-list blocked while
  referenced" governs the Finance recognized sets, which are 03's. The operand is therefore either
  inherited from 03's uniform removal rule or design-introduced and owed a PRD amendment.
  Owner: the PRD owner with this slice. *(Raised by the slice-02 second lens pass.)*
