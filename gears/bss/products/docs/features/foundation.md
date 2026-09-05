# Feature: Registry Foundation

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-foundation-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-foundation`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Create a Product](#create-a-product)
  - [Define a SKU](#define-a-sku)
  - [Save an edit](#save-an-edit)
  - [Discard a never-published draft](#discard-a-never-published-draft)
  - [Publish an entity](#publish-an-entity)
  - [Transition an entity](#transition-an-entity)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Validation pipeline](#validation-pipeline)
  - [Idempotency store](#idempotency-store)
  - [Concurrency doors](#concurrency-doors)
  - [Error taxonomy](#error-taxonomy)
- [4. States (CDSL)](#4-states-cdsl)
  - [Catalog Entity State Machine](#catalog-entity-state-machine)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Entity tables and their guards](#entity-tables-and-their-guards)
  - [Published-version history table](#published-version-history-table)
  - [Audit table and the reserved sealing seam](#audit-table-and-the-reserved-sealing-seam)
  - [Append-only head-row guard](#append-only-head-row-guard)
  - [Validation pipeline with registered validators](#validation-pipeline-with-registered-validators)
  - [Error taxonomy as constants on the raising rules](#error-taxonomy-as-constants-on-the-raising-rules)
  - [Name normalization and absolute uniqueness](#name-normalization-and-absolute-uniqueness)
  - [Code reservation, atomic at insert](#code-reservation-atomic-at-insert)
  - [Create doors](#create-doors)
  - [Parent and scope containment at SKU create](#parent-and-scope-containment-at-sku-create)
  - [Authoring head read](#authoring-head-read)
  - [Save door with field-mutability routing](#save-door-with-field-mutability-routing)
  - [Publish door and frozen version history](#publish-door-and-frozen-version-history)
  - [Discard door and transition guard](#discard-door-and-transition-guard)
  - [Idempotency store](#idempotency-store-1)
  - [Concurrency probes on the races](#concurrency-probes-on-the-races)
  - [Outbox eventing through the toolkit](#outbox-eventing-through-the-toolkit)
  - [Audit trail for the acts that emit no event](#audit-trail-for-the-acts-that-emit-no-event)
  - [Actor-ref resolution ahead of the gate](#actor-ref-resolution-ahead-of-the-gate)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

The shared engine every capability of the products gear publishes through: the `Product`/`SKU`
entity model, the two version counters, the lifecycle state-machine floor, the fail-closed
registered-validator pipeline, append-only published-version history, per-row optimistic
concurrency, tenant-scoped idempotency, the broker-native event fan-out, and the append-only
audit trail.

### 1.2 Purpose

Capability features do not own write surfaces. They author draft state through this feature's
doors, register their validation rules into its pipeline, and call its publish path. So nothing
else in the gear can be built until this exists, and everything else in the gear is constrained
by the shape it takes.

The Foundation deliberately owns **no capability policy**. It does not know what a `PlanTier`, a
metering unit, a materiality threshold or a freeze participant is. Where a rule needs one of
those, the rule belongs to the feature that owns the concept and is registered into the pipeline
rather than built into it.

**Requirements** — carried from [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.1 with its scoping
notes intact:

- Whole: `cpt-cf-bss-products-fr-identifier-contract`,
  `cpt-cf-bss-products-fr-idempotent-authoring`,
  `cpt-cf-bss-products-fr-skucode-reservation-concurrency`,
  `cpt-cf-bss-products-fr-registry-eventing-audit`
- Scoped: `cpt-cf-bss-products-fr-create-product` (uniqueness only),
  `cpt-cf-bss-products-fr-define-sku` (identity only),
  `cpt-cf-bss-products-fr-revision-vs-version` (the two counters and the history),
  `cpt-cf-bss-products-fr-lifecycle-transitions` (the machine core),
  `cpt-cf-bss-products-fr-field-mutability-matrix` (the enforcement frame),
  `cpt-cf-bss-products-fr-expected-failure-behavior` (the taxonomy's home),
  `cpt-cf-bss-products-fr-parent-child-integrity` (the interim containment check),
  `cpt-cf-bss-products-fr-event-delivery-resilience` (durable acceptance)
- Non-functional: `cpt-cf-bss-products-nfr-publication-propagation` (the outbox half),
  `cpt-cf-bss-products-nfr-scale-extensibility` (the head/version split and the index shape),
  `cpt-cf-bss-products-nfr-determinism-integrity` (the frame only)
- Surfaces: `cpt-cf-bss-products-usecase-product-sku-editor`,
  `cpt-cf-bss-products-interface-authoring-publish`,
  `cpt-cf-bss-products-contract-registry-events`

**Principles**: `cpt-cf-bss-products-principle-fail-closed`,
`cpt-cf-bss-products-principle-two-version-counters`,
`cpt-cf-bss-products-principle-registered-validators`,
`cpt-cf-bss-products-principle-publish-through-engine`,
`cpt-cf-bss-products-principle-forward-only`.

**Component**: `cpt-cf-bss-products-component-registry-foundation`.
**Sequence**: `cpt-cf-bss-products-seq-authoring-publish`.

### 1.3 Actors

| Actor | Role |
|-------|------|
| `cpt-cf-bss-products-actor-product-manager` | Authors drafts through the write doors; publishes through the slice-05 gate |
| `cpt-cf-bss-products-actor-catalog-admin` | The same doors with wider grants; discard and deferred operations |
| `cpt-cf-bss-products-actor-events-audit` | Receives the outbox fan-out; owns transport — delivery, retry, dead-letter |
| `cpt-cf-bss-products-actor-oss-ams-idp` | Supplies `tenantId`, brand and region claims, and roles; the registry never mutates tenant topology |

### 1.4 References

- [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.1 — the entry this feature realizes
- [`../design/01-foundation.md`](../design/01-foundation.md) — the design slice. **Its §2 and §3
  are the normative step lists and are not copied here.** This document reuses the six `flow-`
  ids and the three `algo-` ids that slice declares, and points at them; re-spelling the 69
  instruction steps that slice declares would fork the set's own instruction register and leave
  two texts where only one can be true. §2 and §3 below therefore carry the actor, the scenarios and the boundary of
  each flow, and the steps stay at their single source.
  - **The one exception is §4.** A state machine's transitions are a template-required
    id-bearing list, and the slice expresses the same content as two instructions
    (`inst-fd-transition-edges`, `inst-fd-terminal`) rather than six rows, so neither can be
    reused per row. §4's `inst-fe-*` ids are that rendering and nothing more; they add no rule
    the slice does not already state.
  - **A consequence worth stating**: because `artifacts.toml` excludes the slices from
    autodetection, an id whose only definition site is a slice resolves nowhere for `cfs`.
    Eighty-six ids are in that position today. This document and its eleven siblings absorb
    seventy-two of them — the fifty-three `flow-` and nineteen `algo-` ids a FEATURE is permitted
    to define — and leave fourteen, all of kind `contract-`, which a FEATURE may not define and
    the PRD does not declare. Those fourteen are cited in prose rather than as tokens wherever
    they appear here.
- [`../PRD.md`](../PRD.md) §6.1, §6.5, §6.7, §6.13; §9.1; §10; §12 AC #1, #2, #5, #13, #14, #27,
  #28, #38, #42
- [`../DESIGN.md`](../DESIGN.md) §1.3 layering, §2.1 principles, §2.2 constraints, §3.5 tables
- [`../DECISIONS.md`](../DECISIONS.md) — P-D-01, P-D-04, P-D-08, P-D-21, P-D-22, P-D-24, P-D-25,
  P-D-26, P-D-28, P-D-30, P-D-31, P-D-32, P-D-33, P-D-34, P-D-36, P-D-37, P-D-38, P-D-39,
  P-D-40, P-D-41, P-D-42, P-D-45, P-D-46, P-D-47, P-D-48, P-D-49, P-D-50
- `gears/bss/pricing` — the pattern donor for the registered-validator pipeline, the append-only
  triggers with column whitelists, and the draft/published partial unique indexes. The outbox is
  **not** taken from it (P-D-22).

## 2. Actor Flows (CDSL)

**Use cases**: `cpt-cf-bss-products-usecase-product-sku-editor`

The step lists live in [`../design/01-foundation.md`](../design/01-foundation.md) §2 — see §1.4
for why they are not repeated. Each flow below names its actor, what success and failure look
like, and where its boundary runs.

### Create a Product

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-create-product`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- A Product is persisted as `draft` with `published_version = 0` and `internal_revision = 1`, and
  a `ProductCreated` outbox row is written in the same transaction
