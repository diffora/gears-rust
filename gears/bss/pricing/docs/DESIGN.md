<!-- CONFLUENCE_TITLE: [BSS]: Plan & Price Modeling — Technical Design (canonical index) -->
<!-- Related: ./PRD.md, ./ADR/, ./design/ | Owners: BSS Product Catalog team -->

# Technical Design — Plan & Price Modeling

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
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
  - [3.8 Deployment Topology](#38-deployment-topology)
- [4. Additional context](#4-additional-context)
- [5. Traceability](#5-traceability)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-design-main`

> **Canonical design entry point and index.** This document is Plan & Price Modeling's
> top-level technical design and the anchor for spec traceability. The design is authored
> as a **set of slice documents** under [`design/`](./design/) — a shared **Catalog
> Foundation** (the Plan/Price entity model, the canonical scope key, the publish engine,
> the fail-closed validation framework, the frozen read model + `pricingSnapshotRef`
> contract, and the event fan-out) plus per-capability slice designs. This page is the
> single index over that set — architecture overview, the phased slice map, dependency
> order, the cross-cutting normative statements, the ADR index, and the traceability
> surface — and delegates slice-level specifics (schemas, sequences, validation-rule
> internals) to the slice documents so they stay the single source of truth for their
> detail.

## 1. Architecture Overview

### 1.1 Architectural Vision

Plan & Price Modeling is the BSS Product Catalog's authoring surface and **System of
Record** for `Plan`, `Price`, bundle/add-on composition, and billing descriptors. It never
computes a charge, evaluates an overlay, or performs FX — it defines **what** the pricing
primitives MUST contain so that **Tariffs** can evaluate, **Subscriptions** can sell, and
**Rating** can charge deterministically and reproducibly from a frozen snapshot ([`PRD.md`](./PRD.md) §1.1).

The design mirrors the sibling Billing Ledger's shape: a shared **Catalog Foundation**
([`design/01-foundation.md`](./design/01-foundation.md)) that owns the `Plan`/`Price` entity
model, the **canonical scope key**, the draft→publish state machine, the aggregate
**fail-closed validation pipeline** framework, append-only published-row history +
versioning/supersession, the **read-model projection** + `pricingSnapshotRef` stamping, and
the frozen event fan-out + `CatalogVersion`-increment request. Each business capability is a
**slice handler** that authors draft entity state, registers its validation rules and its
projected read-model fields, and **publishes *through* the Foundation** under the invariants
defined there. The Foundation owns no capability policy (it does not know what a billing
cycle is); slices own no publish mechanics (they never emit an event or stamp a snapshot
themselves). This keeps the correctness-critical publish/immutability/determinism core small
and auditable while letting each pricing capability evolve independently.

Where the ledger's contract is *post through the engine* (build balanced lines → commit),
the catalog's contract is **publish through the engine**: author draft → run the fail-closed
validation pipeline → freeze a complete read model + `pricingSnapshotRef` → emit the frozen
event set → request a `CatalogVersion`. Consumers resolve **only** committed versions, never
draft state, and never substitute a default for an absent field (absence must have failed
publish).

Requirements (WHAT/WHY) live in [`PRD.md`](./PRD.md); the "why this way" rationale for the
canonical scope key and the snapshot-versioning strategy is captured as ADRs in
[`ADR/`](./ADR/).

### 1.2 Architecture Drivers

Requirements from [`PRD.md`](./PRD.md) that significantly influence the architecture.

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-pricing-fr-publish-validation-failclosed` | The Foundation runs a single **aggregate fail-closed validation pipeline** at publish; slices register rules into it, and any invalid condition (§17.4) blocks `PlanPublished` and read-model warm — absence of a required field fails publish, never defaults downstream ([`design/01-foundation.md`](./design/01-foundation.md)). |
| `cpt-cf-bss-pricing-fr-published-rows-append-only` | Published `Price` rows are append-only history: `REVOKE UPDATE, DELETE` from the app role + `BEFORE UPDATE/DELETE` triggers; only never-published `draft` rows are deletable. Change is a new immutable row via versioning/supersession. |
| `cpt-cf-bss-pricing-fr-plan-versioning` / `cpt-cf-bss-pricing-fr-supersession` | Versioning creates a new immutable `Price` revision; supersession is versioning scoped to **one canonical scope key**, opening/closing a `PriceWindow` rather than overlapping it (§17.5). |
| `cpt-cf-bss-pricing-fr-pricing-snapshot` | Publish stamps the catalog-side identifiers sufficient for the manifest `pricingSnapshotRef` (resolved price ids + evaluation-policy version + the **pending** version ref, finalized to the committed `CatalogVersion` on `CatalogVersionPublished`, immutable thereafter); posted periods never re-query mutable rows. The catalog-side view MUST NOT diverge from the Tariffs composition SoR. |
| `cpt-cf-bss-pricing-fr-consumer-readmodel-resolution` | The read model is **monotonic per `CatalogVersion`**; consumers resolve `{skuId, planId, priceId}` + model kind + tier bands + evaluation-policy fields exactly as published, no draft read, no default substitution; a rating run pins one version. |
| `cpt-cf-bss-pricing-fr-catalogversion-increment` | On every `PlanPublished` the Foundation requests addressability; the registry (sole incrementer) MAY batch approved publishes; `PlanPublished` carries a **pending** ref and the snapshot pins the committed version on `CatalogVersionPublished` (§17.5). |
| `cpt-cf-bss-pricing-fr-publish-fanout-atomicity` | Post-commit read-model warming retries to the 5s SLO or emits `PlanPublishDegraded`; no state exposes a rateable-but-incomplete plan. |
| `cpt-cf-bss-pricing-fr-event-contract` | A **frozen event-name set** emitted with correlation/idempotency keys, ordered per `(tenantId, aggregateId)`, at-least-once, dedupable. |
| `cpt-cf-bss-pricing-fr-approval-two-person` / `cpt-cf-bss-pricing-fr-approval-threshold-policy` | A material change requires submitter + ≥1 independent approver (two distinct principals); fail-safe materiality (two-person rule unless an explicit threshold is configured and the change is below it and not a first publish). |
| `cpt-cf-bss-pricing-fr-model-kind` / `cpt-cf-bss-pricing-fr-tier-validation` | Explicit `modelKind` (no rating-time default) + `[fromQty, toQty)` tier bands validated ascending/non-overlapping/contiguous with an **always-open** top band (a closed top fails publish — capping is owned by entitlement quotas, D-17). |
| `cpt-cf-bss-pricing-fr-price-amount-validation` | Amount ≥ 0, valid ISO 4217, precision = the currency's ISO 4217 **minor unit** (no flat 2-decimal cap), no implicit FX (fail closed when a `(currency, region)` row is absent). |
| `cpt-cf-bss-pricing-fr-concurrent-edit` / `cpt-cf-bss-pricing-fr-mutation-idempotency` | Optimistic concurrency (ETag/version) rejects stale submits and bulk-vs-interactive collisions; client idempotency keys make create/update replays return the original. |

#### NFR Allocation

Non-functional requirements are specified in [`PRD.md`](./PRD.md) §7. Several load-bearing
targets are **provisional** and MUST be ratified before Design lock (no bare placeholders may
ship — [`PRD.md`](./PRD.md) §14); they are surfaced here with that status.

| NFR theme | Allocated to | Design Response | Status |
|-----------|--------------|-----------------|--------|
| Publish → read-model propagation (p95 ≤ 5s) | Foundation publish engine + event fan-out | Batched `CatalogVersion` commit + retry-to-SLO warm or `PlanPublishDegraded`; pin never lags newest completed by > 5s | Committed target; batching-delay SLO **open with Registry** ([`PRD.md`](./PRD.md) §15) |
| Read / preview latency (p95 < 100ms per tenant partition) | Read-model projection store | Single indexed read of the projected, version-pinned read model; no evaluation on the read path | Committed target |
| Read-model availability / DR RPO/RTO | Read-model store + deployment topology | Fail-closed on read-model outage (never stale); DR numbers **provisional** | **Provisional — ratify before Design lock** |
| Determinism / reproducibility | Foundation snapshot + immutability | Append-only published rows, monotonic version, complete frozen `pricingSnapshotRef` | Committed |
| Audit retention (≥ 7 years, jurisdiction-configurable) | Governance/audit slice + Foundation audit store | Append-only, tamper-evident audit of every mutation + approval trail | Committed; retention-maximum vs minimum **open with Legal** ([`PRD.md`](./PRD.md) §15) |
| Mass-repricing worst-case throughput; plan/tier size caps; idempotency-key TTL | Foundation + operator-efficiency slice | Idempotent, deduplicated bulk events; per-row commit | **Provisional — ratify before Design lock** |

#### Key ADRs

| ADR ID | Decision Summary |
|--------|------------------|
| `cpt-cf-bss-pricing-adr-canonical-scope-key` | The single scope key for row-uniqueness, supersession, `PriceWindow` non-overlap, and coverage is `(planId, currency, region, priceOverlay, phase, priceEligibility, chargeKind, cohort)` — the manifest's `(plan, currency, region, priceOverlay)` key extended **additively**, so a hybrid plan's components and a grandfathered row + its successor are **distinct keys** that can hold concurrent active windows without violating non-overlap. |
| `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis` | Multi-generation grandfathering: the additive `cohort` axis (the cutover instant; `none` on non-grandfathered rows) makes every cutover a **new** coexisting generation; within the grandfathered class Tariffs selects the row by the cohort of the subscription's pinned price id (`pricingSnapshotRef`). |
| `cpt-cf-bss-pricing-adr-pricewindow-consolidation` | The `PriceWindow` machinery (store, state machine, UTC activation job, `PriceWindow*` event production — frozen manifest names) is **owned by this gear** (Slice 7); the legacy effective-dating UC is absorbed as scenario source, and the cutover's multi-window unit is one local ACID transaction. |

Additional ADRs are planned as the dependent slices land (snapshot/versioning strategy,
grandfathered-row immutability, customer-group ownership, derived-meter formula-as-data,
`CatalogVersion` increment/batching, `brand`-as-`PriceOverlay`) — see [§5 Traceability](#5-traceability)
and [`design/README.md`](./design/README.md).

### 1.3 Architecture Layers

```text
Capability slices  plan-definition · price-structure · currency-tax · pricewindow-linkage ·
(authoring policy) consumer-contracts · bundles · price-overlays · advanced-primitives ·
                   governance · lifecycle · operator-efficiency
       │  author draft state; register validation rules + read-model fields; publish through
       ▼           the Foundation API — own no publish/immutability/snapshot mechanics
Catalog            Plan/Price entity model · canonical scope key · draft→publish state machine ·
Foundation         fail-closed validation pipeline · append-only history + versioning/supersession ·
(shared engine)    read-model projection + pricingSnapshotRef · event fan-out + CatalogVersion request
       │           — owns no capability policy
       ▼
Persistence        toolkit-db backend (append-only published-row history; projected read model;
                   audit store; event outbox; ISO 4217 minor-unit money as integer minor units)
```

| Layer | Responsibility | Technology |
|-------|----------------|------------|
| Presentation | REST authoring/publish/preview + read-model surfaces behind the inbound gateway; RFC 9457 problems; OAuth 2.0; ETag optimistic concurrency | Rust, REST/OpenAPI, inbound API gateway |
| Application | Capability slices author draft state and register rules/read-model fields; each is a bounded feature | Rust modules in the `pricing` gear |
| Domain | The Foundation publish engine, canonical scope key, validation pipeline, versioning/immutability, snapshot contract | Rust; GTS + Rust domain structs |
| Infrastructure | Append-only published-row history, projected read model, audit store, event outbox | PostgreSQL, SecureORM |

#### Design set (ordered by implementation phase)

The numeric prefix = **implementation order** (dependency-ordered phasing), **not** the PRD
§6 subsection number. As in the ledger, the two axes deliberately do not line up: a slice is
scoped by PRD decomposition but built when its dependencies exist. The full slice map,
dependency graph, and phase rationale live in [`design/README.md`](./design/README.md).

| Doc | PRD §6 | Phase | What it is |
|-----|--------|-------|------------|
| [`design/01-foundation.md`](./design/01-foundation.md) | 6.2/6.7 core, §17.4/17.5 | 0/1 | **Foundation**: `Plan`/`Price` model, canonical scope key, draft→publish state machine, fail-closed validation pipeline, append-only history + versioning/supersession, read-model projection + `pricingSnapshotRef`, event fan-out + `CatalogVersion` request, tenant isolation, ISO 4217 money, idempotency/ETag. Carries the catalog-wide normative statements. |
| [`design/02-plan-definition.md`](./design/02-plan-definition.md) | 6.1, 6.3 | 1 | Billing cycles, custom frequency, per-seat quantity provenance (`quantitySource` persisted/validated in Slice 3), one-time-setup row, mandatory `PlanTier`, meter injectivity, add-on rules, phases + `convertsToPhaseId`, billing descriptors. |
| [`design/03-price-structure.md`](./design/03-price-structure.md) | 6.2 | 1 | Explicit `modelKind`, graduated/volume tier-band validation, `package` (block) pricing, evaluation-policy placement, joint golden-fixture conformance gate. |
| [`design/04-currency-tax.md`](./design/04-currency-tax.md) | 6.4 | 1/2 | Region/brand taxonomy validation, `taxInclusive`/`taxCategory` display basis + tax-display policy, single-currency-per-invoice binding. |
| [`design/05-governance.md`](./design/05-governance.md) | 6.7, 6.12 | 1/2 | Two-person rule + segregation of duties, per-currency threshold policy, RBAC deny-by-default, tenant/brand/region isolation, historical-import governance, audit completeness/retention. |
| [`design/06-consumer-contracts.md`](./design/06-consumer-contracts.md) | 6.9 | 2 | Proration input contract, `billingTiming`, entitlement grant set, plan-change contract, rating compatibility, canonical `prorationBasis` enum. |
| [`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md) | 6.5 | 2 | `PriceWindow` ownership (store, state machine, activation job, `PriceWindow*` events — D-03), publish-time window coverage + future-gap checks, sellability gate, `priceEligibility`/`cohort`/`grandfatherUntil` + most-specific-wins resolution. |
| [`design/08-bundles.md`](./design/08-bundles.md) | 6.3 (bundle) | 2/3 | Bundle price basis, currency coverage, rev-share reconciliation, `invoiceItemization`. |
| [`design/09-price-overlays.md`](./design/09-price-overlays.md) | 6.6 | 3 | `PriceOverlay` authoring/validation + `customerGroup` segment pricing (BSS-owned taxonomy, effective-dated audited membership). |
| [`design/10-advanced-primitives.md`](./design/10-advanced-primitives.md) | 6.10 | 3 | Reserved capacity (p1), prepaid credit grant (p2), derived (composite) meter formula-as-data (p2), `discountRef` hook, typed `minQtyThreshold`. |
| [`design/11-lifecycle.md`](./design/11-lifecycle.md) | 6.8 | 3/4 | Retirement, scheduled migration + `PlanLink`, idempotency/cancellation, contract-lock protection, legacy `migrated-origin` snapshot synthesis. |
| [`design/12-operator-efficiency.md`](./design/12-operator-efficiency.md) | 6.11 | 4 | Clone, bulk import (all-or-nothing validate / per-row commit), mass repricing, price history + export. |

#### Dependency order

```text
01-foundation (scope key, publish engine, validation pipeline, read model + snapshot, events, immutability)
    │
    ├─→ 02-plan-definition ─┬─→ 03-price-structure   (Phase 1; a rateable plan needs both)
    │                       │
    ├─→ 04-currency-tax     │                          (Phase 1/2)
    ├─→ 05-governance       │  (gates every publish)   (Phase 1/2)
    │                       ▼
    ├─→ 06-consumer-contracts (Phase 2; projects read-model fields onto 02/03 rows)
    ├─→ 07-pricewindow-linkage (Phase 2; needs scope key + price rows)
    ├─→ 08-bundles             (Phase 2/3; references component planIds → needs 02–04)
    ├─→ 09-price-overlays         (Phase 3; overlays on published base rows)
    ├─→ 10-advanced-primitives (Phase 3; reserved p1 may pull earlier)
    ├─→ 11-lifecycle           (Phase 3/4; needs windows + grandfathering + snapshot synthesis)
    └─→ 12-operator-efficiency (Phase 4; clone/bulk/mass over the full authored surface)
```

- `02-plan-definition` + `03-price-structure` are co-required: the minimum rateable plan needs both a shape and a model kind/tier structure.
- `04-currency-tax` and `05-governance` gate the first *sellable* publish (currency/tax display + the approval gate).
- `06-consumer-contracts` and `07-pricewindow-linkage` form the downstream-determinism surface: the read-model fields consumers depend on and the coverage/sellability gate.
- `08-bundles` references published component `planId`s, so it follows 02–04.
- `09-price-overlays`, `10-advanced-primitives` layer overlays/primitives on published base rows.
- `11-lifecycle` needs windows (07) + grandfathering + `migrated-origin` snapshot synthesis.
- `12-operator-efficiency` operates over the whole authored surface, so it is last.

## 2. Principles & Constraints

The catalog-wide normative statements are authored in the Foundation design (§4); they are
surfaced here as design principles/constraints with stable ids.

### 2.1 Design Principles

#### Foundation owns publish; slices own capability policy

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-foundation-owns-publish`

No slice emits an event, stamps a snapshot, or defines the scope key; the Foundation defines
no capability semantics (billing cycle, model kind, bundle). Slices author draft state,
register validation rules and read-model fields, and publish through the Foundation API.

#### Publish through the engine

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-publish-through-engine`

Every state change reaches production one way: author draft → fail-closed validation pipeline
→ freeze read model + `pricingSnapshotRef` → emit the frozen event set → request a
`CatalogVersion`. There is no side door that mutates published state.

#### Fail closed

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-fail-closed`

Any invalid or ambiguous condition blocks publish and read-model warm; the absence of a
required field is a publish failure, never a downstream default. Consumers never read draft
state and never substitute defaults.

#### Published state is append-only

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-published-append-only`

Published `Price` rows are immutable history; change is a new immutable row via
versioning/supersession + `PriceWindow`. Only never-published `draft` rows are deletable.
Grandfathered rows are immutable in price — the only permitted mutation is tightening
`grandfatherUntil`.

#### Determinism via frozen snapshot

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-frozen-snapshot`

Consumers resolve a complete, frozen read model via `pricingSnapshotRef`, monotonic per
committed `CatalogVersion`; posted invoice periods never re-query mutable catalog rows. The
catalog-side snapshot view MUST NOT diverge from the Tariffs composition SoR.

#### No charge computation

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-principle-no-charge-computation`

The catalog persists and publishes structure only; it computes no monetary charge, evaluates
no overlay, and performs no FX. All mathematical formulas belong to Tariffs.

### 2.2 Constraints

#### Canonical scope key

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-canonical-scope-key`

The single scope key for row-uniqueness, supersession, `PriceWindow` non-overlap, and window
coverage is `(planId, currency, region, priceOverlay, phase, priceEligibility, chargeKind, cohort)`.
Rows authored here always carry `priceOverlay = base`; defaults `phase =` the plan's terminal `phase_id` (id-typed axis; implicit terminal phase auto-created for non-phased plans — D-19),
`priceEligibility = all_subscriptions`, `cohort = none` (`cohort` ≠ `none` only on
`existing_grandfathered` generations — each cutover creates a new one). Normative:
[`design/01-foundation.md` §4](./design/01-foundation.md) · ADRs `cpt-cf-bss-pricing-adr-canonical-scope-key`, `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis`.

#### Money is ISO 4217 minor units

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-iso4217-minor-units`

Amount precision follows the currency's ISO 4217 minor unit (0 for JPY/KRW, 2 default, 3 for
BHD/KWD/OMR); a flat 2-decimal cap MUST NOT be assumed. Amounts are `≥ 0` (negatives
rejected; typed credit rows are Future). No implicit FX — a missing `(currency, region)` row
fails closed.

#### UTC time

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-utc-time`

All effective dating, `PriceWindow` boundaries, `grandfatherUntil`, `availableFrom`/`availableTo`,
and anchor math are UTC.

#### Tenant isolation; region decoupled from authz region

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-tenant-isolation`

Plans, prices, and price overlays are tenant-scoped; SecureORM binds every query to the caller's
tenant. The pricing `region` axis is a **commercial territory** and is decoupled from the IdP
authorization-region claim; `region`/`brand` values MUST be members of the tenant's
configured taxonomies, validated before publish.

#### AuthZ: PEP gate + resource/action catalog

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-constraint-authz-catalog`

Every API surface enforces through the shared PEP `access_scope` gate with a
`(resource_type, action)` pair from the single normative catalog — GTS labels
`gts.cf.bss.pricing.<noun>.v1~` (plan, bundle, price_overlay, customer_group, approval,
approval_policy, config, historical_import, audit), all outside `gts.cf.resources.*` so only
explicit catalog roles cover them; actions sit on real objects, never authz tiers.
Normative: [`design/05-governance.md`](./design/05-governance.md) §AuthZ Resource and Action
Catalog · gate constraint in [`design/01-foundation.md`](./design/01-foundation.md) §2.2.

## 3. Technical Architecture

The technical architecture is specified per slice in the [`design/`](./design/) set, with the
shared substrate in [`design/01-foundation.md`](./design/01-foundation.md). This section
summarises the cross-slice shape and declares the component/sequence ids; the phased slice
map and dependency order are in §1.3 and [`design/README.md`](./design/README.md).

### 3.1 Domain Model

Core entities live in the Foundation: `Plan` (binds a published SKU to a billing cycle,
mandatory `PlanTier`, optional phases, composition rules) and `Price` (a price row on the
canonical scope key with amount/currency, `modelKind`, tier bands, evaluation-policy fields,
and lifecycle metadata). Published rows are append-only, with immutable history rows preserved
on every versioning/supersession. A projected **read model** materialises the complete,
frozen per-`CatalogVersion` view consumers resolve via `pricingSnapshotRef`. Full field-level
definitions and the naming discipline are normative in
[`design/01-foundation.md`](./design/01-foundation.md) §4.

### 3.2 Component Model

Components are handlers over the shared Foundation, not independently deployable services.
Each carries a stable `cpt-cf-bss-pricing-component-{slug}` ID; phasing and dependency
order are in §1.3 and the linked slice doc is normative for its internals.

#### Catalog Foundation

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-foundation`

Shared publish engine: `Plan`/`Price` model, canonical scope key, draft→publish state machine,
fail-closed validation pipeline, append-only history + versioning/supersession, read-model
projection + `pricingSnapshotRef`, event fan-out + `CatalogVersion` request ([`design/01-foundation.md`](./design/01-foundation.md)).

#### Plan-definition handler

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-plan-definition`

Billing cycles, custom frequency, per-seat quantity provenance (`quantitySource` in Slice 3),
one-time-setup row, mandatory `PlanTier`, meter injectivity, add-on rules, phases, billing
descriptors ([`design/02-plan-definition.md`](./design/02-plan-definition.md)).

#### Price-structure handler

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-price-structure`

Explicit `modelKind`, graduated/volume tier-band validation, `package` pricing, conformance
fixtures ([`design/03-price-structure.md`](./design/03-price-structure.md)).

#### Currency-tax handler

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-currency-tax`

Per-`(currency, region)` rows, region/brand taxonomies, tax-display basis + `not_sellable_ga`
gate, single-currency-per-invoice binding, base-price preview ([`design/04-currency-tax.md`](./design/04-currency-tax.md)).

#### Governance handler

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-governance`

Two-person rule, per-currency threshold policy, RBAC deny-by-default + the AuthZ
resource/action catalog, isolation, audit/retention
([`design/05-governance.md`](./design/05-governance.md)).

#### Consumer-contracts handler

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-consumer-contracts`

Proration input contract, `billingTiming`, entitlement grant set, plan-change contract, rating
compatibility ([`design/06-consumer-contracts.md`](./design/06-consumer-contracts.md)).

#### PriceWindow-linkage handler

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-pricewindow-linkage`

`PriceWindow` ownership (store/state machine/activation job — D-03), publish-time window coverage + future-gap, sellability gate, grandfathering resolution ([`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md)).

#### Bundles handler

- [ ] `p2` - **ID**: `cpt-cf-bss-pricing-component-bundles`

Bundle price basis, component currency/frequency coverage, rev-share reconciliation,
itemization ([`design/08-bundles.md`](./design/08-bundles.md)).

#### Price-overlays handler

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-price-overlays`

`PriceOverlay` authoring/validation + the customer-group taxonomy, effective-dated
audited membership, resolved-group freezing ([`design/09-price-overlays.md`](./design/09-price-overlays.md)).

#### Advanced-primitives handler

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-advanced-primitives`

Reserved capacity (same-row attributes), prepaid grant (GA-gated), derived meter
formula-as-data, `discountRef` hook, typed `minQtyThreshold`
([`design/10-advanced-primitives.md`](./design/10-advanced-primitives.md)).

#### Lifecycle handler

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-component-lifecycle`

Retirement, scheduled migration + `PlanLink`, contract-lock protection, `migrated-origin`
snapshot synthesis ([`design/11-lifecycle.md`](./design/11-lifecycle.md)).

#### Operator-efficiency handler

- [ ] `p2` - **ID**: `cpt-cf-bss-pricing-component-operator-efficiency`

Clone, two-phase bulk import, journaled mass repricing, history + export
([`design/12-operator-efficiency.md`](./design/12-operator-efficiency.md)).

### 3.3 API Contracts

The two primary contracts — the **authoring + publish** surface
(`cpt-cf-bss-pricing-interface-authoring-publish`) and the **published read model**
(`cpt-cf-bss-pricing-interface-catalog-read-model`) — are owned by the Foundation and
specified in [`design/01-foundation.md`](./design/01-foundation.md) §3.3. The base-price
**preview** (`cpt-cf-bss-pricing-interface-price-preview`) and the external integration
contracts (Tariffs read-model, Subscriptions publish, Registry `CatalogVersion`, Billing
descriptors, PriceWindow linkage — [`PRD.md`](./PRD.md) §9.2) are refined in the slices that
own their payloads. Concrete schemas, proto, and error taxonomies are owned by the slice
designs, not the PRD.

### 3.4 Internal Dependencies

- **`toolkit-db`** — transactional persistence for the append-only published-row history, the projected read model, the audit store, and the event outbox.
- **Coordination lease library** — singleton coordination for background work (read-model warming re-drive, window activation/expiration, mass-repricing runs, scheduled-migration dispatch).

### 3.5 External Dependencies

The catalog integrates with the BSS actors and systems defined in [`PRD.md`](./PRD.md) §3.2 /
§13. These are integration boundaries, not components owned here:

- **Catalog registry (Product & SKU)** — SoR for `Product`/`SKU`/`Category`/`Attribute`/`CatalogVersion`, the `bundle` SKU type, `meteringUnit` declaration, and the `PlanTier` taxonomy; the **sole** `CatalogVersion` incrementer. The catalog consumes published SKUs and freezes content into the version.
- ~~PriceWindow (effective-dating use case)~~ — **consolidated into this gear** (D-03; PRD §15 answered): Slice 7 owns the window store, state machine, UTC activation job, and `PriceWindow*` event emission; the legacy UC document remains scenario source material.
- **Tariffs / PLAL** — consumes the read model and evaluates formulas/overlays/FX; composes the `pricingSnapshotRef` (composition SoR).
- **Subscriptions** — owns the plan-change boundary/mode + runtime, plan-change classification, trial runtime, entitlement enforcement, `PlanLink` migration, sellability checks (proration math = rating gear).
- **Rating** — consumes events + warmed read models; owns Usage → `RatedCharge` orchestration.
- **Billing / Payments** — consumes descriptors via `CatalogVersion`, derives deferral from `billingTiming`, owns refunds/credits and PSP/ERP posting.
- **Tax Engine** — scheme determination + `region` → jurisdiction mapping; **confirmed post-MVP**. MVP is tax-exclusive; `taxInclusive=true` plans are authorable but GA-gated.
- **Contracts** — contract locks + negotiated RI-style reservation rates.
- **Promotions** — coupon/discount authoring + evaluation (**PRD does not yet exist**); `discountRef` resolves to a registered external instrument.
- **Marketplace** — consumes bundle rev-share for fee accrual.

### 3.6 Interactions & Sequences

Per-flow sequences are specified in the corresponding slice documents; the load-bearing ones:

#### Author → validate → publish → CatalogVersion

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-seq-author-publish`

Draft authoring → fail-closed validation pipeline → approval (two-person rule for material
changes) → `PlanPublished` (pending version ref) → registry batches → `CatalogVersionPublished`
→ read-model warm to SLO (or `PlanPublishDegraded`); `pricingSnapshotRef` pins the committed
version ([`design/01-foundation.md`](./design/01-foundation.md)).

#### Consumer read-model resolution

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-seq-readmodel-resolution`

A consumer pins one committed `CatalogVersion` and resolves the complete frozen read model
via `pricingSnapshotRef` — no draft read, no default substitution, monotonic per version
([`design/01-foundation.md`](./design/01-foundation.md)).

#### Grandfathering cutover

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-seq-grandfathering-cutover`

One atomic approval unit shortens the current `all_subscriptions` window `effectiveTo` to the
cutover and schedules (a) an immutable `existing_grandfathered` copy — a **new `cohort`
generation**; prior generations stay untouched and concurrently live — and (b) the successor —
so no coverage gap opens and each grandfathered price stays live-resolvable yet immutable
([`design/07-pricewindow-linkage.md`](./design/07-pricewindow-linkage.md); §17.5).

### 3.7 Database schemas & tables

The canonical schema — `pricing_plan`, `pricing_price` (history = superseded rows retained in-table, keyed by `supersedes_price_id`), the scope-key unique
index, the projected `pricing_read_model`, the `pricing_catalog_version_ref` (pending/committed), the event
outbox, tenant policy objects, and the append-only audit store — is owned by the Foundation
and specified normatively in [`design/01-foundation.md`](./design/01-foundation.md) §4.
Slice-specific tables (phases, add-on rules, bundles, price overlays, customer-group membership,
migration schedules) are introduced by their respective slice documents. Money columns are
stored as integer minor units at the currency's ISO 4217 scale.

### 3.8 Deployment Topology

The catalog runs as a stateless authoring/publish + read-model service over a shared
`toolkit-db` backend; background work (read-model warm re-drive, mass repricing, scheduled
migration) is coordinated as a singleton via the coordination lease library. The read path is
served from the projected read model for the p95 < 100ms target and fails closed (never stale)
on read-model outage. Deployment specifics are platform-standard for a BSS gear.

## 4. Additional context

- **Design decisions register** — the 2026-07-09 review wave's decision register is [`DECISIONS.md`](./DECISIONS.md): **all 39 decisions + 14 ratifications closed 2026-07-10** and propagated into the slice docs + PRD (incl. ADR-0002 `cohort` axis, the window-machinery consolidation, and the closed-top ban); reopening an item = flip its status there and record why.
- **Telemetry** — publish throughput, validation-catch rate, publish→read-model propagation lag, degraded-publish count, and approval outcomes are surfaced per the governance/observability slice ([`design/05-governance.md`](./design/05-governance.md)).
- **Provisional NFRs (Design-lock blockers)** — read-model availability / DR RPO-RTO, mass-repricing worst-case throughput, plan/tier size caps, and idempotency-key TTL are working assumptions in [`PRD.md`](./PRD.md) §14 and MUST be ratified before Design lock; no bare placeholders ship.
- **Conformance fixtures before code** — the jointly-owned golden fixtures (tier-boundary, package, per_unit, proration, reserved, supersession-continuity) MUST be stood up and version-controlled **before** implementation; publish of any `modelKind` lacking a joint fixture is blocked ([`PRD.md`](./PRD.md) §13, AC #60/#61).
- **Open cross-team items** ([`PRD.md`](./PRD.md) §15) that materially shape the design: the `CatalogVersion` increment-trigger taxonomy + max batching-delay SLO value (Registry), the minimum Billing/ERP descriptor field set, the upstream SKU retirement/unpublish joint contract, and the cross-boundary (currency/region/frequency) cancel+new sign-off.
- **Deferred to Future scope** — typed credit/discount (negative-amount) rows, `currencyFallbackPolicy` (FX fallback), `includedAllowance` **extensions** (per-seat scaling; level-meter allowance — the core `includedAllowance {quantity, rolloverPolicy}` is **in launch since D-45, 2026-07-16**, publish-compiled to $0-band / D-43 grant), `aggregationFunction = last | unique` (the `{sum, peak, time_weighted}` set is **in launch since D-44, 2026-07-16** — level-based billing), two-dimensional (seats × usage) single-line pricing, structural freemium flag, per-row `refundable`/`creditPolicy`, self-service term/auto-renew metadata, per-group different-tier structures, plan-level minimum fee / cap per period (the rating-side `PeriodFloorCapObligation` boundary is reserved — deferred part is the catalog authoring field), and committed-usage / drawdown flags on plan (Contracts + Tariffs, Cross-PRD). The consolidated registry is [`PRD.md`](./PRD.md) §17.8.

## 5. Traceability

- **PRD**: [`PRD.md`](./PRD.md)
- **ADRs**: [`ADR/`](./ADR/) — `cpt-cf-bss-pricing-adr-canonical-scope-key`, `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis`, `cpt-cf-bss-pricing-adr-pricewindow-consolidation` (further ADRs planned as dependent slices land: snapshot/versioning strategy, customer-group ownership, `CatalogVersion` increment/batching, `brand`-as-`PriceOverlay`)
- **Design set**: [`design/`](./design/) — Foundation + per-capability slice designs; the phased map and dependency order are in §1.3 and [`design/README.md`](./design/README.md).
