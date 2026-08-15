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

Non-functional requirements are specified in [`PRD.md`](./PRD.md) §7. All load-bearing
targets are **ratified as of 2026-07-28** ([`PRD.md`](./PRD.md) §14/§15), with one deliberate
exception — the audit retention-maximum vs minimum question stays open with Legal; per-row
statuses below.

| NFR theme | Allocated to | Design Response | Status |
|-----------|--------------|-----------------|--------|
| Publish → read-model propagation (p95 ≤ 5s) | Foundation publish engine + event fan-out | Batched `CatalogVersion` commit + retry-to-SLO warm or `PlanPublishDegraded`; pin never lags newest completed by > 5s | Committed target; batching-delay SLO ratified (D-47: p95 ≤ 60s, max 5 min) ([`PRD.md`](./PRD.md) §15) |
| Read / preview latency (p95 < 100ms per tenant partition) | Read-model projection store | Single indexed read of the projected, version-pinned read model; no evaluation on the read path | Committed target |
| Read-model availability / DR RPO/RTO | Read-model store + deployment topology | Fail-closed on read-model outage (never stale); 99.9% per tenant partition, RPO 5m / RTO 30m | Committed — ratified 2026-07-28 |
| Determinism / reproducibility | Foundation snapshot + immutability | Append-only published rows, monotonic version, complete frozen `pricingSnapshotRef` | Committed |
| Audit retention (≥ 7 years, jurisdiction-configurable) | Governance/audit slice + Foundation audit store | Append-only, tamper-evident audit of every mutation + approval trail | Committed; retention-maximum vs minimum **open with Legal** ([`PRD.md`](./PRD.md) §15) |
| Mass-repricing worst-case throughput; plan/tier size caps; idempotency-key TTL | Foundation + operator-efficiency slice | Idempotent, deduplicated bulk events; per-row commit; >= 50 rows/s, caps 100/500 + 366d/24m, TTL 24h | Committed — ratified 2026-07-28 (repricing figure perf-test-verified vs worst case) |
| Data residency (`nfr-data-residency` — zero rows replicated outside a residency jurisdiction) | Deployment topology (§3.8) + Foundation storage | Residency-bound tenants are pinned to an **in-jurisdiction deployment cell**: all gear-owned stores (`pricing_*` tables incl. audit, the read-model projection, outbox, backups and DR replicas) live on that cell's `toolkit-db`; the gear performs no cross-cell replication of its own, so the boundary is enforced by cell pinning + platform storage-class configuration, verified at deployment (a residency-bound tenant on a non-compliant cell fails config validation, fail-closed). DR for residency-bound tenants uses in-jurisdiction replicas only (AC #103 RPO/RTO met within the boundary) | Committed rule; cell/storage-class inventory is platform-owned |

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
| [`design/09-price-overlays.md`](./design/09-price-overlays.md) | 6.6 | 3 | `PriceOverlay` as an **adjustment-line container** (D-42: per-`(planId?, targetSku?)` lines, most-specific wins; per-currency amounts D-08; magnitudes range-bounded D-67) + `customerGroup` segment pricing (BSS-owned taxonomy, effective-dated audited membership). Every overlay mutation is **always material** (D-50) and its own publish unit (D-06). |
| [`design/10-advanced-primitives.md`](./design/10-advanced-primitives.md) | 6.10 | 3 | Reserved capacity (p1; `capacity`-only on level rows — D-53), prepaid credit grant (p2; grants are a table — D-52), publish-compiled `includedAllowance` (p1 — D-45: $0 band + marker / carry grant; forbidden on `volume`, kind-rewriting on untiered rows — D-59), derived (composite) meter formula-as-data (p2), `discountRef` hook, typed `minQtyThreshold`, trailing-tier qualification (D-40; the locked rate is a **Rating-owned per-period pin**, never in `pricingSnapshotRef`, and publish is fixture-gated — D-60). |
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
stored as integer minor units at the currency's ISO 4217 scale. One store sits deliberately
**outside** this set: governed backdated reference rows live in `pricing_historical_price`
([`design/05-governance.md`](./design/05-governance.md), D-76) — never window-linked, never
projected, never sellable, read only by snapshot synthesis — so every invariant stated about
published `pricing_price` rows holds without an exception class.

### 3.8 Deployment Topology

The catalog runs as a stateless authoring/publish + read-model service over a shared
`toolkit-db` backend; background work (read-model warm re-drive, mass repricing, scheduled
migration) is coordinated as a singleton via the coordination lease library. The read path is
served from the projected read model for the p95 < 100ms target and fails closed (never stale)
on read-model outage. Deployment specifics are platform-standard for a BSS gear, with one
gear-stated constraint: for residency-bound tenants every gear-owned store (tables, read
model, outbox, audit, backups, DR replicas) is pinned to an in-jurisdiction deployment cell —
zero cross-boundary replication (`nfr-data-residency`; NFR-allocation row above).

## 4. Additional context

