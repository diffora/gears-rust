# Feature: Taxonomy & Attributes

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-taxonomy-attributes-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-taxonomy-attributes`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Manage the taxonomy](#manage-the-taxonomy)
  - [Assign categories to a Product](#assign-categories-to-a-product)
  - [Manage attribute definitions](#manage-attribute-definitions)
  - [Author localized attribute values](#author-localized-attribute-values)
  - [Write the metadata map](#write-the-metadata-map)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [The governed-live-entity pattern](#the-governed-live-entity-pattern)
  - [Taxonomy integrity mechanics](#taxonomy-integrity-mechanics)
  - [Concurrency](#concurrency)
  - [Error taxonomy](#error-taxonomy)
- [4. States (CDSL)](#4-states-cdsl)
  - [Category State Machine](#category-state-machine)
  - [Attribute Definition State Machine](#attribute-definition-state-machine)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Category table and its guards](#category-table-and-its-guards)
  - [Category assignment table](#category-assignment-table)
  - [Attribute definition table](#attribute-definition-table)
  - [Attribute value table](#attribute-value-table)
  - [Metadata table](#metadata-table)
  - [Well-known seeds](#well-known-seeds)
  - [Governed live op envelope](#governed-live-op-envelope)
  - [Taxonomy writer lock](#taxonomy-writer-lock)
  - [Taxonomy walk and its limits](#taxonomy-walk-and-its-limits)
  - [Name uniqueness within a parent](#name-uniqueness-within-a-parent)
  - [Retire and delete guard](#retire-and-delete-guard)
  - [Category assignment validators](#category-assignment-validators)
  - [Primary category at publish](#primary-category-at-publish)
  - [Definition lifecycle](#definition-lifecycle)
  - [Attribute value validators](#attribute-value-validators)
  - [Default locale at the global coordinate](#default-locale-at-the-global-coordinate)
  - [Locale resolver](#locale-resolver)
  - [Category live value door](#category-live-value-door)
  - [Content PII write block](#content-pii-write-block)
  - [Metadata door](#metadata-door)
  - [Error taxonomy registration](#error-taxonomy-registration)
  - [Taxonomy events](#taxonomy-events)
  - [Version content rendering](#version-content-rendering)
  - [Metadata outside version content](#metadata-outside-version-content)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

This feature owns the two **describing** surfaces of the registry: the **Category taxonomy** — a
hierarchical, cycle-free browse and curation backbone — and the **attribute system**: governed
attribute **definitions**, localized attribute **values** with a brand and region fallback chain,
the seeded **well-known display attributes**, and the ungoverned **metadata map**.

It introduces the pattern the PRD calls a **governed live entity**: an object that is *not*
draft/publish-versioned — no lifecycle state machine of the Foundation's kind, no published
version — but whose material mutations still pass the `05-governance` two-person gate and land
atomically, in place, audited and evented. `03-sku-classification` reuses that pattern verbatim
for `PlanTier` and the recognized code and unit sets.

### 1.2 Purpose

Give Products and SKUs their classification and their sales-facing description without ever
minting a second identity. The taxonomy is for browse and curation and never for pricing;
Product and SKU attribute values are entity content riding the Foundation's revisions; and
display names repeat freely precisely because the canonical internal name is a quasi-code
(P-D-04).

This feature owns **no approval machinery** — it hands `05-governance` an operation envelope and
spends what that feature approves. It owns **no PII detector** — it places the write-block hook
and invokes the policy `10-retention-erasure` owns.

### 1.3 Actors

| Actor | Role in this feature |
|-------|----------------------|
| `cpt-cf-bss-products-actor-catalog-admin` | Taxonomy ops (create, rename, re-parent, retire, delete) and the attribute-definition lifecycle |
| `cpt-cf-bss-products-actor-product-manager` | Authors attribute values, assigns categories, writes the metadata map |
| `cpt-cf-bss-products-actor-presentation` | Consumes localized resolution and category filters through the read models (`08-read-models`) |

### 1.4 References

- [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.2 — the entry this feature realizes
- [`../design/02-taxonomy-attributes.md`](../design/02-taxonomy-attributes.md) — the design slice.
  **This document is the declaration site of the five `flow-` ids and the three `algo-` ids**, and
  the slice's §2 and §3 point here for them; there is one definition site per id. **The slice's
  step lists remain the normative ones and are not copied here**: re-spelling the 26 instruction
  steps it owns would fork the set's own instruction register and leave two texts where only one
  can be true. §2 and §3 below therefore carry the actor, the scenarios and the boundary of each
  flow, and the steps stay at their single source.
  - **The one exception is §4.** A state machine's transitions are a template-required
    id-bearing list, and the slice expresses this content inside `inst-tx-retire-guard` and
    `inst-ad-deprecate-then-remove` rather than as rows, so neither can be reused per row. §4's
    `inst-ce-*` and `inst-de-*` ids are that rendering and nothing more; they add no rule the
    slice does not already state.
  - **The error taxonomy's `contract-` id is deliberately not cited as a token.** A FEATURE
    artifact may define only `flow`, `algo`, `state`, `dod` and `featstatus` ids, and that id's
    only definition site is a design slice, which `artifacts.toml` excludes from autodetection —
    so `cfs` resolves it nowhere and a citation would be a dangling reference rather than a
    trace. Fourteen `contract-` ids in this set are in that position, re-measured against
    `design/` on 2026-08-31: nineteen distinct ids exist there, five of which also resolve in the
    PRD or the DECOMPOSITION.
  - **Four `inst-*` ids this slice cites are owned elsewhere** and are referenced, never claimed:
    `inst-fd-save-txn` (`01-foundation`), `inst-rs-removal-operand` (`03-sku-classification`),
    `inst-rt-initiate` (`04-lifecycle`), `inst-gv-scope` (`05-governance`).
- [`../PRD.md`](../PRD.md) §6.2 (`fr-manage-taxonomy`, the category half of `fr-create-product`),
  §6.4 (`fr-localized-attributes`), §6.11 (the content-PII write block this feature hosts); §7
  `nfr-scale-extensibility`; §12 AC #5 (category part), #6, #12, #35 (write-block clause), #38
  (taxonomy-cycle row)
- [`../DESIGN.md`](../DESIGN.md) §1.3 layering, §2.1 principles, §2.2 constraints
- [`../DECISIONS.md`](../DECISIONS.md) — P-D-04, P-D-06, P-D-21, P-D-27, P-D-29, P-D-32, P-D-36,
  P-D-39, P-D-47, P-D-50
- [`./foundation.md`](./foundation.md) — the doors, the validation pipeline and the outbox this
  feature registers into

## 2. Actor Flows (CDSL)

**Use cases**: `cpt-cf-bss-products-usecase-product-sku-editor`

The step lists live in [`../design/02-taxonomy-attributes.md`](../design/02-taxonomy-attributes.md)
§2 — see §1.4 for why they are not repeated. Each flow below names its actor, what success and
failure look like, and where its boundary runs.

### Manage the taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-manage-taxonomy`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- A category is created, renamed, re-parented, retired or deleted only after the wrapping
  `GovernedLiveOp` clears the `05-governance` gate; **all five ops are material**, so none of
  them reaches the store un-approved
