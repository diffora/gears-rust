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

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-category-assignment-table`

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
`(locale?, region?, brand?)` — the optionality markers are `design/02` §4.1's and are load-bearing,
being the exact subject of open items 6 and 7 — and the value, with a `UNIQUE` constraint over the
full coordinate tuple. **That constraint does not today constrain the mandatory global coordinate**
(open item 7): both engines treat NULLs as distinct. For Product and SKU rows the table **MUST** hold the current head state only, history
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

**None of these is answered here.** A FEATURE artifact records what its design set leaves open;
it does not decide it.

| # | The question | Blocks | Owner |
|---|---|---|---|
| 1 | **Does a brand-less global value survive the scope check on a brand-scoped entity?** Under the gear's stated containment reading an unrestricted coordinate under a restricted entity is *not* contained — so the write `dod-default-locale` demands is the write `dod-value-validators` refuses, and a brand-scoped entity can never publish | `dod-value-validators`, `dod-default-locale` | this feature with 05 |
| 2 | **The taxonomy *and metadata* limits have no interim default anywhere.** `nfr-scale-extensibility` defers the values to the NFR workshop and PRD §17.1 carries neither a taxonomy-limits row nor a metadata-caps row. Both `TAXONOMY_LIMIT` and `METADATA_LIMIT` are rules with no number | `dod-taxonomy-walk`, `dod-metadata-door` | the §17.1 policy owner |
| 3 | *(seam — see below)* | `dod-governed-live-op` | this feature with 05 |
| 4 | *(seam — see below)* | `dod-pii-write-block` | `10-retention-erasure` |
| 5 | **Do 02 and 03 admit a `draft` head as a blocking reference?** This feature's removal operand is the non-terminal head, `03-sku-classification`'s is the non-terminal *published* head. Slice 03 §6 registers the divergence as unanswered and jointly owned | `dod-definition-lifecycle` | this feature with 03 |
| 6 | **The coordinate model admits combinations the resolver never visits, and the per-brand default locale has no store.** The chain's third step needs one; the only store named is the tenant default | `dod-locale-resolver` | this feature |
| 7 | **Both uniqueness guarantees are `UNIQUE` over nullable columns.** `(tenant_id, parent_id, name_normalized)` does not constrain **root** categories, and the attribute-value tuple does not constrain the **global** coordinate — the one row `dod-default-locale` makes mandatory. The gear's answer elsewhere is NOT NULL with a stated absence value (P-D-39) | `dod-category-table`, `dod-attribute-value-table`, `dod-name-in-parent` | this feature with the schema owner |
| 8 | **What is the `global` coordinate's key?** If it is keyed on the default locale it is anchored on the config value the §2 boundary argues against; if it means all three coordinates absent, "a default-locale value at the global coordinate" names a coordinate that carries no locale | `dod-default-locale`, `dod-locale-resolver` | this feature |
| 9 | **The frozen-content sort key is not total for attribute values.** Sorting by the attribute id orders groups, not rows, so two engines can serialize one content two ways — the failure the rule exists to prevent. Amending it is a register change: P-D-29 and `01-foundation` §4.3 state it in the same words | `dod-version-content-rendering` | P-D-29's owner |
| 10 | **Is definition removal a material op?** Removal is absent from the material-op enumeration while deprecation, the step before it, is in it. So §4's `inst-de-edge-remove` carries no approval condition while the re-listing edge does — the destructive edge is cheaper than the restorative one | `dod-definition-lifecycle`, `cpt-cf-bss-products-state-attribute-definition` | this feature |
| 11 | **Does the type-change operand mean the same as the removal operand?** One rule states two: undefined "live values" for the type change, the defined non-terminal head for removal | `dod-definition-lifecycle` | this feature |
| 12 | **Does the PRD carry a live-reference condition for attribute definitions?** The non-terminal-head operand was credited to the PRD and that attribution is struck; it is either inherited from 03 or design-introduced and owed a PRD amendment | `dod-definition-lifecycle` | the PRD owner with this feature |
| 13 | **Where does a definition's display label live?** Label edits are a named non-material op and the definition roster carries no label column, so the op has no target | `dod-attribute-definition-table` | this feature |
| 14 | **Two concurrent metadata writes both pass their precondition.** Metadata rides the entity row's `If-Match` and by P-D-06 bumps no version, so the token never moves and the second write silently overwrites the first, on a map with no history between snapshots | `dod-metadata-door` | this feature |
| 15 | **Which aggregate orders `CategoryDisplayUpdated` and `AttributeDefinitionUpdated`?** Neither falls under the taxonomy-tree key or the metadata key. It is not a free choice: display writes do not take the taxonomy writer lock, so the tree key would claim a serialization the door does not provide | `dod-taxonomy-events` | this feature with 12 |
| 16 | **Three doors name no REST path and one names no grant pair** — the taxonomy-op door, the attribute-definition door and the category live-value door. Only the metadata door carries both | `dod-category-live-value-door`, `dod-definition-lifecycle`, `cpt-cf-bss-products-flow-manage-taxonomy` | this feature with 05 |
| 17 | **Four refusals in this feature have no code**: the unresolvable category, the primary/secondary duplicate, the seeded-definition removal, and the removal refused on a non-terminal head carrying a value. So "sixteen codes" is a floor, not a census | `dod-assignment-validators`, `dod-definition-lifecycle`, `dod-taxonomy-errors` | this feature with the error-contract owner |
| 18 | **Are `CATEGORY_RETIRED` and `ATTRIBUTE_DEFINITION_DEPRECATED` 422 or 409?** Both are the target's current state refusing the act, the shape the convention puts at 409 | the API-contract owner | the API-contract owner |
| 19 | **Which of the value validators run at the category live-value door?** They are registered on the entity draft-save door, and the category branch writes through a different one — so a category value against a `deprecated` definition is admitted today, while the removal guard counts an active category as a value-carrying head | `dod-category-live-value-door`, `dod-value-validators` | this feature with 01 |
| 20 | **What `entity_kind` values does each table admit, and does a definition scope to entity kinds?** The set enumerates them nowhere, while the attribute-value table demonstrably admits `category` and the only named metadata door admits `{products\|skus}` | `dod-attribute-value-table`, `dod-metadata-table` | this feature |
| 21 | **What happens to `products_product_category` rows when a category is physically deleted?** "Unreferenced" reads the Product's lifecycle state, never the link row, so discarded and retired Products still hold rows in the table called the single source of truth; no referential action is stated | `dod-category-assignment-table`, `dod-retire-delete-guard` | this feature with the schema owner |
| 22 | **What does a category rename or delete do to entity versions already frozen against it?** The frozen assignment set holds category **ids**, not copies, so a delete leaves an id resolving to nothing and a rename silently changes what an old version renders. The sibling case is answered explicitly for attribute definitions and not for categories | `dod-version-content-rendering`, `dod-retire-delete-guard` | this feature with 06 and 08 |
| 23 | **Who writes the well-known seeds for a tenant created after the migration?** "Seeded by migration, per tenant bootstrap" names two code paths, and a migration cannot create rows for tenants that do not yet exist | `dod-well-known-seeds` | this feature with 01 |
| 24 | **Does slice 09 have an operator free-text `reason` at all?** The PII enumeration names one and 09's own owed item quotes this enumeration as its evidence, so nothing independent establishes the door | `dod-pii-write-block` | 09's owner with this feature |

### The two seams, in full

- **Open item 3** — the `GovernedLiveOp` envelope is consumed by `05-governance`. That slice's
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