- A repeat of the same request under the same idempotency key replays the stored outcome without
  a second write

**Error Scenarios**:
- The normalized name collides within `(tenant_id, brand_id)` on a non-discarded row —
  `DUPLICATE_NAME`, naming the holder
- `brand_id` names a brand the caller does not hold — `VALIDATION`
- An optional `productCode` collides — `DUPLICATE_CODE`
- The same idempotency key arrives with a different payload — `IDEMPOTENCY_CONFLICT`; with a
  matching payload against an unanswered claim — `IDEMPOTENCY_KEY_IN_FLIGHT`

**Boundary**: this flow writes the entity row and its outbox row **and nothing else**. Content —
category assignments, attribute values, the metering declaration — is written by the save door
under P-D-46. **Whether the create door should also write content is slice 01's open item 11**
and is unresolved; until it is, this flow does not.

### Define a SKU

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-define-sku`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- A SKU is persisted as `draft` under an existing parent, with the slice-03-owned columns present
  but unjudged, and a `SkuCreated` outbox row in the same transaction
- `skuCode` is reserved by the insert itself, admitting exactly one non-discarded holder per
  `(tenant_id, sku_code)`

**Error Scenarios**:
- The parent `productId` does not resolve in the tenant — `VALIDATION`
- The parent is `retired` or `discarded` — `PARENT_TERMINAL`
- The parent holds a live retire intent — `RETIREMENT_PENDING`, raised by a slice-04 validator
  registered on this door, not by the Foundation
- The SKU's scope is not contained in the parent's — `SCOPE_NOT_CONTAINED`
- The `skuCode` race is lost — `DUPLICATE_CODE`, with an audited reason

**Boundary**: typing, `sellable`, `PlanTier`, accounting codes and the metering unit are
`03-sku-classification`'s registered validators; this flow persists the Foundation columns and
runs whatever is registered. Open item 11 applies here as it does to the Product create door.

### Save an edit

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-save-draft`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- A draft, published or deprecated head accepts the edit, `internal_revision` is bumped, and any
  open approval is invalidated
- **The entity's content rows in the owning features' tables — 02's category assignments and
  attribute values, 03's metering declaration — are written by this door in this transaction**
  (P-D-46), together with the `ProductHeadSaved`/`SkuHeadSaved` outbox row
- Bucket-iii and bucket-iv columns are editable while the head is non-terminal; bucket-i and
  bucket-ii columns are editable only while `published_version = 0`

**Error Scenarios**:
- `If-Match` does not match the current internal revision — `STALE_REVISION`
- `If-Match` is absent — `VALIDATION`; the request parsed, so the bare 400 this gear reserves for
  a malformed request does not apply
- A rename colliding on `(tenant_id, brand_id, name_normalized)` — `DUPLICATE_NAME`
- A draft-plane `skuCode` or `productCode` change losing the reservation race — `DUPLICATE_CODE`
- A bucket-i write after first publish, or any `cloned_from` write — `ILLEGAL_FIELD_MUTATION`
- A bucket-ii write after first publish — `ILLEGAL_FIELD_MUTATION`, with the reason naming
  `07-reference-signal`'s correction door rather than forwarding to it
- The head is `retired` or `discarded` — `ENTITY_TERMINAL`

**Boundary**: a save never touches `published_version`.

### Discard a never-published draft

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-discard`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- The head moves to `discarded`, releasing both the name and the code reservations, and the door
  emits `ProductDiscarded`/`SkuDiscarded`

**Error Scenarios**:
- The head has been published — the edge is not admitted, `ILLEGAL_TRANSITION`
- The head is already terminal — `ENTITY_TERMINAL`

### Publish an entity

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-publish`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- A frozen `products_entity_version` row is written **first**, then `published_version` is
  incremented by exactly one in a single head-row `UPDATE` that also carries `internal_revision`,
  `lifecycle_state` where the act is `draft → published`, `composition_pending` where the
  override applied, and any corrected bucket-ii value — all in one transaction with the
  `ProductPublished`/`SkuPublished` outbox row
- The frozen row is the **post-act image**: what the entity looks like after this publish, not
  before it