- The applied op emits its event — `CategoryCreated`, `CategoryRenamed`, `CategoryReparented`,
  `CategoryRetired` or `CategoryDeleted` — in the same transaction as the mutation, carrying the
  op envelope id for approval traceability
- A delete is admitted on a node that is retired, childless and unreferenced, and it is the only
  physical row removal this feature performs

**Error Scenarios**:
- A name collides within the parent on `(tenant_id, parent_id, name_normalized)` —
  `DUPLICATE_CATEGORY_NAME`; re-checked on rename **and** on re-parent, not only on create
- A re-parent whose new ancestor chain contains the node itself — `TAXONOMY_CYCLE`
- A create or re-parent exceeding the configured maximum depth or maximum children per node —
  `TAXONOMY_LIMIT`, naming which limit was exceeded
- A retire or delete while any **non-terminal** Product references the node, or while an active
  child exists — `CATEGORY_REFERENCED`, naming a sample of the holders
- The world moved between approval and apply — `STALE_LIVE_OP`

**Boundary**: this flow mutates the category tree and emits its event. It does **not** implement
the approval ceremony — it submits an envelope and `05-governance` decides. The referencing guard
reads the referencing Product's **lifecycle state**, never the mere presence of a
`products_product_category` row: a discarded draft leaves its category link behind, and on a
row-presence operand one discarded draft would block a category permanently.

### Assign categories to a Product

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-assign-categories`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- A Product carries at most one primary and zero or more secondary categories; the assignment is
  ordinary draft content and rides the Foundation's save transaction
- A draft carrying no primary category is legal and saves; the primary becomes required only at
  publish

**Error Scenarios**:
- The target category does not resolve in the tenant, or the same category is named both primary
  and secondary — refused by this feature's registered save validator
- The target category is retired — `CATEGORY_RETIRED`
- A publish with no primary category — `PRIMARY_CATEGORY_REQUIRED`, raised by this feature's
  registered `→ published` validator

**Boundary**: this flow writes no door of its own. It registers validators into the Foundation's
pipeline and its rows land inside the save door's transaction. `products_product_category` is the
**single source of truth** for assignments; the Foundation's entity tables carry no inline
category columns.

### Manage attribute definitions

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-attribute-definitions`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- A definition — key, value type, localized flag, brand and region visibility scope, state — is
  created, and material changes (type change, visibility narrowing, deprecation) clear the
  `05-governance` gate under the `attribute_definition × write` grant
- A display-label edit is non-material and its effective affected-entity count is `min(N, 1)`
- Deprecation blocks new values while leaving existing ones resolvable
- A removal is admitted once no **non-terminal head** carries a value, and is executed as a state
  flip to a tombstone, never a `DELETE`
- `removed → active` and `deprecated → active` re-list the same identity through the same
  `GovernedLiveOp`; the identity never changed
- Each applied change emits `AttributeDefinitionUpdated` in the same transaction

