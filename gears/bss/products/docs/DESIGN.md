<!-- Related: ./PRD.md, ./DECISIONS.md, ./design/ | Owners: BSS Product Catalog team -->

# Technical Design — Product & SKU Registry

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Interactions & Sequences](#34-interactions--sequences)
  - [3.5 Database schemas & tables](#35-database-schemas--tables)
- [4. Decision register & joint contracts](#4-decision-register--joint-contracts)
- [5. Traceability (PRD §6 → slice)](#5-traceability-prd-6--slice)
- [6. Status](#6-status)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

The **products** gear is the BSS catalog **registry**: the System of Record for Products, SKUs,
categories, attributes/localization, and immutable `CatalogVersion` snapshots — *what can be
sold and how it is described, classified, versioned, and published*. It owns no commercial
concern: Plan/Price/composition are the pricing gear's, evaluation is rating's (PRD §2.1
boundary). Requirements live in [`PRD.md`](./PRD.md) (sign-able as of 2026-08-25 — all §15
gates closed, veto register clean); decisions in [`DECISIONS.md`](./DECISIONS.md) (P-D-NN;
joint contracts D-46/D-47 live in the pricing register).

The design follows the **foundation-plus-handlers** pattern proven by the pricing gear: one
shared engine slice ([`design/01-foundation.md`](./design/01-foundation.md)) owns the entity
model, identity, the lifecycle state machine, the fail-closed validation pipeline, versioning,
idempotency, eventing, and audit; every capability slice is a handler that authors draft state,
**registers its validation rules** with the pipeline, contributes read-model fields, and
publishes through the Foundation. The Foundation carries no capability policy — it does not
know what a `PlanTier` or a metering unit is.

### 1.2 Architecture Drivers

#### Functional Drivers

- Financial-grade governance: two-person/SoD approvals pinned to stored revision snapshots
  (PRD §6.7); forward-only lifecycle, no unpublish (§6.5).
- Byte-identical reproducibility: `CatalogVersion` full snapshots + checksum + freeze protocol
  (§6.6) — the anchor posted invoices and contracts resolve against.
- Stable downstream identity: immutable `skuId`, permanently reserved `skuCode` (§6.1) — every
  sibling gear binds to it.
- Registry-upstream-of-commercial: a SKU publishes before any plan references it; mechanical
  `CatalogVersion` increments serve pricing's D-47 lanes (P-D-02).

#### NFR Allocation

Read p95 < 100 ms / ≥ 2 000 QPS per tenant partition and the staleness contract land on the
slice-08 read models; the < 3 s propagation and < 5 s posting-safe budgets on the slice-01
outbox + slice-06 freeze machine; ≥ 11-nines snapshot durability and cold-resolution p95 < 2 s
on slice 06/10 storage posture; availability split (read 99.9 % / write 99.5 %) on the
read-model/write-path separation (PRD §7).

#### Key decisions

P-D-01 broker-native envelope · P-D-02 mechanical increments, governance at entity publish ·
P-D-03 `SkuReferenceCount` v1 producer = {pricing} · P-D-04 absolute name uniqueness ·
P-D-05 `usageTypeRef` resolvability-only. Joint: D-46 (`sellable`), D-47 (increment lanes +
retirement contract) — pricing register.

### 1.3 Architecture Layers

Standard ToolKit gear, mirroring the sibling BSS gears:

- **`products-sdk`** (`cf-gears-bss-products-sdk`) — consumer-facing contract crate: typed
  client traits, read DTOs (the shapes pricing's `ProductCatalog` trait and the studio consume —
  incl. the `sellable` member pricing's `CatalogSku` currently lacks), error taxonomy, event
  payload types.
- **`products`** (`cf-gears-bss-products`) — the gear: `contract` (REST `/bss-products/v1/…`,
  OpenAPI), `api` (OperationBuilder handlers), `domain` (entities, state machine, validation
  pipeline, uniqueness/scope rules), `infra` (SecureORM repositories, migrations, outbox,
  read-model projector).
- **Identity**: GTS ids under `gts.cf.bss.products.*`; tables `products_*`; dual-engine storage
  (SQLite + Postgres), one migration per table, schema-oracle goldens from day one.

#### Design set (ordered by implementation phase)

| Doc | Content (one line) | PRD §6 | Phase | Depends on |
|-----|--------------------|--------|-------|------------|
| `01-foundation` | shared engine: entities, identity, revision-vs-version, state machine, validation pipeline, idempotency/ETag, eventing (P-D-01), audit | 6.1 core, 6.5 core, 6.7, 6.13 | 0/1 | — |
| `02-taxonomy-attributes` | categories (governed live), attribute definitions + i18n fallback, metadata map, well-known seeds | 6.2, 6.4 | 1 | 01 |
| `03-sku-classification` | SKU typing, `sellable`, `PlanTier`, accounting codes, metering unit + `usageTypeRef` (P-D-05), de-listing | 6.3 | 1 | 01, 02 |
| `04-lifecycle` | deprecation provenance, parent-child + cascade-retire, scheduled publish/retirement, `replacedBy`, containment (P-D-04 residue) | 6.5 | 1/2 | 01 (05, 07 at integration) |
| `05-governance` | materiality matrix, two-person + FinanceReviewer, **stored** pinned approval snapshot, RBAC, break-glass | 6.7, 6.8 | 1/2 | 01 |
| `06-catalog-version` | CatalogVersion machine (P-D-02, D-47 lanes), checksum/reproducibility, freeze protocol, `compositionPending`, version diff | 6.6 | 2 | 01, 02, 03, 04, 05 |
| `07-reference-signal` | `SkuReferenceCount` watermarks, 3-state predicate, producer registration (P-D-03), fresh-zero corrections + tripwire | 6.1 signal, 6.13 | 2 | 01, 04 |
| `08-read-models` | cache-first browse/search, per-state visibility, `asOfCatalogVersion`, degradation, NFR budgets | 6.8, §7 | 2 | 01, 06 |
| `09-bulk-promotion` | bulk import/export, two-phase deps, change report, environment promotion (AC #33a) | 6.9 | 2/3 | 01, 05 |
| `10-retention-erasure` | retention classes, pseudonymization, PII write-block, retention↔grandfathering coupling | 6.11 | 3 | 01, 06 |
| `11-clone` | clone/templating with live re-validation (p3) | 6.10 | 3 | 01–04 |
| `12-consumer-contracts` | seam-suite spec, event schema versioning/replay/bootstrap, §9 interfaces, traceability check | 6.12, §9 | 2/3 | 01, 03, 06, 07 |

#### Dependency order

Phase 0/1: 01 → (02, 03, 04, 05 in parallel). Phase 2: 06 (needs 04+05), 07 (needs 04), 08
(needs 06), 12 starts once 03/06/07 fix their shapes. Phase 2/3: 09; Phase 3: 10, 11. The
numeric prefix is implementation order, not the PRD subsection number.

## 2. Principles & Constraints

### 2.1 Design Principles

#### Fail-closed everywhere

- [ ] `p1` - **ID**: `cpt-cf-bss-products-principle-fail-closed`

Every enumerated failure of PRD AC #38 maps to a named error code (slice 01 §3.3 taxonomy); no
partial application; every rejection audited with reason.

#### Two version counters, never conflated

- [ ] `p1` - **ID**: `cpt-cf-bss-products-principle-two-version-counters`

The internal revision moves on every save and backs optimistic concurrency; the published
version moves only on publish and is the only thing a consumer or `CatalogVersion` may
reference (PRD `fr-revision-vs-version`).

#### Forward-only lifecycle

- [ ] `p1` - **ID**: `cpt-cf-bss-products-principle-forward-only`

No `unpublish`, no in-place rollback; `retired`/`discarded` terminal — physically, via the
append-only trigger whitelist; revival is clone (slice 11).

#### Governance attaches to the human act

- [ ] `p1` - **ID**: `cpt-cf-bss-products-principle-governance-at-entity-publish`

(P-D-02.) Entity publish is where approvals, overrides, and materiality run; `CatalogVersion`
increments and read-model projections are mechanical consequences.

#### Publish through the engine

- [ ] `p1` - **ID**: `cpt-cf-bss-products-principle-publish-through-engine`

Slices never write `published_version`, history, or outbox rows themselves; the Foundation
`PublishDoor` is the single writer, and the governance gate runs inside it — there is no path
around.

#### Registered-validator pattern

- [ ] `p1` - **ID**: `cpt-cf-bss-products-principle-registered-validators`

Slices contribute validation rules to the Foundation pipeline; a rule pairs every refusal with
a positive control in its slice's test plan, and a rule's variant must have a green joint
fixture before the rule counts as wired.

### 2.2 Constraints

#### Registry/commercial boundary

- [ ] `p1` - **ID**: `cpt-cf-bss-products-constraint-no-commercial-concern`

No money, no price, no charge computation anywhere in this gear (PRD §2.1); `region` is
visibility/legal scope, never a pricing dimension.

#### Identity is sacred

- [ ] `p1` - **ID**: `cpt-cf-bss-products-constraint-immutable-identity`

`productId`/`skuId` server-generated and immutable; `skuCode` atomically reserved at create,
permanently reserved from first publish, released only by draft-discard; downstream binds to
`skuId` (PRD `fr-identifier-contract`).

#### Isolation before function

- [ ] `p1` - **ID**: `cpt-cf-bss-products-constraint-tenant-isolation`

Every table carries `tenant_id`; every path is tenant-scoped through SecureORM; cross-tenant is
deny-by-default + audited; break-glass is read/audit-export only in v1; PDP-gated grants per
endpoint (slice 05).

#### GTS: types, never instances

- [ ] `p1` - **ID**: `cpt-cf-bss-products-constraint-gts-types-not-instances`

GTS carries this gear's **types** — the authz resource/action catalog, the domain types as API
resources, and outbound refs to platform-global vocabularies (`usageTypeRef` today; a future
`resourceTypeRef` per PRD §15 would follow the same pattern). **A Product or SKU is never a GTS
instance**: SKUs are tenant-scoped, operator-authored business data at ≥ 10K/tenant scale with
their own identity contract (`skuId` + `skuCode`, exactly two) and their own versioning
(`CatalogVersion`); GTS instances are platform-global, governed vocabularies (UsageTypes,
model-registry models). A third identity or a data-in-type-registry inversion is refused here
by design, not by omission.

#### Events are broker-native

- [ ] `p1` - **ID**: `cpt-cf-bss-products-constraint-broker-native-events`

(P-D-01.) Durable outbox in the mutation's transaction; emission success never reported before
durable broker acceptance; every state-changing instruction names its event or records "no
event" explicitly; UTC everywhere.

## 3. Technical Architecture

### 3.1 Domain Model

Two aggregate kinds behind one `RegistryEntity` trait — `Product` (name identity, taxonomy
links, scope) and `SKU` (typed, coded, classified, optionally metered) — plus the governed live
entities (Category, AttributeDefinition, the PlanTier and recognized-unit/code sets), the
immutable `CatalogVersion` snapshot aggregate, and the approval/audit/outbox support records.
Canonical column-level shape: [`design/01-foundation.md` §4](./design/01-foundation.md); the
capability columns are carried on the core tables but owned by their slices' validators.

### 3.2 Component Model

#### Registry Foundation

- [ ] `p1` - **ID**: `cpt-cf-bss-products-component-registry-foundation`

The shared engine (slice 01): write doors, `ValidationPipeline`, `PublishDoor`, reservation
index, idempotency store, outbox dispatcher, audit writer.

#### Capability handlers

- [ ] `p1` - **ID**: `cpt-cf-bss-products-component-capability-handlers`

One per slice 02–11, each registering validators and read-model fields against the Foundation:
taxonomy/attributes, classification, lifecycle policy, governance gate, CatalogVersion machine,
reference-signal consumer, read models, bulk/promotion, retention, clone.

#### Consumer contracts

- [ ] `p1` - **ID**: `cpt-cf-bss-products-component-consumer-contracts`

Slice 12: the SDK read surface, the registry↔plan-price seam suite, event schema
versioning/replay/bootstrap.

### 3.3 API Contracts

REST under `/bss-products/v1/…` (authoring, lifecycle, approvals, catalog versions + diff,
bulk, read models), OpenAPI-published; SDK traits mirror it 1:1. Idempotency keys on every
mutating verb; `If-Match` on every draft mutation; resolution calls declare intent
(`browse` vs `posted/contractual`, PRD AC #21). Error codes are contract (slice 01 §3.3).
Endpoint tables live per slice; the OpenAPI export is generated from the contract layer.

### 3.4 Interactions & Sequences

Canonical sequences, each owned by its slice doc; listed here as the index.

#### Authoring → publish

- [ ] `p1` - **ID**: `cpt-cf-bss-products-seq-authoring-publish`

Create/save drafts through the Foundation write doors → slice validators → the slice-05
governance gate inside the `PublishDoor` → version bump + history freeze + events (slice 01 §2).

#### CatalogVersion increment + freeze

- [ ] `p1` - **ID**: `cpt-cf-bss-products-seq-catalog-version-freeze`

`SkuPublished`/downstream publish request → mechanical increment over the D-47 lanes → snapshot
+ checksum → `CatalogVersionPublished` fan-out → participant acks → `freezeComplete` (slice 06).

#### Reference-signal decision

- [ ] `p1` - **ID**: `cpt-cf-bss-products-seq-reference-signal`

Producer watermark ingestion → freshness evaluation → 3-state predicate → retirement/correction
admission or fail-closed refusal (slice 07).

#### Deprecate → retire cascade

- [ ] `p1` - **ID**: `cpt-cf-bss-products-seq-retirement-cascade`

Deprecation (provenance-tracked) → scheduled retirement with lead time → cascade with deferred
intent on blocked children (slice 04).

#### Environment promotion

- [ ] `p2` - **ID**: `cpt-cf-bss-products-seq-environment-promotion`

Deterministic export at a `catalogVersionId` → import (identity via codes, ids re-minted, draft)
→ catalog-version diff review → gated bulk publish (slice 09; PRD AC #33a).

### 3.5 Database schemas & tables

`products_product`, `products_sku` (with `ReservationIndex`), `products_entity_version`
(published history), `products_idempotency`, `products_audit_log`, `products_outbox`
(slice 01 §4); slice-owned: `products_category`, `products_attribute_definition`/`_value`,
`products_plan_tier`, recognized-set tables (02/03), `products_approval` + stored pinned
snapshots (05), `products_catalog_version` + freeze-ack/participant tables (06),
`products_reference_watermark` + producer registry (07), read-model projections (08). All
tables tenant-scoped; DDL in one-migration-per-table chains with dual-engine schema-oracle
goldens.

## 4. Decision register & joint contracts

- [`DECISIONS.md`](./DECISIONS.md) — P-D-01…06.
- Joint contracts consumed here: **D-46**, **D-47** (pricing register); **UC3** binding (rating
  `SEAMS.md` §J); contested-surface ownership — rating `SEAMS.md` "Ownership matrix" (five
  products rows, 2026-08-25).
- Cross-gear obligations still open against counterparts (PRD §15): pricing owes
  `BundleCompositionCompleted` (slice 06 consumes it); freeze-participant acks unregistered on
  all three participants; Contracts' "not a quote" position vs the quote-snapshot delegation.

## 5. Traceability (PRD §6 → slice)

6.1 → 01 (identifiers, mutability frame) + 07 (signal, corrections); 6.2 → 02; 6.3 → 03;
6.4 → 02; 6.5 → 01 (machine) + 04 (policy); 6.6 → 06 (incl. `fr-revision-vs-version`'s version-binding-at-freeze clause); 6.7 → 01 (idempotency, eventing) + 05
(approvals); 6.8 → 05 (isolation) + 08 (read models); 6.9 → 09; 6.10 → 11; 6.11 → 10;
6.12 → 12; 6.13 → resident per door (enumerated per slice). Every slice carries a "Traces to"
list; slice 12 owns the completeness check that every `p1`/`p2` FR is claimed by exactly one
slice.

## 6. Status

| Slice | Status |
|-------|--------|
| 01-foundation | **authored + agent-reviewed 2026-08-25**; fix wave applied (H1 head-row model, shared guard, `normalized(name)` pin) |
| 02-taxonomy-attributes | **authored + agent-reviewed 2026-08-25**; fix wave applied (H2 category branch, M2/M5); P-D-06 still flagged for review |
| 03-sku-classification | **authored + agent-reviewed 2026-08-25**; fix wave applied (M2 operand narrowed) |
| 04-lifecycle | **authored + agent-reviewed 2026-08-25**; fix wave applied (provenance pass-through, parent path, runner lease, `RETIREMENT_PENDING`); initiation reading CONFIRMED via §17.1 |
| 05-governance | **authored + agent-reviewed 2026-08-25**; fix wave applied (scheduled-act consumption model, vocabulary-op materiality, transition-fires-hook invariant); quorum-strictness note for small tenants still flagged |
| 06-catalog-version | **authored + agent-reviewed 2026-08-25**; fix wave applied (satisfiedRequests handshake, lifecycle re-validation arm, stored-copy captures, operation_key bulk batching, forced-complete semantics); composition-clear exemption + mechanical-retry reading flagged |
| 07-reference-signal | **authored 2026-08-25** |
| 08-read-models | **authored 2026-08-26** (stamp-binds-to-catalog-versions reading flagged) |
| 09-bulk-promotion | **authored 2026-08-26** (coalesced-event deviation recorded as sanctioned) |
| 10-retention-erasure | **authored 2026-08-26** |
| 11-clone | **authored 2026-08-26** (resolves the 01-flagged clone-vs-P-D-04 interaction) |
| 12-consumer-contracts | **authored + agent-reviewed 2026-08-26**; fix wave applied (eight CoverageChecks incl. id-uniqueness/identity/monetization lints, status vocabulary pinned, register rows split by authorability) |

**The design set is COMPLETE: all twelve slices authored, agent-reviewed, and fix-waved**
(2026-08-25/26; per-slice reports in `~/Documents/pricing-reviews/`). Human flags awaiting the
owner: P-D-06/P-D-07 review, the composition-clear gate exemption, the mechanical-retry AC #40
reading, the M6 role-predicate-replaces-base reading, quorum strictness for small tenants, the
SchemaPin widening proposal. Next phase: implementation planning against the phase column;
first build acts: slice 01 + the P-D-03 watermark joint build with pricing.