- The governance gate phase passes trivially where the act is ungated, and consumes a
  `satisfied` approval record where it is not

**Error Scenarios**:
- No `satisfied`, non-superseded approval record pinned to the door's expected revision —
  `APPROVAL_REQUIRED`. **The door evaluates no materiality**; that judgement is
  `05-governance`'s and reaches the door as the presence or absence of a record
- The pinned revision in the `If-Match` header does not match — `STALE_REVISION`
- The head is `retired` or `discarded` — `ENTITY_TERMINAL`; a re-publish is not an edge, so
  `ILLEGAL_TRANSITION` cannot cover it
- A registered validator refuses — the phase's first failing code, with every violation that
  phase collected returned to the caller and one code recorded on the audit row

**Boundary**: this door writes `composition_pending` on a `bundle` publish that carried the
two-person uncomposed-bundle override. Whether the bundle *is* composed is
`03-sku-classification`'s judgement, not this door's.

### Transition an entity

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-transition`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- A `lifecycle_state` change along an admitted edge, bumping `internal_revision` and firing the
  approval-invalidation hook — except where the transition consumes an approval in the same
  transaction, which bumps once with no hook

**Error Scenarios**:
- Any edge outside the admitted list — `ILLEGAL_TRANSITION`
- A `lifecycle_state` write out of `retired` or `discarded` — refused by the trigger whitelist at
  the physical layer, not only by the application

**Boundary**: the guard reaches `lifecycle_state` changes only. A save is not a transition and a
re-publish is not an edge. Policy conditions on the legal edges — two-person un-deprecation,
scheduled retirement, cascades — are `04-lifecycle` and `05-governance` validators registered on
the edge; the floor stays policy-free. Of the five edges only `draft → discarded` emits from this
feature; `04-lifecycle` announces the three deprecation and retirement edges.

## 3. Processes / Business Logic (CDSL)

Step lists live in [`../design/01-foundation.md`](../design/01-foundation.md) §3; see §1.4.

### Validation pipeline

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-pipeline`

**Input**: the acting principal, the request payload, the head row as it now stands, and the set
of validators registered for `(kind, field set)`

**Output**: either an admitted mutation or one audited rejection carrying every violation the
failing phase collected

**Shape**: one pre-pipeline authorization gate, then seven ordered phases — idempotency
resolution, precondition, shape, state, identity, registered validators, governance gate. The run
stops at the first failing phase. Authorization is **not** a phase: it runs before the pipeline
opens, so a denied caller neither consumes an idempotency key nor writes a claim row (P-D-30).

**Registration** is compile-time code — a feature ships its validators with its handler. The
pipeline exposes `rule_names()` for observability only; attribution in a rejection rides the
error code, never the rule name.

### Idempotency store

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-idempotency`

**Input**: `(tenant, endpoint, client key)` and the request payload digest

**Output**: a replayed stored outcome, a claim on a fresh key, or a refusal

**Shape**: a hit with an identical payload replays the stored outcome. A hit with a different
payload fails `IDEMPOTENCY_CONFLICT`. A matching-payload hit on a `claimed` but unanswered key
fails `IDEMPOTENCY_KEY_IN_FLIGHT`. A refusal stores nothing (P-D-38). The claim `INSERT` is
itself the gate and joins the guarded mutation's transaction (P-D-42), so a rollback frees the
key and no separate in-flight column exists. **Expiry is evaluated at claim time, not by a
reaper**, as a compare-and-swap on the held row's own claim stamp (P-D-49). Retention is at least
24 hours and at least the maximum freeze timeout, which `06-catalog-version` exports and this
store reads as config.

### Concurrency doors

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-concurrency`

**Input**: concurrent writes against the same head row, name, code, or idempotency key

**Output**: exactly one winner per contested resource, every loser refused with a named code

**Shape**: four races, each decided under the write rather than by a read-then-act check — the
reservation index on `sku_code` and on `product_code`, the name index, the `If-Match` draft race,
and publish-versus-edit. The claim `INSERT` of the idempotency store is a fifth, and the
expired-key takeover is a sixth.

### Error taxonomy

The taxonomy is specified by [`../design/01-foundation.md`](../design/01-foundation.md) §3.3. Its
`contract-` id is deliberately not cited as a token here: a FEATURE artifact may define only
`flow`, `algo`, `state`, `dod` and `featstatus` ids, and that id's only definition site is a
design slice, which `artifacts.toml` excludes from autodetection — so `cfs` resolves it nowhere
and a citation would be a dangling reference rather than a trace. Fourteen `contract-` ids in
this set are in that position; see §1.4.

**Input**: a refusal raised by a rule

**Output**: a response envelope carrying every violation the failing phase collected, and one
audit row carrying a single code

**Shape**: a code belongs to the rule that raises it, and the rule belongs to a feature (P-D-36).
The seven phases are the execution order — what runs before what, and therefore which refusal a
caller meets first — and are not a taxonomy.

The Foundation-owned codes are `DUPLICATE_NAME`, `DUPLICATE_CODE`, `STALE_REVISION`,
`IDEMPOTENCY_CONFLICT`, `IDEMPOTENCY_KEY_IN_FLIGHT`, `ENTITY_TERMINAL`, `AUDIT_UNAVAILABLE`,
`ILLEGAL_TRANSITION`, `ILLEGAL_FIELD_MUTATION`, `SCOPE_NOT_CONTAINED`, `PARENT_NOT_PUBLISHED`,
`PARENT_TERMINAL`, `INCOMPLETE_ENTITY`, `APPROVAL_REQUIRED` and `VALIDATION`.
`RETIREMENT_PENDING` appears in this feature's response map but is declared by `04-lifecycle`,
which owns both of its raising arms.

**Precedence** exists for the `state` phase alone, which is the only phase that can collect more
than one code: `ENTITY_TERMINAL` → `PARENT_TERMINAL` → `ILLEGAL_TRANSITION` →
`ILLEGAL_FIELD_MUTATION`. `shape` raises one code with many per-field entries and `identity` is
decided under the write and can return only one, so neither needs an ordering. Whether the other
phases owe one is slice 01's open item 2.

## 4. States (CDSL)

### Catalog Entity State Machine

- [ ] `p1` - **ID**: `cpt-cf-bss-products-state-catalog-entity`