**Error Scenarios**:
- A type change on a definition that has live values — `DEFINITION_IN_USE`; the admitted path is
  deprecate-then-remove
- A removal of a definition seeded by the registry — refused; a seeded definition is deprecatable
  but never removable
- The world moved between approval and apply — `STALE_LIVE_OP`

**Boundary**: the removal operand is the **non-terminal head**. Frozen entity versions are
self-contained copies: they stay renderable after a removal, and they neither block one nor are
touched by one. This is deliberately **not** uniform with `03-sku-classification`'s
`inst-rs-removal-operand`, whose operand is non-terminal *published* heads; that slice's §6
registers the divergence.

### Author localized attribute values

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-attribute-values`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- A Product or SKU value is written as entity content through the Foundation's draft-save door and
  freezes into the entity's published versions
- A **category** display value is written through the category live-value door under an
  `If-Match` on `products_category.mutation_seq`, emitting `CategoryDisplayUpdated` in the same
  transaction; categories have no revisions and no versions, so the table itself is the live state
- Read-side resolution walks `(locale, region, brand) → (locale, brand) → (default-locale, brand)
  → global`, and the chain is **total for every brand** because the global default-locale value
  is guaranteed at publish

**Error Scenarios**:
- The definition does not resolve, or is deprecated — `ATTRIBUTE_DEFINITION_UNKNOWN`,
  `ATTRIBUTE_DEFINITION_DEPRECATED`
- The value does not match the definition's declared type — `ATTRIBUTE_TYPE_MISMATCH`
- The `(locale, region, brand)` coordinates fall outside the definition's visibility scope or the
  entity's own scope — `ATTRIBUTE_SCOPE_VIOLATION`
- A publish with a localized definition carrying no default-locale value at the **global**,
  brand-less coordinate — `DEFAULT_LOCALE_MISSING`; for a category, the same demand lands at the
  first display-value write rather than at a publish
- Free text carries prohibited personal data — `CONTENT_PII_BLOCKED`
- The category live-value door's token does not match — `STALE_CATEGORY_TOKEN`

**Boundary**: totality of the fallback chain is anchored on the **resolution path**, not on the
tenant default-locale config value. The tenant default is ungoverned config with no
re-validation; anchoring on it would un-total the chain for every already-published entity the
moment it changed. So the final step is the global fallback and the tenant default is only a
preference consulted before it, which makes a tenant-default change non-retroactive by
construction.

Product and SKU attribute-value writes emit **no event of their own** — they are entity content
and ride `ProductHeadSaved` and `SkuHeadSaved`. Category display values are the exception and
emit `CategoryDisplayUpdated`, because a category has no head to ride.

### Write the metadata map

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-metadata`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

*Door: `PATCH /bss-products/v1/{products|skus}/{id}/metadata`, grant `metadata × write`.*

**Success Scenarios**:
- A per-entity string-to-string map is merged per key; absent keys are untouched and a `null`
  value removes its key, so a map standing at the configured cap has an exit
- The write lands in place on any non-terminal entity with no version bump, emitting
  `MetadataUpdated` in the same transaction
- A `CatalogVersion` captures the map as of its own snapshot instant, and that copy freezes with
  the snapshot

**Error Scenarios**:
- A configured cap on key count, key byte length or value byte length is exceeded —
  `METADATA_LIMIT`
- Free text carries prohibited personal data — `CONTENT_PII_BLOCKED`, with no carve-out for the
  map
- A write to a **terminal** entity — `ENTITY_TERMINAL`, a Foundation-owned code raised here

**Boundary**: the map lives **beside** the entity, outside the frozen published-version content
(P-D-06). It is non-localized, ungoverned, and excluded from read-model search by construction —
`08-read-models` never projects it into any searchable field and exposes it on the single-entity
read only. `ENTITY_TERMINAL` stays the Foundation's code because P-D-06 places the map outside
the head's *version content*, which governs what a snapshot freezes and not what the terminal
guard refuses.

## 3. Processes / Business Logic (CDSL)

The step lists live in [`../design/02-taxonomy-attributes.md`](../design/02-taxonomy-attributes.md)
§3 — see §1.4. Each process below states its purpose and its boundary.

### The governed-live-entity pattern

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-governed-live`

A `GovernedLiveOp` pins the **operation** — kind, target, payload and the target's expected
current state — rather than an entity revision, because a live entity has no revision to pin.
`05-governance` approves the envelope; the apply step re-validates the expected state against the
live row and refuses `STALE_LIVE_OP` when the world moved. Apply is atomic: the mutation and its
event land in one transaction, so no taxonomy op is ever partially applied.

**Boundary**: this feature defines and exports the envelope; it does not approve one.
`03-sku-classification` reuses the pattern verbatim, which is why the envelope is a published
contract of this feature rather than an internal detail.

### Taxonomy integrity mechanics

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-taxonomy-integrity`

Acyclicity is validated by a `TaxonomyWalk` over the new parent's ancestor chain, executed inside
the write transaction. Uniqueness-in-parent is carried by a unique index rather than a read-then-
write check, so the store decides the race exactly as the Foundation's `ReservationIndex` decides
`skuCode`. Depth and children-per-node limits are validated on the mutation path only.

