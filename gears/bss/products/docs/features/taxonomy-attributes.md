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
  - [The two seams, in full](#the-two-seams-in-full)
  - [Raised here rather than carried](#raised-here-rather-than-carried)

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

**Requirements** — carried from [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.2 with its scoping
notes intact:

- Whole: `cpt-cf-bss-products-fr-manage-taxonomy`,
  `cpt-cf-bss-products-fr-localized-attributes`
- Scoped: `cpt-cf-bss-products-fr-create-product` (the category and attribute content rules; the
  uniqueness clause is `01-foundation`'s),
  `cpt-cf-bss-products-fr-retention-erasure` (the write-block hook placement only; the detector
  policy and the erasure act are `10-retention-erasure`'s)
- Non-functional: `cpt-cf-bss-products-nfr-scale-extensibility` (the extensibility-limits half —
  max taxonomy depth and max children per node; the third limit that NFR names,
  `max attributes/entity`, is claimed by no slice at all and so by no feature either)
- Surfaces: `cpt-cf-bss-products-usecase-product-sku-editor`,
  `cpt-cf-bss-products-contract-registry-events`

**Principles**: `cpt-cf-bss-products-principle-registered-validators`.

**Component**: `cpt-cf-bss-products-component-capability-handlers`.

**Sequence**: none of its own — this feature contributes validators and content to
`cpt-cf-bss-products-seq-authoring-publish`.

**Not applicable or delegated**, stated so a reader can tell considered-and-excluded from
forgotten: **authentication** and **observability** — logging points, metrics, traces, the
correlation id and the health contribution — are `01-foundation`'s, as is outbox delivery and
retry; **read performance**, caching, pagination and faceting are `08-read-models`'; **retention
and reporting** are `10-retention-erasure`'s. This feature states no latency target: its write
paths are human-paced and single-writer by construction, and the one guard that scans
(`dod-retire-delete-guard`'s reference check) is bounded by the tenant's own catalog — its sample
size and the index it reads are unstated and owed. **Operator-facing error message wording** is
not specified here; the sixteen codes are the contract and the rendering is the API layer's.

**Out of scope**, mirroring [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.2: the approval
machinery itself (`05-governance`); read-model and search projections, faceting and category
read-model warming (`08-read-models`); erasure execution (`10-retention-erasure`); and `PlanTier`
and the recognized sets, which are governed live entities too but belong to
`03-sku-classification`, which reuses this feature's pattern rather than extending it.

### 1.3 Actors

| Actor | Role in this feature |
|-------|----------------------|
| `cpt-cf-bss-products-actor-catalog-admin` | Taxonomy ops (create, rename, re-parent, retire, delete) and the attribute-definition lifecycle |
| `cpt-cf-bss-products-actor-product-manager` | Authors attribute values, assigns categories, writes the metadata map |
| `cpt-cf-bss-products-actor-presentation` | Consumes localized resolution and category filters through the read models (`08-read-models`) |

### 1.4 References

- [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.2 — the entry this feature realizes
- [`../design/02-taxonomy-attributes.md`](../design/02-taxonomy-attributes.md) — the design slice.
  **This FEATURE is the declaration site of the five `flow-` ids and the four `algo-` ids**, and
  the slice's §2 and §3 point here for them; there is one definition site per id. Three of the
  four `algo-` ids moved here from the slice; the fourth,
  `cpt-cf-bss-products-algo-error-taxonomy`, is **minted here** because §3.3's code roster was the
  one process section carrying no id a FEATURE may define, and `design/02` §3.3 now points at it
  as its three siblings do. **The slice's
  step lists remain the normative ones and are not copied here**: re-spelling the 26 instruction
  steps it owns would fork the set's own instruction register and leave two texts where only one
  can be true. §2 and §3 below therefore carry the actor, the scenarios and the boundary of each
  flow, and the steps stay at their single source.
  - **The one exception is §4.** A state machine's transitions are a template-required
    id-bearing list, and the slice expresses this content inside `inst-tx-retire-guard` and
    `inst-ad-deprecate-then-remove` rather than as rows, so neither can be reused per row. §4's
    `inst-ce-*` and `inst-de-*` ids are that rendering and nothing more; they add no rule the
    slice does not already state.
  - **§5 restates `design/02` §4.1's table rosters, and that is a second exception owed a
    reason.** The no-copy rule above is about *instruction steps*; a Definition of Done has to
    name the columns, indexes and guards it obliges, or it obliges nothing testable. The cost is
    real and was paid once already: the first draft dropped §4.1's `(locale?, region?, brand?)`
    optionality markers — the exact annotation open items 6, 7 and 8 turn on — and the review
    caught it. Where §5 and §4.1 differ on a column-level fact, **§4.1 governs**.
  - **`contract-` ids are cited but not defined here.** A FEATURE artifact may **define** only
    `flow`, `algo`, `state`, `dod` and `featstatus` ids, so this document defines none of the
    nineteen `contract-` ids the design slices declare. They remain freely **citable**:
    `artifacts.toml` registers `DESIGN_SLICE` with `pattern = "design/*.md"` and
    `traceability = "FULL"`, so `cfs where-defined` resolves every one of them to its slice.
    (`features/foundation.md` §1.4 states the opposite — that the slices are excluded from
    autodetection and the ids resolve nowhere. That was true before the slices were registered
    and is false at this commit; the sentence is owed a correction in that document.)
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
- **Dependencies**: `cpt-cf-bss-products-feature-foundation` is the only build-time dependency.
  `05-governance` and `10-retention-erasure` are **integration** dependencies: their design slices
  exist, their FEATURE artifacts and code do not, and open items 3 and 4 in §7 state what that
  costs.

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
touched by one. This is **not** uniform with `03-sku-classification`'s `inst-rs-removal-operand`,
whose operand is non-terminal *published* heads. **The divergence is open, not decided**: slice
03 §6 registers it as "Do 02 and 03 admit a `draft` head as a blocking reference, or not?" with
*Owner: this slice with 02, jointly* — and this feature is that second owner. It is open item 5
in §7, and it is the operand `dod-definition-lifecycle`'s two-way probe is armed against.

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

**Input**: the acting principal, the operation kind, the target's identity, the payload, and the
target row's expected current state

**Output**: either an atomically applied mutation with its event, or a refusal — `STALE_LIVE_OP`
when the expected state no longer matches the live row, or the gate's own refusal

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

**Input**: the node, its proposed parent, the live tree, and the configured depth and
children-per-node limits (open item 2: those limits have no value yet)

**Output**: admission, or `TAXONOMY_CYCLE`, `TAXONOMY_LIMIT` naming the limit, or
`DUPLICATE_CATEGORY_NAME` from the index

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

**Input**: the tenant, the door being entered, and the precondition token that door carries —
the entity row's `If-Match` for Product and SKU content, `products_category.mutation_seq` for the
category live-value door

**Output**: serialized entry, or a precondition refusal — `STALE_REVISION` (Foundation's) or
`STALE_CATEGORY_TOKEN` (this feature's)

Taxonomy mutations serialize **per tenant** behind a taxonomy writer lock — an advisory lock on
Postgres, the write transaction on SQLite. Taxonomy ops are rare and human-paced, and
single-writer discipline is what makes the walk's verdict trustworthy.

Product and SKU attribute-value and metadata writes need no extra machinery: they ride the entity
row's `If-Match`. The category live-value door is the exception and carries its own token,
`products_category.mutation_seq`, which counts **acts, not row writes** — the door spends a
`GovernedLiveOp`, and an approval subject built from an act identity must render the same subject
on the approved retry, which a counter advanced by non-operator writes would break.

### Error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-error-taxonomy`

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
2. [ ] - `p1` - **FROM** `retired` **TO** *(row deleted)* **WHEN** the node is retired, childless and unreferenced; this is the single physical row removal this feature performs, and there is **NO EDGE** back to `active` from either state - `inst-ce-terminal`

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

Twenty-four, against the donor feature's nineteen — both counts re-measured by `grep` on the two
files rather than recalled. The gloss that stood here compared table and door counts and had both
operands wrong in the donor's favour; it is dropped rather than repaired, since the DoD counts
need no justification beyond being reproducible.

### Category table and its guards

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-category-table`

The system **MUST** create `products_category` on both engines with `category_id`, `tenant_id`,
`parent_id` as a nullable self-referencing foreign key, `name` and `name_normalized`, `state`,
`mutation_seq` and timestamps. `name_normalized` **MUST** use the Foundation's operand — NFKC,
then full casefold, then trim and collapse, computed application-side. The table **MUST** carry
`UNIQUE (tenant_id, parent_id, name_normalized)` and a foreign-key children guard on delete. A
schema-oracle golden **MUST** exist for both engines together with a perturbation case proving
the oracle can fail.

**Shipped with the root half the declared UNIQUE cannot hold** (**P-D-88** arm 1): both engines
treat NULLs as distinct, so root categories carry their own partial
`UNIQUE (tenant_id, name_normalized) WHERE parent_id IS NULL`, probed on both engines — the
Postgres case asserts the refusal by the index's name. A sentinel parent cannot satisfy the
self-referencing FK, and `NULLS NOT DISTINCT` has no `SQLite` twin.

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

**The table ships and the tick does not, because §7 row 21 is about this FK.** Both keys, both
uniqueness guarantees and the role roster are built, and the **store over them ships with them** —
`repo::replace_category_assignments` and `repo::category_assignments`, whole-set writes on the
caller's runner so the save door can land them in its own transaction (P-D-46). At-most-one-primary
is proven by writing two primaries and reading the index's refusal back, never by a prior read.

**Where each guarantee is probed, corrected.** An earlier revision of this paragraph said all three
were *"probed on both engines"*; measured at this commit, the Postgres side probes the **column
roster only** (`tests/postgres_taxonomy_schema.rs`), while the role `CHECK`, the table-level
`UNIQUE` and the partial primary index are probed on the `SQLite` mirror alone. The gap is one
Postgres case, and it matters beyond bookkeeping: the two engines report a uniqueness refusal in
**different shapes** — Postgres names the constraint, `SQLite` names the columns — so the store's
classifier has an engine-specific branch that no executed statement here reaches. It is covered by
a direct unit test over the classifier and is owed a Postgres probe.

The DoD's last clause — *"The Foundation's entity tables **MUST NOT** gain inline category
columns"* — stood asserted by nothing until this commit: it was stated in two module docs, and a
doc comment refuses nothing. It is now read off the engine's own catalogue for both head tables.

What is unbuilt is the row's own subject: *"no referential action is stated"*. The shipped FK takes the default, which
refuses a category's deletion while **any** link row exists — including rows held by discarded and
retired Products, which `inst-tx-retire-guard`'s "unreferenced" test does not count, reading the
Product's lifecycle state and never the link row. So the DDL as written makes the guard's stated
semantics unreachable in one direction, and the choice between a cascade, a restrict, and the guard
clearing link rows in its own transaction is row 21's, co-owned with the schema owner.

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

**The roster and its store ship; the tick waits on §7 row 13.** `repo::insert_attribute_definition`,
`attribute_definition_by_key`, `attribute_definitions` and `flip_definition_state` are built, the
flip **pinned at the state the caller read** so a peer's move between read and write moves no row.
The store offers no delete for this table at all, which is the DoD's own clause made structural
beside the `BEFORE DELETE` trigger that enforces it on both engines. `value_type` stays a string
in the store for the reason the migration gives: no document enumerates the admitted types, and an
enum in the repository would answer that question from the storage layer.

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
`(locale?, region?, brand?)` — the optionality markers are `design/02` §4.1's and are load-bearing,
being the exact subject of open items 6 and 7 — and the value, with a `UNIQUE` constraint over the
full coordinate tuple. **That constraint does not today constrain the mandatory global coordinate**
(open item 7): both engines treat NULLs as distinct. For Product and SKU rows the table **MUST** hold the current head state only, history
living in the frozen version rows. For category rows the table **MUST** be the live state itself,
with no freeze-copy.

**The store ships; the tick waits on §7 row 20.** `repo::upsert_attribute_value`,
`attribute_values_of` and `delete_attribute_value` key on the whole seven-column coordinate, and
the write is an upsert rather than a read-then-write so two authors racing on one coordinate
produce one row instead of a conflict the door would have to translate. The read is ordered
totally over the four coordinate columns, which is **that read's** determinism and answers nothing
about row 9 — the frozen-content sort key is `01-foundation` §4.3's and P-D-29's, a different
site. `entity_kind` is a `&str` through the store, since row 20 is exactly the question of what it
admits; a `category` row is written and read back beside a `product` one, so the kind the row
measures is demonstrably admitted.

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

**The store ships; the tick waits on §7 row 20, which this table shares with the value plane.**
`repo::upsert_metadata`, `metadata_of` and `delete_metadata_key`. The upsert leaves `created_at`
alone on an overwrite, so the column keeps meaning *when this key first appeared* rather than
quietly becoming a second copy of `updated_at`. No cap is enforced here: `METADATA_LIMIT` is the
door's and §7 row 2 records that neither the key count nor the value length has a value anywhere.

**Implements**: `cpt-cf-bss-products-flow-metadata`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_metadata`
- Entities: `MetadataMap`

### Well-known seeds

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-well-known-seeds`

The system **MUST** seed five well-known attribute definitions per tenant bootstrap — ~~by
migration~~, **on the first write that could need one, in that write's own transaction
(P-D-104)** — marked `seeded_by = 'registry'`: `displayName` (localized, for Product, SKU and
Category), `description` (localized), `imageUri` (a URI string, non-localized),
`unitDisplayLabel` (localized, display only and never the metering-unit identity) and
`marketingFeatures` (a localized string list). A seeded definition **MUST** be deprecatable and
**MUST NOT** be removable.

**Answered by P-D-100 as amended by P-D-104, and built: one writer, on the first write that could
need a seed.** This feature reported the DoD blocked because *"seeded by migration, per tenant
bootstrap"* names two paths and neither was available — the migration was another owner's and the
gear has no tenant-bootstrap hook. The owner's first call took that split; the second withdrew it
on two measurements this feature had not made. The migration arm is **unbuildable**: seeding a
per-tenant store needs a list of tenants, no gear's schema has a tenant registry, and **no
migration in the workspace inserts a row at all**. And it was redundant, because the condition is
*"this tenant has no seed rows"* and never *"this tenant is new"* — one writer always reached a
pre-deploy tenant just as readily. The old-versus-new split read a distinction the condition never
made, and this feature's own entry proposed it.

**The trigger site is the content-save path**, at the moment a payload names `attributes` — a
**write**, the earliest act that can need a well-known definition, and the one place where the
existence check is free: the door must read the roster anyway to resolve each named key, so the
check is that read's own `is_empty()`. Only the empty case pays, and only once per tenant. A save
naming just `categories` triggers nothing, since it cannot need a definition. P-D-104 moved this
off the read path deliberately: a lazy read-through makes a `GET` mutate, breaks a read-only
replica, and bills the first reader for a write it did not ask for.

`domain::taxonomy::WELL_KNOWN_SEEDS` is the single definition site — the roster, `REGISTRY_SEEDED_BY`
and `is_removable`, the last probed in **both** directions so a guard refusing every removal cannot
satisfy it. Seeding is idempotent by `uq_products_attribute_definition_key` and never re-materialises
a definition an operator has deprecated, since the state flip is the only removal there is and they
would otherwise have no way to say no.

**One thing in it is this feature's proposal and not the design's.** The `DoD` names five keys and
three **shapes** — a localized string, a URI string, a localized string list — and no **tokens**;
no document in the set enumerates `value_type`'s admitted values, the column being pinned to
non-emptiness only on P-D-74's shape. The three constants are therefore carried in the owed
register as a proposal, and nothing closes the set: the store's `value_type` is still a string.

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

**Three of the four halves ship and the fourth is measurably unbuildable, so this stays unticked.**
`domain::live_op::GovernedLiveOp` is the envelope — kind, target, payload and the target's expected
state, generic over that state so `03` passes its own rather than every slice's operations landing
in this type; `check_still_current` refuses `STALE_LIVE_OP` (409, `design/02` §3.5's own code, and
**not** `STALE_REVISION`, which a live row cannot be stale against, nor `STALE_CATEGORY_TOKEN`,
which is the live-value door's precondition); and `apply` runs the check immediately before the
mutation closure, so a stale op **cannot** write — asserted by observing the closure, because a
check placed after the write would pass a naive test while having already written.

**What is not built is submission to the `05` gate — but the reason above was wrong, and a reader
steered by it would build the wrong thing.** The struck paragraph said
`GovernanceGate::evaluate` takes an `EntityRef`, that submitting therefore *"means inventing a
mapping from a live target to an entity ref"*, and that **P-D-93** recorded this as its first
residue. Measured at this commit, three of those are false:

| The claim | What the code and the register say at `HEAD` |
|---|---|
| `evaluate` takes an `EntityRef` | It takes a **`GateSubject`**. `EntityRef` is one constructor of five |
| a live target needs an invented mapping | `GateSubject::governed_live_op(tenant_id, target)` **is** the constructor, and `SubjectKind::GovernedLiveOp` is one of the five kinds — granted by **P-D-67 arm 4** on 2026-08-31, *"the gate's subject widens to the approval store's own pair"* |
| P-D-93 recorded it as a residue | The sentence is in P-D-93's **arguments-against** paragraph, and it says *"**this decision** does not grant one"* — true of P-D-93, which granted nothing, while P-D-67 had |

**P-D-93 is not the defect; carrying it forward was.** Its argument was *correct when written*:
`043bca636` landed at 17:30 on 2026-09-01, and `evaluate` did still take an `EntityRef` at that
moment. `f894378e9` built P-D-67 arm 4's grant — which that entry had itself deferred *"to the
build"* — at **18:38 the same day**, sixty-eight minutes later. An arguments-against paragraph
records a moment; this DoD quoted one as a standing reason and never re-measured it. The lesson is
the citation's, not the register's.

The subject seam is one call wide and is now probed
(`domain::live_op_tests::the_envelopes_target_is_a_gate_subject_of_its_own_kind`, which asserts the
live-op kind **against** the entity kind so the two cannot collapse into the very mapping the
struck sentence imagined).

**What genuinely remains, narrower and two-fold**: `evaluate`'s **`expected_revision`** operand,
which a live row cannot supply — that absence being why this envelope pins a *state* — and which is
`features/governance.md` §7 row 14, live; and **`05`'s submit door**, whose route is undeclared (05
§7 row 12).

**And one measurement of P-D-93's own does not reach here.** It released this DoD's §7 row partly
on *"**four** doubles ship … and the door tests already turn on which one is passed"*. All four are
private items in two `#[cfg(test)]` door modules with no `pub`, and `test_support` carries no gate
double at all — so the apply path is proven against the closure it is handed, never against a gate,
fake or real. That is recorded rather than used to re-hold the row: the row's other two
measurements stand.

**Implements**: `cpt-cf-bss-products-algo-governed-live`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- Entities: `GovernedLiveOp`

### Taxonomy writer lock

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-taxonomy-writer-lock`

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

**The cycle half ships; the limit half is judged and unreachable, on §7 row 2.** `TaxonomyWalk` runs
under the per-tenant writer lock in `infra::taxonomy::reparent_under_lock` — lock, then read, then
judge, then write, an order whose whole point is that reading first would judge a chain a peer can
still change. `domain::taxonomy::depth_of` and `children_of` measure off the **same** edge list the
cycle rule reads, so the two cannot disagree, and both terminate on a tree that already contains a
cycle. `limit_verdict` refuses above each threshold and names which, with the boundary case probed
in both directions.

`TaxonomyLimits` carries `Option` thresholds and **no `Default`**: row 2 records that these limits
*"have no interim default anywhere"*, so `None` is not a policy of *unlimited* but the absence of a
stated number, and nothing can acquire one by accident. Nothing configures it, so the judge is
declared and unreachable — the honest shape for a rule whose operand is owed, and it means
`TAXONOMY_LIMIT` is a code this feature declares and never raises.

The third MUST is structural rather than tested at a door: `limit_verdict` takes what a mutation
**would** make the tree and never the tree as it stands, so no reading of it judges existing rows,
and a later limit decrease has nothing to invalidate.

**Implements**: `cpt-cf-bss-products-algo-taxonomy-integrity`,
`cpt-cf-bss-products-flow-manage-taxonomy`

**Touches**:
- DB Table: `products_category`

### Name uniqueness within a parent

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-name-in-parent`

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

**Built and probed; the tick waits on §7 row 21 and on the code's own absence.**
`repo::retire_census` reads both halves and `domain::taxonomy::retire_verdict` judges them, the
census taking the caller's read under the writer lock rather than fetching its own rows.

The MUST that decides the shape is *"reads the Product's lifecycle state and **not** the mere
presence of a `products_product_category` row"*, so the Product read is the **outer** query and the
link table is a subquery inside it: a link row held by a discarded draft contributes nothing
because its Product never enters the result. Two round trips would have been wrong in one
direction — a Product **published** between them has a link row the first read never saw, and the
guard would answer *"unreferenced"* about catalog that now references the node. One statement has
no window.

The named test ships and asserts the link row is **still there** afterwards, without which a census
returning nothing because the assignment had vanished would pass while the rule went unmeasured.
Beside it: one Product walked along its own admitted edges with the census re-read at each, so
`draft`, `published` and `deprecated` block and `retired` and `discarded` do not; an active child
blocks and a retired one does not; and the sample reads `bound + 1` so the refusal can say *"at
least N"* without a second counting statement.

**`CATEGORY_REFERENCED` has no `DomainError` variant at this commit** — it is one of twelve of this
feature's sixteen codes still absent, which is `dod-taxonomy-errors`' work. So the verdict returns
`domain::taxonomy::CategoryReferenced`, carrying the code as a constant and the sample in its
detail, and the door maps it exactly as it maps `repo::AssignmentWrite`'s two conflicts.

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

**The three rules are registered and reach the wire** (group A6). `content_save_pipeline` is the
one list both save doors run, and the Product door builds its subject from the payload plus two
reads — each named category's state, and the tenant's definition roster. All three refusals are
probed **through the door**, and each is shown to write no assignment row and move no head
revision.

The transaction clause holds end to end: the pipeline runs in the registered-validators phase
before the gate, and `repo::replace_category_assignments` runs after the head `UPDATE` on the same
transaction (**P-D-46**), so a refusal costs nothing and a rollback leaves neither.

**Still not ticked, on §7 row 17.** Two of the three refusals — the unresolvable category and the
primary/secondary duplicate — have no code of their own and ride the Foundation's `VALIDATION`.
This DoD's own sentence names a code for one refusal only, so the row and the DoD may not be asking
the same thing; that is registered rather than decided here, on the standard `A-OWED-02` set.

**Two of the three refusals have no code**, which is §7 row 17's own list — the unresolvable
category and the primary/secondary duplicate. Both raise the Foundation's declared `VALIDATION`
rather than a seventeenth code minted here, and the violation's `subject` and `detail` are what tell
them apart until row 17's owner acts. `CATEGORY_RETIRED` is one of the sixteen and is raised as
itself.

**The transaction half is measured on the half this strand owns**:
`replace_category_assignments` takes the caller's runner and opens nothing, so a rolled-back save
leaves no assignment row —
`assignment_rows_roll_back_with_the_transaction_they_ride_in`. The head-update half is the door's.

**Implements**: `cpt-cf-bss-products-flow-assign-categories`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_product_category`

### Primary category at publish

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-primary-at-publish`

The system **MUST** register a `→ published` validator requiring the primary category, refusing
`PRIMARY_CATEGORY_REQUIRED`, and **MUST** admit a draft that carries none. A paired positive
control **MUST** prove the publish succeeds once a primary is assigned.

**It is the first registered validator on this gear that is neither `04`'s nor `05`'s.** The
Foundation's own note said `Phase::RegisteredValidators` was *"empty, and that is a real gap"*
because the validators it named belonged to those two slices; the Product publish path is no longer
vacuous. Three properties made the wiring non-obvious and each is built deliberately:

- **Its own pipeline.** The shared re-validation pipeline runs on the publish edge **and** on the
  save door, so a rule registered there would refuse a save on a draft carrying no primary — the
  case the PRD explicitly admits. The `→ published` pipeline runs only from `run_publish`.
- **The operand is read outside the pipeline.** `ValidationRule::evaluate` is synchronous and the
  assignment lives in another table, so the door reads the fact and the subject carries it.
- **The refusal carries the rule's own code**, not the generic `INCOMPLETE_ENTITY`.
  `inst-fd-publish-revalidate` names *"`INCOMPLETE_ENTITY`/rule-named code"* — a disjunction — and
  `PRIMARY_CATEGORY_REQUIRED` is this slice's declared code (P-D-36). Only codes this crate declares
  are surfaced, so a future rule cannot leak an unmapped one onto the wire.

**Sixteen shipped fixtures published a Product with no primary assignment and went red**, which is
the rule working: each was asserting a publish the design forbids. All sixteen seed the assignment
now; none relaxed the rule. *(Two earlier counts of this same population — "twelve" here and "six"
in the helper's own doc — were both written in the commit that landed the rule and neither
reproduced against the crate; the number is the `assign_primary_category` call census.)*

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

**The machine and the operand ship; the routing does not.** `definition_edge` admits exactly §4's
four edges — both re-listings included, `active → removed` excluded because
`inst-de-deprecate-then-remove` puts deprecation between them — and every other pair, self-edges
among them, is refused. `seeded_edge` holds `dod-well-known-seeds`' clause in **both** directions: a
seed deprecates and re-lists and never removes, while an operator-added definition does remove, so
a guard refusing every act on a seed fails as surely as one refusing none. Removal is a flip and
this strand's store offers no delete for the table at all, beside the `BEFORE DELETE` trigger.

**The named both-ways probe ships on its exact scenario.** `repo::definition_value_holders` reads
the operand across the three tables `entity_kind` spans — non-terminal Products and SKUs, and
`active` categories, the last because §6 records that the guard *"counts an active category as a
value-carrying head"*. The Product is then walked `draft → published → deprecated → retired`
through real edges, so a frozen version genuinely exists, and the census is empty at that end while
the `products_attribute_value` row is asserted to still be there — without which a census answering
empty because the *value* had vanished would pass while the rule went unmeasured.

**What is not built, and it is not the machine.** The `GovernedLiveOp` routing the DoD's first
clause requires needs `05`'s submit door, which has no route (05 §7 row 12) — the same blocker
`dod-governed-live-op` carries — and the `attribute_definition × write` grant pair is undeclared
(§7 row 16). Four §7 rows also hold this DoD's *operands* rather than its mechanism: rows 5, 10,
11 and 12. Row 11 is the sharpest — the DoD states **two** operands in one sentence, an undefined
*"live values"* for the type change and the defined non-terminal head for removal — so
`definition_in_use_verdict` takes whatever census it is handed and judges it, and which census a
type change should read stays that row's.

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

**All four rules are registered at both doors** (group A6) — including the SKU save door, which
ran **no pipeline at all** before it while SKUs have carried attribute values since `02`'s tables
landed. One `content_save_pipeline` serves both, so the two lists cannot drift.

**A rule's code reaches the wire as itself, and group A5 reported otherwise.** A5 said these codes
would fall back to `INCOMPLETE_ENTITY` until twelve `DomainError` variants landed; that read
`transition_refusal`'s ladder, which is the **publish** path. The save path is
`DomainError::Validation`, whose mapping arm renders **each violation's own `code`** as the wire
`type`. So all four are attributed today with no variant at all, which is measured at both doors.
The consequence for §7 row 18: a pipeline violation renders through `failed_precondition`, the
architectural 422 — so answering that row 409 would move these refusals **out** of the pipeline,
which makes it a placement question and not only a status one.

Beyond the paired controls the DoD asks for, one case holds the property the shared phase makes
possible: an unresolved definition raises **one** violation and not four, because every rule skips
what it cannot judge.

Three readings inside them are worth stating, because each could have gone the other way silently:

- **A `removed` definition is refused as `ATTRIBUTE_DEFINITION_UNKNOWN`, not `_DEPRECATED`.** The
  tombstone is a row that exists and is *outside the set* — `repo::recognized`'s own words for the
  sibling roster. It keeps a terminal head's value resolving and admits no new write.
- **A type token the gear does not know is not judged.** `ValueShape` maps the three tokens
  `WELL_KNOWN_SEEDS` proposes and answers `None` for everything else; refusing an unmapped token
  would close the feature to every operator-defined type, and `design/02` §6 owes the roster.
- **§6's brand-less-global item is deferred, in the one direction that leaves both DoDs
  satisfiable.** The item records that under a containment-only reading *"the write the publish
  validator demands is the write the save validator refuses"*, so a brand-scoped entity could never
  publish. `AttributeScopeRule` judges a coordinate **only where the payload names one**: `brand:
  ""` is P-D-88 arm 2's absence, not a brand called empty-string, and there is nothing to contain.
  Taken as forced rather than chosen, pinned by
  `a_brand_less_global_value_survives_a_brand_scoped_entity`, and registered — if its owner decides
  otherwise, that is the test which changes.

Both scope columns are read through `ResolvedScope::parse`, so an **empty column is unrestricted**
(P-D-39) rather than empty — the predicate written as membership alone would refuse every
coordinate under nearly every definition in the gear — and a column that will not parse refuses
rather than admits.

**Untouched**: §7 row 19, which asks *which* of these run at the category live-value door. They are
registered on the entity save door by construction here; the category branch writes through another
one and that door is `dod-category-live-value-door`'s.

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

**The rule is registered at the publish door** (group A6): `published_content_pipeline` runs in
the `→ published` phase beside `inst-tx-primary-at-publish`, over the entity's **stored** values
grouped by definition. Both directions are probed through the door — a localized definition with no
global value refuses the publish naming `DEFAULT_LOCALE_MISSING`, and the same entity publishes the
moment the global value lands. A non-localized definition holds no publish, which is the arm that
keeps `imageUri` from refusing every entity that carries an image.

**The category half does not ship.** `DefaultLocaleRequired` refuses
a localized definition carrying values but none at the global coordinate, skips a non-localized one
and one carrying nothing at all, and -- the case that matters -- is **not** satisfied by a value at
`(default-locale, brand A)`. That is the whole reason per-brand values are *overrides*: a brand-B
reader never visits brand A's default, which the resolver's matrix measures from the reading side.
It goes in the `-> published` pipeline and nowhere else, for the reason
`inst-tx-primary-at-publish`'s sibling gives -- a rule in the shared pipeline would refuse a draft
save the design admits.

**§7 row 8 is answered and struck (P-D-102): the global coordinate is absent on all three axes.**
This feature reported the row as a naming defect with both readings closed elsewhere, and the
decision came out **larger** than that. Read in full, `inst-av-default-locale` said *"the
default-locale value at the global **(brand-less)** coordinate"* — and that parenthetical is a
third, live reading: brand absent, locale present and equal to the tenant default. It is settled on
P-D-101's own argument, that such a value carries the locale which was default when it was written.
`DefaultLocaleRequired` and `GLOBAL_COORDINATE` already had it right; the register entry that
compressed the fork was the weaker artifact, and that is worth recording where the next reader will
find it.

**§7 row 1 still holds the tick**, and it is now the only thing that does.

**The category half is the live-value door's** and lands with it -- see
`dod-category-live-value-door`, whose route is undeclared (§7 row 16).

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

**The chain and its matrix ship.** `resolve_localized` walks the four steps and reports **which**
one answered, so the matrix asserts the step and not only the value -- a resolver whose first step
matched everything would satisfy a value-only assertion at every row of it. The DoD's named case is
there: brand A reaches its own default, brand B does not and falls to global. So is the
non-retroactivity proof, and it is the interesting one -- moving the tenant default to a locale
nothing is stored under moves the reader from step 3 to step 4 and the resolution **does not
fail**, which is `inst-av-resolve`'s item-37 claim made executable.

One case beyond the DoD's list, because it is the chain's one silent failure mode: steps 2 and 3
name a locale and a brand and no region, so both look for a value whose region is **absent**. A
region-insensitive step 2 would hand an `eu` value to an `apac` reader.

**One thing the DoD needs that does not exist, and it is not what §7 row 6 asked.** That row is
answered and struck (**P-D-101**): the default-locale is the **tenant** default only and *"resolves
per brand"* is gone from `inst-av-resolve`, so the per-brand store the row wanted is no longer
owed. What remains is the gap this feature found beside it and no row carried: **`ProductsConfig`
has no `default_locale` field**, so the chain's one input has no source and every caller supplies
it as an argument. The resolver is correct for whatever arrives and nothing arrives — the same
shape `TaxonomyLimits` takes. `config.rs` is not this strand's; the DoD ticks when the field lands.

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

**The token ships; the door does not, and cannot here.** `repo::write_category_display_value`
carries the caller's `If-Match` value **inside the counter bump's own `WHERE` clause**, so the
token is spent by the same statement that advances it and no read-then-write window is left for a
peer act to fit into. A lost token answers `domain::taxonomy::StaleCategoryToken` carrying both
counters, and the probe asserts the negative that matters: on a refusal **no value lands** and the
counter does not move. A door that checked the token and then wrote, or wrote and then checked,
would pass a counter-only assertion while the display value it was refusing had already been
committed.

*"Counts acts, not row writes"* (**P-D-50**) is probed as its own case: writing a category's value
through the plain store path leaves the counter alone. Without that, a counter advanced by any row
write would change under an approval subject built from an act identity, and the approved retry
would render a different subject -- the exact failure P-D-50 names.

**What is not here, and none of it is this strand's.** The REST path and the grant pair are
undeclared (§7 row 16); wire doors are the lead's; `CategoryDisplayUpdated` has no payload type in
`infra::events` and no `SCHEMA_REFS` entry, so the *"same transaction"* clause has nothing to
enqueue -- that is `dod-taxonomy-events`' patch; and the non-material classification with effective
count `min(N, 1)` lives in the governance host, which this feature does not own.
`STALE_CATEGORY_TOKEN` also has no `DomainError` variant yet, which is `dod-taxonomy-errors`'.

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

**The hook ships and is placed at both of this feature's attribute call sites** (group A7).
`domain::taxonomy::content_pii_block` is the single raiser, a free function over a `PiiDetector`
seam that `10-retention-erasure` fills — a free function and not a check inside either door,
because `inst-av-pii-reason` enumerates doors in `01`, `04`, `05` and `07` that spend the same
block, and a door that inlined the rule would be a second raiser the moment the next one needed it.
**For `04-lifecycle`'s retirement `reason`: it is `domain::taxonomy::content_pii_block`**, called
with the field's own name and the operator's text.

**Fail-closed lives in the hook, not in the detector.** `PiiVerdict::Uncertain` blocks. Leaving
that to each detector would let one opt out of the rule by answering its own doubt as `Clean`, and
the DoD puts *"failing closed on uncertainty"* on the hook.

**The default host admits and says so.** `NoPiiPolicyDetector` is the shape
`NoMaterialityPolicyGate` takes for its own missing slice: it inspects nothing, and
`NO_PII_POLICY_REASON` states the **deviation** rather than a justification, because that string is
what an operator sees. Refusing every string was the alternative and §6 already ruled it out —
*"a stub that refuses every string satisfies both `dod-pii-write-block` and acceptance criterion 22"*
vacuously, which is why the clean-text positive control is part of the criterion and is asserted
beside the refusal.

**Not ticked, and this feature cannot tick it alone** — §5's own note says so. The enumeration
reaches six doors owned by `01`, `04`, `05` and `07`; the hook and this feature's two call sites are
its testable core and they ship. The metadata call site waits with the metadata door below. §7 row 4
also stands: the detector itself is `10`'s and does not exist, so nothing here has been measured
against a real policy.

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

**BLOCKED — the door cannot be authorized, and the two files that would authorize it are not this
strand's.** The store surface ships (`repo::upsert_metadata`, `metadata_of`,
`delete_metadata_key`), and the route is buildable now that the door files are granted. The grant
pair is not. `metadata × write` is declared **nowhere in the code**, and standing it up needs both:

- `src/authz.rs` — a `resource_types::METADATA`, a `labels::METADATA`, and that label added to
  `labels::ALL`;
- `src/gts/permissions.rs` — the matching `AuthzPermissionV1` instance
  (`…products.metadata_write.v1`, `resource_type: labels::METADATA`, `action: actions::WRITE`).

**Both, together.** `permissions.rs`'s own `catalog_resource_types_match_authz_labels_all` asserts
**equality** between the declared instances and `labels::ALL`, so a label without a permission fails
the gate and a permission without a label fails it the other way.

**And the contradiction worth an owner's glance**: `permissions.rs` is on this strand's forbidden
list, while that file's own module doc says the `metadata` row is *"deliberately absent: they belong
to the slices that build those doors"* — which is this one. The grant that opened the door files was
made on the reading that they were the only obstacle; they are not. No grant was invented and no
existing pair was borrowed: authorizing a new door against `product × write` would be an
authorization decision taken by a strand, which is the one class of thing worth stopping for.

**Two §7 rows stand behind it in any case.** Row 2 leaves `METADATA_LIMIT` with **no number** — the
key count and the byte lengths have no value anywhere — so the DoD's *"configured caps … MUST be
enforced"* has nothing to enforce, and the required *"a map standing at the cap can be reduced"*
test has no cap to stand at. Row 14 records that two concurrent metadata writes both pass their
precondition, since metadata rides the entity's `If-Match` and by P-D-06 bumps no version.

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

**Fifteen of sixteen are declared, and A6 corrected what *registration* means here.** A5 reported
that all twelve missing codes needed a `DomainError` variant and would reach the wire as
`INCOMPLETE_ENTITY` until they had one. That read `transition_refusal`'s ladder, which is the
**publish** path. The save path is `DomainError::Validation`, and `infra::error_mapping`'s arm for
it renders **each violation's own `code`** as the wire `type` — so a code raised through the
pipeline is registered and attributed with no variant at all, and six of the twelve were never
missing anything.

**One variant landed, not twelve**, and the rule is the one `infra::error_mapping` states for
itself: *"mapping a code this gear cannot raise would be a dead `match` arm."* Measured at this
commit, exactly one of the sixteen is raised outside a report by production code —
`CONTENT_PII_BLOCKED`, whose hook both content-save doors now call. It gets
`DomainError::ContentPiiBlocked`, a `code()` arm and a mapping arm at 422-architectural, and a door
test that drives a detector double through `save_product_under_gate` so the arm is reachable rather
than merely present.

| Code | Where it stands |
|---|---|
| `DUPLICATE_CATEGORY_NAME`, `TAXONOMY_CYCLE`, `PRIMARY_CATEGORY_REQUIRED`, `STALE_LIVE_OP` | variant + arm, since before this feature |
| `CATEGORY_RETIRED`, `ATTRIBUTE_DEFINITION_UNKNOWN`, `ATTRIBUTE_DEFINITION_DEPRECATED`, `ATTRIBUTE_TYPE_MISMATCH`, `ATTRIBUTE_SCOPE_VIOLATION`, `DEFAULT_LOCALE_MISSING` | **registered through the pipeline** — each reaches the wire carrying its own code, measured at both doors. No variant needed, and one would be unreachable |
| `CONTENT_PII_BLOCKED` | **variant + arm, this group.** Raised outside the pipeline, as §3.3 requires |
| `TAXONOMY_LIMIT`, `CATEGORY_REFERENCED`, `DEFINITION_IN_USE`, `STALE_CATEGORY_TOKEN` | judge and producer built; **no production caller**, because the taxonomy's three doors have no route (§7 row 16). A variant now would be the dead arm |
| `METADATA_LIMIT` | no raiser and no number (§7 rows 2 and 18's neighbour); it lands with its door |

**The counted rosters moved 51 → 52**, both of them: `error_mapping_tests`'
`DOMAIN_ERROR_VARIANTS` and its `one_of_every_variant`, and `error_tests`' `wire_code_roster` with
its literal. Re-derived against `DomainError::code`'s arms rather than bumped, which is what that
file's own note asks for.

**§7 row 18's consequence, now measurable.** A pipeline violation renders through
`failed_precondition`, the architectural 422. So answering that row **409** for `CATEGORY_RETIRED`
and `ATTRIBUTE_DEFINITION_DEPRECATED` would move those two refusals **out** of the pipeline and into
variants of their own — it is a placement question, not only a status one. They stay at 422 and the
row stays open.

**Implements**: `cpt-cf-bss-products-algo-error-taxonomy`

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

**The aggregates and the six payload types ship; two of the eight are held.**
`infra::taxonomy::TAXONOMY_TREE_AGGREGATE` is one sentinel per tenant for the five tree acts —
matching `inst-tc-writer-lock`'s per-tenant serialization, since a per-node key would promise an
ordering across nodes that nothing enforces — and `metadata_aggregate` is the owning entity, which
is the ordering a metadata write's own `If-Match` actually provides. The sentinel carries UUID
version `0` and every id this gear mints is v7, so it cannot collide with a category, a Product or
a SKU; that is asserted rather than assumed.

**`CategoryDisplayUpdated` and `AttributeDefinitionUpdated` get no aggregate here.** §7 row 15 asks
which orders them and states why it is not a free choice — *"display writes do not take the taxonomy
writer lock, so the tree key would claim a serialization the door does not provide"*. Giving them
the tree key would be that claim, made from an infra module.

**The no-event declaration is a named constant**, `ATTRIBUTE_VALUE_WRITES_EMIT_NO_EVENT`, so a
census looking for what this feature announces finds the absence too. Its reason is C2: those
values are entity content, so the act already announces itself as `ProductHeadSaved` /
`SkuHeadSaved` and at publish as `ProductPublished` / `SkuPublished`; a second event would announce
one act twice and give a consumer no way to tell an independent change from a component of one it
has already seen.

**Each payload type needs its own `SCHEMA_REFS` entry, and that is the half no `match` catches.** A
type added without one compiles clean, `schema_ref_for` answers `None`, and the act rolls back at
runtime rather than at build time. Both halves landed together for exactly that reason.

**Declared ahead of their emitters, and the roster says so.** `events_tests` carries slice `02`'s
six as `THE_TAXONOMY_SIX`, its own array beside `04`'s and `03`'s — folding them into `THE_EIGHT`
would claim §4.5 announces them, and §4.5 announces eight. Nothing enqueues any of the six yet: the
taxonomy's doors have no route (§7 row 16) and the metadata door is blocked on its grant pair. That
array's own doc says **read six as six, not as a mislaid eight**, so a later reader does not go
looking for the two row 15 holds.

**Implements**: `cpt-cf-bss-products-flow-manage-taxonomy`,
`cpt-cf-bss-products-flow-attribute-definitions`, `cpt-cf-bss-products-flow-metadata`

**Constraints**: `cpt-cf-bss-products-constraint-broker-native-events`

**Contract**: `cpt-cf-bss-products-contract-registry-events` — the envelope these eight ride. The
op-envelope id §2 says the taxonomy events carry for approval traceability is a field **beyond**
that contract's base and is declared nowhere; open item 16's door work owes it.

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

**Both renderers ship, and the whole-coordinate sort is the set's own rule — not an excess over
it.** `assignment_collection` sorts by category id, total because
`uq_products_product_category` admits one row per `(product, category)`. `value_collection` sorts
by the **whole coordinate**, since an identifier sort orders groups and not rows.

**§7 row 9 is answered and struck (P-D-103), and this feature had the reasoning wrong.** It
reported the sort as an excess over P-D-29's letter, owed an amendment in two documents — and
checked only that those two documents say it in the same words. It did **not** grep the register
for a rule already in force: **P-D-80** arm 1, *"keyed collections sort by their key"*, had
generalized *"by the collection's own identifier"* to *"a keyed collection sorts by its own key
rendering"* and simply never restated it for P-D-29's two collections. So the decision is a
consistency fix, the code was already conformant, and the lesson is the entry's: grep the register
for the rule before calling something an excess over it.

**What holds the tick now is two concrete gaps, neither of them a question.**

1. **The renderers have no caller.** `products::product_content` builds a frozen version's content
   from head columns alone, so neither collection reaches `products_entity_version.content` today.
   Adding them is a **`DIGEST_VERSION`** matter — `domain::canonical` states that *"the first
   content change after deployment must bump"* and that the present non-bump is correct only
   because **no stored row exists** — plus an entry in each entity's version-content roster
   (`SKU_VERSION_CONTENT_ROSTER` and its Product twin). That is a larger decision than a wiring and
   it is in no group of this strand's current assignment.
2. **The golden vector is a pure-function pin, not a cross-engine one.** The rendering never
   queries, so the only way an engine could change it is the **input order**, which is held against
   every rotation and the reverse of the hard case. What the DoD asks for literally — bytes
   compared across both engines — lives in `tests/postgres_golden_vector.rs`, which neither carries
   these two collections nor is this strand's file.

**No second serialization rule is minted.** `canonical::render_into` sorts every object's keys and
preserves every array's order, recursively, so these functions owe the array order and nothing
else; handing the result to `canonical_rendering` field-orders the elements by the one rule the
gear has. `domain::canonical`'s own doc says the sort is owed *"**here**, rather than at its own
call site"* — that sentence is about a generic array sort, while these two keys are slice-02's, so
they live in slice 02's module and the tension is registered rather than resolved by an edit to a
Foundation file this strand does not own.

The probe is the permutation, not the fixture: four rows of **one** definition, rendered from every
rotation and from the reverse, all byte-identical. A fixture using four *different* definitions
would pass under the very sort row 9 says is wrong.

**Implements**: `cpt-cf-bss-products-flow-assign-categories`,
`cpt-cf-bss-products-flow-attribute-values`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_entity_version`

### Metadata outside version content

- [x] `p2` - **ID**: `cpt-cf-bss-products-dod-metadata-placement`

The system **MUST** keep the metadata map outside frozen published-version content, and a
`CatalogVersion` **MUST** capture it as of its own snapshot instant. A byte-identity probe **MUST**
mutate the map after a snapshot and prove the old snapshot's checksum does not move.

The snapshot builder now writes the `metadata_maps` capture beside the freeze-participant and
reference-producer sets — **three of the seven kinds have readers**. The sentence this paragraph
first carried claimed three of the seven kinds had shipped **sources**, which is false: all seven
source stores ship, and the four uncaptured kinds are owed to their slices' **doors**, not waiting
on a table. A consumer reading a frozen version must not take a missing capture for a missing
store. Each row renders as an object so the
entity coordinate travels with its key, and the rows arrive sorted by
`(entity_kind, entity_id, key)` **from SQL**, because the rendering is checksummed and two engines
must order it identically. The probe writes `team-a`, snapshots, writes `team-b`, and asserts both
the version's checksum and the captured bytes are unmoved — a capture that were a reference would
fail on the second assertion while passing the first.

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
      valid. **This criterion runs against a test-fixture limit value, which is distinct from the
      production default open item 2 leaves unset** — with no value configured nothing is
      exceedable and the criterion cannot execute
- [ ] `name_normalized` is byte-identical across SQLite and Postgres for a case-varied,
      whitespace-varied, NFKC-decomposable category name, and the schema-oracle golden has a
      perturbation case proving it can fail
- [ ] A category display-value write demands the global default-locale value at the first write
      for that definition, and succeeds once it is present
- [ ] Clean operator free text is **admitted** at every door the PII hook guards, and a curated
      allow-list entry for a legitimately person-named product is admitted
- [ ] `GovernedLiveOp` is consumed by `03-sku-classification` without redefinition
- [ ] The writer-lock probe runs on the Postgres tier, which is named, and is not `#[ignore]`d
      without one
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
      corresponding legal act. **The controls are owed per code, not in bulk** — the review of
      2026-08-31 found eight of the sixteen carrying **none**: `TAXONOMY_CYCLE`, `TAXONOMY_LIMIT`,
      `STALE_LIVE_OP`, `CATEGORY_RETIRED`, `DEFINITION_IN_USE`, `STALE_CATEGORY_TOKEN`,
      `METADATA_LIMIT` and `DEFAULT_LOCALE_MISSING` had none, and a blanket criterion is ticked by
      inspection rather than by a test
- [ ] An applied `GovernedLiveOp` against a moved world is refused `STALE_LIVE_OP`, and no partial
      taxonomy mutation is observable
- [ ] No `#[ignore]`d test exists without a CI tier that runs it

## 7. Known unknowns

[`../design/02-taxonomy-attributes.md`](../design/02-taxonomy-attributes.md) §6 carries
**23 open items**, and **18 of them are owned by "this slice" — which is this feature**. The
first version of this section carried four and said the rest stayed with their owners; that was
false, and the three-lens review of 2026-08-31 measured it so. Every item below blocks a named
Definition of Done in §5, and the DoD it blocks is stated so an implementer meets the question
before the code rather than after.

**Five of the twenty-four are now answered and struck in place — 3, 6, 7, 8 and 9 — leaving
nineteen open.** **P-D-88** answered row 7 (the nullable-`UNIQUE` gap) and **P-D-93** row 3 (the
`GovernedLiveOp` seam, whose premise had gone stale on three counts); **P-D-101**, **P-D-102** and
**P-D-103** answered rows 6, 8 and 9 on 2026-09-02, from this feature's own owed register. Each was
answered by a register entry, not here; the struck rows point at it.

**Nothing else here is answered.** A FEATURE artifact records what its design set leaves open;
it does not decide it.

| # | The question | Blocks | Owner |
|---|---|---|---|
| 1 | **Does a brand-less global value survive the scope check on a brand-scoped entity?** Under the gear's stated containment reading an unrestricted coordinate under a restricted entity is *not* contained — so the write `dod-default-locale` demands is the write `dod-value-validators` refuses, and a brand-scoped entity can never publish | `dod-value-validators`, `dod-default-locale` | this feature with 05 |
| ~~2~~ | **Answered by P-D-107 arm 1 (2026-09-03).** Deferred is not absent: a rule with no number is not a rule, and two `DoD`s could not be built at all. Five interim ceilings land in `ProductsConfig` on `bulk_max_rows_per_batch`'s idiom — `taxonomy_max_depth` **8**, `taxonomy_max_children_per_node` **1000**, `metadata_max_keys` **50**, `metadata_max_key_bytes` **128**, `metadata_max_value_bytes` **2048** — each anchored rather than picked: depth bounds the hold on the per-tenant taxonomy writer lock, fan-out is anchored to PRD §7's *"≥ 10K SKUs per tenant"*, and the three metadata caps exist so a map P-D-06 puts outside version content cannot become a shadow content store. Zero is refused at boot. They are **interim** and say so in their own docs; the NFR workshop overrides them by configuration with no code change | ~~`dod-taxonomy-walk`, `dod-metadata-door`~~ | **struck** |
| ~~3~~ | ~~*(seam — see below)*~~ **Answered (owner call, 2026-09-01 — P-D-93): the row's premise is stale on three counts — 05's FEATURE artifact ships, its approval and decision stores ship with their guards, and the in-test approval double the row names as its obligation ships four times over. The envelope is buildable; what stays owed to 05's own door is a test that drives a live op through a REAL approval record.** | no DoD — resolved by P-D-93 | was this feature with 05; **closed** |
| 4 | *(seam — see below)* | `dod-pii-write-block` | `10-retention-erasure` |
| 5 | **Do 02 and 03 admit a `draft` head as a blocking reference?** This feature's removal operand is the non-terminal head, `03-sku-classification`'s is the non-terminal *published* head. **03's half is answered (P-D-89): its operand excludes `draft`, and the row is not a joint decision after all — each slice states its own operand and the divergence is registered on both sides. What is still open is THIS feature's half**: whether the wider operand is right for attribute definitions, whose values have no unit-style publish-time re-recognition to fall back on | `dod-definition-lifecycle` | this feature |
| ~~6~~ | ~~**The coordinate model admits combinations the resolver never visits, and the per-brand default locale has no store.**~~ **Answered (owner call, 2026-09-02 — P-D-101): the default-locale is the *tenant* default only, and *"resolves per brand, falling back to"* is struck from `inst-av-resolve`.** The per-brand default had no store, and a second config value under a step that cannot change *whether* resolution succeeds doubles the exposure that row's own next sentence argues against. What remains is not this row's: `ProductsConfig` carries no `default_locale` field, so the chain's one input has no source and the DoD waits on that rather than on this question | no DoD — resolved by P-D-101 | was this feature; **closed** |
| ~~7~~ | ~~**Both uniqueness guarantees are `UNIQUE` over nullable columns.**~~ **Answered (owner call, 2026-09-01 — P-D-88): roots take a partial `UNIQUE (tenant_id, name_normalized) WHERE parent_id IS NULL`, since a sentinel cannot satisfy the self-FK and `NULLS NOT DISTINCT` has no `SQLite` twin; the value coordinates ship `NOT NULL` with `''` as the stated absence (P-D-39's convention). Both probed on both engines.** Original text: `(tenant_id, parent_id, name_normalized)` does not constrain **root** categories, and the attribute-value tuple does not constrain the **global** coordinate — the one row `dod-default-locale` makes mandatory. The gear's answer elsewhere is NOT NULL with a stated absence value (P-D-39) | no DoD — resolved by P-D-88 | was this feature with the schema owner; **closed** |
| ~~8~~ | ~~**What is the `global` coordinate's key?**~~ **Answered (owner call, 2026-09-02 — P-D-102): absent on *all three* axes, `("", "", "")`.** The decision is **larger than this feature reported it**: the register entry called it a naming defect with both readings closed elsewhere, having read `inst-av-default-locale` as offering only the two this row names. Its full text says *"the default-locale value at the global **(brand-less)** coordinate"*, and that parenthetical is a third, live reading — brand absent, locale present and equal to the tenant default. It is settled on P-D-101's argument: such a value carries the locale that was default *when it was written*, so a config change leaves step 3 matching nothing. The code already had it right; the entry was the weaker artifact | no DoD — resolved by P-D-102 | was this feature; **closed** |
| ~~9~~ | ~~**The frozen-content sort key is not total for attribute values.**~~ **Answered (owner call, 2026-09-02 — P-D-103): the attribute-value set sorts by its *whole coordinate*, and this is a consistency fix rather than an amendment.** **P-D-80** arm 1 — *"keyed collections sort by their key"* — had already generalized *"by the collection's own identifier"* to *"a keyed collection sorts by its own key rendering"*; it simply never restated the rule for P-D-29's two collections. So what ships **is** the set's rule. This feature reported it as an excess owed an amendment in two documents, having checked that those two documents say it in the same words and **not** having grepped the register for a generalization already in force | no DoD — resolved by P-D-103 | was P-D-29's owner; **closed** |
| ~~10~~ | **Answered by P-D-108 arm 1 (2026-09-03): removal is material.** The row is right that the list as written prices the irreversible act below the reversible one, and that is an omission on the slice's own text — `inst-ad-deprecate-then-remove` routes removal **and** the `removed → active` re-listing *"through the same `GovernedLiveOp`"*, and one envelope cannot be material in one direction only, while **P-D-47** makes removal a **state flip** and never a DELETE, exactly as deprecation is. `05 inst-mt-inputs` (d) already registers 02's envelope kinds as material by kind. `inst-ad-governed`'s enumeration gains removal — a `design/02` edit riding the door work | ~~`dod-definition-lifecycle`, `cpt-cf-bss-products-state-attribute-definition`~~ | **struck** |
| 11 | **Does the type-change operand mean the same as the removal operand?** One rule states two: undefined "live values" for the type change, the defined non-terminal head for removal | `dod-definition-lifecycle` | this feature |
| 12 | **Does the PRD carry a live-reference condition for attribute definitions?** The non-terminal-head operand was credited to the PRD and that attribution is struck; it is either inherited from 03 or design-introduced and owed a PRD amendment | `dod-definition-lifecycle` | the PRD owner with this feature |
| ~~13~~ | **Answered by P-D-108 arm 2 (2026-09-03): the label is an attribute value on the definition**, keyed `entity_kind = 'attribute_definition'`. Measured: the definition row carries **no label column at all**, so the non-material *display-label edit* had no target. The label is localized and this gear already owns a localized-value store, a resolver and a fallback chain, and `displayName` is one of the five seeds — so the label rides that, which is `inst-av-category-branch`'s live-entity-content shape applied one level up, a definition having no revisions or versions either. Self-referential and the entry says so; the rejected alternative was a `display_label` column, which for a localized value needs a second store this gear already has | ~~`dod-attribute-definition-table`~~ | **struck** |
| ~~14~~ | **Answered by P-D-107 arm 3 (2026-09-03) — accepted, not closed.** The observation is correct. But `dod-metadata-door` asks for a per-key merge, three caps, an `ENTITY_TERMINAL` refusal and a reduce-from-cap test, and asks for **no** optimistic concurrency; adding a counter column would be adding a requirement no `DoD` carries. The merge narrows the exposure to a **same-key** lost update, since concurrent writes to different keys leave each other untouched. **The residue is real and stays open.** If it must be closed the donor is in this gear already: **P-D-50** gave the live-value door `products_category.mutation_seq` for this exact property — mutable on a published entity — and the cost here is a `metadata_seq` on both head tables, which is why it is not paid on an observation with no requirement behind it | ~~`dod-metadata-door`~~ | **struck (residue recorded)** |
| 15 | **Which aggregate orders `CategoryDisplayUpdated` and `AttributeDefinitionUpdated`?** Neither falls under the taxonomy-tree key or the metadata key. It is not a free choice: display writes do not take the taxonomy writer lock, so the tree key would claim a serialization the door does not provide | `dod-taxonomy-events` | this feature with 12 |
| ~~16~~ | **Answered by P-D-106 (2026-09-03).** One route family each: `POST /bss-products/v1/categories` plus `…/{categoryId}/operations` for the taxonomy ops (one door for four acts, because the design already makes them one envelope, one gate and one apply path); `POST /bss-products/v1/attribute-definitions` plus `…/{key}/operations` for creates, material changes, the state flips and the non-material label edit, materiality judged by the envelope's kind and never by the path; and `PATCH /bss-products/v1/categories/{categoryId}/attribute-values` for the live-value door, which takes the metadata door's `PATCH` shape rather than an envelope because `inst-av-category-branch` makes it non-material with its own `mutation_seq` precondition. **The grants were never in question** — `02` and `05` §3.2 name all three — and they arrive **with** the doors, which is `authz_tests.rs`' own census rule, so A holds a scoped one-time grant over the label block and the three `02` permission rows | ~~`dod-category-live-value-door`, `dod-definition-lifecycle`, `cpt-cf-bss-products-state-attribute-definition`~~ | **struck** |
| 17 | **Four refusals in this feature have no code**: the unresolvable category, the primary/secondary duplicate, the seeded-definition removal, and the removal refused on a non-terminal head carrying a value. So "sixteen codes" is a floor, not a census | `dod-assignment-validators`, `dod-definition-lifecycle`, `dod-taxonomy-errors` | this feature with the error-contract owner |
| 18 | **Are `CATEGORY_RETIRED` and `ATTRIBUTE_DEFINITION_DEPRECATED` 422 or 409?** Both are the target's current state refusing the act, the shape the convention puts at 409 | the API-contract owner | the API-contract owner |
| ~~19~~ | **Answered by P-D-107 arm 2 (2026-09-03).** The four **value** rules run at the live-value door — `AttributeDefinitionKnown`, `AttributeDefinitionActive`, `AttributeValueType`, `AttributeScope` — and the three **assignment** rules do not, having no operand when the subject *is* a category. So the defect the row names is real: a value against a `deprecated` definition would be admitted while the removal guard counts it live. **Plus one the entity door does not run**: `inst-av-category-branch` requires the global default-locale value at the first write of a definition for that category, the write-time analogue of the publish-time check. The door becomes a **fifth caller of the one registration list**, not a second list | ~~`dod-category-live-value-door`, `dod-value-validators`~~ | **struck** |
| ~~20~~ | **Answered by P-D-108 arm 3 (2026-09-03): four kinds — `product`, `sku`, `category`, `attribute_definition` — and a definition does **not** scope to kinds.** The measurement inverted the question: `chk_products_attribute_value_entity_kind` reads **`CHECK (entity_kind <> '')`** on both engines, an open complement admitting any non-empty string, while `products_metadata`'s own constraint enumerates. The set was never enumerated anywhere and a typo'd kind wrote silently into the table this `DoD` calls authoritative. Tightening the constraint to the four rides the door work; migrations here are edited in place, so it is one file and its poison rows — the `CorruptRow` case a closed set makes testable is what the open guard denied | ~~`dod-attribute-value-table`, `dod-metadata-table`~~ | **struck** |
| 21 | **What happens to `products_product_category` rows when a category is physically deleted?** "Unreferenced" reads the Product's lifecycle state, never the link row, so discarded and retired Products still hold rows in the table called the single source of truth; no referential action is stated | `dod-category-assignment-table`, `dod-retire-delete-guard` | this feature with the schema owner |
| 22 | **What does a category rename or delete do to entity versions already frozen against it?** The frozen assignment set holds category **ids**, not copies, so a delete leaves an id resolving to nothing and a rename silently changes what an old version renders. The sibling case is answered explicitly for attribute definitions and not for categories | `dod-version-content-rendering`, `dod-retire-delete-guard` | this feature with 06 and 08 |
| ~~23~~ | **Answered by P-D-100 as amended by P-D-104 (2026-09-02), and built — struck by the lead on the same day the `DoD` was ticked.** The row asked who writes the seeds for a tenant created after the migration. P-D-104's answer: **nobody writes them by migration.** That arm is unbuildable — a per-tenant store needs a tenant list, no gear's schema has a tenant registry, and no migration in this workspace inserts a row at all — and it was redundant, the condition being *"this tenant has no seed rows"* and never *"this tenant is new"*. One writer on the content-save path reaches a pre-deploy tenant just as readily. The row outlived its answer by a day because the `DoD` body was updated and the row was not | ~~`dod-well-known-seeds`~~ | **struck** |
| 24 | **Does slice 09 have an operator free-text `reason` at all?** The PII enumeration names one and 09's own owed item quotes this enumeration as its evidence, so nothing independent establishes the door | `dod-pii-write-block` | 09's owner with this feature |

### The two seams, in full

- ~~**Open item 3**~~ **— answered 2026-09-01 by P-D-93: at this commit 05's FEATURE artifact ships, its approval and decision stores ship, and four in-test approval doubles ship, so the remedy below is an obligation the DoD carries rather than a reason to wait.** Original text: the `GovernedLiveOp` envelope is consumed by `05-governance`. That slice's
  **design exists** (`design/05-governance.md`, whose `inst-gv-scope` §1.4 cites); what does not
  exist is its FEATURE artifact and its code. So this feature can define, submit and re-validate
  an envelope, and **no test can drive one through an approval** — an in-test approval double is
  therefore an obligation on `dod-governed-live-op`, without which every apply-path DoD and
  acceptance criteria 1 and 31 go green on a gate that approves nothing.
  *Owner: this feature with 05.*
- **Open item 4** — the PII detector and its allow-list belong to `10-retention-erasure`, whose
  **design also exists** (`design/10-retention-erasure.md`). Its FEATURE and its code do not, so
  the hook is testable only against a stub — and a stub that refuses every string satisfies both
  `dod-pii-write-block` and acceptance criterion 22. A clean-text positive control is therefore
  part of the criterion, not an extra. *Owner: `10-retention-erasure`.*

### Raised here rather than carried

- **`dod-pii-write-block` cannot be ticked by this feature alone.** Its enumeration reaches six
  doors owned by `01-foundation`, `04-lifecycle`, `05-governance` and `07-reference-signal`. The
  hook and this feature's own two call sites are its testable core; the enumeration is a register
  those features tick. This contradicts §5's "each entry below is separately testable" and the
  contradiction is stated rather than smoothed.
- **The sixteen codes carry no precedence.** Several co-occur on one save —
  `ATTRIBUTE_DEFINITION_DEPRECATED` with `ATTRIBUTE_TYPE_MISMATCH`, `CATEGORY_RETIRED` with
  `PRIMARY_CATEGORY_REQUIRED` — and a caller writing two violations at once has no stated answer
  for which it meets. `01-foundation` §3 states precedence for its `state` phase and registers
  the residue; this feature states none.
- **§1.1 promises "audited" and no DoD delivers it.** The acts this feature owns that emit no
  event — a refused `GovernedLiveOp`, a blocked PII write, a refused retire — are covered by
  `01-foundation`'s audit-trail DoD only if that DoD's operand reaches doors this feature adds,
  which is unstated. *Owner: this feature with 01.*
- **The `retired → active` category edge is undeclared** while the attribute-definition machine
  declares both of its re-listings. The review's implementability lens judged the machine **total
  without it** — `retired` has a defined exit through physical deletion and nothing gets stuck —
  so this is recorded as an asymmetry worth an owner's glance, **not** as an item that binds
  implementation.