The machine is shared by `Product` and `SKU`. This feature owns the edge list and terminality;
every policy condition on a legal edge is a registered validator belonging to another feature.
The six rows below are the template's id-bearing rendering of the slice's
`inst-fd-transition-edges` and `inst-fd-terminal`; see §1.4 for why they carry their own ids.

**States**: `draft`, `published`, `deprecated`, `retired`, `discarded`

**Initial State**: `draft`

**Transitions**:
1. [ ] - `p1` - **FROM** `draft` **TO** `published` **WHEN** the publish door admits the act, which requires the governance gate to pass and a frozen version row to exist for the new `published_version` - `inst-fe-edge-publish`
2. [ ] - `p1` - **FROM** `draft` **TO** `discarded` **WHEN** the discard door is invoked on a head with `published_version = 0`; the name and code reservations are released - `inst-fe-edge-discard`
3. [ ] - `p1` - **FROM** `published` **TO** `deprecated` **WHEN** a `04-lifecycle` validator admits the deprecation; no event is emitted here, `04-lifecycle` announces it - `inst-fe-edge-deprecate`
4. [ ] - `p1` - **FROM** `deprecated` **TO** `published` **WHEN** the two-person un-deprecation ceremony is satisfied through the governance gate phase - `inst-fe-edge-undeprecate`
5. [ ] - `p1` - **FROM** `deprecated` **TO** `retired` **WHEN** a `04-lifecycle` validator admits the retirement against the `07-reference-signal` predicate - `inst-fe-edge-retire`
6. [ ] - `p1` - **NO EDGE** back to `draft` from any state, and **no edge out of** `retired` or `discarded`; both are terminal at the physical layer, the trigger whitelist admitting no `lifecycle_state` write out of them - `inst-fe-states-terminal`

## 5. Definitions of Done

### Entity tables and their guards

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-entity-tables`

The system **MUST** create `products_product` and `products_sku` with their Foundation columns,
on both engines, one migration per table, with the partial unique index on
`(tenant_id, brand_id, name_normalized) WHERE lifecycle_state <> 'discarded'`, the partial unique
index on `(tenant_id, product_code)` where that column is set and the row is not discarded, and
the `ReservationIndex` partial unique index on `(tenant_id, sku_code) WHERE lifecycle_state <>
'discarded'`. `region_scope` and `brand_scope` are `NOT NULL` and default to the empty set, where
the empty set means *unrestricted* (P-D-39). A **schema-oracle golden** for both engines **MUST**
exist from this first migration, together with a perturbation case proving the oracle can fail.

**Implements**: `cpt-cf-bss-products-flow-create-product`, `cpt-cf-bss-products-flow-define-sku`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`,
`cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Entities: `Product`, `SKU`

### Published-version history table

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-version-history-table`

The system **MUST** create `products_entity_version` carrying the frozen content of each published
version together with its `digest_version` constant and its content digest, on both engines. The
canonical serialization **MUST** be pinned by a golden vector asserted byte-identical across
engines under the `digest_version` it was computed with. Frozen rows admit **no UPDATE ever** and
**exactly one DELETE**, under the referential predicate that refuses the delete while a
`products_catalog_version_entry` still references the row (P-D-40).

**Implements**: `cpt-cf-bss-products-flow-publish`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_entity_version`
- Entities: `EntityVersion`

### Audit table and the reserved sealing seam

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-audit-table`

The system **MUST** create `products_audit_log` with the same append-only posture as the entity
tables: a trigger whitelist admitting no UPDATE or DELETE except the sealing seam's one-way arm
and the retention DELETE, the latter as a row-image predicate (P-D-34).

It **MUST** carry the **reserved platform-sealing seam** (P-D-08) from this first migration and
never seal it here: `seal_state` (`NOT NULL`, roster `unsealed | sealed`), `chain_id`, `seq`,
`prev_hash` and `row_hash`, the last four nullable. `seal_state` is written **`unsealed` at
INSERT, always**, in v1 and after activation alike, so the unproven era is queryable rather than
inferred from a deployment date. One `CHECK` ties the group so no half-populated row exists:
`unsealed` implies all four `NULL`; `sealed` implies `chain_id`, `seq` and `row_hash` `NOT NULL`,
a `NULL` `prev_hash` staying legitimate as the segment head. The gear computes no hash and runs
no verification job.

**Implements**: `cpt-cf-bss-products-algo-pipeline`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_audit_log`
- Entities: `AuditRecord`

### Append-only head-row guard

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-append-only-guard`

The system **MUST** enforce the head-row trigger whitelist on **both** engines, admitting exactly:
`lifecycle_state` along the edge list; `published_version` only as `+1` and only where the
matching frozen version row exists; bucket-iii and bucket-iv columns only while the state is
non-terminal; **bucket-i columns only while `published_version = 0` and the state is
non-terminal, and never after first publish**; **bucket-ii columns on those same terms, and after
first publish only in the same statement as a `published_version` bump** — the interim row-image
predicate P-D-41 and P-D-34 pin, standing in for the tighter one `07-reference-signal` owes;
`internal_revision` as `+1` on every admitted update without exception; the row-image-predicated
`deprecation_provenance`, `replaced_by_sku_id` and `composition_pending`; and the update
timestamp. `cloned_from` is admitted in no update at all, and neither are `tenant_id`, the
primary key or `created_by` (P-D-34).

**Bucket-ii has members and bucket-iv has none, and the roster says which rather than reading as
uniformly built.** **Bucket-ii's first members arrived with `03-sku-classification`'s meter pair**
— `metering_unit` and `usage_type_ref`, on **`products_sku` only** — so that table's trigger now
installs the interim predicate and `products_product` still installs none. `design/01` §5's
agreement test moved with them, as its own text said it would: it is a **membership comparison**
for the SKU table and stays the **emptiness assertion** for the Product table, and it is
re-pointed again when 07 supplies the tighter predicate. **Bucket-iv** carries no column on either
table and needs no clause of its own: §4.2's whitelist admits iii and iv **together**, which is
why the same §5 test compares them as one class. `composition_pending`'s
same-statement-as-a-bump predicate is **not** the bucket-ii clause — the column is registered
outside the bucket scheme as mechanical, and it earns its place in the row-image trio above on
its own account.

**The guard judges the data, never the door** (P-D-31). A `CorruptRow`-style probe **MUST** exist
per guarded column class, proving the whitelist refuses a write outside that column's admitted
state.