**Boundary**: the walk's correctness rests on the single-writer discipline below, not on the walk
alone — two concurrent re-parents could each pass and jointly close a cycle. A later limit
**decrease** never invalidates existing structure: a config change is not retroactive, and
over-limit subtrees are reported informationally rather than repaired.

### Concurrency

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-taxonomy-concurrency`

Taxonomy mutations serialize **per tenant** behind a taxonomy writer lock — an advisory lock on
Postgres, the write transaction on SQLite. Taxonomy ops are rare and human-paced, and
single-writer discipline is what makes the walk's verdict trustworthy.

Product and SKU attribute-value and metadata writes need no extra machinery: they ride the entity
row's `If-Match`. The category live-value door is the exception and carries its own token,
`products_category.mutation_seq`, which counts **acts, not row writes** — the door spends a
`GovernedLiveOp`, and an approval subject built from an act identity must render the same subject
on the approved retry, which a counter advanced by non-operator writes would break.

### Error taxonomy

This feature declares sixteen codes and registers them into the Foundation's taxonomy:
`DUPLICATE_CATEGORY_NAME`, `TAXONOMY_CYCLE`, `TAXONOMY_LIMIT`, `CATEGORY_REFERENCED`,
`CATEGORY_RETIRED`, `ATTRIBUTE_DEFINITION_UNKNOWN`, `ATTRIBUTE_DEFINITION_DEPRECATED`,
`DEFINITION_IN_USE`, `ATTRIBUTE_TYPE_MISMATCH`, `ATTRIBUTE_SCOPE_VIOLATION`,
`DEFAULT_LOCALE_MISSING`, `PRIMARY_CATEGORY_REQUIRED`, `STALE_CATEGORY_TOKEN`,
`CONTENT_PII_BLOCKED`, `METADATA_LIMIT` and `STALE_LIVE_OP`.

The taxonomy is specified by
[`../design/02-taxonomy-attributes.md`](../design/02-taxonomy-attributes.md) §3.3, including each
code's RFC 9457 problem-response status. Its `contract-` id is deliberately not cited as a token
here; see §1.4.

`CONTENT_PII_BLOCKED` is declared once, here, and raised by a single hook invoked from every door
that accepts operator free text — a code raised outside the pipeline needs no phase status and
gets none. `ENTITY_TERMINAL` and `STALE_REVISION` remain the Foundation's codes even where this
feature's doors raise them.

## 4. States (CDSL)

### Category State Machine

- [ ] `p1` - **ID**: `cpt-cf-bss-products-state-category`

A category is a governed live entity: it has no draft, no published version and no revision. The
two rows below are the template's id-bearing rendering of the slice's `inst-tx-retire-guard`; see
§1.4 for why they carry their own ids.

**States**: `active`, `retired`

**Initial State**: `active`

**Transitions**:
1. [ ] - `p1` - **FROM** `active` **TO** `retired` **WHEN** an approved `GovernedLiveOp` applies and no non-terminal Product references the node and no active child exists; the node is thereafter closed to new assignment - `inst-ce-edge-retire`
2. [ ] - `p1` - **NO EDGE** out of `retired` other than physical deletion, which is admitted only on a retired, childless, unreferenced node and is the single physical row removal this feature performs - `inst-ce-terminal`

**Declared absence**: the slice declares no `retired → active` edge, while the attribute-definition
machine below declares its `removed → active` and `deprecated → active` re-listings explicitly.
Whether a retired category may be re-listed is **open item 1** in §7 and is not decided here.

### Attribute Definition State Machine

- [ ] `p1` - **ID**: `cpt-cf-bss-products-state-attribute-definition`

The four rows below are the template's id-bearing rendering of the slice's
`inst-ad-deprecate-then-remove`; see §1.4.

**States**: `active`, `deprecated`, `removed`

**Initial State**: `active`

**Transitions**:
1. [ ] - `p1` - **FROM** `active` **TO** `deprecated` **WHEN** an approved material `GovernedLiveOp` applies; new values are thereafter refused and existing values keep resolving - `inst-de-edge-deprecate`
2. [ ] - `p1` - **FROM** `deprecated` **TO** `removed` **WHEN** no non-terminal head carries a value for the definition and the definition is not registry-seeded; the row survives as a tombstone outside the set, so no `products_attribute_value` row is ever orphaned and a value on a terminal head keeps resolving - `inst-de-edge-remove`
3. [ ] - `p1` - **FROM** `deprecated` **TO** `active` and **FROM** `removed` **TO** `active` **WHEN** an approved `GovernedLiveOp` re-lists the same identity, which never changed - `inst-de-edge-relist`
4. [ ] - `p1` - **NO DELETE** in any state: a removal is the `removed` state flip and never a row deletion (P-D-47) - `inst-de-no-delete`

## 5. Definitions of Done

Twenty-four, against the donor feature's nineteen. The count is stated rather than smoothed: this
feature carries five tables, sixteen codes, eight events and two doors of its own, where
`01-foundation` carried four tables and one door family. Each entry below is separately testable.

### Category table and its guards

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-category-table`