- **Design decisions register** — the decision register is [`DECISIONS.md`](./DECISIONS.md), now spanning **D-01…D-178** across the review and implementation-side waves summarised below (the "five waves" this sentence used to claim stopped being true around D-79; corrected 2026-08-03, this summary being kept current per D-72): **D-01…D-39 + all 14 ratifications closed 2026-07-10** (incl. ADR-0002 `cohort` axis, the window-machinery consolidation, and the closed-top ban); **D-40…D-55** decided 2026-07-12…2026-07-28 (overlay adjustment lines D-42, level aggregation D-44, `includedAllowance` D-45, `CatalogVersion` batching D-47, the Billing v1 descriptor set D-48, overlay materiality D-50, retirement coverage D-51); **D-56** from the 2026-07-28 code-readiness review (plan draft-revision rows); **D-57…D-68** from the 2026-07-29 review; **D-69…D-78** from the 2026-07-30 propagation review (D-69…D-75 propagation repairs and the two PRD↔design contradictions; **D-76** the disjoint backdated-reference store + the two-tier snapshot-synthesis selection rule, **D-77** the level-row granularity pairing, **D-78** the overlay `cohort` eligibility filter that exempts grandfathered generations); **D-79…D-86** from the 2026-07-30 manual slice review ([`reviews/2026-07-30-slice-design-review.md`](./reviews/2026-07-30-slice-design-review.md)): **D-79** the in-flight-subscriber lane (the D-51/D-62 predicate's missing data source), **D-80** the exemption narrowing + the sellability coverage horizon, **D-81** the D-76 temporal half (open-ended reference intervals + per-trigger synthesis `t`), **D-82** the supersession unit guard, **D-83** revision-scoped plan child tables (D-56 completion), **D-84** hybrid/usage per-market completeness, **D-85** the operator-plane flag store, **D-86** read-model delta storage + retention; **D-87…D-98** from the 2026-07-31 manual slice review ([`reviews/2026-07-31-slice-design-review.md`](./reviews/2026-07-31-slice-design-review.md)): **D-87** the self-contained `migrated-origin` payload + reference-row band storage (tier-2 synthesis becomes consumable), **D-88** the supersession change unit + the changeover-instant floor (interactive + bulk), **D-89** the phase-axis extension of the D-82 unit guard, **D-90** plan-revision flip-at-commit (one current revision), **D-91** subject-typed read-model deltas (D-86 amendment — the D-06 publish units become representable), **D-92** revision discipline for bundle/overlay tables (D-83 completion), **D-93** read-time boundary classification (D-25 revision), **D-94** the sellability-gate key conjunction + plan-market exemption semantics, **D-95** required-add-on coverage per `(currency, region)`, **D-96** `invoiceGroupingKey` homed in S2, **D-97** override refs bind the scope key, **D-98** `model_kind` joins the supersession-preserved set (D-82 amendment); **D-99…D-112** from the 2026-07-31 **second** manual slice review of the day ([`reviews/2026-07-31b-slice-design-review.md`](./reviews/2026-07-31b-slice-design-review.md)) — the wave that audited the **read** side: **D-99** window mutations become publish units (the read model carries window *intervals*, so activation/expiry stay projection-free), **D-100** the cutover flips its predecessor (its successor shares that key, so the commit previously died on the scope-key partial `UNIQUE`), **D-101** version-level **pin-eligibility** (D-91 amendment — per-subject fallback had made one pin resolve two contents over time), **D-102** the `migrated-origin` read surface + the one named exception to Tariffs-composes-the-snapshot, **D-103** meter injectivity is per `(meter, dimensionKey)` line (a plan may price several meters), **D-104** bundle composition/rev-share changes are always material (the D-50 hole one slice over), **D-105** child-table keys carry their row discriminators + `pricing_bundle.plan_id`, **D-106** revision discipline reaches `pricing_plan_grant`/`pricing_composite_meter`, **D-107** overlay uniqueness/overlap checks are revision-scoped, **D-108** the contract lock is structural (price movement is Contracts-owned), **D-109** retirement is always material unconditionally, **D-110** `taxCategory` rides the price row + one display basis per market, **D-111** a bulk run's per-row commit re-validates row-local rules with the plan-level aggregate pass once per plan per run, **D-112** an `overlay_index` subject gives overlay evaluation an access path; **D-113…D-122** from the 2026-07-31 **third** manual slice review of the day ([`reviews/2026-07-31c-slice-design-review.md`](./reviews/2026-07-31c-slice-design-review.md)) — the wave that read the **consuming side** of every snapshot-frozen contract field in the sibling gears' own documents: **D-113** the plan-change `usageCounterOnPlanChange` flag gets its pricing home (Rating/Subscriptions consumed it "from the pinned snapshot" while no pricing document defined it — the D-01 class between gears; reset default, `carry` per unit-matched shared line only), **D-114** pin-eligibility is prefix-closed (D-101 amendment — a monotonic frontier, so a stuck older version's late warm cannot make one pin resolve two contents), **D-115** the materiality delta domain (band-wise iff geometry unchanged; geometry/quantity changes and no-computable-delta mutations — row contract fields, plan shape — always material), **D-116** the add-on override ref binds the key family modulo market (D-97 amendment), **D-117** orphan phase-scoped usage overrides are forbidden, **D-118** bulk import is draft-plane authoring (published-row changes are repricing runs), **D-119** one tax display basis per bundle-market (the D-110 rule across components, with the D-54-pattern reverse guard), **D-120** overlay scope values validate against declared universes (region; new partner/orgTier tenant taxonomies), **D-121** the plan-subject projection's row/window set + horizon (`H` = 2 × longest cycle sold; `expired` windows projected within it, so arrears resolve; older `t` replays from old pins), **D-122** `package_size` joins the supersession/phase-override preserved set (D-82/D-98/D-89 amendment). **D-123…D-125** from the 2026-07-31 billing-domain review ([`reviews/2026-07-31d-billing-domain-review.md`](./reviews/2026-07-31d-billing-domain-review.md) — a deliberately partial-coverage domain-lens pass): **D-123** the proration/anchor contract uniform per plan-market (it was N-valued under the phase axis while Subscriptions reads one cycle clock), **D-124** the bulk aggregate pass moved to commit entry over the plan's full post-run row set, **D-125** the gear-wide cursor-pagination contract. **D-126…D-138** from the 2026-08-01 manual slice review ([`reviews/2026-08-01-slice-design-review.md`](./reviews/2026-08-01-slice-design-review.md) — fifth sequential pass, lensed on **failure and re-entry paths**): 4 [H] — **D-126** the grandfathering *entry* hole (the cohort selector closes only from the second cutover, so a pre-first-cutover pin carrying `cohort = none` matched no generation), **D-127** the cutover successor escaping the ×24 unit guard (the guard binds the key, not the mechanism), **D-128** retirement as a consumer-visible fact with no publish unit and no projector source afterwards, **D-129** the compiled `carry` grant bound to a `price_id` its own change mechanism replaces — plus 9 [M]: **D-130** the allowance compile becomes a projection (it had destroyed its own input), **D-131** the D-79 lane returns a per-price-id presence map, **D-132** the market-uniformity rules exclude immutable grandfathered generations, **D-133** `overlay_index` sharded + horizon-bounded, **D-134** a repricing run commits per plan, **D-135** the audit hash chain segmented, **D-136** the pin-eligibility frontier materialized + published, **D-137** bulk import is never material, **D-138** `fixed` replaces the running amount. Live qualifiers: **every veto-flagged decision through D-86 was CONFIRMED per-item on 2026-07-31** (D-79/D-80 stay joint with Subscriptions — the SUB-P8 presence read is owed their side); **every veto-flagged decision through D-98 — incl. D-93/D-94/D-98 — was CONFIRMED per-item on 2026-07-31** (D-93/D-94 joint with Subscriptions — their SEAMS SUB-P1/SUB-P5 adoptions are owed); **every veto-flagged decision of the 2026-07-31b wave — D-103, D-104, D-108, D-109, D-110 — was CONFIRMED per-item on 2026-07-31** (D-110 on both halves); **the 2026-07-31c wave's flagged set — D-113 (joint with Rating + Subscriptions), D-115, D-117, D-118, D-119, D-122 — plus D-123 was CONFIRMED per-item on 2026-08-01**; **the 2026-08-01 wave's flagged set — D-126 (joint with Rating), D-132 and D-138 (joint with Rating) — was CONFIRMED per-item the same day**, each against its stated alternatives, so **nothing in pricing awaits veto**; **D-48** awaits the Billing countersign, and **D-67**'s stacked-result floor is an open product fork (§F.1). Owed cross-gear adoptions from that wave: Rating (D-126 cohort bootstrap, D-138 `fixed` semantics), Subscriptions (D-131 lane response shape). **D-139** was added 2026-08-02 from outside the review cycle — found while authoring the `reserved` joint fixture, which could not be written because the two gears carried two formulas for one charge: pricing's `capacityCharge = reservedRate × reservedQuantity` (D-53) against rating's `× coveredGranules` (T-D-25, adopted and confirmed 2026-08-01 and never propagated here). Pricing adopts T-D-25; the frozen field set is unchanged, since `coveredGranules` is Rating-computed runtime coverage. **CONFIRMED per-item 2026-08-02** against both alternatives — two factors would read a per-granule rate as a period charge (a units error, not a policy choice), and making `reservedRate` a period rate would break the level-billed product D-53 exists for. **Joint with Rating; nothing in pricing awaits veto.** **D-140** was added the same day from the implementation side — the first finding raised by scaffolding the gear crate rather than by a review pass: every documented REST path was unimplementable under the platform route convention the build enforces (a version-first prefix and three colon-suffixed custom methods, both denied by the `DE0801` lint that runs in the phase gate and in CI). Decided by the product owner the same day — the wire path is `/bss-pricing/v1/{resource}` with actions as sub-resource segments (normative in [`design/01-foundation.md`](./design/01-foundation.md) §3.3, inherited by every slice) — and applied as a mechanical, contract-neutral rewrite: no `(resource_type, action)` pair, payload, status code, error name or SLO moved. Not flagged for veto. **D-141…D-148** were added the same day from the implementation side too — the **second** such wave, and the first raised not by a lint refusing to build the documents but by writing Group **G3**, the gear's draft-authoring plane, *against* them: fifteen places where the code needed a rule this set does not state, of which **twelve** became eight decisions (three collapses removing four would-be entries) and three were applied as mechanical fixes carrying no id; a sixteenth item was withdrawn on inspection. **D-141** the price row gets an ETag/row-version column of its own and every draft mutation presents it, `DELETE` included; **D-142** the dedup row has two states (`claimed`/`answered`, the second column pair set together), with expiry evaluated at claim time and **before** the payload digest, a compare-and-swap takeover, and write-once answers whose second write is the replay path; **D-143** `IDEMPOTENCY_KEY_IN_FLIGHT` (409) for the duplicate that arrives while the original is still running — reachable with no contract violation by anyone; **D-144** every authored, published or compared instant is UTC at **millisecond** resolution and finer precision is refused (`TIMESTAMP_PRECISION_EXCEEDED`), the `cohort` axis included; **D-145** `revision` is an identity, never re-minted — a discarded draft revision flips to a terminal `abandoned` state instead of being deleted, so `(plan_id, revision)` stays a durable name and revision numbers may show gaps; **D-146** `PLAN_RETIRED_NO_SUCCESSOR` (422) and `OPEN_DRAFT_REVISION_EXISTS` (409) narrowed **out of** `LIFECYCLE_FORBIDDEN`, which retains only the refusals with no alternative action to describe; **D-147** `grandfatherUntil` is a grandfathered-row field, its violation failing publish (`GRANDFATHER_UNTIL_FORBIDDEN`, 422) instead of surfacing as an internal fault; **D-148** a second partial `UNIQUE` on the canonical scope key over the **draft** plane, rendering as the existing `DUPLICATE_SCOPE_KEY`, so D-21's save-time check becomes the explanatory path and the index the guarantee. The three mechanical fixes carry no id by design — the cross-reference between the two slices declaring columns on `pricing_price`, the nullable-tolerant package checks, and `max_hold_granules` widened to `bigint`. **D-143 was the wave's only veto-flagged item** (it narrows `fr-mutation-idempotency`'s **MUST** with "once the original has completed") and was **CONFIRMED per-item 2026-08-02** against block-and-replay — a client can act on a discriminated, retriable code and can do nothing with a held connection, which would pin one open for the length of the guarded mutation and needs a timeout policy this set has nowhere to put; the `abandon` endpoint D-145 implies was put to the owner in the same round and **kept**, against narrowing D-145 to the retirement case, which would leave the authoring path with neither a deletion nor a state. **D-145 was amended the same day**, by an independent review of what G3 built rather than by the wave that wrote it: its "a new draft opens immediately" is true of every plan except one that has **never published**, whose only revision is `abandoned` — no current revision to open a successor from, revision `0` already consumed, so the plan id is spent and the two authoring arms answer 500 and 404. The owner kept the state and made the refusal honest — a new Foundation-owned `PLAN_ABANDONED_NO_SUCCESSOR` (422), owed by the authoring REST surface that does not exist yet — against minting `max(revision) + 1` in the create path too (a retried create with the same id would silently open a second revision of an existing plan, where creation idempotency requires a refusal) and against re-minting revision `0` alone (the unstable name D-145 exists to remove, on the one number every plan starts at). **Nothing in pricing awaits veto.** **D-144 is joint with Rating**, its owed adoption being one clause on the already-owed D-126 cohort bootstrap rather than a new adoption. The dedup store's **retention** is deliberately left open (§F.1), and two decisions are owed back to the implementation branch as follow-through (D-144's refusal where the code truncates, D-145's tombstone where the code deletes). **D-149…D-154** were added 2026-08-03 from the implementation side as well — the **third** such wave, raised by building Group **G4**, the *shape of a plan* (Slice 2's four validator sets plus the three revision-scoped child tables), against this set. Where G3 found rules the documents do not state, G4 found rules they **state and cannot enforce**: fourteen findings, **six** decisions after three collapses, **four** mechanical fixes carrying no id, and one obligation recorded rather than decided. **D-149** — §5 owes a code to every rule §3 states, and four cycle-shape requirements had none: `BASE_MARKET_INCOMPLETE` (422) for a sold market with no base row of the `chargeKind` its cycle mandates (one code over the one-time and recurring cardinality rules, the base-side sibling of `USAGE_MARKET_INCOMPLETE`), `CYCLE_METADATA_MISSING` (422) for an absent `frequency` **and** for a plan that declared no `billing_cycle` at all — the fifth finding, and the reason the others mattered, since every rule of the step is conditioned on the cycle and an undeclared one passed the whole step vacuously — and the recurring-only add-on moved to `inst-cmp-addons` under the existing `ADDON_INCOMPATIBLE`, with the term it never had, because deciding it needs the add-on SKU's own plans; `inst-cs-recurring`, which had registered no rule whatsoever, registers two. **D-150** — the add-on rule's quantity bounds become a rule with a code, `ADDON_QTY_RANGE_INVALID` (422): `maxQty ≥ 1 WHERE required` was a §6 `CHECK` with no code, so its violation reached an author as a driver error in a slice that promises an enumerated report, and the two bounds the implementation had to add itself (`minQty ≤ maxQty`, `stepQty > 0`) are named in §6 beside it. **D-151** — `displayTrialDays` is a `trial`-phase field (`DISPLAY_TRIAL_DAYS_INVALID`, 422), D-147's treatment one slice over: the §6 drift `CHECK` is silent on the phase `kind` **and** NULL-satisfied whenever the duration is absent, so the `evergreen` terminal phase — where the duration rule and the terminal-kind rule are each correctly silent — could publish a trial length for a plan with no trial phase; the `CHECK` is deliberately kept as written, since tightening it makes a half-authored phase graph unsavable. **D-152** — the descriptor required-set's config extension and the four ratified "tenant-configurable" caps get their carrier, `pricing_policy_object`, the store this gear already uses for its per-tenant policies; **flagged for veto**, since it fixes tenant scope for a required-set inside Billing's pinned contract — **CONFIRMED per-item 2026-08-03**, against a per-deployment section (one deployment's tenants would share a Billing contract they do not share) and against pinning the required-set at v1 behind a schema change, and **with one qualification the owner added: the carrier is provisional**. It holds those two additions *for now*, against a **settings gear** they are expected to move to later — a gear that does not exist in this repository yet (`gears/simple-user-settings` is a per-**user** `theme`/`language` store with validation schemas out of scope, not a per-tenant policy service), so the note is a placeholder for one that has to be built. It is recorded in the register entry and in [`design/01-foundation.md`](./design/01-foundation.md) §3.7, which is what an implementer reads when they wonder why a per-tenant cap lives in a pricing table. **D-153** — the price row's **draft** plane is transition-guarded too: a column whitelist cannot constrain a draft row, and `draft → superseded` took a row outside *both* scope-key partial `UNIQUE` predicates, undoing D-148's guarantee with one UPDATE (no new code — no endpoint offers the transition). **D-154** — the row-borne `taxCategory` is the **resolved effective** category, frozen with the row as `rounding_policy_ref`'s resolved id already is, and a row with no effective category fails publish outside the tenant tax-display policy, because D-48 v1 pins the element; **flagged for veto** — **CONFIRMED per-item 2026-08-03 with no qualification**, on both halves, against an unconditional per-row `tax_category_ref` (which deletes the coalesce rule S4 has confirmed twice) and against narrowing D-48 so the element is not publish-blocking (which drops a pinned element from a contract Billing has not countersigned): the policy keeps its `ratePresent=false` arm, where the missing fact is a rate no one in this gear owns, and loses the category arm, because a pinned contract element is not a display preference. The four mechanical fixes carry no id by design — `tenant_id` stated in the three Slice-2 child tables' column lists, the semantically inert `AND meter IS NOT NULL` on the injectivity index, the add-on edge columns glossed as JSON arrays on both backends, and the `custom_every_n` token named as what the `frequency` column holds. The descriptor rule's coverage of **three of D-48 v1's five** elements is recorded as **total rather than deferred** (neither Slice 4's nor Slice 6's storage exists) inside D-154. **The wave's flagged pair — D-152 and D-154 — was CONFIRMED per-item by the product owner 2026-08-03**, D-154 outright and D-152 with the provisional-carrier qualification above, so **nothing in pricing awaits veto.** **D-155…D-161** were added 2026-08-03 from the implementation side as well — the **fourth** such wave, raised by building Group **G5**, the **publish commit**: the pipeline re-run inside the commit transaction, the lifecycle flips, the transactional outbox, the fail-closed `CatalogVersion` request and the segmented audit chain. Where G3 found rules this set does not state and G4 found rules it states and cannot enforce, G5 found the set **contradicting itself** and **promising values with no producer** — two of each, accounting for four of the seven entries. Ten findings became seven decisions, two mechanical fixes carrying no id, one open fork, and three findings recorded as needing no change. **D-155** [H] — nothing said *which* row set the commit flips, so re-deriving the draft set at flip time was a conforming reading of §4.2 and published rows the rule set never judged, through a window containing the registry's network round-trip; the flip is now pinned to the `(price_id, row_version)` pairs the re-run judged (D-141's token spent on what it was minted for), the rule set's other inputs are enumerated with the mechanism holding each, and **one of them is stated as a premise**: the *membership* of the published row set is held only by this gear having no producer for `published → superseded`, so whoever builds D-88's supersession unit or D-100's cutover deletes that premise and owes the published half a guard of its own. **D-156** — §3.6 enqueued `PlanPublished` before requesting the version ref while §4.2 requires the event to *carry* it; §4.2 wins, the request sits inside the transaction after re-validation and before the writes, the `request_id` is derived from `(tenant, plan, revision)` so a retried commit re-requests the same handle, and the cost (a network round-trip inside an open transaction) is named rather than hidden. **D-157** — the pending-ref row carries `(subject_kind, subject_ref)` from `pricing_read_model`'s own universe, without which no path led from a committed handle back to the subjects D-86/D-91 require the projection to write; an overlay unit's two subjects owe a widening. **D-158** — the audit log's `action` and `subject_kind` get a declared vocabulary, `subject_kind` being `pricing_approval`'s enumeration **verbatim** rather than a second spelling of the same aggregates, and `action` an additive `snake_case` verb set seeded from the records this set already requires (never a frozen event name, never a token with no writer). **D-159** — `CONCURRENT_MUTATION` (409) for the loser of a same-aggregate race, which had been arriving as an internal fault indistinguishable from a dead connection; one code over the gear's three per-aggregate serialization points, and distinct from both `STALE_VERSION` and `IDEMPOTENCY_KEY_IN_FLIGHT` by D-146's line. **D-160** — the advisory `warnings[]` channel is code-carrying like the blocking one, and its two stated advisories get their codes: `TIER_BAND_PRICE_INCREASE` (which the crate had been raising under a token no document declared) and `PLAN_SIZE_SOFT_CAP_EXCEEDED`, without which §14's ratified soft caps had nothing to be reported through and were built as nothing. **D-161** [H] — `pricingSnapshotRef`'s catalog-side stamp has three parts and the commit can produce two; **no document names a producer or a format for the evaluation-policy segment**, in either gear. The decidable half is decided — publish stamps **no placeholder**, since Rating's `SnapshotComposer` fail-closes on a missing pre-stamp (correctly) and would be *satisfied* by a fabricated one (fatally, on posted money, years later) — and the segment's content is an **open fork for the product owner jointly with Rating** (§F.1: a vocabulary generation, a content digest, or no segment at all), which is **launch-blocking**, because while pricing stamps nothing that composer rates no charge. The two mechanical fixes carry no id by design — the outbox ordering named as the unique sequence that enforces it, and §1.2's NFR row separating the two *advisory* soft caps from the two *blocking* interval caps. Three findings were checked and need nothing: `ROUNDING_POLICY_UNRESOLVED` has two renderings in the code and one in the documents (§3.3 already homes it in the pipeline); `inst-tp-distinct` is a step of the **approval-decision** algorithm, so the publish commit consuming an approval rather than re-deciding it is the single-owner arrangement already specified; and D-153 is closed in code on both backends. **Nothing in this wave is flagged for veto** — every entry is a technical call and the one item carrying commercial weight is forked rather than decided. **D-162** closed that fork the same day, the product owner taking §F.1's recommended option **(a)**: the evaluation-policy segment is a **vocabulary generation**, `ep-<n>`, a declared constant of the gear naming which evaluation-policy field set a snapshot's frozen row content is read under, opening at `ep-1`. Declaring it meant declaring the field set, which no document had — the phrase runs through the set sixty-odd times unexpanded and is expanded once, in the PRD glossary, as three fields D-40, D-44 and D-45 have each added to since — so the entry writes down a **nine-field roster** (`model_kind`, `package_size`, `billing_granularity`, `tier_aggregation_window`, `tier_qualification_window`, `aggregation_function`, `aggregation_granularity`, `max_hold_granules`, `included_allowance`) and, beside it, what is deliberately outside and why: the row's identity, its money, and the fields saying where a quantity *comes from* rather than how it is *derived*. The decision's weight sits in the guard rather than the constant — a generation nobody remembers to bump asserts a stability that is not there, on posted money, where the whole point of the segment is that a period rated under one evaluation semantics is tellable from one rated under another. So the bump is a **mechanism**: [`design/01-foundation.md`](./design/01-foundation.md) §4.4 carries a normative append-only log of `ep-<n>  <decision>  ± <field>` lines, the roster is what replaying that log produces, the last generation is the declared one, and the gear's build reads that block — a field cannot join the roster without a log line, a log line is a generation, and the last generation is the constant. What the generation does **not** claim is recorded with it (it tracks the field *set*, not the meaning of a field — the coverage a content digest would have bought), and `ep-1` claims no retroactive history, since generations minted for the four decisions that moved the set before it was written down would be versions no snapshot ever carried. D-161's ban on a placeholder is unchanged: publish stamps a real generation or none. **Rating's adoption is owed** (equality comparison, fail-closed on an unrecognised generation, and the "two segments" count corrected in its `design/01` §4.3 and `design/11` §4), and one hole is recorded rather than rounded off — D-113's plan-scoped `usageCounterOnPlanChange` is an evaluation input outside the roster only because the plan-content type it would be classified against does not exist in the crate yet, and the slice landing it joins the roster under a bump. **D-163…D-168** were added 2026-08-03 from the implementation side as well — the **fifth** such wave, raised by building Group **G6**, the **read side**: turning a pending `CatalogVersion` handle into a committed number, projecting the frozen per-subject read model, advancing the pin frontier, and running the degraded path. Where G3 found rules this set does not state, G4 found rules it states and cannot enforce, and G5 found it contradicting itself and promising values with no producer, **G6 found rules decidable only under premises nobody had written down, and clauses that cannot be satisfied with what exists.** Fourteen findings across three review rounds became six decisions, three mechanical fixes carrying no id, and one open fork. **D-163** [H] — §4.4's pin-eligibility counts "every subject row that version projects", which is decidable only if a version's subject set is **closed** once any of its refs commits, i.e. only if a batch commits atomically into one version; §3.6 says the registry "batches approved publishes" and never says that. The property is now **normative and the registry's** (vendored per D-47, so there is a document to write it in and a team to owe it), the projector may decide a version complete only from a pass that could have seen the whole subject set (a saturated pending scan and a per-ref registry error both deny it that, and the scan is bounded **per tenant** so the deferral is a tenant's own), and a ref resolving into a version at or below the frontier is refused — a detection rather than a prediction, whose **price is stated with the guarantee**: that publish becomes unresolvable at any version and the remedy is out of band. **D-164** [H] — pin-eligibility is **this tenant's** (the `CatalogVersion` sequence is cross-tenant, so read globally no frontier could ever advance past a version another tenant's publish consumed) and the frontier **walks forward** through every already-complete following version, without which §4.4's own D-114 worked example strands a complete `V6` behind `V5` permanently. **D-165** [H] — the projector sourced the plan's **current** revision, so a second publish inside D-47's five-minute batching maximum froze its content into the earlier version, permanently, in an INSERT-only store whose contract is that a completed version never changes; the ref row now pins `subject_revision` **and** `subject_lifecycle_state` — the revision and the state the publish's own judgement produced — with only the two tokens D-128 sanctions storable. **D-166** — the degraded mark had no column in §3.7, no start instant and a threshold that would fire on every healthy publish: `commit_observed_at` is recorded on the ref row (the 5s post-commit clock starts at `CatalogVersionPublished` and `committed_at` is stamped by a finalize that never runs on the failing path), the degraded state stays **derived** with no column, `commit_overdue` narrows to "the registry has not answered", and `pin_eligibility_overdue` fires on the frontier's `advanced_at` only in conjunction — a stale frontier alone is a tenant that has not published. **D-167** — the delta's field list is declared nowhere (the D-158 shape one store over), D-121's horizon filters a window set Slice 7 has not built, so the projection proceeds under a stated premise (no producer for `published → superseded` ⇒ no accumulated history to exclude) that the **D-88**/**D-100** group deletes and then owes the horizon and `H`'s own input; and `inst-sg-pinned`'s six sellability predicates are a claim about the **finished** gear, a version produced today answering three. **D-168** — Slice 6 requires a **pair** of read-model fields on every plan subject row and names the value of one, so the pair is stamped as a pair or not at all (D-161 clause (1) one artifact over, and worse: a frozen version can never be corrected), and where the warning text comes from is an **open fork for the owner** (§F.1) with a recommendation. The three mechanical fixes carry no id by design — the ref table's instants named in its §3.7 bullet, §4.4's "consumers observe completion via the marker" repointed at the pin frontier the marker advances, and the statement that a projection writes no audit row. `pricing.catalogversion.commit_overdue` was checked and **needs nothing**: §3.6 states its predicate precisely, on the ref's age and nothing else. **Nothing in this wave is flagged for veto** — every entry is a technical call and the item carrying commercial weight is forked rather than decided — and one owed cross-gear adoption is new: the registry gear owes batch atomicity as a property of `CatalogVersionPublished` on its own side, and the commit instant carried with the committed version. **D-169** closed that wave's fork the same day, the product owner taking option **(c)**: the catalog publishes the K3 marker (`crossBoundaryChangePolicy = cancel_plus_new`) and **no copy** — `crossBoundaryWarningText` leaves the consumer contract, and the surface that renders the warning owns its wording. The answer turned on a fact the fork statement did not have: [`PRD.md`](./PRD.md) **AC #66** already required the preview/migration UI to warn that in-place credit is forfeited and to take an explicit confirmation, so the field was a *second* home for a sentence this set had already placed on the surface that shows it — and the second home is the one with an INSERT-only ≥ 7-year store behind it, where a customer-visible string is frozen in one language for every version already stamped and this set has no localization story anywhere. A per-tenant configurable (option (b)) makes the copy re-authorable **going forward only**, leaving every stamped version exactly as frozen, on the carrier D-152's confirmation marks provisional. D-168's both-or-neither is **discharged rather than overridden** — with a single field there is no half to stamp, so the Slice-6 AC it recorded as unsatisfiable is satisfiable, and the projector (which stamps neither half today) owes the marker. **Subscriptions' countersign is owed**, the wording being theirs: removing a field from a published consumer contract is a counterparty transaction, and their SUB-P1 seam already carries the plan-change classification D-93 moved to their side. **D-170…D-174** were added 2026-08-03 from the implementation side as well — the **sixth** such wave, and the one that closes Phase 2 — raised by building Group **G7**, the gear's **REST surface**: the nine authoring routes, their preconditions, their authz gate and the `OpenAPI` registration a client is generated from. Where G3 found rules this set does not state, G4 rules it states and cannot enforce, G5 the set contradicting itself and promising values with no producer, and G6 rules decidable only under premises nobody had written down, **G7 found the set stating a contract and never stating its transport** — four of its five entries are that. Roughly twenty-three divergences across two independent review rounds became five decisions, three mechanical fixes carrying no id, no new fork, and one correction to the register's standing list. **D-170** — a plan route resolves to one of **two** revisions (the open draft when there is one, the current revision otherwise) and no document gave a plan tag a shape at all, so a token carrying only a row version could not say which revision it came from; the counters being unrelated, a tag read from revision *N* satisfied the compare-and-swap on *N+1* with **no race window at all**, which is D-145's lost update reopened in transport. The tag now names both, a mismatch in either component is `STALE_VERSION` (nothing minted), the price plane's bare token is deliberately untouched (a price route addresses one row by id, so there is nothing to disambiguate), and the tag is declared **opaque** — the rendering is stated so the set says what a client receives, not so a client may parse it. **D-171** — `If-Match` and `Idempotency-Key` appear nowhere in the PRD, in this document, in the register or in any of the twelve slices, while nine slice §5 tables carry an Idempotency column naming the concepts; both headers are **required**, the mapping is stated once in [`design/01-foundation.md`](./design/01-foundation.md) §3.3 beside the D-125 pagination contract and the D-140 route shape, and cells naming a natural idempotency (`per revision`, `per decision`) require no header at all. **D-172** — D-145's spent-plan paragraph opened "every surface above names a `planId`" and then enumerated one that does not, `POST /bss-pricing/v1/plans`, whose collision mechanism presupposed a caller-supplied id that §4.3's own "a plan id is minted server-side" rules out; three arms owe the refusal, `GET` answers **404**, and D-145's substance is untouched. **D-173** — one verb named four facets of a plan's shape while the storage versions all four against the revision's single row version and each advances it, so a two-facet request matches the caller's tag on the first and never on the second: one facet per call, with the coherent multi-facet operation **named as a capability nobody has designed**. **D-174** — the digest is over the request as this gear models it rather than the bytes it arrived in, the byte reading refusing a **correct** retry with a 409 the caller cannot fix whenever anything in the path re-serializes. The three mechanical fixes carry no id: the undecodable cursor named as a malformed request, the statement that a cursor walk is not a snapshot, and the discharge of the "this gear has no authoring REST surface yet" sentences that G7's landing made false. **Nothing in this wave is flagged for veto** and **no cross-gear adoption is new**. The standing list needed **one correction**, made at the phase boundary: §F.1's **`fixed` arithmetic** fork was answered by D-138 on 2026-08-01 and stood open in the table for five waves regardless; it is struck, and the rider it carried — the **stack sort direction**, the total order `precedence → class order → overlay id` stated without ascending or descending, which D-138 makes strictly consequential since a `fixed` line discards every layer beneath it — is given its own row as a joint contract with Tariffs. Open forks (§F.1) and carried-forward findings (§F.2) are listed there and are otherwise **not** closed. Reopening an item = flip its status there and record why. **D-175…D-178** were added 2026-08-03 from the implementation side as well — the **seventh** such wave, and the first raised by a group whose purpose was **closing** the register's owed-back clauses rather than building a new plane: Group **G8** built six waves' worth of follow-through (761 → 847 tests; five audit writers where there had been one; the four plan-shape rules, the soft-cap advisory, the contention refusal, the degraded instant, the completeness bound and the cross-boundary marker), and closing them is what surfaced what the design set owed back. Where G3 found rules this set does not state, G4 rules it states and cannot enforce, G5 the set contradicting itself, G6 rules decidable only under unwritten premises and G7 a contract with no stated transport, **G8 found the set's own accounts of itself untrue.** **D-177** ([H], the wave's only one) — `includedAllowance` and `tierQualificationWindow` were storable, echoed back, projected into the read-model delta and rostered in `ep-1` while **none** of their ten publish refusals existed and neither the allowance compile nor the rate lock did, so the single reader of either field was a guard asserting it must not *change*; both are now refused at authoring on **every** path (Slice 12's bulk import included), the two members **stay modelled** because D-174 clause (1) would otherwise turn an authored allowance into a silently ignored one, the two fields stay in the `ep-1` roster because removing one bumps a generation for a field nobody can author, and the refusal is declared **load-bearing** — removable only in the change that lands the rules, since the day `POST …/plans/{planId}/publish` is mounted it is the one thing between an unjudged allowance and an immutable ≥ 7-year version. **D-175** declares the three draft-authoring audit verbs `create` / `update` / `delete`, which five of the six mutating authoring surfaces were normatively obliged to write records under and had no token for, and states the vocabulary's **closure rule** — D-158's opening list gave the set's *provenance* and was read as its *closure*, so "no token without a writer" gains its companion "**no writer without a token**"; the product owner ratified the implementation's minting of the three verbs on the condition that the debt land in [`design/05-governance.md`](./design/05-governance.md) §6 in this wave. **D-176** states where a precondition is evaluated — **inside** the transaction that writes the mutation it guards — after the plan `PATCH` arm whose write is an *insert* was found comparing the caller's tag outside it, so a concurrent publish could leave the successor revision copied from a revision the caller never read while the call answered as though its precondition held: D-170's defect on D-170's own arm. **D-178** gives the **correlation id** a producer — the request-scoped value the HTTP edge establishes, minted there when nothing is propagated inbound — the field having been required by `inst-au-complete` and by the `pricing_outbox` bullet while occurring twice in the whole design set, both times as a field in a list of fields. Five mechanical corrections carry no id, four of them to the register itself: D-172's owed-back clause claimed a discharge on an arm that is **not a mounted route** (the second register entry in this program to overstate its own discharge); an unsourced "twenty-five codeless gaps" count is struck; §F.2's projector-fairness item is narrowed to what is actually open (the value, and an ascending **non-rotating** tenant order whose tail is not swept at all); D-164's filed contradiction gains its owner; and `OPEN_DRAFT_REVISION_EXISTS`'s caller is named as the loser of the race for the plan's editable slot. **Nothing in this wave is flagged for veto** — D-175 was ratified before it, and D-177 weakens no `MUST`. The standing list needed **two corrections**, both omissions: the **`trailing_tier` joint fixture** owed with Rating since D-60 — `inst-tt-fixture` names a `FixtureGate` variant that exists in this design set and in no conformance registry, so the block cannot fire, while rating SEAMS M12 carries it as open *and* asserts pricing carries the variant — and Slice 12's arm of D-177. Everything else was re-checked against the sibling gears' documents and is accurate: all six cross-gear adoptions are genuinely uncited on their own side, D-48's Billing countersign is still pending, and §F.1's five open rows are all still open.
- **Telemetry** — publish throughput, validation-catch rate, publish→read-model propagation lag, degraded-publish count, and approval outcomes are surfaced per the governance/observability slice ([`design/05-governance.md`](./design/05-governance.md)).
- **NFR values ratified (2026-07-28)** — read-model availability / DR RPO-RTO (99.9% / 5m / 30m), mass-repricing throughput (>= 50 rows/s, perf-test-verified vs tenant worst case), plan/tier size caps (100/500 soft + 366d/24m interval caps), and idempotency-key TTL (24h) are committed launch defaults ([`PRD.md`](./PRD.md) §14/§15); the former Design-lock blocker is closed.
- **Conformance fixtures before code** — the jointly-owned golden fixtures (tier-boundary, package, per_unit, **flat**, proration, reserved, supersession-continuity, **level-aggregation** — granule fold, late-sample re-fold, `maxHold` gap; D-44) MUST be stood up and version-controlled **before** implementation; publish of any `modelKind` — or non-`sum` `aggregationFunction` row — lacking a joint fixture is blocked ([`PRD.md`](./PRD.md) §13, AC #60/#61). *`flat` added to the enumeration 2026-08-01 (cleanup, no D-number): the normative rule already read "any `modelKind`", but the family list omitted `flat` — the one catalog kind no family gated — so a literal reading of `inst-fx-gate` made every `flat` row unpublishable. The corpus now gates all five kinds and asserts that completeness structurally ([`gears/bss/fixtures`](../../../fixtures/README.md)), so the next kind added cannot reopen the hole.*
- **Cross-team items closed 2026-07-28** ([`PRD.md`](./PRD.md) §15): the `CatalogVersion` increment-trigger taxonomy + batching SLO and the SKU retirement/unpublish joint contract (D-47, registry vendored); the Billing/ERP descriptor v1 field set and the drawdown placement (D-48 — Billing countersigns at its gear PRD); the cross-boundary cancel+new sign-off (D-49 — GTM constraint entry owed). Remaining deliberate opens: Legal retention max-vs-min, F-34 GA gate, F-36/F-37 Future.
- **Deferred to Future scope** — typed credit/discount (negative-amount) rows, `currencyFallbackPolicy` (FX fallback), `includedAllowance` **extensions** (per-seat scaling; level-meter allowance — the core `includedAllowance {quantity, rolloverPolicy}` is **in launch since D-45, 2026-07-16**, publish-compiled to $0-band / D-43 grant), `aggregationFunction = last | unique` (the `{sum, peak, time_weighted}` set is **in launch since D-44, 2026-07-16** — level-based billing), two-dimensional (seats × usage) single-line pricing, structural freemium flag, per-row `refundable`/`creditPolicy`, self-service term/auto-renew metadata, per-group different-tier structures, and committed-usage / drawdown flags on plan (Contracts + Tariffs, Cross-PRD). **Plan-level minimum fee / cap per period left this list on 2026-08-15 (D-319)**: the deferred part was only ever the catalog authoring field, and it is now `pricing_plan_period_floor_cap` — a revision-scoped bound per sold `(currency, region)`, frozen in the snapshot and published on the read model, with the reserved rating-side `PeriodFloorCapObligation` boundary unchanged. What stays deferred there is an authored *comparison basis*, which rating §15 has not resolved. The consolidated registry is [`PRD.md`](./PRD.md) §17.8.

## 5. Traceability

- **PRD**: [`PRD.md`](./PRD.md)
- **ADRs**: [`ADR/`](./ADR/) — `cpt-cf-bss-pricing-adr-canonical-scope-key`, `cpt-cf-bss-pricing-adr-grandfathering-cohort-axis`, `cpt-cf-bss-pricing-adr-pricewindow-consolidation` (further ADRs planned as dependent slices land: snapshot/versioning strategy, grandfathered-row immutability, customer-group ownership, derived-meter formula-as-data, `CatalogVersion` increment/batching, `brand`-as-`PriceOverlay`)
- **Design set**: [`design/`](./design/) — Foundation + per-capability slice designs; the phased map and dependency order are in §1.3 and [`design/README.md`](./design/README.md).