**Implements**: `cpt-cf-bss-products-state-catalog-entity`,
`cpt-cf-bss-products-flow-save-draft`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`

### Validation pipeline with registered validators

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-validation-pipeline`

The system **MUST** implement the pre-pipeline authorization gate and the seven ordered phases,
stopping at the first failing phase and collecting violations per field within that phase into
one rejection. A feature registers its validators at compile time, keyed by kind plus transition,
target state or field set; execution order inside the phase is registration order, and no rule
may read another rule's verdict. The pipeline **MUST** expose `rule_names()` for observability
only.

**Implements**: `cpt-cf-bss-products-algo-pipeline`

**Constraints**: `cpt-cf-bss-products-constraint-no-commercial-concern`

**Touches**:
- Entities: `ValidationPipeline`, `RegisteredValidator`

### Error taxonomy as constants on the raising rules

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-error-taxonomy`

The system **MUST** declare the Foundation-owned codes as constants on the rules that raise them
(P-D-36), map each to its HTTP status through the RFC-9457 `Problem` ladder, return every
violation the failing phase collected in the response envelope, and record exactly one code on
the audit row (P-D-37). Two codes are exempt from the constant-on-the-raising-rule clause because
their raising rules belong elsewhere: `RETIREMENT_PENDING`, declared by `04-lifecycle`, and
`PARENT_NOT_PUBLISHED`, registered by `04-lifecycle` on the `→ published` target state (P-D-32)
and named here only so the response map is complete. Whether that leaves this feature owning a
code it never raises is slice 01's open item 1.

The `state` phase's precedence **MUST** be `ENTITY_TERMINAL` → `PARENT_TERMINAL` →
`ILLEGAL_TRANSITION` → `ILLEGAL_FIELD_MUTATION`. No other phase carries one.

**Implements**: `cpt-cf-bss-products-algo-pipeline`

**Touches**:
- API: every door in this feature

### Name normalization and absolute uniqueness

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-name-uniqueness`

The system **MUST** compute `name_normalized` application-side as Unicode NFKC, then full
casefold, then trim and collapse internal whitespace to single spaces, so both engines store
identical bytes; and **MUST** enforce uniqueness on `(tenant_id, brand_id, name_normalized)`
through the partial unique index, refusing a collision as `DUPLICATE_NAME` naming the holder.
Region scope plays no part (P-D-04).

**Implements**: `cpt-cf-bss-products-flow-create-product`

**Touches**:
- DB Table: `products_product`
- API: `POST /bss-products/v1/products`

### Code reservation, atomic at insert

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-code-reservation`

The system **MUST** treat the insert itself as the reservation for both `skuCode` and
`productCode`, refusing the loser of a concurrent race as `DUPLICATE_CODE` with an audited
reason, and making the column immutable once `published_version > 0`. A reservation is released
on discard **and by a draft-plane change of the code itself**, the row update freeing the old
value for a concurrent create.

**Implements**: `cpt-cf-bss-products-flow-define-sku`,
`cpt-cf-bss-products-flow-create-product`, `cpt-cf-bss-products-flow-save-draft`

**Touches**:
- DB Table: `products_sku`, `products_product`
- API: `POST /bss-products/v1/skus`, `POST /bss-products/v1/products`

### Create doors

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-create-doors`

The system **MUST** serve `POST /bss-products/v1/products` and `POST /bss-products/v1/skus`
through `OperationBuilder` with authentication and the standard error set, minting the entity id
server-side and refusing a caller-supplied id as `VALIDATION`, validating `brand_id` against the
caller's brand claims (P-D-33), writing `region_scope` and `brand_scope` from the payload when
present, and persisting as `draft` with `published_version = 0` and `internal_revision = 1`
together with the creation outbox row in the same transaction — **and writing nothing else**.
Content rows belong to the save door (P-D-46); slice 01's open item 11 asks whether that should
change, and until it resolves this door does not write them.

**Implements**: `cpt-cf-bss-products-flow-create-product`,
`cpt-cf-bss-products-flow-define-sku`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- API: `POST /bss-products/v1/products`, `POST /bss-products/v1/skus`
- DB Table: `products_product`, `products_sku`

### Parent and scope containment at SKU create

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-containment`

The system **MUST** refuse a SKU whose parent does not resolve in the tenant as `VALIDATION`,
whose parent is `retired` or `discarded` as `PARENT_TERMINAL`, and whose scope is not provably
contained in the parent's as `SCOPE_NOT_CONTAINED`. Containment is defined over restrictions
(P-D-39): an unrestricted parent contains every child, an unrestricted child is contained only by
an unrestricted parent, and between two non-empty sets it is ordinary subset. A SKU whose payload
omits either set **MUST** take the parent's.

**Implements**: `cpt-cf-bss-products-flow-define-sku`

**Touches**:
- API: `POST /bss-products/v1/skus`
- DB Table: `products_sku`, `products_product`

### Authoring head read

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-read-door`

The system **MUST** serve `GET /bss-products/v1/{products|skus}/{id}` under `… × read`, returning
`200` with the head's `internal_revision` as the `ETag`. This is the surface the `If-Match`
precondition of every mutating door depends on: without it an author who has not just written can
obtain no precondition at all (P-D-33). Whether the mutating doors also return the new `ETag` on
success is slice 01's open item 6.

**Implements**: `cpt-cf-bss-products-flow-save-draft`, `cpt-cf-bss-products-flow-publish`

**Touches**:
- API: `GET /bss-products/v1/{products|skus}/{id}`
- DB Table: `products_product`, `products_sku`