The system **MUST** create `products_category` on both engines with `category_id`, `tenant_id`,
`parent_id` as a nullable self-referencing foreign key, `name` and `name_normalized`, `state`,
`mutation_seq` and timestamps. `name_normalized` **MUST** use the Foundation's operand — NFKC,
then full casefold, then trim and collapse, computed application-side. The table **MUST** carry
`UNIQUE (tenant_id, parent_id, name_normalized)` and a foreign-key children guard on delete. A
schema-oracle golden **MUST** exist for both engines together with a perturbation case proving
the oracle can fail.

**Implements**: `cpt-cf-bss-products-flow-manage-taxonomy`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_category`
- Entities: `Category`

### Category assignment table

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-category-assignment-table`

The system **MUST** create `products_product_category` as the single source of truth for
assignments, keyed `(tenant_id, product_id, category_id, role)` with `role` in
`{primary, secondary}`, carrying `UNIQUE (tenant_id, product_id, category_id)` and a **partial**
`UNIQUE (tenant_id, product_id) WHERE role = 'primary'`. At-most-one-primary **MUST** be an index
rather than an application convention. The Foundation's entity tables **MUST NOT** gain inline
category columns.

**Implements**: `cpt-cf-bss-products-flow-assign-categories`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_product_category`
- Entities: `Product`, `Category`

### Attribute definition table

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-attribute-definition-table`

The system **MUST** create `products_attribute_definition` with `definition_id`, `tenant_id`,
`key` unique per tenant, `value_type`, a `localized` flag, brand and region visibility sets,
`state` in `{active, deprecated, removed}`, a nullable `seeded_by` marker and timestamps. The
`removed` value **MUST** be reachable only as a state flip; no migration or door may delete a row.

**Implements**: `cpt-cf-bss-products-flow-attribute-definitions`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`,
`cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_attribute_definition`
- Entities: `AttributeDefinition`

### Attribute value table

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-attribute-value-table`

The system **MUST** create `products_attribute_value` keyed by the owned entity's coordinates
`(tenant_id, entity_kind, entity_id)` plus `definition_id` plus the locale coordinates
`(locale, region, brand)` and the value, with a `UNIQUE` constraint over the full coordinate
tuple. For Product and SKU rows the table **MUST** hold the current head state only, history
living in the frozen version rows. For category rows the table **MUST** be the live state itself,
with no freeze-copy.

**Implements**: `cpt-cf-bss-products-flow-attribute-values`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_attribute_value`
- Entities: `AttributeValue`

### Metadata table

- [ ] `p2` - **ID**: `cpt-cf-bss-products-dod-metadata-table`

The system **MUST** create `products_metadata` keyed `(tenant_id, entity_kind, entity_id, key)`
with a value and timestamps, on both engines. The table **MUST** sit outside frozen version
content (P-D-06).

**Implements**: `cpt-cf-bss-products-flow-metadata`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_metadata`
- Entities: `MetadataMap`

### Well-known seeds

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-well-known-seeds`

The system **MUST** seed five well-known attribute definitions per tenant bootstrap, by
migration, marked `seeded_by = 'registry'`: `displayName` (localized, for Product, SKU and
Category), `description` (localized), `imageUri` (a URI string, non-localized),
`unitDisplayLabel` (localized, display only and never the metering-unit identity) and
`marketingFeatures` (a localized string list). A seeded definition **MUST** be deprecatable and
**MUST NOT** be removable.

**Implements**: `cpt-cf-bss-products-flow-attribute-definitions`

**Touches**:
- DB Table: `products_attribute_definition`
- Entities: `AttributeDefinition`, `WellKnownSeed`

### Governed live op envelope

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-governed-live-op`

The system **MUST** implement `GovernedLiveOp` as a pinned envelope of operation kind, target,
payload and the target's expected current state, submit it to the `05-governance` gate, and on
apply re-validate the expected state against the live row, refusing `STALE_LIVE_OP` on a
mismatch. The mutation and its event **MUST** land in one transaction. The type **MUST** be
exported for `03-sku-classification` to reuse without redefinition.

**Implements**: `cpt-cf-bss-products-algo-governed-live`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- Entities: `GovernedLiveOp`

### Taxonomy writer lock

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-taxonomy-writer-lock`

The system **MUST** serialize taxonomy mutations per tenant behind a writer lock — a Postgres
advisory lock, the write transaction on SQLite. A concurrency probe **MUST** exist in which two
re-parents that would jointly close a cycle run concurrently and exactly one fails; the
perturbation **MUST** be aimed at the loser's guard, and the probe **MUST** be shown to go red
when the lock is removed.

**Implements**: `cpt-cf-bss-products-algo-taxonomy-concurrency`

**Touches**:
- DB Table: `products_category`

### Taxonomy walk and its limits

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-taxonomy-walk`

