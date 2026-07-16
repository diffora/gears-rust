<!-- CONFLUENCE_TITLE: [BSS]: Plan & Price Modeling — Catalog Foundation (shared publish engine) (Design) -->
<!-- Related: ../PRD.md, ../DESIGN.md | Upstream: Product & SKU registry, Effective-dating PriceWindows | Downstream: Tariffs, Subscriptions, Rating, Billing, Marketplace | Owners: BSS Product Catalog team -->

# DESIGN — Catalog Foundation (shared publish engine) (Slice 1)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-design-foundation`

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles and Constraints](#2-principles-and-constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions and Sequences](#36-interactions-and-sequences)
  - [3.7 Database Schemas and Tables](#37-database-schemas-and-tables)
  - [3.8 Deployment Topology](#38-deployment-topology)
- [4. Additional Context](#4-additional-context)
  - [4.1 Canonical Scope Key (normative)](#41-canonical-scope-key-normative)
  - [4.2 Publish-Through-The-Engine Contract (normative)](#42-publish-through-the-engine-contract-normative)
  - [4.3 Immutability and Change Mechanisms (normative)](#43-immutability-and-change-mechanisms-normative)
  - [4.4 Read Model and pricingSnapshotRef (normative)](#44-read-model-and-pricingsnapshotref-normative)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

The Catalog Foundation is the shared publish engine that every Plan & Price Modeling
capability builds on. It owns the `Plan`/`Price` entity model, the **canonical scope key**,
the draft→publish state machine, the **fail-closed validation pipeline** framework,
append-only published-row history with versioning/supersession, the **read-model projection**
and `pricingSnapshotRef` stamping, and the **frozen event fan-out** plus the
`CatalogVersion`-increment request to the registry. It owns **no capability policy**: what a
billing cycle means, how a tier band validates, what a bundle is — all live in a handler slice
(plan-definition, price-structure, and the rest), each of which authors draft state, registers
its validation rules and its read-model fields, and **publishes through** this Foundation's
API under the invariants defined here.

The catalog's contract is the mirror image of the sibling Billing Ledger's *post through the
engine*: it is **publish through the engine**. Every state change reaches production one way —
author draft → run the aggregate fail-closed validation pipeline → freeze a complete read
model + `pricingSnapshotRef` → emit the frozen event set → request a `CatalogVersion`. There
is no side door that mutates published state, no consumer that reads draft, and no default
substituted for an absent field (absence must have failed publish). This keeps the
correctness-critical publish/immutability/determinism core small and auditable while letting
each pricing capability evolve independently ([`../PRD.md`](../PRD.md) §1.1, §2).

The gear is **one deployable modular monolith** (`pricing`) running in two roles — a
synchronous authoring/publish/preview API and a read-model service — over one PostgreSQL
backend. The authoring path is transactional — draft mutation, validation, publish-commit, and event
enqueue commit atomically **at the post-approval publish step** (the Slice 5 approval gate for
material changes sits between the validation pre-check and that commit); the read path is
served from a projected read model for the p95 < 100ms target and **fails closed** (never
stale) on read-model outage.

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-pricing-fr-publish-validation-failclosed` | A single **aggregate fail-closed validation pipeline** runs at publish; slices register rules keyed to the §17.4 rule set; any invalid condition blocks the publish transaction (no `PlanPublished`, no read-model warm). The validation report enumerates every failure so authoring can remediate. |
| `cpt-cf-bss-pricing-fr-published-rows-append-only` | `pricing_price` rows in a published state are append-only: `REVOKE UPDATE, DELETE` from the app role + `BEFORE UPDATE OR DELETE` triggers that RAISE, with a **column whitelist** for the two sanctioned transitions (state-machine `lifecycle_state` flips; monotonic `grandfather_until` tightening — §3.7/§4.3); only never-published `draft` rows are deletable. There is no deletion event to fan out. |
| `cpt-cf-bss-pricing-fr-plan-versioning` | A price/tier change versions the `Plan` and writes **new** immutable `pricing_price` rows; prior rows are retained as history; bound subscriptions continue on their frozen snapshot until renewal or migration. |
| `cpt-cf-bss-pricing-fr-supersession` | Supersession is versioning scoped to **one canonical scope key**: a new immutable row plus opening/closing the corresponding `PriceWindow` (never an in-place mutate, never an overlap), within one `priceEligibility` class and one `chargeKind`. |
| `cpt-cf-bss-pricing-fr-pricing-snapshot` | Publish stamps the catalog-side identifiers sufficient for the manifest `pricingSnapshotRef` (resolved price ids + evaluation-policy version + the **pending** version ref, finalized to the committed `CatalogVersion` on `CatalogVersionPublished`); posted periods never re-query mutable rows; the catalog-side view MUST NOT diverge from the Tariffs composition SoR. |
| `cpt-cf-bss-pricing-fr-consumer-readmodel-resolution` | The projected read model is **monotonic per `CatalogVersion`** (ignored until `CatalogVersionPublished` + a warm-completion marker); consumers resolve exact published values with no draft read and no default substitution; a rating run pins one version and the pin never lags the newest completed version by > 5s. |
| `cpt-cf-bss-pricing-fr-catalogversion-increment` | On every `PlanPublished` the Foundation requests addressability; the registry is the **sole** incrementer and MAY batch approved publishes; `PlanPublished` carries a **pending** ref and the snapshot pins the committed version on `CatalogVersionPublished`. |
| `cpt-cf-bss-pricing-fr-publish-fanout-atomicity` | Post-commit read-model warming retries to the 5s SLO or marks the publish degraded (`PlanPublishDegraded`); no state exposes a rateable-but-incomplete plan; the pre-commit batching delay is governed by the max batching-delay SLO, not by degraded handling. |
| `cpt-cf-bss-pricing-fr-event-contract` | A **frozen event-name set** (`PlanCreated`, `PlanUpdated`, `PlanPublished`, `PlanRetired`, and conditionally `PlanMigrationScheduled`, `PlanPublishDegraded`, `BundleUpdated`, `PriceCreated`, `PriceUpdated`, plus the manifest `PriceWindowScheduled`/`Activated`/`Expired`/`Cancelled` — produced by this gear since the window consolidation, D-03) emitted from a transactional outbox, ordered per `(tenantId, aggregateId)`, at-least-once, carrying correlation/idempotency keys. |
| `cpt-cf-bss-pricing-fr-price-amount-validation` | Amount ≥ 0, valid ISO 4217, precision = the currency's ISO 4217 minor unit; a missing `(currency, region)` row fails closed (no implicit FX). |
| `cpt-cf-bss-pricing-fr-mutation-idempotency` | Plan/Price create/update accept a client idempotency key; a duplicate returns the original outcome without a second mutation. |
| `cpt-cf-bss-pricing-fr-concurrent-edit` | Optimistic concurrency (ETag/row version) rejects a stale submit and a bulk-vs-interactive collision with a conflict; neither change is silently overwritten. |

#### NFR Allocation

| NFR theme | Allocated To | Design Response | Verification / Status |
|-----------|--------------|-----------------|-----------------------|
| Publish → read-model propagation (p95 ≤ 5s) | Publish engine + outbox + read-model warmer | Batched `CatalogVersion` commit; retry-to-SLO warm or `PlanPublishDegraded`; pin never lags newest completed by > 5s | Load test on the publish→warm path; **max batching-delay SLO value open with Registry** ([`../PRD.md`](../PRD.md) §15) |
| Read / preview latency (p95 < 100ms per tenant partition) | Read-model projection store | Single indexed, version-pinned read; no evaluation on the read path | APM on read APIs |
| Determinism / reproducibility | Snapshot + append-only history | Complete frozen `pricingSnapshotRef`, monotonic per version, append-only rows | Design + integration test (later-version publish does not alter a prior snapshot) |
| Read-model availability / DR RPO-RTO | Read-model store + topology | Fail-closed on outage (never stale) | **Provisional — ratify before Design lock** ([`../PRD.md`](../PRD.md) §14) |
| Idempotency-key TTL; plan/tier size caps | Publish engine | Idempotency-dedup store; publish-time size validation | **Provisional — ratify before Design lock** |

#### Key ADRs

| ADR ID | Decision Summary |
|--------|------------------|
| `cpt-cf-bss-pricing-adr-canonical-scope-key` | The single scope key is `(planId, currency, region, priceOverlay, phase, priceEligibility, chargeKind, cohort)` — the manifest key extended additively so hybrid components and a grandfathered row + its successor are distinct keys with concurrent active windows. |
| `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis` | Multi-generation grandfathering: the additive `cohort` axis (= the cutover instant; `none` on non-grandfathered rows) makes every cutover a **new** generation on its own key; within the grandfathered class Tariffs selects by the cohort of the subscription's pinned price id. |
| `cpt-cf-bss-pricing-adr-pricewindow-consolidation` | The `PriceWindow` machinery is gear-owned (Slice 7): store, state machine, activation job, `PriceWindow*` production; multi-window units are local ACID transactions. |

### 1.3 Architecture Layers

- [ ] `p3` - **ID**: `cpt-cf-bss-pricing-tech-stack`

```text
Capability slices (modules)   plan-definition · price-structure · currency-tax · governance ·
        │  (publish API: authorDraft / validate / publish / projectReadModel / requestVersion)
        ▼
Publish Engine (Foundation)   ScopeKey · DraftStateMachine · ValidationPipeline ·
                              VersioningStore · ReadModelProjector · SnapshotStamper · EventOutbox
        │
        ▼
PostgreSQL                    plan / price (truth + append-only history) · read_model (projection) ·
                              catalog_version_ref · outbox · policy objects · audit store
```

| Layer | Responsibility | Technology |
|-------|----------------|------------|
| Presentation | REST authoring/publish/preview + read-model surfaces behind the inbound gateway; RFC 9457 problems; OAuth 2.0; ETag optimistic concurrency | Rust, REST/OpenAPI, inbound API gateway |
| Application | Capability slices author draft state and register rules/read-model fields | Rust modules in the `pricing` monolith |
| Domain | The publish engine, canonical scope key, validation pipeline, versioning/immutability, snapshot contract | Rust; GTS + Rust domain structs |
| Infrastructure | Append-only history, projected read model, audit store, transactional outbox | PostgreSQL (single primary + replicas), SecureORM |

## 2. Principles and Constraints

### 2.1 Design Principles

#### Foundation owns publish; slices own capability policy

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-foundation-owns-publish-fnd`

No slice defines the scope key, emits an event, or stamps a snapshot; the Foundation defines
no capability semantics. Slices author draft state, register validation rules and read-model
fields, and publish through the Foundation API. Normative: [§4.2](#42-publish-through-the-engine-contract-normative).

#### Fail closed, always

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-fail-closed-fnd`

Any invalid or ambiguous condition blocks the publish transaction; the absence of a required
field is a publish failure, never a downstream default. Consumers never read draft state.

#### Published state is append-only

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-append-only-fnd`

Published `pricing_price` rows are immutable history; change is a new immutable row via
versioning/supersession + `PriceWindow`. Only never-published `draft` rows are deletable.
Normative: [§4.3](#43-immutability-and-change-mechanisms-normative).

### 2.2 Constraints

#### Money is ISO 4217 minor units

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-money-fnd`

Amounts are stored as integer minor units at the currency's ISO 4217 scale (0 for JPY/KRW, 2
default, 3 for BHD/KWD/OMR); a flat 2-decimal cap MUST NOT be assumed; amounts are `≥ 0`
(negatives rejected); no implicit FX — a missing `(currency, region)` row fails closed.

#### Author-driven mutation; UTC time

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-author-driven-fnd`

The catalog mutates state only in response to explicit authoring/publish/lifecycle calls; it
does not self-originate rows. All effective dating, window boundaries, `grandfatherUntil`,
`availableFrom`/`availableTo`, and anchor math are UTC.

#### AuthZ gate before the repository

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-authz-gate-fnd`

Every ctx-bearing service path calls the shared PEP `access_scope` gate
(`authz_resolver_sdk`) with its catalogued `(resource_type, action)` pair **before** touching
the repository; the PDP-compiled `AccessScope` is the SQL filter SecureORM binds (reads) and
the write-target membership assertion (writes). The resource/action catalog, the
endpoint mapping, and the role matrix are normative in the governance slice
([`05-governance.md`](./05-governance.md) §AuthZ Resource and Action Catalog); labels are
GTS ids `gts.cf.bss.pricing.<noun>.v1~` outside `gts.cf.resources.*`, registered as stub
type-schemas at gear init.

## 3. Technical Architecture

### 3.1 Domain Model

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-domain-model-fnd`

The Foundation owns four core aggregates; capability slices extend them with their own fields
and child tables (phases, add-on rules, bundles, price overlays) without redefining the core.

- **`Plan`** — binds a **published** `skuId` to a billing cycle, a mandatory `PlanTier`, optional plan phases and composition rules, and optional `availableFrom`/`availableTo` purchasability dates. Carries a lifecycle state (`draft` → `published` → `retired`) and a monotonic revision. Capability meaning of the cycle/composition fields is owned by the plan-definition slice.
- **`Price`** — a price row on the **canonical scope key** (§4.1) with an amount (ISO 4217 minor units), `modelKind`, tier bands, evaluation-policy fields, `taxInclusive`, lifecycle metadata (`priceEligibility`, optional `grandfatherUntil`), and a supersession pointer to the row it replaces within its scope key. Published rows are append-only; a prior row is retained as history.
- **`ReadModel`** — the projected, per-`CatalogVersion` frozen view a consumer resolves: `{skuId, planId, priceId}`, model kind, ordered tier bands, evaluation-policy fields, phase→price map, **phase→grant-set map** (per-phase entitlement grant set when authored — D-41; else the plan-level `PlanTier`-driven grant set), billing descriptors, and the consumer contracts (proration/plan-change/entitlement) contributed by their slices. Monotonic per version; never reflects draft state.
- **`pricingSnapshotRef`** — the composite reference (resolved price ids + evaluation-policy version + version ref) whose catalog-side identifiers are stamped at publish with a **pending** version ref and **finalized** to the committed `CatalogVersion` on `CatalogVersionPublished`, immutable thereafter; pinned by consumers (composition SoR: Tariffs).

Supporting Foundation objects: the **tenant policy objects** (approval-threshold policy,
tax-display policy — both fail-safe), the **idempotency-dedup** store, the **transactional
outbox**, and the **audit store** (append-only, actor/before-after/approval trail).

### 3.2 Component Model

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-foundation-fnd`

The Foundation is a set of in-process components behind one publish API:

- **`ScopeKey`** — constructs and validates the canonical scope key, applies the axis defaults (`priceOverlay = base`, `phase =` the plan's terminal `phase_id`, `priceEligibility = all_subscriptions`, `cohort = none`), and backs the row-uniqueness index.
- **`DraftStateMachine`** — the `draft` → `published` → `retired` transitions; only `draft` rows are mutable/deletable.
- **`ValidationPipeline`** — runs the aggregate fail-closed rule set at publish; slices register rules; a single failure blocks the publish transaction and populates the validation report.
- **`VersioningStore`** — writes new immutable rows on versioning/supersession, retains history, and enforces append-only via role + triggers.
- **`ReadModelProjector`** — materialises the frozen per-version read model and drives warm-completion; fails closed on outage.
- **`SnapshotStamper`** — stamps the catalog-side identifiers sufficient for the manifest `pricingSnapshotRef` (composition SoR: Tariffs) and requests the `CatalogVersion` increment from the registry.
- **`EventOutbox`** — emits the frozen event set transactionally, ordered per `(tenantId, aggregateId)`, at-least-once with dedup keys.

### 3.3 API Contracts

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-interface-authoring-publish-fnd`

The **authoring + publish** contract (`cpt-cf-bss-pricing-interface-authoring-publish`):
create/update/clone plans and price rows in `draft`, run fail-closed validation, submit for
approval (two-person rule for material changes), and publish — emitting the frozen event set
and requesting a `CatalogVersion`. Accepts client idempotency keys; enforces optimistic
concurrency via ETag/row version.

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-interface-catalog-read-model-fnd`

The **published read model** contract (`cpt-cf-bss-pricing-interface-catalog-read-model`):
per committed `CatalogVersion`, the complete plan/price read model resolvable via
`pricingSnapshotRef`, monotonic per version, no draft reads, additive-only within a major
version.

The base-price **preview** (`cpt-cf-bss-pricing-interface-price-preview`) and the external
integration contracts ([`../PRD.md`](../PRD.md) §9.2) are refined in the slices that own their
payloads. Concrete schemas, proto, and **slice-specific** error taxonomies are owned by these
slice designs. Failure modes of the engine itself carry **Foundation-owned** RFC 9457 problem
types, referenced (never redefined) by slices: `DUPLICATE_SCOPE_KEY` (409 — canonical
scope-key uniqueness), `STALE_VERSION` (409 — ETag/row-version conflict),
`IDEMPOTENCY_PAYLOAD_MISMATCH` (409 — same key, different payload), the aggregate validation
report envelope (422 — enumerating blocking `violations[]` plus advisory `warnings[]`), and
publish-accepted/pending (202).

### 3.4 Internal Dependencies

- **`toolkit-db`** — transactional persistence for the append-only history, the owned window store (Slice 7, D-03), the projected read model, the outbox, and the audit store.
- **Coordination lease library** — singleton coordination for read-model warm re-drive and the window activation/expiration job (Slice 7).

### 3.5 External Dependencies

- **Catalog registry (Product & SKU)** — published `skuId`, `bundle` SKU type, `meteringUnit` declaration, `PlanTier` taxonomy; the **sole** `CatalogVersion` incrementer. Bidirectional `CatalogVersion`-increment contract (`cpt-cf-bss-pricing-contract-registry-catalogversion`).
- ~~Effective-dating PriceWindows use case~~ — **consolidated into this gear** (Slice 7 owns the window store, state machine, activation job, and `PriceWindow*` emission — D-03); `cpt-cf-bss-pricing-contract-pricewindow` is thereby internal, not an external boundary.
- **Tariffs / Subscriptions / Rating / Billing** — consume the read model / events; their payloads are refined in the consumer-contracts and capability slices.

### 3.6 Interactions and Sequences

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-seq-author-publish-fnd`

**Author → validate → approve → publish → CatalogVersion.** A slice authors/updates draft rows
(ETag checked, idempotency-deduped); submit runs the ValidationPipeline as a **pre-check** and
routes a material change through the Slice 5 approval gate; on approval (or immediately for a
below-threshold change) the publish commit **re-runs the aggregate rule set inside the commit
transaction** (approval approves content; the commit re-validates state — a failure at commit
voids the approval and returns the subject to draft with the report); on success the row set
transitions to `published` (append-only), the EventOutbox enqueues `PlanPublished` with a
**pending** version ref, and the SnapshotStamper requests a `CatalogVersion`. The registry batches approved publishes and emits
`CatalogVersionPublished`; the ReadModelProjector warms the projection and marks completion, or
the publish is marked degraded (`PlanPublishDegraded`). `pricingSnapshotRef` pins the committed
version. No intermediate state exposes a rateable-but-incomplete plan. A
`pricing_catalog_version_ref` still `pending` past the max batching-delay SLO raises a
Critical alarm (`pricing.catalogversion.commit_overdue`) and surfaces on the publish status
API; a `CatalogVersionPublished` batch that omits an expected pending ref is treated the same
— remediation is a registry re-request, never a silent re-emit.

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-seq-readmodel-resolution-fnd`

**Consumer read-model resolution.** A consumer pins one committed `CatalogVersion` and resolves
the complete frozen read model via `pricingSnapshotRef` — no draft read, no default
substitution, monotonic per version; the pin never lags the newest completed version by > 5s.

### 3.7 Database Schemas and Tables

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-schema-fnd`

Foundation-owned tables (tenant-scoped, SecureORM). Money columns are integer minor units at
the currency's ISO 4217 scale. **Table-naming discipline (normative):** every physical table
of this gear carries the gear-name prefix **`pricing_`** — matching the sibling ledger's
`ledger_*` convention (`ledger_journal_entry`, `ledger_idempotency_dedup`, …). Domain entity
names (`Plan`, `Price`, `ReadModel`) stay unprefixed; the prefix applies to physical tables
only, including slice-owned tables in every other slice design.

- **`pricing_plan`** — `plan_id` (PK), `tenant_id`, `sku_id`, `plan_tier`, `billing_cycle`, `lifecycle_state` (`draft`|`published`|`retired`), `revision`, `available_from`/`available_to`, ETag/row-version. Updated in place only through DraftStateMachine transitions (`revision` increments monotonically, `lifecycle_state` per §3.2), fully audited; physical append-only enforcement applies to `pricing_price`.
- **`pricing_price`** — `price_id` (PK), `tenant_id`, the **canonical scope-key columns** (`plan_id`, `currency`, `region`, `price_overlay`, `phase`, `price_eligibility`, `charge_kind`, `cohort` — `none` unless `existing_grandfathered`, ADR-0002), `amount_minor`, `model_kind`, `tax_inclusive`, `billing_timing` (recurring), evaluation-policy columns (usage), `grandfather_until`, `supersedes_price_id`, `lifecycle_state`. **Partial `UNIQUE`** on the scope key over **current** rows (`lifecycle_state = 'published'` and not superseded, via the supersession link) enforces at most one current row per key — **temporal `PriceWindow` non-overlap and coverage are enforced by the publish-time validation pipeline (Slice 7, gear-owned per D-03), not by this index**, so a published predecessor and its scheduled successor legally coexist. Append-only via `REVOKE UPDATE, DELETE` + `BEFORE UPDATE/DELETE` trigger with a **column whitelist**: the trigger rejects any UPDATE of a published row except (a) `lifecycle_state` transitions permitted by the state machine (`published → superseded` on supersession/cutover) and (b) monotonic tightening of `grandfather_until` (setting it when null, or moving it earlier); all price/scope/model columns are immutable and DELETE is always rejected — controlled transitions run through the engine's transition path, never ad-hoc SQL.
- **Price history** — history is the set of superseded rows retained **in `pricing_price` itself**, keyed by `supersedes_price_id`; no rows are ever moved or deleted (no separate history table).
- **`pricing_read_model`** — the projected frozen view keyed by `(tenant_id, catalog_version, plan_id)` with a `warm_completed` marker; monotonic per `catalog_version`.
- **`pricing_catalog_version_ref`** — `pending` vs `committed` version linkage per publish.
- **`pricing_policy_object`** — the approval-threshold and tax-display policies (fail-safe defaults).
- **`pricing_idempotency_dedup`** — PK `(tenant_id, operation, client_key)` + a request-payload hash; the at-most-once gate + replay-response source. A replay with a matching hash returns the stored response; a mismatching hash is rejected with `IDEMPOTENCY_PAYLOAD_MISMATCH` (never replayed, never re-executed); the idempotency check precedes the ETag check.
- **`pricing_outbox`** — the transactional event outbox (frozen event names, dedup/correlation keys, `(tenantId, aggregateId)` ordering).
- **`pricing_audit_log`** — append-only actor/before-after/approval trail; ≥ 7-year configurable retention.

### 3.8 Deployment Topology

- [ ] `p3` - **ID**: `cpt-cf-bss-pricing-deployment-fnd`

Stateless authoring/publish + read-model service over a shared `toolkit-db` backend;
background work (read-model warm re-drive) is coordinated as a singleton via the coordination
lease library. The read path is served from the projected read model and fails closed on
outage. Deployment specifics are platform-standard for a BSS gear.

## 4. Additional Context

### 4.1 Canonical Scope Key (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-normative-scope-key`

The single scope key for **row-uniqueness, supersession, `PriceWindow` non-overlap, and window
coverage** is:

```text
(planId, currency, region, priceOverlay, phase, priceEligibility, chargeKind, cohort)
```

- Axis defaults: `priceOverlay = base` (rows authored here always carry `base`; partner/orgTier/brand overlays are separate `PriceOverlay` rows evaluated downstream by Tariffs), `phase =` **the plan's terminal `phase_id`** (D-19: the axis is always uuid-typed — for a phased plan its authored terminal phase; for non-phased/one-time plans an **implicit terminal phase row** (kind `evergreen`) is auto-created at plan creation and its id is the default; the literal `evergreen` survives only as the phase *kind*), `priceEligibility = all_subscriptions`, `chargeKind` per row, `cohort = none`.
- `cohort` (ADR-0002) is the **grandfathering generation discriminator** — the UTC cutover instant that created the generation. Publish validation enforces `cohort ≠ none ⇔ priceEligibility = existing_grandfathered`; every cutover creates a **new** generation on its own key, so repeated repricing with per-cohort retention never violates non-overlap. Within the `existing_grandfathered` class, Tariffs selects the row whose `cohort` equals the cohort of the subscription's **pinned price id** (`pricingSnapshotRef`); class ordering (most-specific-wins) is unchanged. Unrelated to `customerGroup` segment pricing.
- `chargeKind ∈ {recurring, usage, one_time, one_time_setup}` distinguishes the components a single plan legitimately carries at once: a hybrid plan holds a `recurring` **and** a `usage` row (optionally a `one_time_setup` row) on one `planId`, and a one-time plan's base row is `one_time` — so they are **distinct keys**, not duplicates.
- `brand` is **NOT** a price-row axis: brand-differentiated pricing is a **brand-scoped `PriceOverlay`** overlay (manifest §4.1 invariant).
- This key **extends the manifest `(plan, currency, region, priceOverlay)` key additively** with `phase`, `priceEligibility`, `chargeKind` (ADR `cpt-cf-bss-pricing-adr-canonical-scope-key`) and `cohort` (ADR `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis`), and **supersedes** the narrower effective-dating `(plan, currency, region, priceOverlay)` key for normative purposes.

Because `priceEligibility` and `cohort` are part of the key, a grandfathered generation and
its successor — and any number of prior generations — are **distinct keys** that hold active
windows concurrently at the same instant without violating non-overlap (§4.3).

### 4.2 Publish-Through-The-Engine Contract (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-normative-publish-contract`

Every state change that reaches production follows one path:

1. **Author draft** — create/update/clone in `draft` (ETag-checked, idempotency-deduped). Only `draft` rows are mutable/deletable.
2. **Validate (fail closed)** — the aggregate validation pipeline runs the §17.4 rule set plus every slice-registered rule; a single failure blocks the submission with an enumerated report. This step is a **pre-check**: the same rule set re-runs inside the publish-commit transaction of step 4 (approval approves content; the commit re-validates state — a commit-time failure voids the approval and returns the subject to draft with the report). No `PlanPublished`, no read-model warm on any failure.
3. **Approve** — a material change (above the configured threshold, or a first publish) requires the submitter **plus ≥ 1 independent approver** (two distinct principals; self-approval rejected + audited). Fail-safe: the two-person rule applies unless an explicit threshold is configured and the change is below it and it is not a first publish. Pending approval is **not** a `lifecycle_state`: the subject stays `draft` and remains mutable — the open Slice 5 approval record marks it, and any mutation of the subject **voids** that record ("returns to draft" in the PRD means the approval record closes).
4. **Freeze + emit** — the publish commit re-runs the pipeline, transitions the row set, and stamps the catalog-side `pricingSnapshotRef` identifiers; the frozen event set is enqueued transactionally (`PlanPublished` with a **pending** version ref).
5. **Version + warm** — the registry (sole incrementer) batches approved publishes and emits `CatalogVersionPublished`; the read model warms to the 5s SLO or the publish is marked degraded (`PlanPublishDegraded`). No intermediate state exposes a rateable-but-incomplete plan.

Publish units are not only plans: Slice 9's `PriceOverlays` and customer-group
membership mutations publish through the **same engine** (validation → pending ref → warm;
D-06) — nothing becomes consumer-visible outside a committed `CatalogVersion`.

Consumers never read draft state and never substitute a default for an absent
evaluation-policy field (absence must have failed step 2). The catalog computes **no** monetary
charge, evaluates **no** overlay, and performs **no** FX.

### 4.3 Immutability and Change Mechanisms (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-normative-immutability`

Published `pricing_price` rows are **append-only history**: `REVOKE` + a trigger with a
**column whitelist** reject any UPDATE except the state-machine `lifecycle_state` transitions
(`published → superseded`) and monotonic `grandfather_until` tightening — price/scope/model
columns are immutable and DELETE is always rejected (§3.7); only never-published `draft` rows
are deletable; there is no deletion event to fan out. Change over time uses four **distinct,
composable** mechanisms ([`../PRD.md`](../PRD.md) §17.5):

- **Versioning** — captures a structural/price change as a new immutable revision; prior rows retained as history (`PlanUpdated` / `PriceCreated`).
- **Supersession** — versioning scoped to **one canonical scope key**: a new immutable row plus opening/closing the corresponding `PriceWindow` (never overlap), within one `priceEligibility` class and one `chargeKind` (`PriceUpdated`).
- **`PriceWindow`** — schedules **when** a versioned/superseded row is effective (window store, state machine, and activation job owned by Slice 7 in this gear — D-03).
- **Grandfathering cutover** — one atomic approval unit that shortens the current `all_subscriptions` window `effectiveTo` to the cutover and schedules (a) an immutable `existing_grandfathered` copy and (b) the `all_subscriptions` successor, so **no coverage gap opens**.

An `existing_grandfathered` row is **immutable in price** and MUST NOT be superseded; the only
permitted mutation is **setting or tightening `grandfatherUntil`** (never loosening, never the
price), which is a material change. Because it is a distinct scope key (via `priceEligibility`),
it holds an active window concurrently with its successor and is **live-resolved** by Tariffs
against an immutable row — reconciling live resolution with the frozen-snapshot doctrine.

### 4.4 Read Model and pricingSnapshotRef (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-normative-read-model`

The published read model is **monotonic per `CatalogVersion`**: a version is ignored until its
`CatalogVersionPublished` **and** the warm-completion marker are both present. A consumer pins
**one** `CatalogVersion` for the duration of a resolution/rating run and resolves the complete
frozen view via `pricingSnapshotRef`; at pin time the pinned version MUST NOT lag the newest
completed version by more than 5s. There is **no** draft read and **no** default substitution.

`pricingSnapshotRef` is the composite reference (`CatalogVersion` + resolved price ids +
evaluation-policy version) pinned on charges and `BillableItem`s — stamped at publish with the
**pending** version ref, finalized to the committed `CatalogVersion` on
`CatalogVersionPublished`, and immutable thereafter; posted invoice periods MUST NOT re-query
mutable catalog rows. The **normative composition SoR is Tariffs**; the catalog-side view is
the aligned entry and MUST NOT diverge from it. On read-model outage the read path **fails
closed** (never serves stale). After a degraded publish the warm **re-drive continues past the
SLO**; on completion it sets the warm-completion marker (the version becomes resolvable —
monotonicity unaffected) and clears the degraded mark, raising an operations alarm meanwhile;
no new event name is introduced — consumers observe completion via the marker.

## 5. Traceability

- **PRD**: [`../PRD.md`](../PRD.md) — §2.2 (canonical scope key), §6.2/§6.7 (model kind, publish, events), §6.8 (versioning/immutability/supersession), §6.9 (consumer resolution), §9 (interfaces), §17.4 (validation rules), §17.5 (change mechanisms + `CatalogVersion` increment)
- **DESIGN**: [`../DESIGN.md`](../DESIGN.md) — canonical index (slice map, dependency order, cross-cutting statements)
- **ADRs**: [`../ADR/`](../ADR/) — `cpt-cf-bss-pricing-adr-canonical-scope-key`, `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis`, `cpt-cf-bss-pricing-adr-pricewindow-consolidation`

This slice directly addresses:

- `cpt-cf-bss-pricing-fr-publish-validation-failclosed` — the aggregate fail-closed validation pipeline
- `cpt-cf-bss-pricing-fr-published-rows-append-only` / `cpt-cf-bss-pricing-fr-plan-versioning` / `cpt-cf-bss-pricing-fr-supersession` — append-only history + versioning/supersession
- `cpt-cf-bss-pricing-fr-pricing-snapshot` / `cpt-cf-bss-pricing-fr-consumer-readmodel-resolution` — snapshot + monotonic read model
- `cpt-cf-bss-pricing-fr-catalogversion-increment` / `cpt-cf-bss-pricing-fr-publish-fanout-atomicity` / `cpt-cf-bss-pricing-fr-event-contract` — `CatalogVersion` request, degraded handling, frozen event set
- `cpt-cf-bss-pricing-fr-price-amount-validation` / `cpt-cf-bss-pricing-fr-mutation-idempotency` / `cpt-cf-bss-pricing-fr-concurrent-edit` — money/precision, idempotency, optimistic concurrency
- `cpt-cf-bss-pricing-interface-authoring-publish` / `cpt-cf-bss-pricing-interface-catalog-read-model` — the two primary API contracts