### Save door with field-mutability routing

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-save-door`

The system **MUST** serve `PATCH /bss-products/v1/{products|skus}/{id}`, require `If-Match` on the
internal revision, refuse a mismatch as `STALE_REVISION` and an absent precondition as
`VALIDATION`, route every published-state field by its bucket tag, refuse a bucket-i write after
first publish and a `cloned_from` write in any update as `ILLEGAL_FIELD_MUTATION`, refuse a
bucket-ii write after first publish as `ILLEGAL_FIELD_MUTATION` naming the correction door rather
than forwarding to it, and invalidate any open approval on a successful save.

**This door writes the entity's content rows** in the owning features' tables — 02's category
assignments and attribute values, 03's metering declaration — in the same transaction (P-D-46).
The door writes; the owning feature registers the validators; no third registration point exists.

A `BucketRegistry` lookup that finds no tag for a published-state column **MUST fail closed**
(P-D-50) — the write is refused at the door rather than routed to a default bucket.

**Ticked with P-D-142**, clause by clause against the probes: the precondition pair
(`a_save_without_if_match_is_refused_validation`, `a_save_with_a_stale_if_match_is_refused_and_writes_nothing`,
and their SKU twins); bucket routing (`a_bucket_iii_save_on_a_draft_is_admitted_and_bumps_the_revision_once`,
`a_bucket_i_save_is_admitted_before_first_publish_and_refused_after_it` — the SKU twin drives
`sku_code`, the code column); the create-only pair (`a_save_naming_the_lineage_pair_is_refused_by_the_create_only_rule`);
the bucket-ii refusal naming the correction door (the refusal text every re-publish probe reads);
the fail-closed miss (`an_unregistered_field_is_refused_by_the_fail_closed_miss`, P-D-50); the owning
features' rows in the same transaction (`a_save_naming_categories_files_them_in_the_same_transaction`);
and the invalidation of an open approval (`a_frozen_content_write_supersedes_the_open_approval_and_resubmits_nothing`).

**Implements**: `cpt-cf-bss-products-flow-save-draft`

**Touches**:
- API: `PATCH /bss-products/v1/{products|skus}/{id}`
- DB Table: `products_product`, `products_sku`, `products_product_category`,
  `products_attribute_value`
- Entities: `BucketRegistry`

### Publish door and frozen version history

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-publish-door`

The system **MUST** serve `POST /bss-products/v1/{products|skus}/{id}/publish`, pin the revision
through the same `If-Match` header (P-D-33), run the governance gate phase, and in one
transaction: write the frozen `products_entity_version` row **first** — the whitelist admits the
`published_version` bump only where the matching version row already exists, so the order is
forced — carrying the **post-act image**, then perform **one** head-row `UPDATE` carrying
`published_version += 1`, `internal_revision += 1`, `lifecycle_state` where the act is
`draft → published`, `composition_pending` where the override applied, and any corrected
bucket-ii value; then enqueue the publish event.

The door **MUST** take a gate mode as an explicit argument (P-D-30): under **`Gate`** it looks for
a `satisfied` record and consumes it; under **`PreAuthorized(approvalId)`** it verifies the named
record and **does not consume** it, which is what lets `04-lifecycle`'s scheduled-publish runner
drive this same door without the gate seeing an already-`consumed` record and failing the run
terminally. The REST and SDK publish surfaces always call in `Gate` mode.

Where the act is a retirement re-announcement (P-D-48), the door **MUST** re-emit rather than
treat the unchanged content as a no-op.

**Ticked with P-D-142.** The gate phase is the stored host (`api::rest::resolve_host`) rather than
`NoMaterialityPolicyGate`; under `Gate` the door consumes the satisfied record in its own
transaction and under `PreAuthorized` it verifies without consuming
(`api::rest::settle_authorization`). Probes: the pin (`a_publish_with_a_stale_if_match_is_refused_and_writes_nothing`,
`a_publish_without_if_match_is_refused_validation_and_audited`); the one transaction and the two
counters (`a_first_publish_freezes_one_version_row_and_moves_both_counters_by_exactly_one`); the gate
(`a_gate_that_answers_no_refuses_approval_required_and_writes_nothing`,
`a_preauthorized_publish_reaches_the_host_in_that_mode_and_consumes_nothing`,
`two_publishes_off_one_satisfied_record_spend_it_once_and_the_second_is_refused`); the
re-announcement (`a_publish_during_the_lead_window_reannounces_retirement`).

**Implements**: `cpt-cf-bss-products-flow-publish`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- API: `POST /bss-products/v1/{products|skus}/{id}/publish`
- DB Table: `products_entity_version`, `products_product`, `products_sku`
- Entities: `EntityVersion`

### Discard door and transition guard

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-transition-guard`

The system **MUST** serve `POST /bss-products/v1/{products|skus}/{id}/discard`, and **MUST** admit
exactly the five edges of the state machine, refusing anything else as `ILLEGAL_TRANSITION` and
any head write on a `retired` or `discarded` row as `ENTITY_TERMINAL` (P-D-25, P-D-32). Every
transition bumps `internal_revision` and fires the approval-invalidation hook, except a
transition that consumes an approval in the same transaction, which bumps once with no hook
(P-D-26, P-D-34).

**Implements**: `cpt-cf-bss-products-flow-discard`,
`cpt-cf-bss-products-flow-transition`, `cpt-cf-bss-products-state-catalog-entity`

**Touches**:
- API: `POST /bss-products/v1/{products|skus}/{id}/discard`
- DB Table: `products_product`, `products_sku`

### Idempotency store

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-idempotency-store`

The system **MUST** create `products_idempotency` and resolve `(tenant, endpoint, client key)` as
the first pipeline phase of every mutating flow that carries an `Idempotency-Key`, skipping the
phase on a keyless request rather than failing it (P-D-34).

- The claim `INSERT` **is** the gate and **MUST** join the guarded mutation's transaction
  (P-D-42), so a rollback frees the key; no separate in-flight column exists.
- `endpoint` **MUST** be the concrete resource path, not the route template, with the three
  reserved `internal:` lane names for the non-HTTP callers (P-D-42).
- The payload hash **MUST** be taken over the canonical rendering of the **parsed** request,
  excluding the precondition header (P-D-34).
- Expiry **MUST** be evaluated at claim time as a compare-and-swap on the held row's own claim
  stamp (P-D-49); the loser is refused `IDEMPOTENCY_KEY_IN_FLIGHT` having executed nothing.
- A refusal stores nothing (P-D-38); the response columns carry a success's answer only.
- Retention is at least 24 hours and at least the configured maximum freeze timeout.

**Implements**: `cpt-cf-bss-products-algo-idempotency`

**Touches**:
- DB Table: `products_idempotency`
- Entities: `IdempotencyRecord`

### Concurrency probes on the races

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-concurrency`

The system **MUST** decide each contested resource under the write rather than by a read-then-act
check, and the implementation **MUST** carry real concurrency probes — not read-then-assert — for
the reservation index on both code columns, the name index, the `If-Match` draft race,
publish-versus-edit, the claim insert of the idempotency store, and the expired-key takeover.

**Implements**: `cpt-cf-bss-products-algo-concurrency`

**Touches**:
- DB Table: `products_product`, `products_sku`, `products_idempotency`

### Outbox eventing through the toolkit

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-outbox-eventing`