The system **MUST** execute `TaxonomyWalk` inside the write transaction, refusing `TAXONOMY_CYCLE`
when the new ancestor chain contains the node itself and `TAXONOMY_LIMIT`, naming the limit, when
a create or re-parent exceeds the configured maximum depth or maximum children per node. Limits
**MUST** be validated on the mutation path only, so a later limit decrease never invalidates
existing structure.

**Implements**: `cpt-cf-bss-products-algo-taxonomy-integrity`,
`cpt-cf-bss-products-flow-manage-taxonomy`

**Touches**:
- DB Table: `products_category`

### Name uniqueness within a parent

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-name-in-parent`

The system **MUST** decide the uniqueness race on the unique index rather than a read-then-write
check, and **MUST** re-check on rename **and** on re-parent, refusing `DUPLICATE_CATEGORY_NAME`.
A concurrency probe with a positive control **MUST** prove both paths.

**Implements**: `cpt-cf-bss-products-algo-taxonomy-integrity`

**Touches**:
- DB Table: `products_category`

### Retire and delete guard

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-retire-delete-guard`

The system **MUST** refuse a retire or delete while any **non-terminal** Product references the
category as primary or secondary, or while an active child exists, raising
`CATEGORY_REFERENCED` and naming a sample of the holders. The guard **MUST** read the referencing
Product's lifecycle state and **MUST NOT** read the mere presence of a
`products_product_category` row. A test **MUST** prove that a discarded draft holding a category
link does not block the retire.

**Implements**: `cpt-cf-bss-products-flow-manage-taxonomy`,
`cpt-cf-bss-products-state-category`

**Touches**:
- DB Table: `products_category`, `products_product_category`

### Category assignment validators

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-assignment-validators`

The system **MUST** register save-door validators rejecting an unresolvable category, a category
that is retired (`CATEGORY_RETIRED`) and a category named both primary and secondary. Assignment
rows **MUST** land inside the save door's transaction, and a rollback **MUST** leave neither the
head update nor the assignment rows.

**Implements**: `cpt-cf-bss-products-flow-assign-categories`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_product_category`

### Primary category at publish

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-primary-at-publish`

The system **MUST** register a `→ published` validator requiring the primary category, refusing
`PRIMARY_CATEGORY_REQUIRED`, and **MUST** admit a draft that carries none. A paired positive
control **MUST** prove the publish succeeds once a primary is assigned.

**Implements**: `cpt-cf-bss-products-flow-assign-categories`

**Touches**:
- DB Table: `products_product_category`

### Definition lifecycle

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-definition-lifecycle`

The system **MUST** route creation and material changes through `GovernedLiveOp` under the
`attribute_definition × write` grant, refuse a type change on a definition with live values
(`DEFINITION_IN_USE`), and implement deprecate-then-remove with the **non-terminal head** as the
removal operand. Removal **MUST** be a state flip to a tombstone and never a `DELETE`, and
`removed → active` **MUST** re-list the same identity. The probe **MUST** be armed both ways:
removal refused while a non-terminal head carries a value, and removal **admitted** while only a
frozen version carries one.

**Implements**: `cpt-cf-bss-products-flow-attribute-definitions`,
`cpt-cf-bss-products-state-attribute-definition`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_attribute_definition`, `products_attribute_value`

### Attribute value validators

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-value-validators`

The system **MUST** register save-door validators refusing an unknown definition
(`ATTRIBUTE_DEFINITION_UNKNOWN`), a deprecated one (`ATTRIBUTE_DEFINITION_DEPRECATED`), a value
whose type does not match the declared type (`ATTRIBUTE_TYPE_MISMATCH`), and coordinates outside
either the definition's visibility scope or the entity's own scope
(`ATTRIBUTE_SCOPE_VIOLATION`). Every refusal **MUST** carry a paired positive control.

**Implements**: `cpt-cf-bss-products-flow-attribute-values`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_attribute_value`

### Default locale at the global coordinate

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-default-locale`

The system **MUST** register a `→ published` validator requiring, for every localized definition
the entity carries values for, a default-locale value at the **global brand-less** coordinate,
refusing `DEFAULT_LOCALE_MISSING` at publish and not at draft save. Per-brand default-locale
values **MUST** be optional overrides. For a category the same demand **MUST** land at the first
display-value write for that definition.

**Implements**: `cpt-cf-bss-products-flow-attribute-values`

**Touches**:
- DB Table: `products_attribute_value`

### Locale resolver

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-locale-resolver`

The system **MUST** implement `LocaleResolver` walking `(locale, region, brand) → (locale, brand)
→ (default-locale, brand) → global`, with the tenant default locale consulted only as a
preference before the global step. A resolution-matrix fixture **MUST** cover every chain step
including the brand-default and tenant-default fallbacks, and **MUST** include the case of a
brand-B reader against a value present only at `(default-locale, brand A)`, which resolves only
through the global coordinate. A test **MUST** prove a tenant-default change is non-retroactive.

**Implements**: `cpt-cf-bss-products-flow-attribute-values`

**Touches**:
- DB Table: `products_attribute_value`
- Entities: `AttributeValue`

### Category live value door

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-category-live-value-door`

