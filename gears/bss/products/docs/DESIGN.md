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
- [4. Additional context](#4-additional-context)
- [5. Traceability](#5-traceability)
- [6. Status](#6-status)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-products-design-main`

## 1. Architecture Overview

### 1.1 Architectural Vision

The **products** gear is the BSS catalog **registry**: the System of Record for Products, SKUs,
categories, attributes/localization, and immutable `CatalogVersion` snapshots — *what can be
sold and how it is described, classified, versioned, and published*. It owns no commercial
concern: Plan/Price/composition are the pricing gear's, evaluation is rating's (PRD §2.1
boundary). Requirements live in [`PRD.md`](./PRD.md) (sign-able as of 2026-08-25 — all §15
gates closed, veto register clean for P-D-01…P-D-13, with **P-D-14…P-D-20 flagged and awaiting the owner** (§6)); decisions in [`DECISIONS.md`](./DECISIONS.md) (P-D-NN;
joint contracts D-46/D-47 live in the pricing register).

The design follows the **foundation-plus-handlers** pattern proven by the pricing gear: one
shared engine slice ([`design/01-foundation.md`](./design/01-foundation.md)) owns the entity
model, identity, the lifecycle state machine, the fail-closed validation pipeline, versioning,
idempotency, eventing, and audit; every capability slice is a handler that authors draft state,
**registers its validation rules** with the pipeline, contributes read-model fields, and
publishes through the Foundation. The Foundation carries no capability policy — it does not
know what a `PlanTier` or a metering unit is.

### 1.2 Architecture Drivers

#### Requirement coverage

*All 57 requirement ids of PRD §6 and §7 — 56 `p1`/`p2` plus `fr-clone`, which the PRD declares
`p3` — by full id, against the slice that owns it.
Added 2026-08-26: this section cited requirements in prose only, so every tool that walks
these documents by the id convention read this design as citing none — and the CFS
reference-coverage rule for `fr`/`nfr` into DESIGN is satisfied by nothing until a
requirement is ticked, at which point it fails for every id at once.*

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-products-fr-create-product` / `cpt-cf-bss-products-fr-define-sku` / `cpt-cf-bss-products-fr-event-delivery-resilience` / `cpt-cf-bss-products-fr-expected-failure-behavior` / `cpt-cf-bss-products-fr-field-mutability-matrix` / `cpt-cf-bss-products-fr-idempotent-authoring` / `cpt-cf-bss-products-fr-identifier-contract` / `cpt-cf-bss-products-fr-lifecycle-transitions` / `cpt-cf-bss-products-fr-parent-child-integrity` / `cpt-cf-bss-products-fr-registry-eventing-audit` / `cpt-cf-bss-products-fr-revision-vs-version` / `cpt-cf-bss-products-fr-skucode-reservation-concurrency` / `cpt-cf-bss-products-nfr-determinism-integrity` / `cpt-cf-bss-products-nfr-publication-propagation` / `cpt-cf-bss-products-nfr-scale-extensibility` | **Slice 01** — The Foundation owns identity, the head-vs-version split, the single fail-closed publish pipeline, idempotency, the outbox and the audit plane. Every slice registers its validators into that one door. |
| `cpt-cf-bss-products-fr-create-product` / `cpt-cf-bss-products-fr-localized-attributes` / `cpt-cf-bss-products-fr-manage-taxonomy` / `cpt-cf-bss-products-fr-retention-erasure` | **Slice 02** — Taxonomy and attribute definitions are governed live entities; assignment tables carry the exactly-one-primary index, and localization resolves through a total fallback chain. |
| `cpt-cf-bss-products-fr-accounting-codes` / `cpt-cf-bss-products-fr-define-sku` / `cpt-cf-bss-products-fr-metering-unit-declaration` / `cpt-cf-bss-products-fr-metering-unit-delisting` / `cpt-cf-bss-products-fr-plantier-classification` / `cpt-cf-bss-products-fr-sku-sellable` | **Slice 03** — Typing and classification per `TypeProfile`, with the recognized-set tables behind every closed vocabulary and the publish-time collector call made once per publish. |
| `cpt-cf-bss-products-fr-deprecation` / `cpt-cf-bss-products-fr-lifecycle-transitions` / `cpt-cf-bss-products-fr-parent-child-integrity` / `cpt-cf-bss-products-fr-retirement-eol` / `cpt-cf-bss-products-fr-undeprecation` | **Slice 04** — Lifecycle policy: the edge list, deprecation provenance, cascades, and retirement as a scheduled transition with its joint plan-price contract. |
| `cpt-cf-bss-products-fr-breakglass-action-scope` / `cpt-cf-bss-products-fr-materiality-gated-publish` / `cpt-cf-bss-products-fr-tenant-isolation-breakglass` | **Slice 05** — Governance: materiality, the tenant-configured approver quorum, the RBAC catalog, and break-glass elevation bounded to read and audit-export. |
| `cpt-cf-bss-products-fr-bundle-adoption-guard` / `cpt-cf-bss-products-fr-catalog-publish-concurrency` / `cpt-cf-bss-products-fr-catalog-version-diff` / `cpt-cf-bss-products-fr-catalog-version-publish` / `cpt-cf-bss-products-fr-freeze-atomicity` / `cpt-cf-bss-products-fr-freeze-participant-governance` / `cpt-cf-bss-products-fr-freeze-recovery` / `cpt-cf-bss-products-fr-grandfathered-retention-coupling` / `cpt-cf-bss-products-fr-grandfathering-invariant` / `cpt-cf-bss-products-fr-prepublish-lint` / `cpt-cf-bss-products-fr-revision-vs-version` / `cpt-cf-bss-products-fr-snapshot-reproducibility` / `cpt-cf-bss-products-nfr-posting-safe-budget` / `cpt-cf-bss-products-nfr-publication-propagation` / `cpt-cf-bss-products-nfr-scale-extensibility` / `cpt-cf-bss-products-nfr-snapshot-archival-dr` | **Slice 06** — `CatalogVersion` is demand-driven and mechanical: request intake, the counter, full snapshots with checksums, and the freeze protocol with its force-completion recovery. |
| `cpt-cf-bss-products-fr-failsafe-tripwire` / `cpt-cf-bss-products-fr-immutable-field-correction` / `cpt-cf-bss-products-fr-reference-producer-registration` / `cpt-cf-bss-products-fr-reference-signal` | **Slice 07** — The reference signal: registered producers, per-producer watermarks, the reference predicate, and the correction door with its three gates — fresh-zero, break-glass behind its flag, and P-D-16's unresolvable-target arm outside it. |
| `cpt-cf-bss-products-fr-cache-first-browse` / `cpt-cf-bss-products-fr-event-delivery-resilience` / `cpt-cf-bss-products-nfr-availability-audit` / `cpt-cf-bss-products-nfr-graceful-degradation` / `cpt-cf-bss-products-nfr-read-latency` / `cpt-cf-bss-products-nfr-read-throughput` | **Slice 08** — Read models are projections with a staleness stamp on every response, rebuildable from the frozen versions and the outbox. |
| `cpt-cf-bss-products-fr-bulk-import-export` | **Slice 09** — Bulk import, export and promotion run per-row through the Foundation publish door under one batch-scoped approval. |
| `cpt-cf-bss-products-fr-expected-failure-behavior` / `cpt-cf-bss-products-fr-grandfathered-retention-coupling` / `cpt-cf-bss-products-fr-retention-erasure` / `cpt-cf-bss-products-nfr-snapshot-archival-dr` | **Slice 10** — Retention clocks per class, the identity-ref map as the single erasure operand, and the retention gate that never forces a collection. |
| `cpt-cf-bss-products-fr-clone` | **Slice 11** — Clone copies content and never identity, resetting lifecycle and version counters and reserving new codes atomically. |
| `cpt-cf-bss-products-fr-deprecation` / `cpt-cf-bss-products-fr-event-versioning-replay` / `cpt-cf-bss-products-fr-freeze-atomicity` / `cpt-cf-bss-products-fr-monetization-traceability` / `cpt-cf-bss-products-fr-plan-price-seam` / `cpt-cf-bss-products-nfr-backward-compatible-evolution` | **Slice 12** — The consumer surface: the SDK, the event compatibility corpus, the obligation register and the coverage checks over this design set. |

#### Functional Drivers

- Financial-grade governance: SoD approvals at the tenant's **configured approver quorum** (P-D-11: default 2, floor 0 — "two-person" is a retained name, never a fixed count; P-D-13 enumerates where the shorthand reaches), pinned to stored revision snapshots
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
P-D-05 `usageTypeRef` resolvability-only · P-D-06 metadata outside frozen content · P-D-07 the
staleness stamp is a floor · P-D-08 audit sealing is a platform capability (reserved seam) ·
P-D-09 stage-vs-commit fail-closed per lane · P-D-10 no gear-side Legal role · P-D-11 the
approver count is a policy value, default 2 floor 0 · P-D-12 the `SchemaPin`'s membership is a
rule · P-D-13 the quorum shorthand's enumerated reach (six sites) · P-D-14 `system_signal` is
an approval subject kind, not an exemption · P-D-15 the inbound machine contracts are
`products-sdk` clients, not REST doors · P-D-16 the unresolvable-target correction arm ·
P-D-17 promotion identity collision is update-as-draft · P-D-18 version liveness ends by an
explicit release · P-D-19 a force-completed version stays refused for posted use · P-D-20 a
publish during the retirement lead window re-announces `SkuRetired`. Joint: D-46 (`sellable`),
D-47 (increment lanes + retirement contract) — pricing register.

**P-D-14…P-D-20 are FLAGGED and await the owner** — all seven were found by the 2026-08-26
branch review, **five** of them already built into the design and never registered (P-D-14…P-D-18) and **two** reversing a delivery — P-D-19 (a force-completed version stays refused for posted use) and P-D-20, which strikes a publish freeze slice 04 had already shipped (recounted 2026-08-26: this read six-and-one, and the count hid the more product-visible of the two reversals)
the design had made. None was ever put to the owner, which is what makes them flags and not
history.

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
- **Identity**: GTS **types** (never instances — §2.2), declared as `gts.cf.bss.products.product.v1~`, `gts.cf.bss.products.sku.v1~`, `gts.cf.bss.products.category.v1~`, `gts.cf.bss.products.attribute_definition.v1~`, `gts.cf.bss.products.catalog_version.v1~` and `gts.cf.bss.products.approval.v1~` (the name slice 05's RBAC catalog uses) — these six are the **domain** types exposed as API resources; the authz resource/action catalog of slice 05 §3.2 declares 21 GTS-typed resources and is enumerated there rather than duplicated here (2026-08-26) — (the only GTS token here had been the namespace glob `gts.cf.bss.products.*`, which carries no type name, no version and no trailing `~`, so `guidelines/GTS.md`'s identifier grammar had nothing to match and the §2.2 constraint had no enumerable operand); tables `products_*`; dual-engine storage
  (SQLite + Postgres), one migration per table, schema-oracle goldens from day one.

#### Design set (ordered by implementation phase)

| Doc | Content (one line) | PRD §6 | Phase | Depends on |
|-----|--------------------|--------|-------|------------|
| `01-foundation` | shared engine: entities, identity, revision-vs-version, state machine, validation pipeline, idempotency/ETag, eventing (P-D-01), audit | 6.1 core, 6.5 core, 6.7, 6.13 | 0/1 | — |
| `02-taxonomy-attributes` | categories (governed live), attribute definitions + i18n fallback, metadata map, well-known seeds | 6.2, 6.4 | 1 | 01 |
| `03-sku-classification` | SKU typing, `sellable`, `PlanTier`, accounting codes, metering unit + `usageTypeRef` (P-D-05), de-listing | 6.3 | 1 | 01, 02 |
| `04-lifecycle` | deprecation provenance, parent-child + cascade-retire, scheduled publish/retirement, `replacedBy`, containment (P-D-04 residue) | 6.5 | 1/2 | 01 (05, 07 at integration) |
| `05-governance` | materiality matrix, the configured approver quorum (P-D-11/P-D-13) + FinanceReviewer, **stored** pinned approval snapshot, RBAC, break-glass | 6.7, 6.8 | 1/2 | 01 |
| `06-catalog-version` | CatalogVersion machine (P-D-02, D-47 lanes), checksum/reproducibility, freeze protocol, `compositionPending`, version diff | 6.6 | 2 | 01, 02, 03, 04, 05 |
| `07-reference-signal` | `SkuReferenceCount` watermarks, 3-state predicate, producer registration (P-D-03), fresh-zero corrections + tripwire | 6.1 signal, 6.13 | 2 | 01, 04 |
| `08-read-models` | cache-first browse/search, per-state visibility, `asOfCatalogVersion`, degradation, NFR budgets | 6.8, §7 | 2 | 01, 06 |
| `09-bulk-promotion` | bulk import/export, two-phase deps, change report, environment promotion (AC #33a) | 6.9 | 2/3 | 01, 05 |
| `10-retention-erasure` | retention classes, pseudonymization, PII write-block, retention↔grandfathering coupling | 6.11 | 3 | 01, 06 |
| `11-clone` | clone/templating with live re-validation (p3) | 6.10 | 3 | 01–04 |
| `12-consumer-contracts` | seam-suite spec, event schema versioning/replay/bootstrap, §9 interfaces, traceability check | 6.12, 6.7 (replay), §9 | 2/3 | 01, 03, 06, 07 |

#### Dependency order

Phase 0/1: 01 → (02, 03, 04, 05 in parallel). Phase 2: 06 (needs 04+05), 07 (needs 04), 08
(needs 06), 12 starts once 03/06/07 fix their shapes. Phase 2/3: 09; Phase 3: 10, 11. The
numeric prefix is implementation order, not the PRD subsection number.

## 2. Principles & Constraints

### 2.1 Design Principles

#### Fail-closed everywhere

- [ ] `p1` - **ID**: `cpt-cf-bss-products-principle-fail-closed`

Every enumerated failure of PRD AC #38 **that a registry door can refuse** maps to a named
error code (slice 01 §3.3 taxonomy); no partial application; every rejection audited with reason.
Three of the fifteen AC #38 rows are outside that universe by design and enumerated in slice 12's
lint 2 — the retention-orphan **alarm**, the `compositionPending` **consumer duty** and AC #38's
**post-v1 EOL row**, whose only candidate code refuses the feature rather than the named
condition — so the
principle and its lint say the same thing (item 32 of the 2026-08-26 review).

#### Two version counters, never conflated

- [ ] `p1` - **ID**: `cpt-cf-bss-products-principle-two-version-counters`

The internal revision moves on every save and backs optimistic concurrency; the published
version moves only on publish and is the only thing a consumer or `CatalogVersion` may
reference (PRD `cpt-cf-bss-products-fr-revision-vs-version`).

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
`skuId` (PRD `cpt-cf-bss-products-fr-identifier-contract`).

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

Registered downstream addressability request or the operator catalog-publish act (an entity publish never enqueues — P-D-02/06 `inst-cv-request`) → mechanical increment over the D-47 lanes → snapshot
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

**35 tables, by the slice that defines each** (re-censused 2026-08-26 from the slices
themselves — this section is the canonical index migration planning is scoped off, and it had
listed 13 tables, named `products_plan_tier`, which no slice defines because slice 03 folds
tiers into `products_recognized_set` under `set_kind`, and omitted about twenty real ones):

- **01** — `products_audit_log`, `products_entity_version`, `products_idempotency`, `products_product`, `products_product_category` (defined by slice 02, written by slice 01's create door), `products_sku`
  *(the outbox is the toolkit's, not a gear table — **P-D-22**)*
- **02** — `products_attribute_definition`, `products_attribute_value`, `products_category`, `products_metadata`
- **03** — `products_recognized_set`
- **04** — `products_deferred_retirement`, `products_scheduled_transition`
- **05** — `products_approval`, `products_approval_decision`, `products_breakglass_session`
- **06** — `products_catalog_version`, `products_catalog_version_counter`, `products_catalog_version_entry`, `products_catalog_version_request`, `products_freeze_ack`, `products_freeze_participant`
- **07** — `products_correction_override`, `products_reference_member`, `products_reference_producer`, `products_reference_watermark`
- **08** — `products_read_deferred_intent`, `products_read_delivery_state`, `products_read_entity`, `products_read_freeze_status`
- **09** — `products_bulk_batch`, `products_bulk_row`
- **10** — `products_identity_ref`, `products_pii_allowlist`

All tables tenant-scoped; DDL in one-migration-per-table chains with dual-engine
schema-oracle goldens.

## 4. Additional context

**On ADRs — an open convention question, raised 2026-08-26.** This gear has no `ADR/`
directory. Pricing carries three, rating two, subscriptions three, and their `DESIGN.md`
files reference those ADR ids. `docs/spec-templates/README.md` reserves an ADR for a
decision where "there was a meaningful discussion/debate and the rationale needs to be
preserved as a historical decision record", and at least three register entries have that
shape: **P-D-01** (a broker-native envelope weighed against CloudEvents 1.0), **P-D-11**
(the approver count as a policy value with floor 0) and **P-D-15** (SDK clients from
`ClientHub` against out-of-process REST doors). Each carries its options and its rejected
alternatives here rather than in an ADR. No written rule says a `DECISIONS.md` register
substitutes for an ADR, and none says it does not. **Owner: Architecture** — either promote
those three to ADRs under the template's naming, or record that this gear's register is its
decision-record artifact. Until then three of the seven BSS gears ship no `docs/ADR/` — `rate-provider` keeps a
rejected-alternative record in its `DESIGN.md` instead — so the question is which convention this
family means to hold, not whether products is unique (corrected 2026-08-27: the escalation had
rested on a uniqueness that does not hold).

**Decision register & joint contracts.**

- [`DECISIONS.md`](./DECISIONS.md) — P-D-01…20 (both summaries said "…06" / listed five while
  the register held twelve — item 26 of the 2026-08-26 review; P-D-13 landed with that review's
  fix wave).
- Joint contracts consumed here: **D-46**, **D-47** (pricing register); **UC3** binding (rating
  `SEAMS.md` §J); contested-surface ownership — rating `SEAMS.md` "Ownership matrix" (five
  products rows, 2026-08-25).
- Cross-gear obligations still open against counterparts (PRD §15): pricing owes
  `BundleCompositionCompleted` (slice 06 consumes it); freeze-participant acks unregistered on
  all three participants; Contracts' "not a quote" position vs the quote-snapshot delegation.

## 5. Traceability

*PRD §6 → slice.*

6.1 → 01 (identifiers, mutability frame) + 07 (signal, corrections); 6.2 → 02; 6.3 → 03;
6.4 → 02; 6.5 → 01 (machine) + 04 (policy); 6.6 → 06 (incl. `cpt-cf-bss-products-fr-revision-vs-version`'s version-binding-at-freeze clause); 6.7 → 01 (idempotency, eventing) + 05
(approvals); 6.8 → 05 (isolation) + 08 (read models); 6.9 → 09; 6.10 → 11; 6.11 → 10;
6.7 also → 12 (`cpt-cf-bss-products-fr-event-versioning-replay`, which slice 12 claims in its own §1.4 and
Traces-to while both sites here said 6.12 + §9 only — 2026-08-26 branch review);
6.12 → 12; 6.13 → resident per door (enumerated per slice). Every slice carries a "Traces to"
list; slice 12 owns the completeness check that every `p1`/`p2` **requirement-bearing PRD id** —
`fr-*`, `nfr-*`, and §9's `interface-*`/`contract-*`, the universe M5 widened it to, plus
`usecase-*` — is claimed by exactly one slice.

## 6. Status

| Slice | Status |
|-------|--------|
| 01-foundation | **authored + agent-reviewed 2026-08-25**; fix wave applied (H1 head-row model, shared guard, `normalized(name)` pin) |
| 02-taxonomy-attributes | **authored + agent-reviewed 2026-08-25**; fix wave applied (H2 category branch, M2/M5); P-D-06 **CONFIRMED 2026-08-26** |
| 03-sku-classification | **authored + agent-reviewed 2026-08-25**; fix wave applied (M2 operand narrowed) |
| 04-lifecycle | **authored + agent-reviewed 2026-08-25**; fix wave applied (provenance pass-through, parent path, runner lease) — the `RETIREMENT_PENDING` publish freeze this row also credited was **struck again by P-D-20** on 2026-08-26 and is not an applied fix; the code's remaining arms are un-deprecation here and slice 01's create-door parent guard; initiation reading CONFIRMED via §17.1 |
| 05-governance | **authored + agent-reviewed 2026-08-25**; fix wave applied (scheduled-act consumption model, vocabulary-op materiality, transition-fires-hook invariant); quorum strictness **resolved 2026-08-26 — P-D-11** (approver count is a typed-policy value, default 2, floor 0); role-predicate question **resolved — P-D-10** (C8: predicates narrow, never replace) |
| 06-catalog-version | **authored + agent-reviewed 2026-08-25**; fix wave applied (satisfiedRequests handshake, lifecycle re-validation arm, stored-copy captures, operation_key bulk batching, forced-complete semantics); composition-clear **resolved 2026-08-26** (`system_signal` approval subject); mechanical-retry AC #40 reading **resolved 2026-08-26 — P-D-09 amended the FR and the AC to state the lane split** |
| 07-reference-signal | **authored 2026-08-25**; fix wave applied 2026-08-26 (F1–F8, Blocking 3, review items 19/20/21); quorum sweep + P-D-16 applied 2026-08-26 (branch review) |
| 08-read-models | **authored 2026-08-26**; P-D-07 stamp floor **CONFIRMED 2026-08-26** (conditional on the projection existing — PRD §15 now asks whether browse needs a serving store at all) |
| 09-bulk-promotion | **authored 2026-08-26** (coalesced-event deviation recorded as sanctioned) |
| 10-retention-erasure | **authored 2026-08-26**; role-predicate question **resolved 2026-08-26 — P-D-10**: no gear-side Legal role, the allow-list runs the base quorum with a mandatory recorded Legal sign-off reference |
| 11-clone | **authored 2026-08-26** (resolves the 01-flagged clone-vs-P-D-04 interaction) |
| 12-consumer-contracts | **authored + agent-reviewed 2026-08-26**; fix wave applied (CoverageChecks incl. id-uniqueness/identity/monetization lints, status vocabulary pinned, register rows split by authorability); SchemaPin widening **resolved 2026-08-26 — P-D-12**: membership is the rule "operands the §2.2 guards read", `inst-cc-pin` lints it, nine lints total |

**The design set is COMPLETE: all twelve slices authored** (2026-08-25/26). **Review status is
per slice — read the table above, not this line**: the rows carrying "agent-reviewed" plus a fix
wave are the ones this repository can evidence. *(This sentence has now named the wrong slice
twice: it first said slice 11 carried no review-finding markers when slice 11 carries thirteen —
H1, L1–L6, M1–M6, item 26 of the 2026-08-26 review — and the correction then said the same of
slice 07, which carries eight: F1–F8 plus a Blocking-3 fix and review items 19/20/21. A marker
census is derivable from the slices and this sentence keeps going stale from restating it, so
the claim is dropped rather than corrected a third time — 2026-08-26 branch review.)* The
earlier aggregate here claimed all twelve were agent-reviewed and fix-waved,
which the table does not support (CodeRabbit, 2026-08-26); per-slice review reports are working
artifacts rather than repository content, so the table is the only in-repo record and the claim
is narrowed to what it holds.

**Human flags awaiting the owner: seven — P-D-14…P-D-20**, opened by the 2026-08-26 branch review. The six below were answered on 2026-08-26 and stay answered; what the review found is a different class, and the distinction cost this branch a wave: **"no open questions" is not "correct"**. Five of the seven are decisions the design had already *made* and never registered — an undeclared decision is invisible to a flag count precisely because nobody asked anything. The six answered flags, in a single
session with the product owner, one decision at a time:

| # | Question | Outcome |
|---|----------|---------|
| 1 | Metadata-map placement | **P-D-06 confirmed** as designed — the map lives beside the entity; the accepted cost (no history between snapshots, structural: `products_metadata`'s key has no version dimension) is recorded with it |
| 2 | Staleness-stamp semantics | **P-D-07 confirmed, conditionally** — the floor is a property of a lagging projection, so PRD §15 now carries the prior question of whether browse needs a separate serving store at all, and `cpt-cf-bss-products-fr-cache-first-browse`'s rationale was re-derived off its uncalibrated read-NFR numbers onto the availability split and structural stale-but-safe |
| 3 | AC #40's "rejected" with no operator | **P-D-09 amended** `cpt-cf-bss-products-fr-catalog-publish-concurrency` **and** AC #40 to state the stage-vs-commit lane split, rather than leave the design reading standing against normative text that said the opposite in two places |
| 4 | Who approves a Legal-owned change | **P-D-10: no gear-side Legal role** — the allow-list runs the base quorum and records an external Legal sign-off reference, which is what AC #35 specified all along; role predicates narrow within the base set and never replace it (05 C8), retiring the grant in `inst-mt-inputs` (d). No PRD edit owed |
| 5 | Quorum vs a two-person company | **P-D-11**: the approver count is a typed-policy value, **default 2, floor 0** — a one-person tenant could previously publish nothing at all, while the plan-price sibling ships `submitter + 1` in its schema and an approver-less path. Fixed and not configurable: the FinanceReviewer predicate, the self-approval refusal at `N ≥ 1`, explicit configuration only, provisioning-time initial value |
| 6 | `SchemaPin` widening | **P-D-12**: membership became the rule "the operands the §2.2 `ObligationRegister` guards read", after measuring **four** such operands outside the FR's five-item list — of which only three were comparable fields at all. `cpt-cf-bss-products-fr-plan-price-seam` amended; `inst-cc-pin` lints the coupling both ways |

Two further decisions landed in the same session without having been flagged: **P-D-08** defers
audit sealing to a platform capability (below), and the transport wave named the two inbound
machine contracts as `products-sdk` clients resolved from `ClientHub` rather than as the REST
doors that bind them out-of-process, registering the increment request in PRD §9.2 beside its
sibling — **that second one is now P-D-15**, which it should have been from the start: this
paragraph called it a decision that landed and then left it out of the register, so of the two
it names, one had an id and one did not. *(The composition-clear gate exemption left the flag
list earlier the same day — the 2026-08-26 CodeRabbit pass forced its resolution: a
`system_signal` approval subject with the inbound governed signal as the authorizing principal,
**now P-D-14** on the same reasoning.)*

**P-D-08 (2026-08-26):** audit sealing is deferred to a platform capability — the gear ships the
complete append-only trail over a reserved, unwritten seam, with the requirements that capability
must satisfy stated as P-D-08 S1–S9 and owned by Architecture (PRD §15/§16). Next phase:
implementation planning against the phase column; first build acts: slice 01 + the P-D-03
watermark joint build with pricing.