The system **MUST** enqueue events through `toolkit_db::outbox` rather than a gear-private outbox
table (P-D-22), publish through the event-broker SDK's outbox-backed producer (P-D-47), carry the
broker-native envelope with no CloudEvents field anywhere in the payload path (P-D-01), and
obtain per-tenant ordering from the broker's partition selection. Emission success **MUST NOT** be
reported before the event is durably accepted.

This feature owns **eight** events: `ProductCreated`, `SkuCreated`, `ProductHeadSaved`,
`SkuHeadSaved`, `ProductPublished`, `SkuPublished`, `ProductDiscarded`, `SkuDiscarded`. All eight
**MUST** carry the same body core — `{tenantId, entityKind, entityId, internalRevision,
lifecycleState}` — with the two publish events additionally carrying `publishedVersion`. The
envelope **MUST** carry correlation and causation ids, a versioned schema reference, and the
acting principal's `actor_ref` — **each on the envelope where the transport has a slot for it and
in the payload where it does not** (**P-D-51**, on P-D-01's own "envelope-agnostic" wording).
Against the broker-native envelope that means: the schema reference is the event's `type_id`, the
correlation id its `trace_parent`, the ordering key its partition selection; and the **causation
id** and **`actor_ref`** ride the payload, the broker's `Event` having no field for either. A value
that moves onto the envelope once the transport grows a slot does not break this clause; a value
dropped does.

**Events carry the pseudonymous `actor_ref` only, never a direct operator identity.** Any column
holding an operator identity is named `*_actor_ref`, and only `products_identity_ref` declares one
(P-D-45).

The outbox half of the sub-3-second publication-propagation budget belongs here. **The probe
exists and reports a number; the 01/06 split of that budget is still open at the PRD owner.**

`infra::broker`'s `the_outbox_half_of_the_propagation_budget_is_measured` times the path this gear
owns — enqueue, the sequencer, the leased processor's pickup, the SDK's publish call — from the
enqueue to the event being readable at the broker, and **asserts no budget**, because the number a
budget splits into is the owner's to set and a threshold invented in a test would guard nothing.
On the author's machine on 2026-08-30 it reported **single-digit milliseconds** against an
in-process `MockBroker`. That figure is a **floor**, not a prediction: it carries no network, no
disk beyond the local `SQLite` outbox and no ingest work. What it establishes is that this half is
three orders of magnitude below the whole budget under zero-latency transport, so the split's
difficulty is on the broker side rather than here — which is the input the split needs and which
no measurement previously supplied.

**Ticked on a measurement, with no new code, and here is what was measured** (2026-09-01): the
pipeline is `toolkit_db::outbox` with the SDK's outbox-backed producer when an `EventBrokerApi` is
in the hub, and the holding processor only otherwise — `require_broker` refuses to boot into that
mode rather than accumulating undelivered catalog events; the queue takes `Partitions::of`, which is
where per-tenant ordering comes from; all **eight** payload tokens carry versioned schema references
in one `SCHEMA_REFS` roster, which `events_tests` checks for coverage against a hand-written list
precisely so a ninth event cannot reach the wire without one; `EventBodyCore` is the five fields and
`publishedVersion` sits **outside** it, which is §4.5's own reading; the envelope carries the
correlation id as `trace_parent` and the schema reference as the `type_id`, with the causation id
and `actor_ref` in the payload where the transport has no slot (**P-D-51**); and
`infra::broker::the_outbox_half_of_the_propagation_budget_is_measured` exists, reports a number and
asserts no threshold. The DoD's one remaining sentence — the 01/06 split of the budget — is the PRD
owner's and is not this DoD's obligation.

**Implements**: `cpt-cf-bss-products-flow-create-product`,
`cpt-cf-bss-products-flow-define-sku`, `cpt-cf-bss-products-flow-save-draft`,
`cpt-cf-bss-products-flow-publish`, `cpt-cf-bss-products-flow-discard`

**Constraints**: `cpt-cf-bss-products-constraint-broker-native-events`,
`cpt-cf-bss-products-constraint-gts-types-not-instances`

**Touches**:
- DB Table: the toolkit outbox

### Audit trail for the acts that emit no event

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-audit-trail`

The system **MUST** write an append-only audit row for every refusal, every read under elevation,
and every committed act the design declares emits no broker event — and for nothing else, the
event being the success-path audit record (P-D-21). Two carve-outs apply: `AUDIT_UNAVAILABLE`
itself, whose own row is by construction the one that could not be written and which is recorded
out-of-band as log and metric (P-D-34); and `10-retention-erasure`'s erasure act, which is
eventless only for events carrying identity and emits a minimal `ActorErased` of its own.

Three transaction disciplines, and they differ:

- **A refusal's row** commits **in its own transaction**, independently of the refused mutation,
  and is a **precondition of answering the caller**: if it cannot be written the door answers
  `AUDIT_UNAVAILABLE` (503) and does not report the domain refusal.
- **A committed eventless act's row** commits **inside the guarded mutation's transaction**.
- **An elevated read's row** commits in its own transaction and is a precondition of serving the
  read.

**Audit rows carry the pseudonymous `actor_ref` only, never a direct operator identity.**

**Implements**: `cpt-cf-bss-products-algo-pipeline`

**Touches**:
- DB Table: `products_audit_log`
- Entities: `AuditRecord`

### Actor-ref resolution ahead of the gate

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-actor-ref`

The system **MUST** resolve the acting principal to an `actor_ref` through the identity-ref map,
**in its own transaction, before the authorization gate and any phase that can refuse** (P-D-26),
minting one on a principal's first appearance with no live ref and emitting no event for the
mint. A refusal rolls the door's transaction back while its audit row commits independently and
requires an `actor_ref`, so a first-time principal whose opening act is refused must still have a
ref to attribute it to. Resolution advances `last_seen_at`, which `10-retention-erasure`'s
age-based erasure reads.

**Implements**: `cpt-cf-bss-products-flow-create-product`

**Touches**:
- DB Table: `products_identity_ref`
- Entities: `IdentityRefMap`

## 6. Acceptance Criteria

*Ticks measured clause by clause at **P-D-156** (2026-09-05); the criterion-to-probe map is in that
entry. A box left open names a clause no probe asserts yet.*

- [x] Creating a Product persists it as `draft` with `published_version = 0` and
      `internal_revision = 1`, writes exactly one `ProductCreated` outbox row in the same
      transaction, and writes no content row
- [ ] A second Product whose normalized name equals an existing non-discarded row's within the
      same `(tenant_id, brand_id)` is refused `DUPLICATE_NAME` and the response names the holder
- [x] Discarding a draft releases its name, and a subsequent create under that name succeeds
- [x] Changing a draft's `skuCode` frees the old code for a concurrent create
- [x] Two concurrent creates racing the same `skuCode` produce exactly one row; the loser is
      refused `DUPLICATE_CODE` and an audit row records the refusal
- [x] The same race is proven with a real concurrency probe, not a read-then-assert
- [ ] `name_normalized` is byte-identical across SQLite and Postgres for a case-varied,
      whitespace-varied, NFKC-decomposable input
- [x] A `GET` on a head returns an `ETag` that a subsequent `PATCH` accepts as `If-Match`
- [x] A save without `If-Match` is refused `VALIDATION`; a save with a stale `If-Match` is refused
      `STALE_REVISION`
- [x] A save writes the entity's category assignments and attribute values in the same transaction
      as the head-row update, and a rollback leaves neither
- [x] A bucket-i write on a head with `published_version > 0` is refused
      `ILLEGAL_FIELD_MUTATION`, and the same write on a head with `published_version = 0` succeeds
- [x] A bucket-ii write after first publish is refused `ILLEGAL_FIELD_MUTATION` and the reason
      names the correction door; the same column moves when the statement also bumps
      `published_version`
- [x] An untagged published-state column is refused at the head door rather than routed to a
      default bucket
- [x] A code column is refused after first publish
- [ ] Every refusal enumerated in §2 has a paired positive control proving the door admits the
      corresponding legal act
- [x] Publishing writes one frozen version row **before** the head-row update, increments
      `published_version` by exactly one in a single `UPDATE`, and the frozen row is thereafter
      refused by any update on both engines
- [x] A `PreAuthorized` publish against an already-`consumed` approval record succeeds
- [x] Deleting a frozen version row that a catalog-version entry still references is refused **by
      the guard**, proven with the garbage collector bypassed
- [ ] A `CorruptRow`-style probe exists per guarded column class, proving the whitelist refuses a
      write outside that column's admitted state
- [x] The `BucketRegistry` tag map and the trigger's column classes name the same columns in the
      same classes, with iii and iv asserted as one combined class, and no published-state column
      is named by neither artifact
- [x] A canonical-serialization golden vector for frozen version content and its digest is
      byte-identical on both engines and pins the `digest_version` constant it was computed under
- [ ] A schema-oracle golden exists for both engines from the first migration, together with a
      perturbation case proving the oracle can fail
- [x] An audit-log `INSERT` always lands `unsealed`, and a second `unsealed → sealed` update on an
      already-sealed row is refused
- [x] `internal_revision` bumps on every admitted write, transitions and publishes included
- [x] A non-admitted edge is refused `ILLEGAL_TRANSITION`, and a transition out of `retired` or
      `discarded` is refused at the physical layer with the application check bypassed
- [ ] Each of the eight named events is emitted by its door and carries the shared body core; the
      two publish events additionally carry `publishedVersion`
- [ ] No event body and no audit row carries a direct operator identity; both carry `actor_ref`
- [ ] An audit row is written for every refusal and is readable by `(tenant, subject, error_code)`
- [x] Replaying a request under the same idempotency key with an identical payload returns the
      stored outcome and writes nothing; a differing payload is refused `IDEMPOTENCY_CONFLICT`
- [x] Two concurrent claims on one expired key execute the guarded mutation exactly once
- [x] A keyless mutating request skips the idempotency phase rather than failing it
- [x] An authorization denial consumes no idempotency key and writes no claim row, and its audit
      row carries a resolved `actor_ref`
- [ ] No `#[ignore]`d test exists without a CI tier that runs it *(The tier: `make test-products-pg` — `DESIGN.md` §3.8's runbook; on demand by P-D-132, so this box stays open by the owner's decision — P-D-161.)*

## 7. Known unknowns

Slice 01 carried one standing risk and twelve open items; **as of P-D-123 (2026-09-03) ten are
answered and two are routed** (item 4 to `05`, item 7 to strand C's runner). The five that bound
implementation were restated here and are struck below with their answers; the rest are at
`design/01` §6.

- ~~**Open item 1**~~ **Answered (P-D-97): `01` declares `PARENT_NOT_PUBLISHED`, `04`'s parent guard raises it as a publish-phase continuation.** *(stood as:)* — whether this feature should own `PARENT_NOT_PUBLISHED`, a code whose raising
  rule belongs to `04-lifecycle`. *Owner: this feature with 04.*
- ~~**Open item 2**~~ **Answered (P-D-123, 2026-09-03): the audit row stores the refusal's own code, and the order of evaluation is the precedence; no precedence beyond the four `state` codes is owed.** *(stood as:)* — whether any phase other than `state` owes a code precedence. *Owner: this
  feature.*
- ~~**Open item 6**~~ **Answered by the crate (P-D-123, 2026-09-03): every mutating door returns the new `ETag`** — create, `GET`, save and the head acts, both entities. *(stood as:)* — whether the mutating doors return the new `ETag` on success, or whether an
  author must re-read. *Owner: this feature.*
- ~~**Open item 11**~~ **Answered (P-D-123, 2026-09-03): no — create stays entity-only; the clone door is the second content writer, inside its own transaction on the save door's terms, so 11's `internal_revision = 1` holds.** *(stood as:)* — whether the create door writes content as the save door does. Until it
  resolves, create writes the entity row and its outbox row only, which leaves an entity whose
  content arrives at creation — `11-clone`'s case — with no admitted writer. *Owner: this feature
  with 11.*
- ~~**Open item 12**~~ **Answered (P-D-51, 2026-08-30): `gts.cf.core.events.subject.v1~cf.bss.products.product.v1` and `…sku.v1`**; the broker-side registration is the half still owed. *(stood as:)* — which GTS type the envelope's `subject_type` names for a Product or a SKU.
  *Owner: this feature.*

The standing risk and the remaining items are stated in
[`../design/01-foundation.md`](../design/01-foundation.md) §6 and are not duplicated here.