The system **MUST** provide a category live-value door taking `If-Match` on
`products_category.mutation_seq` and refusing a mismatch with `STALE_CATEGORY_TOKEN`. The counter
**MUST** advance on operator acts and **MUST NOT** advance on non-operator row writes, so an
approval subject built from an act identity renders identically on the approved retry. The door
**MUST** emit `CategoryDisplayUpdated` in the same transaction and **MUST** be classified
non-material with an effective count of `min(N, 1)`.

**Implements**: `cpt-cf-bss-products-flow-attribute-values`

**Touches**:
- DB Table: `products_category`, `products_attribute_value`

### Content PII write block

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-pii-write-block`

The system **MUST** place a single write-block hook that invokes `10-retention-erasure`'s detector
and allow-list and raises `CONTENT_PII_BLOCKED`, failing closed on uncertainty. The hook **MUST**
run on attribute and description free text, on the metadata map with no carve-out, and on **every
operator free-text reason**: audit rows, approval rejections and break-glass session reasons,
correction-override, break-glass-correction and producer-retirement reasons, and the retirement
reason carried into the `SkuRetired` payload. The hook **MUST** be the single raiser and this
feature the single declaration site.

**Implements**: `cpt-cf-bss-products-flow-attribute-values`,
`cpt-cf-bss-products-flow-metadata`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- API: `PATCH /bss-products/v1/{products|skus}/{id}/metadata`
- Entities: `AttributeValue`, `MetadataMap`

### Metadata door

- [ ] `p2` - **ID**: `cpt-cf-bss-products-dod-metadata-door`

The system **MUST** implement `PATCH /bss-products/v1/{products|skus}/{id}/metadata` as a per-key
merge under the `metadata × write` grant, leaving absent keys untouched and removing a key whose
value is `null`. Configured caps on key count, key byte length and value byte length **MUST** be
enforced at the door with `METADATA_LIMIT`. A write to a terminal entity **MUST** be refused
`ENTITY_TERMINAL`. A test **MUST** prove a map standing at the cap can be reduced.

**Implements**: `cpt-cf-bss-products-flow-metadata`

**Touches**:
- API: `PATCH /bss-products/v1/{products|skus}/{id}/metadata`
- DB Table: `products_metadata`

### Error taxonomy registration

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-taxonomy-errors`

The system **MUST** declare all sixteen codes as constants on their raising rules and register
them into the Foundation's taxonomy, each carrying the RFC 9457 problem-response status the design
slice assigns it. No code carrying a registry code may reach the wire as a 422; the architectural
422s **MUST** render as 400 carrying their code.

**Implements**: `cpt-cf-bss-products-algo-governed-live`,
`cpt-cf-bss-products-algo-taxonomy-integrity`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- Entities: `CanonicalError`

### Taxonomy events

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-taxonomy-events`

The system **MUST** emit eight events through the Foundation's outbox in the mutating
transaction: `CategoryCreated`, `CategoryRenamed`, `CategoryReparented`, `CategoryRetired`,
`CategoryDeleted`, `CategoryDisplayUpdated`, `AttributeDefinitionUpdated` and `MetadataUpdated`.
Taxonomy events **MUST** order on `(tenant, category tree)` as one aggregate, matching the
single-writer discipline; metadata events **MUST** order on `(tenant, entity)`. Product and SKU
attribute-value writes **MUST** emit no event of their own, and that absence **MUST** be recorded
as an explicit no-event declaration.

**Implements**: `cpt-cf-bss-products-flow-manage-taxonomy`,
`cpt-cf-bss-products-flow-attribute-definitions`, `cpt-cf-bss-products-flow-metadata`

**Constraints**: `cpt-cf-bss-products-constraint-broker-native-events`

**Touches**:
- Entities: `CategoryCreated`, `CategoryRenamed`, `CategoryReparented`, `CategoryRetired`,
  `CategoryDeleted`, `CategoryDisplayUpdated`, `AttributeDefinitionUpdated`, `MetadataUpdated`

### Version content rendering

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-version-content-rendering`

Inside a frozen entity version this feature's two row collections — the category-assignment set
and the attribute-value set — **MUST** be rendered as JSON arrays sorted by the collection's own
identifier, each element following the Foundation's field-ordering rule (P-D-29). A golden vector
**MUST** prove the rendering byte-identical across both engines, because
`10-retention-erasure`'s restore drill compares those digests byte for byte.

**Implements**: `cpt-cf-bss-products-flow-assign-categories`,
`cpt-cf-bss-products-flow-attribute-values`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_entity_version`

### Metadata outside version content

- [ ] `p2` - **ID**: `cpt-cf-bss-products-dod-metadata-placement`

The system **MUST** keep the metadata map outside frozen published-version content, and a
`CatalogVersion` **MUST** capture it as of its own snapshot instant. A byte-identity probe **MUST**
mutate the map after a snapshot and prove the old snapshot's checksum does not move.

**Implements**: `cpt-cf-bss-products-flow-metadata`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_metadata`

## 6. Acceptance Criteria

- [ ] All five taxonomy ops queue through the governance gate, and none of them mutates the store
      before approval
- [ ] A re-parent whose new ancestor chain contains the node is refused `TAXONOMY_CYCLE`
- [ ] Two concurrent re-parents that would jointly close a cycle produce exactly one success, and
      the probe goes red when the writer lock is removed
- [ ] A category name colliding within its parent is refused `DUPLICATE_CATEGORY_NAME` on create,
      on rename **and** on re-parent
- [ ] A create or re-parent past the configured depth or children limit is refused
      `TAXONOMY_LIMIT` naming the limit; lowering the limit afterwards leaves existing structure
      valid
- [ ] A retire is refused `CATEGORY_REFERENCED` while a non-terminal Product references the node,
      and **succeeds** while only a discarded draft holds a link
- [ ] A delete is admitted only on a retired, childless, unreferenced node
- [ ] A Product draft with no primary category saves; publishing it is refused
      `PRIMARY_CATEGORY_REQUIRED`, and succeeds once a primary is assigned
- [ ] Assigning a retired category is refused `CATEGORY_RETIRED`
- [ ] Two concurrent inserts racing the at-most-one-primary index produce exactly one row
- [ ] A type change on a definition with live values is refused `DEFINITION_IN_USE`
- [ ] A definition removal is refused while a non-terminal head carries a value, and **admitted**
      while only a frozen version carries one
- [ ] A removed definition is a tombstone row, not a deletion, and its values still resolve on a
      terminal head
- [ ] A registry-seeded definition can be deprecated and cannot be removed
- [ ] `removed → active` re-lists the same `definition_id`
- [ ] A value whose type does not match its definition is refused `ATTRIBUTE_TYPE_MISMATCH`
- [ ] Coordinates outside the definition's or the entity's scope are refused
      `ATTRIBUTE_SCOPE_VIOLATION`
- [ ] Publishing without a global default-locale value for a localized definition is refused
      `DEFAULT_LOCALE_MISSING`; the same content saves as a draft
- [ ] A brand-B reader resolves a value present only at `(default-locale, brand A)` through the
      global coordinate
- [ ] Changing the tenant default locale does not change the resolution of an already-published
      entity
- [ ] A category display write with a stale `mutation_seq` is refused `STALE_CATEGORY_TOKEN`, and
      `mutation_seq` does not advance on a non-operator write
- [ ] Every enumerated operator free-text reason field is covered by the PII hook, and prohibited
      content is refused `CONTENT_PII_BLOCKED` at the door
- [ ] A metadata `PATCH` leaves absent keys untouched, and a `null` value removes its key
- [ ] A metadata map at the configured cap can be reduced by a subsequent `PATCH`
- [ ] Exceeding a metadata cap is refused `METADATA_LIMIT`; a metadata write to a terminal entity
      is refused `ENTITY_TERMINAL`
- [ ] Mutating the metadata map after a `CatalogVersion` snapshot leaves the old snapshot's
      checksum unmoved
- [ ] The frozen rendering of the category-assignment and attribute-value collections is
      byte-identical across SQLite and Postgres under a pinned golden vector
- [ ] Each of the eight named events is emitted by its door in the mutating transaction, and a
      Product or SKU attribute-value write emits none of its own
- [ ] Each of the sixteen codes is raised by exactly one rule and carries its declared problem
      status
- [ ] Every refusal enumerated in §2 has a paired positive control proving the door admits the
      corresponding legal act
- [ ] An applied `GovernedLiveOp` against a moved world is refused `STALE_LIVE_OP`, and no partial
      taxonomy mutation is observable
- [ ] No `#[ignore]`d test exists without a CI tier that runs it

## 7. Known unknowns

Four items bind implementation and are restated here so they reach the implementer rather than
only the design reader. The rest stay at their owners.

- **Open item 1** — whether a retired category may be re-listed. The slice declares no
  `retired → active` edge while the attribute-definition machine declares its re-listings
  explicitly, so the asymmetry is undecided rather than deliberate as far as the text shows.
  Until it resolves, §4 declares no such edge. *Owner: this feature.*
- **Open item 2** — the **values** of the taxonomy depth and children-per-node limits. PRD §7
  `nfr-scale-extensibility` defers them to the NFR workshop and §17.1 carries no interim default,
  so `TAXONOMY_LIMIT` has a rule and no number. The door must be built configurable and the
  configuration left unset. *Owner: the PRD owner.*
- **Open item 3** — the `GovernedLiveOp` envelope is consumed by `05-governance`, which does not
  exist. Until it does, this feature can define, submit and re-validate an envelope but cannot be
  end-to-end tested through an approval. *Owner: this feature with 05.*
- **Open item 4** — the PII detector and its allow-list belong to `10-retention-erasure`, which
  does not exist. This feature places the hook and declares the code; the verdict policy behind it
  is absent, so the hook is testable only against a stub. *Owner: `10-retention-erasure`.*

The remaining risks and open items are stated in
[`../design/02-taxonomy-attributes.md`](../design/02-taxonomy-attributes.md) §6 and are not
duplicated here.
