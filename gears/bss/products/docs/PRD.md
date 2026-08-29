---
refs:
  - bss/manifest/vz-arch-manifest-bss-only.md
  - bss/prd/PRD-billing-ledger-balances-202604041200
  - bss/prd/PRD-billing-module-202601120119
  - bss/prd/PRD-billing-system-202601120119
  - bss/prd/PRD-contracts-agreements-202601120119
  - bss/prd/PRD-metering-pricing-module-202601120119
  - bss/prd/PRD-plan-price-modeling-202605281200
  - bss/prd/PRD-product-catalog-marketplace-202601120119
  - bss/prd/PRD-rating-engine-202604031200
  - bss/prd/PRD-subscriptions-entitlements-202601120119
  - bss/prd/PRD-subscriptions-lifecycle-202604021200
  - bss/prd/PRD-tariffs-pricing-logic-202604011200
---

# PRD — Product & SKU Management

> **Provenance (2026-07-16):** vendored from `constructorfabric/gears-rust` PR **#4177**
> (`add-product-sku-prd` @ `6d3aab4`, author Corw1n-of-Amber) — this branch is the canonical home
> (upstream detach decision, 2026-07-15/16); no back-port or drift-tracking obligation.
> **Local changes applied at vendoring:** gear renamed **`product-sku` → `products`** (2026-07-16,
> incl. the ID prefix `cpt-cf-bss-product-sku-*` → `cpt-cf-bss-products-*`), and the RG2 fix in `cpt-cf-bss-products-fr-metering-unit-declaration`
> (unit ≠ dimension; the separate-SKUs mandate for multi-dimension usage replaced with the
> plan-price-owned dimension-set model — rating `SEAMS.md` §I RG2). **Known localization debt
> (tracked as rating SEAMS §I RG3):** pre-consolidation names (`PRD-tariffs-pricing-logic`,
> `PRD-rating-engine`, actor `…-actor-tariffs` — post ADR-0002 both map to the one **rating** gear,
> `gears/bss/rating/docs/PRD.md`) and `refs` front-matter paths in the legacy `docs/bss/prd/…`
> layout (kept verbatim as provenance). **RG3 reconciled 2026-07-16** at the first substantive
> edit: actor `…-actor-tariffs` merged into `…-actor-rating`; §2.1 delegations and the §17
> reference table localized to `gears/bss/rating/docs/PRD.md` / `gears/bss/subscriptions/docs/PRD.md`.
> **Further local change (D-46):** the `sellable` flag FR (`fr-sku-sellable`) —
> offering eligibility, enforced as pricing sellability-gate predicate 6.
> **Consistency fix wave :** AC #8 aligned with the post-RG2 FR (the stale
> separate-SKUs-per-dimension mandate removed; the `usageTypeRef` publish validation mirrored from
> the FR, veto flag preserved); dangling risk-table references repaired (`AC #48`/`#55`/`#45` did
> not exist — now named AC/NFR-show-stopper refs); remaining RG3 tails closed (§13 dependency rows
> merged into the one rating gear; §17.2 evaluation pointer localized). **Same-day cross-review
> round (PRD↔code + PRD↔designs):**
> "CI-verified" seam claims re-tensed to gated-future (the suite does not exist yet), the §5.2
> manual-publish bullet reconciled with D-47's demand-driven lanes, and five cross-gear opens
> added to §15 (event-envelope GATE, D-47-vs-governance GATE, `compositionPending` counterpart,
> freeze-ack silence, UC3(c) operand). **Two of those gates were then decided the same day
> (product calls, §15):** the registry adopts the event-broker's **broker-native envelope**
> (not CloudEvents 1.0; manifest §7.2 amendment owed), and a `CatalogVersion` increment is
> **mechanical** — the uncomposed-bundle two-person override moves to the bundle's entity
> publish; a system-initiated D-47 increment never waits on approval. **Two more the same day:**
> `SkuReferenceCount` v1 producer set = **{pricing}**, built jointly with this gear's development
> (Subscriptions/Contracts register at their own build); and Product **name uniqueness made
> absolute** per `(tenant, brand)` — region-independent, the region algebra surviving only as the
> parent-child containment check. **Veto round:** the UC3 `usageTypeRef` block
> CONFIRMED as amended — "is active" dropped (a UsageType has no lifecycle state) and the UC3(c)
> dimension-set cross-validation re-placed onto pricing's meter-binding rule (**P-D-43** strikes the donor id; priced ⊆
> `metadata_fields` at plan publish); nothing in this PRD awaited veto **as of that round**.
> ** branch review:** seven decisions — **P-D-14…P-D-20** — were registered
> FLAGGED for the owner; all seven have since been confirmed — P-D-19 as amended (P-D-47), the other six in P-D-48, with P-D-14 amended to *deferred*, P-D-18 given a v1 participant set of {plan-price} and P-D-20 its emitting door. They amend this PRD's normative text in many places (a count is deliberately not given here: it has been restated wrongly twice), and
> two of them reverse delivered design: P-D-19 (a force-completed version stays refused for
> posted use) and P-D-20 (the retirement lead window imposes no publish freeze). **Industry-gap wave
> (same day, vs Stripe/Zuora/Kill Bill):** environment promotion (rides bulk import/export,
> AC #33a + use case), catalog-version diff (`fr-catalog-version-diff`, AC #20a), well-known
> attribute seed widened (`imageUri`, `unitDisplayLabel`, `marketingFeatures[]`), feature-vocabulary
> ownership question → §15, `skuCode`-vs-`lookup_key` strictness contrast recorded in the
> identifier rationale.

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Architecture Alignment](#2-architecture-alignment)
  - [2.1 Catalog Decomposition and Registry Boundary](#21-catalog-decomposition-and-registry-boundary)
  - [2.2 Predecessor PRDs and Scope Migration](#22-predecessor-prds-and-scope-migration)
- [3. Actors](#3-actors)
  - [3.1 Human Actors](#31-human-actors)
  - [3.2 System Actors](#32-system-actors)
- [4. Operational Concept & Environment](#4-operational-concept--environment)
  - [4.1 Module-Specific Environment Constraints](#41-module-specific-environment-constraints)
- [5. Scope](#5-scope)
  - [5.1 In Scope](#51-in-scope)
  - [5.2 Out of Scope](#52-out-of-scope)
- [6. Functional Requirements](#6-functional-requirements)
  - [6.1 Identifiers & Integrity](#61-identifiers--integrity)
  - [6.2 Product & Taxonomy Definition](#62-product--taxonomy-definition)
  - [6.3 SKU Definition & Classification](#63-sku-definition--classification)
  - [6.4 Attributes & Localization](#64-attributes--localization)
  - [6.5 Versioning, Lifecycle & Deprecation](#65-versioning-lifecycle--deprecation)
  - [6.6 Catalog Versioning & Snapshots](#66-catalog-versioning--snapshots)
  - [6.7 Approval, Publishing & Eventing](#67-approval-publishing--eventing)
  - [6.8 Multi-Tenancy & Read Models](#68-multi-tenancy--read-models)
  - [6.9 Bulk Operations](#69-bulk-operations)
  - [6.10 Cloning](#610-cloning)
  - [6.11 Data Retention & Erasure](#611-data-retention--erasure)
  - [6.12 Cross-PRD Consistency](#612-cross-prd-consistency)
  - [6.13 Operational Resilience & Concurrency](#613-operational-resilience--concurrency)
- [7. Non-Functional Requirements](#7-non-functional-requirements)
  - [7.1 NFR Inclusions](#71-nfr-inclusions)
  - [7.2 NFR Exclusions](#72-nfr-exclusions)
- [8. Five Quality Vectors Analysis](#8-five-quality-vectors-analysis)
- [9. Public Library Interfaces](#9-public-library-interfaces)
  - [9.1 Public API Surface](#91-public-api-surface)
  - [9.2 External Integration Contracts](#92-external-integration-contracts)
- [10. Use Cases](#10-use-cases)
- [11. User Interaction and Design](#11-user-interaction-and-design)
- [12. Acceptance Criteria](#12-acceptance-criteria)
  - [Identifiers & Integrity](#identifiers--integrity)
  - [Product & Taxonomy Definition](#product--taxonomy-definition)
  - [SKU Definition & Classification](#sku-definition--classification)
  - [Attributes & Localization](#attributes--localization)
  - [Versioning, Lifecycle & Deprecation](#versioning-lifecycle--deprecation)
  - [Catalog Versioning & Snapshots](#catalog-versioning--snapshots)
  - [Approval, Publishing & Eventing](#approval-publishing--eventing)
  - [Multi-Tenancy & Read Models](#multi-tenancy--read-models)
  - [Bulk Operations](#bulk-operations)
  - [Cloning](#cloning)
  - [Data Retention & Erasure](#data-retention--erasure)
  - [Cross-PRD Consistency](#cross-prd-consistency)
  - [Error & Negative Paths](#error--negative-paths)
  - [Operational Resilience & Concurrency](#operational-resilience--concurrency)
  - [Non-Functional Requirements (Show-Stoppers)](#non-functional-requirements-show-stoppers)
- [13. Dependencies](#13-dependencies)
- [14. Assumptions](#14-assumptions)
- [15. Open Questions](#15-open-questions)
- [16. Risks](#16-risks)
- [17. Reference Materials](#17-reference-materials)
  - [17.1 Configurable-Policy Interim Defaults](#171-configurable-policy-interim-defaults)
  - [17.2 Monetization-Model Traceability](#172-monetization-model-traceability)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

**Product & SKU Management** is the authoritative, multi-tenant **catalog registry** for VHP BSS: the System of Record for *what can be sold, how it is described, classified, versioned, and published*. It owns Products, SKUs, categories/taxonomy, attributes/localization, and immutable catalog versions, with **financial-grade governance** (approval-gated publishing, immutable audit, deterministic snapshots) so that Plan & Price Modeling, Subscriptions, Contracts, Tariffs/Rating, Billing, Marketplace, and Presentation build on **stable, versioned, reproducible** catalog references.

It owns the **registry** half of BSS manifest §4.1 and stops at the SKU (including the `bundle` type flag and the metering-unit declaration). All commercial-pricing concerns are delegated by reference to the sibling decomposition PRDs (§2.1).

### 1.2 Background / Problem Statement

BSS must monetize diverse offerings (IaaS, PaaS, SaaS, marketplace services) across a multi-tenant, brand/region-scoped hierarchy. Without a single authoritative catalog registry, plan/price authoring, subscriptions, contracts, rating, and billing bind to mutable, non-reproducible product state — breaking posted invoices, active contracts, and in-flight subscriptions when the catalog changes, and leaving governance (who approved what, when) unauditable.

This PRD carves the **registry** scope out of the combined predecessor (`PRD-product-catalog-marketplace-202601120119`), completing the §4.1 decomposition already begun by Tariffs and Plan & Price Modeling. It fixes lifecycle/versioning semantics (draft → published [↔ deprecated] → retired, immutable history), a catalog publish contract (approval-gated, idempotent, event-fanned-out), a catalog-wide immutable `CatalogVersion` snapshot, and a stable SKU contract (identity, type, `PlanTier`, metering-unit declaration) that downstream modules can assume without re-validation.

### 1.3 Goals (Business Outcomes)

- **Flexibility / time-to-market**: Product Managers self-serve Product, SKU, category, and attribute changes across offering types without engineering involvement.
- **Stable monetization foundation**: every published SKU exposes a stable identifier, type, `PlanTier` classification, and metering-unit declaration, so plan/price authoring and rating bind to a fixed reference.
- **Auditable governance**: two-person approval for material catalog changes, immutable version history, and a complete event + audit trail satisfy financial and regulatory controls.
- **Safe evolution**: backward-compatible schema evolution and immutable `CatalogVersion` snapshots let the catalog change without breaking posted invoices, active contracts, or in-flight subscriptions.
- **Single source of truth**: one authoritative registry feeds partner/brand/region-scoped offerings, marketplace listings, and contract quotes.

> **Note**: The registry-vs-commercial boundary is stated canonically in §2.1. Where requirements or acceptance criteria touch commercial concerns, they define only the **registry-side contract** and reference the owning PRD.

### 1.4 Glossary

| **Term** | **Definition** |
|----------|----------------|
| **Catalog (registry)** | The authoritative registry of products/services/bundles/SKUs, categories, and localized attributes, and the catalog-wide version/publish mechanism (manifest §4.1). SoR: BSS. Defines *what can be sold and how it is described, classified, and published* — not how it is priced. |
| **Product** | A sellable or describable offering record with a name, **one required primary category plus optional secondary categories**, lifecycle state, brand/region scope, and version. The top of the catalog hierarchy. Identified by a system-generated `productId`. |
| **SKU (Stock Keeping Unit)** | A uniquely identifiable variant of a Product, typed as `product`, `service`, or `bundle`, optionally carrying a **metering-unit declaration** (for usage products) and stable accounting codes (`taxCategory`, `glCode`). A SKU has two identifiers: a system-generated immutable `skuId` and an operator-supplied human-readable `skuCode`. A SKU carries its own brand/region scope, **contained within its parent Product's scope**; the SKU→Product link is immutable after first publish. |
| **Usage SKU** | Definition, not detection: a SKU that **carries a metering-unit declaration**. There is no separate "is-usage" flag — declaring a metering unit **is** what makes a SKU a usage SKU. "A usage SKU missing its declaration" is not a detectable registry state; usage-completeness is enforced at the plan-price seam, never at registry publish. |
| **Sellable** | Per-SKU offering-eligibility flag (`sellable`, default `true`; D-46). `sellable = false` = **composition/metering-only**: the SKU publishes normally, MAY be referenced as a bundle/plan component and MAY carry a metering-unit declaration, but MUST NOT be offered **standalone** (pricing sellability-gate predicate 6). Distinct from lifecycle (`published` = *referenceable*) and from per-market GA gates (`not_sellable_ga`). The migration cover for technical/component SKUs of existing catalogs. |
| **Identifier** | The registry distinguishes **system identity** from **human/business code**. `productId`/`skuId` are server-generated immutable UUIDs. `skuCode` is operator-supplied, fixed-format, tenant-unique, immutable after first publish. Products MAY carry an optional `productCode` under the same reservation rules. Downstream consumers bind to `skuId`; humans/external catalogs reference `skuCode`/`productCode`. |
| **Bundle (SKU type)** | A SKU whose `type = bundle`. This PRD owns only the **type flag and identity**; the bundle's commercial composition (included SKUs, constraints, revenue share, invoice itemization) is authored in plan-price. A published bundle is commercially incomplete until composed. |
| **Category** | A node in the catalog taxonomy for browse, search, curation, and marketplace listing classification; supports hierarchy. |
| **Attribute** | A **governed** (defined, typed, optionally localized) key/value descriptor attached to a Product or SKU with brand/region visibility, managed via attribute **definitions**. Contrast the ungoverned **Metadata map**. |
| **Metadata map** | An **ungoverned**, per-entity free-form key/value channel for machine metadata (external ids, sync markers, migration tags): tenant-scoped, size-bounded, non-localized, excluded from read-model search, still PII-prohibited, captured in `CatalogVersion` snapshots. |
| **Brand** | A commercial/presentation identity within a tenant under which Products/SKUs/attributes are scoped for visibility, isolation, and localized display. A **visibility/legal scope, not a pricing dimension**. |
| **Region** | A geography/jurisdiction scope on Product/SKU/Attribute governing **visibility, legal availability, and localization fallback** — **never** pricing (currency/price-region/FX are plan-price/Tariffs). Drives read-model scoping and parent-child scope containment; name uniqueness is **region-independent** (§15 decision). Region-set semantics are needed only for containment (pinned in Design; interim conservative subset-check, fail-closed). |
| **PlanTier** | Mandatory classification carried on SKUs/Plans (manifest §4.1) consumed by Subscriptions, `SlaPolicy`, and quota/entitlement policies. This PRD owns the **PlanTier taxonomy and the SKU-level value**; plan-price enforces presence at **plan** publish. Distinct from **OrgTier** (a partner commercial standing that never changes tenant topology). |
| **Metering-unit declaration** | The unit identity (e.g. vCPU-hours, GB-storage) declared on a usage SKU. This PRD owns the **declaration and its validation**; usage collection is OSS metering, plan-level meter binding is plan-price, and rating is Rating. |
| **Lifecycle state** | The Product/SKU state machine: `draft → published [↔ deprecated] → retired`, plus `draft → discarded` for never-published entities. `deprecated` is a governed sub-state of `published` (referenceable by existing consumers, closed to new adoption). `retired` is terminal (revival only via clone). `discarded` is terminal for an abandoned never-published draft (releases the `skuCode` reservation, audited, emits a discard event). |
| **Deprecation** | A governed marking of a `published` SKU that blocks **new adoption** while existing references continue, ahead of eventual retirement; modeled as the `deprecated` sub-state. |
| **Retirement / EOL** | Retirement runs as a **scheduled transition**: at initiation the entity is forced into `deprecated` (new adoption blocked immediately, still browsable) for the lead-time window (≥ 30 days interim), then flips to `retired` at `effectiveAt`. **EOL** is the optional stronger variant that additionally sets a `mustMigrateBy` date; the registry guarantees only the event + lead-time contract, while live-subscription migration is owned by subscriptions-lifecycle. |
| **Revision vs published version** | Two counters. The **internal revision** increments on every save (incl. draft edits) for optimistic concurrency and audit. The **published version** increments only on publish and is what downstream consumers and `CatalogVersion` reference. Draft churn MUST NOT inflate the published version. |
| **CatalogVersion** | An immutable, checksummed, **full** published snapshot of the catalog at publish time (monotonic `catalogVersionId`), enumerating the published Product/SKU/Category/Attribute set and their published versions. Re-resolving a `catalogVersionId` MUST always yield a byte-identical checksum. plan-price/Contracts/Billing freeze their own content keyed to `catalogVersionId`. One **component** of a downstream `pricingSnapshotRef` (defined in Tariffs); not equal to it. |
| **Material change / Materiality threshold** | A change is material when it touches the enumerated material-field set (canonically defined in the Materiality-gated-publish FR/AC) or exceeds a configured count of affected entities. The threshold is a typed, configurable policy with a default; material changes trigger the two-person rule. |
| **Recognized-unit set** | The configured set of metering units a usage SKU may declare. Owner and add-unit approval path are an owned dependency (§15); the registry validates declarations against the configured set. |
| **Two-person rule** | A multi-approver control requiring the tenant's **configured approver quorum `N`** — distinct approvers, each distinct from the author — for material/above-threshold catalog changes before publication. `N` is a typed-policy value with **default 2** and **floor 0** (P-D-11); the name is retained because 2 is the default and because the phrase is used as shorthand throughout this PRD and the design set, but it **never** denotes a fixed count. The predicates it carries are not configurable: ≥ 1 FinanceReviewer on finance-material fields, self-approval refused at every `N ≥ 1`, and at `N = 0` the record states the predicate as unsatisfiable rather than blocking. Wherever "two-person" appears as shorthand below, read it as this quorum — with the reach **enumerated** by P-D-13  rather than left to the reader: cross-tenant break-glass elevation (AC #30) is **not `N`-governed at all** (its principal is a platform owner, not a tenant's, so it carries a fixed floor of two distinct platform principals or the post-hoc-review arm), while freeze force-completion (AC #22), the uncomposed-bundle override (AC #19), un-deprecation (AC #17), the slice-07 correction door (AC #4) and the slice-07 **break-glass correction** (AC #4, the flag-gated lane — a sixth site, added: it is neither the cross-tenant break-glass of AC #30 nor the ordinary correction door, so it inherited no disposition) follow `N` and **record the reduction** (`quorumReduced` on the authorizing record and the act's event) whenever the effective count falls below the retained-name default of 2. |
| **Read model** | Cache-first, query-optimized projection of published catalog content for high-throughput browse/search; converges within a bounded window of the write model. |
| **Active-reference count** | Whether a SKU has live downstream references (plans, subscriptions, contracts). Derived as a **3-state predicate** from the `SkuReferenceCount` signal: **fresh-zero ⇒ unreferenced** (enables correction), **fresh > 0 ⇒ referenced**, **stale/never-received ⇒ conservatively referenced**. |
| **`SkuReferenceCount` signal** | A named, owned integration contract by which the **registered producer set** publishes reference liveness (**v1 = plan-price (pricing gear)** — P-D-03; Subscriptions and Contracts are post-v1 candidates and are **not** registered, because an unregistered producer's silence must never read as a reference, while a registered producer that never posts makes every SKU conservatively referenced and un-retirable) via a **per-producer watermark** ("as of `T`, my complete live-reference set is {…}"). Absence of a `skuId` under a fresh watermark ⇒ zero for that producer. `referenced` is a boolean OR across producers; the registry never sums across producers. A stale watermark is conservatively referenced + alerted; never-received is conservative but flagged distinctly. |
| **freezeComplete** | A per-`CatalogVersion` flag indicating that all registered freeze-participants (the v1 registered set is **{plan-price}** — P-D-48; Contracts and Billing register at their own build time) have acknowledged freezing their referenced content for that `catalogVersionId`. Resolution for posted/contractual use is rejected until `freezeComplete`, with a bounded timeout that fails closed. |
| **Freeze-participant set** | The configured list of modules that MUST acknowledge a freeze for a `CatalogVersion`. Membership is governed (two-person) and snapshotted into each `catalogVersionId`. |
| **Grandfathering** | Continuation of an existing reference on its frozen snapshot after the underlying SKU is deprecated/retired. The registry guarantees the snapshot is never mutated; eligibility policy is owned by plan-price / subscriptions-lifecycle. |
| **`compositionPending`** | A registry flag on a `bundle` SKU published-with-override while still uncomposed. While `true` the SKU is not-yet-adoptable for new references; cleared by a plan-price composition signal (`BundleCompositionCompleted`, **inbound**, owned by plan-price) and emitted as `SkuCompositionCleared` (**outbound**, owned here — renamed: one name had been carrying both directions). |
| **OrgTier** | A partner's commercial standing (a partner commercial projection that MUST NOT change tenant topology). Distinct from `PlanTier`. |

## 2. Architecture Alignment

| **Field** | **Value** |
|-----------|----------|
| **Applicable Manifest(s)** | BSS |
| **Manifest Chapters** | §4.1 Product and Service Catalog (primary — catalog **registry**: Product, SKU, Category, Attribute, CatalogVersion, lifecycle, approvals, publish); §4.4 Billing and Invoicing (posting/snapshot immutability); §4.3 Subscriptions, §4.2 Rating, §4.6 Contracts, §4.8 Marketplace (consumers of published SKUs); §2.1.3 Multi-tenant semantics; §7.2 Event governance (CloudEvents 1.0 — **re-scoped for this gear to the broker-native envelope, §15 decision; manifest amendment owed**) |

> **Normative alignment**: This PRD owns the **catalog registry** half of BSS §4.1 — the SoR for **Product, SKU, Category, Attribute, and CatalogVersion**, plus their authoring, versioning, lifecycle, taxonomy, localization, governance, publishing, and catalog-wide snapshotting. It MUST NOT contradict the sibling decomposition PRDs and MUST delegate, by reference, every commercial-pricing concern.

### 2.1 Catalog Decomposition and Registry Boundary

The combined Catalog (§4.1) capability is split across complementary PRDs (registry **plus** plan-price together realize "Catalog"). This PRD implements the **registry** half and is authoritative ONLY for the registry concerns above. The following are owned elsewhere and MUST NOT be re-specified here:

- **Plan, Price, PriceWindow, PriceList, Bundle composition, add-on rules, billing descriptors (invoice line template / tax category / GL code), plan lifecycle/migration/grandfathering, plan publish validation & approval, price history** → `PRD-plan-price-modeling-202605281200`.
- **Price resolution / override precedence / tier-volume-hybrid-commitment math / FX / `pricingSnapshotRef` composition** → the **rating** gear (evaluation core), `gears/bss/rating/docs/PRD.md`.
- **Usage → RatedCharge → BillableItem** → the **rating** gear (pipeline), `gears/bss/rating/docs/PRD.md`. **Subscription lifecycle** → `gears/bss/subscriptions/docs/PRD.md`. **Marketplace vendor ops (§4.8)** → `PRD-product-catalog-marketplace-202601120119`.

> **Registry vs commercial boundary (normative):** A **SKU** is the unit of *what exists and how it is described/classified/published*; a **Plan/Price** is the unit of *how it is sold and charged*. This PRD stops at the SKU (the `bundle` type flag and the metering-unit declaration). Two corollaries: **(a) No monetization-model marker** — the registry carries no monetization-model field; only **usage** leaves a footprint (the metering-unit declaration); absence of a model marker is intentional, not a gap. **(b) `region` is visibility/legal scope only** — never a pricing dimension here.

### 2.2 Predecessor PRDs and Scope Migration

- **PRD-product-catalog-marketplace-202601120119** (combined Catalog §4.1 + Marketplace §4.8) — the **registry** scope items ("Product & SKU Management", "Catalog Versioning", catalog-level approval workflows, "Localization & Branding Infrastructure", taxonomy/attributes) are **superseded by this PRD**; its `UC-product-sku-management-202601121200` use-case doc is superseded here. Marketplace (§4.8) scope remains authoritative there pending a dedicated Marketplace PRD.
- **PRD-plan-price-modeling-202605281200** — authoritative for Plan, Price, PriceWindow, PriceList, Bundle composition, add-ons, billing descriptors, plan lifecycle/migration, plan publish validation & approval. This PRD provides the published SKU/Category/Attribute foundation and `CatalogVersion` that plan-price builds on. The two MUST stay consistent on: SKU identity & `bundle` type, metering-unit declaration, `PlanTier` taxonomy, and `CatalogVersion`.
- The **rating** gear (`gears/bss/rating/docs/PRD.md`; post-ADR-0002 consolidation of the former tariffs-pricing-logic + rating-engine PRDs) — downstream consumer of published SKUs and `CatalogVersion`; owns evaluation and charging.

> **Recommendation on the combined PRD (§15):** Do **not** delete `PRD-product-catalog-marketplace-202601120119`. After this PRD + plan-price + Tariffs are approved, refactor it into a Marketplace-only PRD (§4.8). Until then, this PRD is authoritative for catalog-registry requirements only.

## 3. Actors

### 3.1 Human Actors

#### Product Manager

**ID**: `cpt-cf-bss-products-actor-product-manager`

**Role**: Self-serves Product, SKU, category, and attribute authoring across offering types.
**Needs**: Product/SKU editor, taxonomy manager, attribute/localization editor, metering-unit and `PlanTier` selection, clone/templating.

#### Catalog Admin

**ID**: `cpt-cf-bss-products-actor-catalog-admin`

**Role**: Governs the catalog: publishes `CatalogVersion`, runs bulk import/export, manages break-glass, force-completes stuck freezes, requests immutable-field corrections.
**Needs**: Approval/publish console, bulk-operations console, freeze monitoring & recovery, break-glass elevation.

#### Finance Reviewer

**ID**: `cpt-cf-bss-products-actor-finance-reviewer`

**Role**: Reviews and approves finance-material catalog changes (`taxCategory`, `glCode`, `PlanTier`); second approver under the two-person rule for finance-bearing changes.
**Needs**: Pending-approval queue with diffs, pre-publish lint report, separation-of-duties enforcement.

#### Auditor

**ID**: `cpt-cf-bss-products-actor-auditor`

**Role**: Inspects immutable version history, audit trail, and lineage; exports for compliance.
**Needs**: Version timeline with diffs, tenant-scoped audit retrieval, break-glass audit-export.

#### Platform Owner

**ID**: `cpt-cf-bss-products-actor-platform-owner`

**Role**: Privileged cross-tenant operator; accesses foreign-tenant catalog only under time-boxed break-glass.
**Needs**: Break-glass read/audit-export (writes separately gated or disallowed in v1).

### 3.2 System Actors

#### Plan & Price Modeling

**ID**: `cpt-cf-bss-products-actor-plan-price`

**Role**: Consumes published SKU identity/type, metering-unit declaration, `PlanTier`, `CatalogVersion`; produces `SkuReferenceCount`, `freezeComplete` ack, and the bundle composition-completed signal that clears `compositionPending`.

#### Rating (evaluation core + pipeline)

**ID**: `cpt-cf-bss-products-actor-rating`

**Role**: The one **rating** gear (post ADR-0002 consolidation; absorbs the former "Tariffs / Pricing Logic" consumer — id `…-actor-tariffs` retired): consumes published SKU refs + `CatalogVersion` for price resolution, and the metering-unit declaration to map usage. No authoring here.

#### OSS Metering

**ID**: `cpt-cf-bss-products-actor-oss-metering`

**Role**: Emits usage values (external); consumes the metering-unit **declaration** on usage SKUs.

#### Subscriptions

**ID**: `cpt-cf-bss-products-actor-subscriptions`

**Role**: Consumes published SKU refs + `PlanTier` + `replacedBy` for eligibility/composition/migration; produces `SkuReferenceCount`; consumes `mustMigrateBy` (post-v1 EOL). Owns live-subscription migration.

#### Contracts & Agreements

**ID**: `cpt-cf-bss-products-actor-contracts`

**Role**: Consumes `CatalogVersion` snapshots for quotes; produces `SkuReferenceCount` (incl. draft/quote refs per producer contract) and `freezeComplete` ack.

#### Billing & Invoicing

**ID**: `cpt-cf-bss-products-actor-billing`

**Role**: Consumes published SKU refs + `CatalogVersion`; produces `freezeComplete` ack (descriptor freeze). Billing descriptors are authored in plan-price and frozen into `CatalogVersion`.

#### Marketplace & Vendor Portal

**ID**: `cpt-cf-bss-products-actor-marketplace`

**Role**: References published SKUs in vendor listings. Vendor ops remain in the Marketplace PRD (§4.8).

#### Presentation / Portals

**ID**: `cpt-cf-bss-products-actor-presentation`

**Role**: Consumes catalog read models for browse/search cache warming.

#### Events & Audit (Common Core)

**ID**: `cpt-cf-bss-products-actor-events-audit`

**Role**: Provides the shared event system: durable acceptance, per-consumer delivery/dead-letter state, bounded-backoff retry. Transport mechanics owned there.

#### Tenant Identity (OSS/AMS + IdP)

**ID**: `cpt-cf-bss-products-actor-oss-ams-idp`

**Role**: Supplies `tenantId`, brand/region claims, OrgTier projection targets, and role claims. The registry MUST NOT mutate tenant topology.

## 4. Operational Concept & Environment

### 4.1 Module-Specific Environment Constraints

- **Registry is upstream of all commercial modeling**: a SKU MUST be published before a Plan/Price can reference it. The registry MUST NOT require any downstream consumer to re-interpret mutable catalog state for **posted** periods; the `CatalogVersion` snapshot contract is authoritative (manifest §4.4).
- **Multi-tenant isolation**: tenant/brand/region scoping via IdP claims; deny-by-default at the gateway; cross-tenant access audited; time-boxed break-glass for platform-owner access.
- **`region` is visibility/legal scope, never pricing**; currency/price-region/FX live in plan-price/Tariffs.
- **Time**: scheduled publish (`publishAt`) and scheduled retirement (`effectiveAt`) are UTC; retirement lead-time ≥ 30 days (interim).
- **Eventing**: every state-changing mutation emits an event in the platform event-broker's **broker-native envelope** (event-broker ADR-0003 — **not** CloudEvents 1.0; §15 decision) with a versioned (semver) schema reference, correlation/causation, and per-aggregate ordering keys `(tenant, aggregate)`; **pseudonymous actor references only** (no direct PII). Delivery/ordering/dead-letter mechanics are owned by the event-broker, not re-specified here.
- **Snapshots are financial records**: `CatalogVersion` snapshots + version history require a durability class (interim ≥ 11 nines / replicated storage), backup/restore with periodic checksum verification, and a cross-region/DR posture.

## 5. Scope

### 5.1 In Scope

| **Feature** | **Priority** | **Notes** |
|-------------|--------------|-----------|
| Product definition | `p1` | Create/update Products: name, one required primary category + optional secondary, description, brand/region scope, lifecycle, version (§4.1 Product). |
| Category & taxonomy | `p1` | Hierarchical Category tree; cycle-free; uniqueness within parent. |
| SKU definition & typing | `p1` | Define SKUs typed `product`/`service`/`bundle`; stable accounting codes; `bundle` type flag only (composition is plan-price). |
| Metering-unit declaration | `p1` | Declare/validate the usage metering unit (unit identity only); governed de-listing. Consumed by plan-price, metering, Rating. |
| PlanTier taxonomy & SKU classification | `p1` | Own the `PlanTier` taxonomy and the SKU-level value; plan-price enforces presence at plan publish. Distinct from OrgTier. |
| Attribute management & localization (i18n) | `p1` | Extensible attribute schema; i18n with brand/region visibility and fallback `(locale,region,brand) → (locale,brand) → (default-locale,brand) → global`. |
| Identifiers & integrity | `p1` | Server-generated immutable `productId`/`skuId`; operator `skuCode` fixed-format, tenant-unique, immutable after first publish; field-mutability rules. |
| Product/SKU versioning & immutable history | `p1` | Internal revision per save; published version on publish; immutable history with diff; optimistic concurrency. |
| Product/SKU lifecycle, deprecation & retirement | `p1` | `draft → published [↔ deprecated] → retired` state machine; scheduled publish (`publishAt`) + scheduled retirement (`effectiveAt`); parent-child publish-ordering + cascade; retirement/EOL blocks new adoption, preserves references/snapshots, emits consumer handoff + lead-time. |
| Catalog versioning & snapshots (CatalogVersion) | `p1` | Stage/publish immutable full `CatalogVersion` with checksum + monotonic id; byte-identical re-resolution; emit `CatalogVersionPublished`; expose `freezeComplete`. |
| Catalog approval & publishing workflow | `p1` | Approval-gated publish with the **configured approver quorum** (default 2, floor 0 — P-D-11) above a typed materiality policy; approvals pinned to the approved revision; idempotent mutation boundary. Categories & attribute definitions are governed live entities. |
| Multi-tenant isolation & brand/region scoping | `p1` | Tenant/brand/region scoping via IdP claims; deny-by-default; audited cross-tenant; time-boxed break-glass; RBAC. |
| Eventing, audit & integration surface | `p1` | Publish a broker-native event (§15 decision) for every state-changing mutation with versioned schema ref + correlation/causation + ordering keys; pseudonymous actors; replay/bootstrap; immutable audit. |
| Data retention & right-to-erasure | `p1` | Defined retention for retired entities/versions/audit; reconcile immutable audit + version history with GDPR/CCPA (pseudonymize operator PII, retain financial records). |
| Catalog read models (core browse/search) | `p1` | Cache-first read models scoped by tenant/brand/region, bounded convergence; premise of the show-stopper NFRs (§7). |
| Bulk import (catalog onboarding at scale) | `p1` | Bulk create/update/import with idempotency + per-row partial-failure; imports land in draft and pass the gated publish. Required at ≥ 10K-SKU scale. Deterministic export/import doubles as **environment promotion** (staging → prod; AC #33a). |
| Advanced search, filter & faceting | `p2` | Rich faceted search/filter over the core read model. |
| Catalog lint / validation & snapshot export | `p2` | `validate(lint)` before publish (blocking-with-override at the **bundle's entity publish** for an uncomposed bundle — §15 decision; informational report at `CatalogVersion` publish); `export snapshot`. |
| Bulk export & bulk lifecycle tooling | `p2` | Deterministic snapshot export; bulk lifecycle (mass deprecate/retire) beyond parent→child cascade; catalog-version diff (`fr-catalog-version-diff`, AC #20a). |
| Product/SKU cloning / templating | `p3` | Clone to a new draft with new identifiers; explicit copy/reset field disposition; pricing not copied. |

### 5.2 Out of Scope

- **API schemas, storage DDL/data models, error-code taxonomies** — Design document(s).
- **Plan, Price, PriceWindow, PriceList, Bundle composition, add-on rules, billing descriptors, plan lifecycle/migration/grandfathering, plan publish validation & approval, price history, bulk price import, effective-price preview** — plan-price. This PRD owns only the SKU `bundle` **type flag** and stable SKU accounting codes, not their commercial use.
- **Price resolution, override precedence, tier/volume/hybrid/commitment math, FX, `pricingSnapshotRef` composition** — Tariffs.
- **Usage collection/normalization and charge rating** — OSS metering + metering-pricing-module + Rating.
- **Subscription lifecycle, entitlement enforcement, proration, recurring charge generation** — subscriptions-lifecycle + subscriptions-entitlements.
- **Contract negotiation, customer-specific overrides, committed-usage true-up, SLA penalties** — contracts-agreements.
- **Tax determination/statutory invoicing and revenue recognition** — Tax Engine / Billing / Finance. Catalog supplies only the tax-category/GL **code** on the SKU.
- **Marketplace vendor onboarding/certification, listings, fee schedules, payouts, fraud holds** — Marketplace PRD (§4.8).
- **Customer-facing storefront UI** — Presentation layer; Catalog provides read-model APIs.
- **Promotional/coupon pricing, eligibility, lifecycle** — Promotions (TBD) + plan-price. A *sellable* promo/$0/"Free" offering is a **normal registry SKU** here (identity, type, `PlanTier`, visibility); only its $0/promotional **price** is out of scope. Registry rules apply identically; no separate promo entity.
- **Tenant merge/split and brand transfer between tenants** — explicit non-goal for v1.
- **Binary / media assets** (images, icons, datasheets) — not stored here in v1; the registry MAY carry asset reference URIs as attribute values.
- **Recurring availability windows and time-triggered `CatalogVersion` publish** — out of scope for v1 (entity-level scheduled publish IS in scope). **Operator-initiated** `CatalogVersion` publish stays manual; the **demand-driven increment lanes of D-47** (`fr-catalog-version-publish`: interactive/bulk downstream publish requests) are IN scope and system-initiated. The composition is decided (§15): **an increment is mechanical** — every governance gate (incl. the uncomposed-bundle two-person override) attaches to the **entity publish** that introduces the exception, never to the increment itself.
- **Configurable products / CPQ** — not modeled; the registry uses a concrete SKU per variant.
- **Product-to-product relationships beyond `bundle`, parent-child, and `replacedBy`/supersedes** — not modeled in v1; a governed catalog-relationship block is a post-v1 consideration (§15).

## 6. Functional Requirements

> **Content boundary**: FRs define WHAT the registry must do, not schemas. Any concrete field/flag/event/idempotency-key name or format is an illustrative handle; canonical field schemas, formats/regexes, error codes, event catalog, and payloads are owned by the gear's DESIGN (`gears/bss/products/docs/DESIGN.md` — authored as the canonical index over the `design/` slice set; the design set is complete — all twelve slices authored). Full Given/When/Then acceptance detail is preserved in §12; interim configurable-policy defaults are in §17.1.

### 6.1 Identifiers & Integrity

#### Identifier contract

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-identifier-contract`

`productId` and `skuId` **MUST** be server-generated immutable identifiers (never operator-supplied). `skuCode` **MUST** be operator-supplied, fixed-format, tenant-unique, reserved **atomically at create time**, and immutable after first publish; once first published a `skuCode` **MUST** be permanently reserved within the tenant and **MUST NOT** be reissued. Downstream consumers **MUST** bind to `skuId`. Products **MAY** carry an optional `productCode` under the same rules.

**Rationale**: Stable system identity plus a protected human/external code is the foundation every downstream reference depends on. Permanent `skuCode` reservation is deliberate and **stricter than industry** (e.g. Stripe's `lookup_key` is atomically re-pointable to another price): a re-pointable human key trades auditability for flexibility, and this registry is financial-grade — the code is also the portable identity for environment promotion (AC #33a).

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### Product/SKU field-mutability matrix

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-field-mutability-matrix`

Mutability **MUST** be classified by lifecycle state: in `draft` all fields editable (incl. SKU→parent link and `skuCode`/`productCode`); after publish four buckets apply — **(i) structural identity** immutable and never correctable in place (remedied only by retire + clone); **(ii)** `type` and metering-unit declaration immutable but correctable via the governed fresh-zero path; **(iii) material-but-mutable** (`PlanTier`, `taxCategory`, `glCode`, `sellable`) change via a new published version under governance; **(iv)** other descriptive fields via a new published version. Illegal changes **MUST** be rejected fail-closed with an audited reason. The active-reference count **MUST** be sourced from `SkuReferenceCount` as the 3-state predicate; the registry **MUST NEVER** treat an entity as unreferenced absent a fresh watermark.

**Rationale**: Protecting identity/external caches while allowing governed evolution requires a per-state, per-field classification.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### Reference-signal sourcing, freshness & counting

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-reference-signal`

The registry **MUST** consume the named `SkuReferenceCount` signal (per-producer watermark). Absence of a `skuId` under a **fresh** watermark ⇒ zero for that producer; `referenced` **MUST** be a boolean OR (any registered producer > 0); the registry **MUST NOT** sum across producers, and each producer dedups within itself. A **fresh-zero** across all registered producers ⇒ unreferenced; a **stale** or **never-received** watermark ⇒ conservatively referenced (stale alert distinct from never-received). Only **registered** producers count; Contracts **MUST** declare whether draft/quote references count, with identical semantics across mutability/correction/retirement.

**Rationale**: A watermark-based OR predicate scales to 10K+ SKUs × N producers without dense zero publishing and never falsely frees a referenced SKU.

**Actors**: `cpt-cf-bss-products-actor-plan-price`

#### Immutable-field correction (zero-reference & break-glass)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-immutable-field-correction`

For a correctable immutable field (`type` or metering-unit declaration; **never** structural identity), if `SkuReferenceCount` is **fresh and zero** across all producers the system **MUST** allow a governed re-publish under the two-person rule, increment the published version, and emit `SkuImmutableFieldCorrected`; absent a fresh-zero signal it **MUST** reject fail-closed. While the signal is entirely unavailable, a single-SKU correction **MAY** proceed only via **break-glass** (two-person + mandatory reason + `SkuCorrectionOverride` recording signal-unavailability), behind a feature flag OFF by default. **Third admission arm (P-D-16):** when the subject's declared `usageTypeRef` **no longer resolves** — the resolver answers not-found, never a timeout — a **meter-declaration** correction **MAY** proceed **regardless of the reference predicate**, under the same ceremony, with `SkuCorrectionOverride` recording **`unresolvable-target`** rather than unavailability evidence. The binding is already broken, so the fail-closed default preserves nothing; without this arm such a SKU is wedged in every lane at once (§15, closed by P-D-16).

**Rationale**: Corrections must be provably safe (fresh-zero) or explicitly break-glass-audited, never silent.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

### 6.2 Product & Taxonomy Definition

#### Create a product

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-create-product`

Creating a Product **MUST** generate a `productId`, persist as `draft` (published version 0) with full multi-tenant isolation and an audit entry (primary category optional at draft, required at publish; optional secondary categories allowed). Name uniqueness **MUST** be enforced on `(tenantId, brandId, normalized(name))` **absolutely — region-independent** (§15 decision): two same-named Products under one tenant+brand are forbidden regardless of region scope. The uniqueness key is the **canonical internal name** (a quasi-code); localized display name/description are well-known attributes and **MAY repeat freely** — regional variants keep distinct internal names with identical display names. Relaxing to region-disjoint coexistence is a compatible post-v1 widening; the reverse would be a breaking tightening, which is why v1 starts strict.

**Rationale**: The canonical internal name is a quasi-code, unique across all regions (P-D-04) — deterministic collisions with no region algebra at the create door; multi-region catalogs get same-*display*-name coexistence through localized display attributes, never through duplicate internal names.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### Manage taxonomy (create, rename, re-parent, retire, delete)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-manage-taxonomy`

Category operations **MUST** validate name uniqueness within parent (re-checked on rename/re-parent), reject cycles, and reject exceeding configured max depth/children. A Product carries **exactly one primary category + zero or more secondary**; the read model **MUST** make it filterable under every assigned category. Categories are **governed live entities** (in-place, two-person-gated on material ops, audited) — not draft/publish-versioned. Retire/delete **MUST** be blocked while any active Product references the category (as primary or secondary) or active child categories exist. Each operation emits the corresponding `Category*` event.

**Rationale**: Taxonomy reshaping affects browse for every published product, so it must be governed and cycle-free.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

### 6.3 SKU Definition & Classification

#### Define a SKU

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-define-sku`

Defining a SKU **MUST** link it to the Product, assign `skuId`/`skuCode`, type it `product`/`service`/`bundle`, and validate the per-type required-field set. A `bundle`-typed SKU **MUST** persist only the type flag and identity (composition authored in plan-price; publishing it **uncomposed** requires the explicit two-person override at its **entity publish** — §15 decision). Promotional/$0/"Free" SKUs **MUST** follow identical registry rules; there is no separate promo entity.

**Rationale**: A uniform, type-aware SKU contract lets downstream bind without re-validation.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### Sellable flag (offering eligibility)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-sku-sellable`

A SKU **MUST** carry a dedicated `sellable` flag (default `true`), independent of lifecycle state (D-46). `sellable = false` marks a **composition/metering-only** SKU: it publishes normally, MAY be referenced as a bundle/plan component and MAY carry a metering-unit declaration, but **MUST NOT** be offerable standalone — plan-price enforces this as sellability-gate predicate **(6)** for standalone lines (bundle-**component** references are exempt; the component conjunction keeps predicates (1)–(5)). The flag is **material-but-mutable** (bucket iii of the mutability matrix): a change takes a new published version under governance and is frozen per `CatalogVersion`.

**Rationale**: `published` means *referenceable*, not *offerable* — migrated catalogs carry technical/component SKUs that must exist, meter, and compose without ever being sold alone; conflating the two forces either unpublishable components or accidentally offerable internals.

**Actors**: `cpt-cf-bss-products-actor-product-manager`, `cpt-cf-bss-products-actor-plan-price`

#### Declare metering unit

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-metering-unit-declaration`

Declaring a metering unit **MUST** validate it against the configured recognized-unit set and reject an unrecognized unit unless elevated approval marks a new validated unit. A usage SKU declares **exactly one** unit — the counted identity only. Pricing dimensions (`dimensionKey`) are **not** units: the declared dimension set over a meter is a plan-price concern (plan-level meter binding, §2.1), persisted on the plan-price revision and frozen into `pricingSnapshotRef` by Rating; multi-dimension usage on one unit **does not require separate SKUs**, and separate SKUs remain available where variants differ commercially (accounting codes, lifecycle). For a composite (derived) meter the SKU declares the composite's **output** unit; input units are referenced by the plan-price formula against the recognized-unit set and need no SKUs of their own. A draft whose unit was `deprecated` before first publish **MUST** be treated as a new declaration and rejected. The declared unit **MUST** be carried on publish; this PRD **MUST NOT** compute charges.

**Usage-source binding (UC3 seam adoption; veto round — CONFIRMED as amended; **P-D-05**)**: a metering-unit declaration **MUST** carry a **`usageTypeRef`** — the usage-collector `gts_id` of the UsageType (a platform-global catalog row `(gts_id, kind, metadata_fields)`) whose accepted entries feed this meter — as the authoritative attribution binding (rating SEAMS **UC3**(a); the phase-2 usage-event feed the **rating PRD** commits to as the UC1 remedy — not yet authored in the usage-collector design set). Publish **MUST** validate that the referenced UsageType **exists** (resolves in the catalog); *"and is active" was dropped by the veto round — a UsageType carries no lifecycle state (its catalog offers register/get/list/delete only, deletion FK-guarded against usage records)*. The UC3(c) **dimension-set cross-validation is NOT performed here** (vetoed as written: this PRD assigns dimension sets to plan-price, so the registry holds no operand to compare) — it lives at the **plan-price meter binding**: pricing's meter-binding rule (confirmed — **specified, not built** as of: pricing's own design calls the binding deferred and neither code is raised anywhere, so **no gate enforces this today on either side**) blocks plan publish when a priced `dimensionKey` falls outside the UsageType's declared `metadata_fields` keys. Rating never guesses the binding: an unresolvable `usageTypeRef` quarantines the usage record rather than mis-attributing it — which also fail-safes a UsageType deleted after publish (the collector's delete-RESTRICT guards only its own usage records, not catalog meters). **Quarantine is a fail-safe, not an operating mode**: a deleted UsageType leaves a sold-but-unrateable meter until remediation; closing that hole (extend the collector's delete guard to published declarations, or a deletion signal feeding pricing's `meter_binding_divergent` remediation) is a cross-gear open (§15).

**Rationale**: Declaring the unit is what defines a usage SKU; validation prevents downstream rate corruption.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### Metering-unit de-listing

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-metering-unit-delisting`

De-listing a metering unit **MUST** be rejected while ≥ 1 `published` SKU references it; the system **MUST** instead support marking it **deprecated** (no new declarations) with full removal only once unreferenced. A unit's identity/semantics are **immutable** (e.g. `GB-storage` MUST NOT be silently redefined to GiB); a correction is a new unit + deprecation of the old. De-listing/deprecation **MUST** be audited and **MUST NOT** mutate any frozen snapshot.

**Rationale**: Redefining a live unit corrupts every downstream rate; deprecate-then-remove protects existing references.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### PlanTier classification

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-plantier-classification`

Assigning `PlanTier` **MUST** validate against the taxonomy owned here and carry it on the published SKU. A `PlanTier` value has a **stable tier code** as identity; **rename affects the display label only**. Managing the taxonomy (add/rename/retire) is a governed (two-person), audited operation emitting `PlanTierUpdated`; a value **MUST NOT** be retired while any `published` SKU carries it (deprecate-then-retire). The taxonomy **MUST** be seeded with a neutral value. `PlanTier` **MUST NOT** be conflated with OrgTier; plan-publish presence enforcement is delegated to plan-price.

**Rationale**: Stable tier codes keep SLA/quota policies from rippling on rename; universal presence is a manifest mandate.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### Stable accounting codes on SKU

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-accounting-codes`

Setting tax-category and GL codes **MUST** persist them as stable codes and **validate each against a configured recognized set** (owner = Finance, deprecate-then-remove governance). The codes are **required at publish for `product`/`service`-type SKUs**; a SKU published without a required code is unpostable and **MUST** be rejected. The system **MUST NOT** compute tax or post to GL (codes only).

**Rationale**: Validating codes at authoring prevents unpostable SKUs surfacing weeks later at ERP export.

**Actors**: `cpt-cf-bss-products-actor-finance-reviewer`

### 6.4 Attributes & Localization

#### Localized attributes, well-known display fields, and definition lifecycle

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-localized-attributes`

Adding i18n attribute **values** **MUST** validate against the attribute **definition**, require the default-locale value (rejected at publish if absent), and resolve locale via `(locale, region, brand) → (locale, brand) → (default-locale, brand) → global` (default-locale resolved per brand, falling back to tenant default). The registry **MUST** seed **well-known display attribute definitions**: localized display name/description for Product/SKU/Category, plus (industry parity) `imageUri` (asset reference URI — binaries stay out per §5.2), `unitDisplayLabel` (the sales-facing unit label, e.g. "per vCPU-hour" — display only, never the metering-unit identity), and `marketingFeatures[]` (localized feature bullets for price pages). Managing attribute **definitions** is a governed live-entity operation emitting `AttributeDefinitionUpdated`; changes **MUST** be backward-compatible and follow a deprecate-then-remove lifecycle. The registry **MUST** provide an ungoverned, size-bounded, non-localized, search-excluded, PII-prohibited **metadata map** for machine metadata.

**Rationale**: Localized display without a second identity key, plus a governed/ungoverned split, keeps portals correct and integrations from flooding the definition registry.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

### 6.5 Versioning, Lifecycle & Deprecation

#### Revision vs published version

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-revision-vs-version`

The system **MUST** increment the **internal revision** on every save (rejecting stale-revision writes via optimistic concurrency) while the **published version** increments only on publish. Consumers and `CatalogVersion` **MUST** reference the published version; historical versions **MUST** be retained with a diff and **MUST NOT** be modifiable. **Version binding** **MUST** be explicit: a new reference binds to the latest published version at bind time; a frozen reference keeps its snapshot; a bound-but-not-yet-frozen reference re-resolves at freeze and **MUST** surface a version-change diff to the freezing module rather than silently swapping.

**Rationale**: Separating draft churn from published versions keeps downstream snapshots and quotes stable and auditable.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### Lifecycle transitions & reversibility

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-lifecycle-transitions`

The state machine `draft → published [↔ deprecated] → retired` (+ `draft → discarded`) **MUST** allow only its defined transitions, treat `retired` and `discarded` as terminal (revival only via clone), and allow downstream referencing only for `published`/`deprecated`. Entity publish **MAY** be **scheduled** (`publishAt`, UTC) with approval pinned at scheduling time and re-validated fail-closed at activation. Publication of incomplete entities **MUST** be rejected. There is **no `unpublish`** and **no in-place rollback** — retraction/reversion is `deprecate`/`retire` + a new version (forward-only).

**Rationale**: A constrained, forward-only state machine keeps caches/snapshots/posted content consistent.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### Parent-child (Product↔SKU) lifecycle integrity

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-parent-child-integrity`

A SKU **MUST NOT** reach `published` while its parent Product is not `published`, and the system **MUST NOT** orphan a `published` SKU under a `retired` Product. A SKU's brand/region scope **MUST** be contained within its parent's; a scope-narrowing Product publish **MUST** fail closed while any non-`retired` child would fall outside. Retiring a Product with non-`retired` SKUs **MUST** require confirmed **cascade-retire** (partial by design, recording `direct` vs `cascaded` provenance; EOL-requiring children left un-retired and listed; never-published children auto-`discarded`). When a partial cascade leaves children, the parent **MUST** remain non-`retired` and its deferred-retire intent tracked/queryable.

**Rationale**: Hierarchy and scope-containment invariants prevent orphaned or out-of-scope published content.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### Deprecation (governed sub-state)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-deprecation`

Marking a `published` SKU `deprecated` **MUST** move it to the `deprecated` sub-state, mark it so consumers **block new adoption** while existing references continue, and emit `SkuDeprecated`. `deprecated` **MUST** be a tracked, queryable state (not a flag), recording provenance `direct` (vs `cascaded`). The registry marks and exposes; the consumer enforces the new-adoption block (to be CI-verified via the `fr-plan-price-seam` suite — which does not exist yet, §15).

**Rationale**: A tracked sub-state makes the new-adoption guard testable and consistent with the composition-pending pattern.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### Un-deprecation

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-undeprecation`

`deprecated → published` **MUST** be allowed under the two-person rule, re-open new adoption, and emit `SkuUndeprecated`. Un-deprecating a **Product** reverses **only `cascaded`** child deprecations and **MUST NOT** revive a child's `direct` deprecation. The transition **MUST** be audited; a `retired` entity **MUST NOT** be reversible.

**Rationale**: Provenance-aware reversal prevents accidentally reviving individually-deprecated children.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### Retirement / EOL consumer handoff

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-retirement-eol`

Retiring a referenced SKU **MUST** require explicit confirmation with the active-reference count shown, then run as a **scheduled transition**: force `deprecated` at initiation (new adoption blocked immediately, still browsable), preserve snapshots, emit `SkuRetired`/`ProductRetired` with `{ skuId, fromVersion, reason, replacedBy?, mustMigrateBy?, effectiveAt }` honoring the ≥ 30-day lead-time, then flip to `retired` at `effectiveAt`. **`fromVersion` is the entity's `published_version` at the initiation instant** — the instant `SkuRetired` is emitted. The lead window imposes **no** publish freeze on the retiring entity (P-D-20): new adoption is blocked from initiation, publishing a further version stays permitted, and a publish during the window **MUST re-emit** `SkuRetired` carrying the new `fromVersion` with the same `effectiveAt` and retirement identity. Consumers key on `(skuId, effectiveAt)` and take the latest `fromVersion`; a re-announcement is an update, never a second retirement. The registry is SoR for `replacedBy` (a successor `published` SKU). **Joint contract with plan-price (closed, pricing D-47/AC #82):** the registry never flips a SKU to `retired` while the `SkuReferenceCount` predicate reads referenced (fresh > 0, or stale/never-received — conservatively referenced, fail-closed); plan-price's side is AC #82 — referencing plans are flagged on the deprecation signal and new adoption is blocked while in-flight subscribers keep their frozen snapshots (grandfathering). **v1 = plain retirement + grandfathering only; EOL-with-`mustMigrateBy` is a defined-but-deferred post-v1 follow-on** that MUST stay disabled until the consuming subscriptions-lifecycle AC exists and is referenced by number, and requires a consumer acknowledgment contract (lapsed ack ⇒ suspend fail-closed + `SkuEolSuspended`).

**Rationale**: A defined lead-time state and successor pointer let consumers migrate safely without undefined limbo.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

### 6.6 Catalog Versioning & Snapshots

> **Two publish layers.** A Product/SKU has its own **published version** that increments each time that entity is published. A **`CatalogVersion`** is a catalog-wide immutable snapshot; a SKU can be published yet not appear in any `CatalogVersion` until the next catalog publish. New references bind to the latest published entity version at bind time; posted/contractual references freeze to a `catalogVersionId`.

#### Publish an immutable catalog version

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-catalog-version-publish`

Publishing a `CatalogVersion` **MUST** persist a **full** snapshot (published Product/SKU set + their versions + current categories/attributes captured together), assign a monotonic `catalogVersionId`, generate a checksum, record `stagedAt`/`publishedAt`, capture the freeze-participant set, and make it immutable (storing references to plan-price/Contracts/Billing content, not that content). An uncomposed `bundle` SKU enters the snapshot flagged `compositionPending = true`; its explicit two-person override (blocking-with-override) is exercised at its **entity publish** — a `CatalogVersion` increment, operator- or system-initiated, is **mechanical** over already-governed content and is never itself a new approval gate (§15 decision). It **MUST** emit `CatalogVersionPublished` and expose per-version `freezeComplete`. A published `CatalogVersion` **cannot be withdrawn or rolled back** (roll-forward N+1 only); snapshot boundary is the whole tenant (serialized). **Increment-trigger taxonomy + batching SLO (pricing D-47, ratified — the joint contract with plan-price's `PlanPublished` pending-ref model):** an **interactive** downstream publish request increments immediately (coalescing window <= 5s); **bulk** operations (mass repricing, migrations, bulk enrollments) coalesce into one version with a **5-minute hard max delay**; the delay from a pending publish request to `CatalogVersionPublished` is bounded by **p95 <= 60s / max 5 min**, and the registry stays the **sole** incrementer. A system-initiated increment **MUST NOT** wait on any human approval — it snapshots only content whose governance already happened at entity publish.

**Rationale**: A full, immutable, checksummed snapshot is the reproducibility anchor for posted invoices and contracts.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### Snapshot reproducibility

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-snapshot-reproducibility`

Re-resolving a `catalogVersionId` at any future time **MUST** yield a byte-identical checksum and unchanged **registry** content (manifest §4.4). `CatalogVersion` **MUST** be exposable as **one component** of a downstream `pricingSnapshotRef` without asserting it equals the full snapshot; referenced-module content reproducibility is governed by the freeze protocol.

**Rationale**: Byte-identical re-resolution is required for posting immutability and dispute defensibility.

**Actors**: `cpt-cf-bss-products-actor-billing`

#### Catalog-version diff

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-catalog-version-diff`

The system **MUST** compute a structured diff between any two `catalogVersionId`s of one tenant covering **every member of the snapshot** — entities added/removed, per-entity published-version deltas (reusing the per-version field diff of `fr-revision-vs-version`), **and the captured live content: categories and their localized display values, attribute definitions, recognized sets, per-entity metadata maps** (a metadata-only or category-only change between two versions MUST be visible) — deterministic for a given pair, **read-only** (neither frozen snapshot is touched or re-frozen). It is the reviewer's view for approvals and the operator's view for environment promotion (AC #33a). *(Added, industry parity: catalog-compare tooling — Zuora Deployment Manager class.)*

**Rationale**: Two immutable full snapshots make the diff cheap, and both approval review and promotion need "what changes between versions" as a first-class read.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### Cross-module snapshot freeze atomicity

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-freeze-atomicity`

The system **MUST** expose a `freezeComplete` flag per `catalogVersionId` and **reject resolution for posted/contractual use** until all registered freeze-participants acknowledge, with a bounded timeout that **fails closed**. Read-only browse **MAY** proceed during the freeze window. The resolution API **MUST** require the consumer to **declare intent** (`browse` vs `posted/contractual`) so a consumer cannot post against a not-yet-`freezeComplete` version by mislabeling its call (consumer-side obligation, to be CI-verified in the `fr-plan-price-seam` suite once it exists, §15).

**Rationale**: Cross-module atomicity prevents posting against a partially-frozen snapshot.

**Actors**: `cpt-cf-bss-products-actor-plan-price`

#### Freeze recovery & force-completion

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-freeze-recovery`

For a `CatalogVersion` past the freeze timeout the system **MUST** identify each non-acknowledging participant, support an **idempotent re-trigger** of the fan-out, and support **force-completion** under the two-person rule that records each missing participant as explicitly **not-frozen** and emits `FreezeForceCompleted`. Force-completion **MUST NOT** mark missing content as frozen; the default is **pinned fail-closed** for that participant's content. **The registry enforces that pin on its own door** (P-D-19): a version at `complete(forced)` is **refused for `posted` resolution** (`VERSION_FORCED_INCOMPLETE`, naming each not-frozen participant) until every forced participant has since frozen or released **through its own release door** (the retention-release marker that force-completion itself writes does not count) — P-D-47 (auto-fallback is an off-by-default later enhancement, not a v1 disjunct). Browse resolution is unaffected. Consumer-side refusal of a not-frozen participant's content remains a seam obligation, but it is belt-and-braces: the fail-closed default MUST NOT depend on it.

**Rationale**: A stuck freeze must be recoverable without silently marking content frozen.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### Freeze-participant set governance

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-freeze-participant-governance`

Freeze-participant membership changes **MUST** be governed (two-person), audited, and each `CatalogVersion` **MUST** snapshot the participant set at publish time so a historical version re-resolves `freezeComplete` against its original participants. A participant removed after publish **MUST NOT** retroactively flip that version's `freezeComplete`.

**Rationale**: Historical versions must re-resolve against the participants that existed at publish.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### Grandfathering invariant

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-grandfathering-invariant`

The registry **MUST** guarantee a grandfathered frozen snapshot is **never mutated**; retirement/deprecation affects only new adoption, never existing frozen references. Grandfathering **eligibility policy** is owned by plan-price / subscriptions-lifecycle; this requirement makes the delegation auditable from the registry side.

**Rationale**: Existing terms must persist byte-identically after the underlying SKU is deprecated/retired.

**Actors**: `cpt-cf-bss-products-actor-subscriptions`

#### Uncomposed-bundle adoption guard

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-bundle-adoption-guard`

A `bundle` SKU published with the uncomposed override (exercised at its entity publish, §15 decision) **MUST** carry `compositionPending = true` until plan-price composes it, and consumers **MUST** treat `compositionPending` SKUs as **not-yet-adoptable** for new references. Clearing it **MUST** be driven by a plan-price composition signal (`BundleCompositionCompleted` — **inbound**, plan-price's to produce), audited, and emitted as `SkuCompositionCleared` (**outbound**, this gear's own state-change event; the two directions had shared one name until, which made the registry appear to emit the event it consumes), producing a new published version and never mutating a prior frozen `CatalogVersion`.

**Rationale**: An incomplete bundle must be reproducible-as-pending and blocked from new adoption until composed.

**Actors**: `cpt-cf-bss-products-actor-plan-price`

### 6.7 Approval, Publishing & Eventing

#### Materiality-gated publish

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-materiality-gated-publish`

A **material** change (touching `PlanTier`/metering-unit/`taxCategory`/`glCode`, a lifecycle transition to `published`/`deprecated`/`retired`, a Category create/rename/re-parent/retire/delete, a material attribute-definition change, or exceeding the configured affected-entity count) **MUST** require a **configured approver quorum**: `N` distinct approvers, each distinct from the author and holding CatalogAdmin or FinanceReviewer, where `N` is part of the typed materiality policy with **default 2** and **floor 0** (amended, **P-D-11** — the previous text fixed `N ≥ 2`, which made a two-person tenant unable to publish any material change and a one-person tenant unable to publish at all, while the sibling plan-price gear ships both `N = 1` and an approver-less path for below-threshold non-first publishes). Constraints on that quorum, none of them configurable: a **finance-material** field (`taxCategory`, `glCode`, `PlanTier`) **MUST** include ≥ 1 FinanceReviewer among the approvers — the predicate governs *who*, never *how many*. At `N = 0` the predicate has **no subject**: it **MUST NOT** be imposed on a descriptor no principal can satisfy (that would raise `APPROVAL_REQUIRED` with nothing able to clear it, re-blocking the one-person tenant this floor exists for, since `taxCategory` is required at publish for product/service types), and the record **MUST** instead carry an explicit unsatisfiable-predicate marker so the absent control is a stored fact rather than a silent pass; self-approval **MUST** remain refused at every `N ≥ 1` (a tenant wanting the author to decide alone configures `N = 0`, which the trail records as "no approval required by policy" — an author signing as their own approver is indistinguishable from a bypassed control and is never the mechanism for it); `N` **MUST** be reachable only by explicit configuration, an absent value falling back to the default so `0` is never reached by omission; the **initial** value is set at tenant provisioning while every later change to it is itself material under the *then-current* quorum. An approval **MUST** be **pinned to the internal revision**; any subsequent edit invalidates it and re-queues with the diff re-presented. The materiality rule **MUST** be a typed, configurable policy with an enforceable interim default (§17.1); a rejection returns the entity to `draft` with reason recorded.

**One subject kind carries no human approver (P-D-14).** A publish whose sole content is a
system-owned flag cleared by an inbound governed signal — in v1 exactly the `compositionPending`
clearing — is recorded under subject kind `system_signal`, auto-satisfied with the signal reference as
the authorizing principal, and audited like any other decision. It is a subject kind and **not** an
exemption: the act still produces a record. The tenant's configured `N` has no standing over it,
because its principal is not a tenant principal (slice 05 `inst-gv-one-shot`), so the quorum sentence
above and its `N = 0` clause do not apply to it. **On a dirty head the clear is deferred, never refused** (P-D-14 as confirmed by P-D-48): the
signal is durable and idempotent, the flag stays set, `design/06-catalog-version.md` §3.2 raises no
error code for it by design, and the clear re-evaluates when the head next goes clean.

**Rationale**: Two-person control with separation of duties and revision-pinning prevents unauthorized or bypassed publishes.

**Actors**: `cpt-cf-bss-products-actor-finance-reviewer`

#### Idempotent authoring boundary

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-idempotent-authoring`

For a retried create/update/publish with an idempotency key, the same key + identical payload **MUST NOT** create duplicate entities, versions, or events; the key **MUST** be scoped per tenant + endpoint + client key and retained ≥ 24h **and never less than the maximum freeze timeout**. Reuse with a **different** payload **MUST** be rejected as a conflict (no silent no-op).

**Rationale**: Idempotency prevents duplicate publishes on retry, including after the freeze window.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### Registry eventing & audit

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-registry-eventing-audit`

Every state-changing mutation **MUST** publish the corresponding event onto the platform event-broker in its **broker-native envelope** (event-broker ADR-0003), stamping correlation/causation + idempotency key + per-aggregate ordering keys `(tenant, aggregate)`; delivery/ordering/durability are owned by the event-broker. **Envelope decision (§15):** the registry adopts the broker-native schema — **not** CloudEvents 1.0; the manifest §7.2 CloudEvents mandate is re-scoped for this gear (manifest amendment owed). The semantic obligations of this section are envelope-agnostic and bind unchanged: versioned schemas with `vN`→`vN+1` compatibility, correlation/causation, ordering keys, pseudonymous actors. Every state-changing requirement **MUST** map to exactly one named event (or an explicit "no event" decision in Design). Event payloads **MUST** carry **pseudonymous actor references only** (never direct operator PII). The mutation **MUST** be recorded in an immutable, queryable audit trail. Plan/Price/Bundle-composition events **MUST NOT** be emitted here (owned by plan-price).

**Rationale**: Complete, pseudonymous, ordered eventing + immutable audit is what makes erasure (AC #35) and downstream consumption work.

**Actors**: `cpt-cf-bss-products-actor-events-audit`

#### Event schema versioning & replay

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-event-versioning-replay`

Every event **MUST** carry a versioned (semver) schema reference — the broker-native equivalent of CloudEvents `dataschema` (§15 envelope decision); a consumer pinned to `vN` **MUST** deserialize `vN+1` (new fields optional with defaults); out-of-order/duplicate delivery beyond the idempotency window **MUST** be detectable via `(tenant, aggregate, sequence)`. The system **MUST** provide a **bootstrap path** (latest `CatalogVersion` + event tail) for published-scope consumers, and **MUST** detect when a consumer checkpoint predates the available event tail and **fail loudly**.

**Rationale**: Forward-compatible schemas + a bootstrap path let consumers evolve and recover without full historical replay.

**Actors**: `cpt-cf-bss-products-actor-events-audit`

### 6.8 Multi-Tenancy & Read Models

#### Tenant/brand/region isolation & break-glass

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-tenant-isolation-breakglass`

Cross-scope query/mutation **MUST** be denied by default at the API gateway and the attempt audited. Privileged platform-owner cross-tenant access **MUST** use **break-glass** elevation that is time-boxed, reason-required, separately alertable, and itself two-person-approved or post-hoc-reviewed; standing cross-tenant access **MUST NOT** be granted.

**Rationale**: Cross-tenant catalog leakage is a critical commercial/competitive incident class.

**Actors**: `cpt-cf-bss-products-actor-platform-owner`

#### Break-glass action scope

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-breakglass-action-scope`

Break-glass **MUST** permit **read and audit-export only**; any write/publish under break-glass **MUST** be separately gated (two-person + distinct alert) or disallowed in v1. Every break-glass action **MUST** be individually audited with the elevation reason and correlation ID.

**Rationale**: Elevation must not silently grant write authority in a foreign tenant.

**Actors**: `cpt-cf-bss-products-actor-platform-owner`

#### Cache-first browse/search with bounded convergence

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-cache-first-browse`

Browse/search/filter **MUST** be served from cache-first read models scoped to the caller's tenant/brand/region, converging within its own budget (interim p99 < 2 s after write commit). Stale reads during the window **MUST** be safe (never expose unpublished or cross-scope content) and **MUST** carry the `asOfCatalogVersion` staleness signal. The per-state visibility contract **MUST** hold: `published` browsable; `deprecated` browsable with a machine-readable flag and excludable by filter; `retired` excluded from default browse and retrievable only via explicit history query.

**Rationale** *(re-derived — the previous wording rested on the read NFR numbers, and those are uncalibrated design targets the NFR workshop owns; see §15)*: **two properties a separate serving store gives by construction and query tuning cannot.** (1) **Availability split** — the read path's health is independent of the write path's (read 99.9% vs write 99.5%, NFR #10: "reads must not block downstream when writes degrade"), which matters because the browse consumers are revenue-facing: portals and Marketplace vendor listings show a customer what they may buy, and a write-path outage must not take the storefront with it. The probe is a read served during a simulated write-path outage. (2) **Structural stale-but-safe** — the projection is built only from frozen published-version rows, never from head rows, so an unpublished draft edit cannot reach a browse response at all; without a serving store the same guarantee (AC #32, NFR #7's zero-leakage threshold) becomes a per-call-site join discipline. The latency/throughput targets ride on top of these and do not, by themselves, establish the requirement.

**Actors**: `cpt-cf-bss-products-actor-presentation`

### 6.9 Bulk Operations

#### Bulk import/export

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-bulk-import-export`

Bulk import/export **MUST** apply **per-row idempotency**, report per-row success/failure (no hidden partial failure), and never leave a partially-inconsistent published state. Dependent rows **MUST** apply two-phase (stage-all-then-commit) or dependency-ordered, never committing an orphan. Idempotency operates at two levels (batch key + per-row keys). A bulk operation **MUST** emit a coalesced `CatalogBulkOperationCompleted` (no event storm). **Bulk import lands entities in `draft`**; publication remains gated, approved against an **aggregated change report** (counts, per-type summary, sample, lint findings). Export **MUST** be deterministic for a given `catalogVersionId`. **Environment promotion (, industry parity — Stripe test/live, Zuora Deployment Manager):** a deterministic export from one environment **MUST** be importable into another with identity carried by stable codes: `skuCode` for SKUs, and for Products `productCode` when present, else `(brandId, canonical internal name)` — total by construction, since P-D-04 makes the name absolutely unique per tenant+brand (system ids are re-minted by the target); imported rows land in `draft` and pass the same gated publish — promotion IS bulk import, never a governance bypass. An identity collision is classified **exhaustively** (P-D-17): unknown identity ⇒ **create**; identity bound to **matching** content ⇒ **no-op**; identity bound to **different** content ⇒ **update-as-draft** against the existing entity; identity bound to an **incompatible kind/type, a `retired` holder, or a dirty head** ⇒ **per-row conflict**. **Never a silent merge**: the update lands in `draft`, publication stays gated, and the change report shows the row — and a target holding unpublished edits conflicts, so promotion can never clobber work in progress.

**Rationale**: Onboarding/migration at ≥ 10K-SKU scale cannot be row-by-row and must stay consistent and governed.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

### 6.10 Cloning

#### Clone a product/SKU

- [ ] `p3` - **ID**: `cpt-cf-bss-products-fr-clone`

Cloning a source Product/SKU (draft, published — **`deprecated` included, it being a governed sub-state of published and the state an entity occupies for the whole retirement lead window** — or **retired**, the sanctioned revival path) **MUST** create a new `draft` with new `productId`/`skuId` and a new `skuCode`/optional `productCode` (system-suggested, operator-overridable, atomically reserved), copying structure/attributes/scoping/category/`PlanTier`/metering-unit while resetting lifecycle and version counters and **never copying** pricing/plan content. The cloned metering unit, `PlanTier`, and category assignment **MUST** be re-validated against live registries; the clone **MUST** fail or force re-selection if any was de-listed/deprecated/retired. It **MUST** record a `clonedFrom` reference and **MUST NOT** affect the source.

**Rationale**: Cloning accelerates catalog expansion and is the safe revival path for retired items, provided re-validation runs.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

### 6.11 Data Retention & Erasure

#### Retention & right-to-erasure

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-retention-erasure`

The system **MUST** retain financial/version/audit records for the configured retention duration and satisfy erasure of **actor PII** by **pseudonymizing** it across audit, entity version fields, and the actor identity-reference map — never deleting immutable financial/version records; because events carry only pseudonymous actor references, updating the reference map completes erasure without touching immutable event streams. Attribute/description free-text **MUST NOT** contain personal data — enforced by a **validation block at write** (hard prohibition, no erasure carve-out, fail-closed on uncertainty, curated allow-list for legitimate person-named products). Erasure **MUST NOT** break `CatalogVersion` reproducibility or audit completeness.

**Rationale**: Content-erasure is logically incompatible with byte-identical reproducibility, so PII is kept out at write and actor PII is pseudonymized.

**Actors**: `cpt-cf-bss-products-actor-auditor`

### 6.12 Cross-PRD Consistency

#### Registry ↔ plan-price seam

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-plan-price-seam`

There **MUST** be a shared schema-version pin and a CI contract test that fails when registry and plan-price diverge on a pinned surface; a runtime divergence **MUST** fail closed (reject the dependent plan publish). **The pin's membership is a rule, not a list** (amended, **P-D-12**): it covers exactly the **operands the consumer obligations below are enforced on**, so adding an obligation adds its operand, and the coupling is itself doc-linted. Derived v1 membership: `skuId`, `type` (the `bundle` discriminator), the metering-unit declaration **and** `usageTypeRef`, `PlanTier`, `status` **together with its value vocabulary** (a renamed or added blocking state must break the build, not silently stop matching a consumer's adoption guard), `sellable`, `compositionPending`. Explicitly **outside** the pin, with the reason recorded so its absence is not read as an oversight: `skuCode` and `name`, read only by a pick-list that validates nothing, where drift is cosmetic. `CatalogVersion` is pinned as a **surface** rather than a field. *(The previous list named five items of which only three were comparable fields of the consumer's shape — `bundle` type was absent from it and `CatalogVersion` is not a SKU field — while `status`, `sellable`, `compositionPending` and `usageTypeRef` were operands of obligations this very requirement states and pinned nothing.)* The same suite **MUST** assert consumer-side lifecycle obligations: reject adoption of `compositionPending`/`deprecated` SKUs; reject a usage binding when the target SKU has no declared unit (and reject/warn when its unit is `deprecated`) — **this is where usage-completeness is enforced**; consume `mustMigrateBy` (post-v1); resolve grandfathered refs against the frozen snapshot; re-validate on `SkuImmutableFieldCorrected`; and declare intent before `freezeComplete` on posted/contractual resolution. Assertions are authorable only once the referenced counterpart AC exists.

**Rationale**: A CI-verified seam turns delegated boundaries into enforced contracts rather than assumptions.

**Actors**: `cpt-cf-bss-products-actor-plan-price`

#### Monetization-model traceability

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-monetization-traceability`

The PRD **MUST** expose a traceability map (§17.2) so the registry's deliberate lack of a monetization-model marker does not read as an unmet requirement: flat/per-seat/tiered/volume/hybrid/commitment → authored/evaluated in plan-price + Tariffs; usage → metering-unit declaration here + binding/rating downstream. Absence of a model marker on a SKU **MUST** be treated as intentional, not a missing field.

**Rationale**: Explicit traceability prevents the boundary from being mistaken for a gap.

**Actors**: `cpt-cf-bss-products-actor-finance-reviewer`

### 6.13 Operational Resilience & Concurrency

#### Expected failure behavior

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-expected-failure-behavior`

Invalid/conflicting authoring **MUST** fail closed with an audited reason and **MUST NOT** partially apply, for each of: stale-revision write, duplicate idempotency key with different body, taxonomy cycle, unrecognized metering unit without elevation, publish of an incomplete entity, immutable-field change without a valid correction path, reissue of a reserved `skuCode` and concurrent `skuCode` collision, EOL retirement without an acknowledged migration consumer (post-v1), publishing a SKU under a non-`published` parent, a SKU scope falling outside its parent, authoring/cloning against a **de-listed** unit, authoring/cloning against a **deprecated** unit, a bulk row whose in-batch dependency failed, adopting a `compositionPending` bundle, and a retention process that would orphan a live grandfathered reference.

**Rationale**: A single enumerated fail-closed contract keeps negative paths deterministic and auditable.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### Event delivery resilience

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-event-delivery-resilience`

The **shared event system** **MUST** provide bounded-backoff retry, per-consumer delivery state, and an **audited dead-letter** path with alerting (transport owned there). The **registry's own** obligations are limited to: not reporting emission success until the event is **durably accepted**, **surfacing** the per-consumer delivery/dead-letter state as a projection, and never mutating registry state on a delivery failure. During a bus outage, mutations **MAY** commit with events to a durable **outbox** for later emission; the propagation clock starts at durable bus acceptance, not at commit.

**Rationale**: Resilience mechanics belong to the bus; the registry must not falsely report propagation or lose events.

**Actors**: `cpt-cf-bss-products-actor-events-audit`

#### CatalogVersion publish concurrency

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-catalog-publish-concurrency`

Concurrent publishes within a tenant **MUST** serialize; `catalogVersionId` **MUST** be allocated monotonically without gaps or collisions; a staged entity whose published version **or lifecycle state** moved between stage and commit **MUST** cause that run to **re-validate fail-closed** rather than freeze stale or partial content. Fail-closed is delivered **per lane** (amended, P-D-09): the **operator lane** rejects, naming the changed entity; the **mechanical lane** — D-47 demand-driven increments, which have no operator to reject to — re-collects fresh content and retries within its lane SLO, and **MUST NOT** lose the request. In neither lane may stale content be frozen.

**Rationale**: Per-tenant serialization + re-validation guarantees no published version contains concurrently-invalidated content. Rejection is the operator-facing *delivery* of that guarantee, not the guarantee itself; a retry with fresh collection delivers the same invariant to a caller that is a machine, and additionally delivers the request.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`, `cpt-cf-bss-products-actor-plan-price` *(the second added with P-D-09: the requirement named only the operator, which is the historical reason its remedy assumed one)*

#### Fail-safe duration tripwire

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-failsafe-tripwire`

While operating in `SkuReferenceCount`-unavailable fail-safe mode, when break-glass immutable-field corrections exceed a configured rate (interim > 5 in 30 days) the system **MUST** raise an escalation alert and **reclassify `SkuReferenceCount` delivery as a release blocker**, so unbounded degraded operation is detected and escalated, not normalized.

**Rationale**: The tripwire bounds the fail-safe operational debt in time, not merely acknowledging it.

**Actors**: `cpt-cf-bss-products-actor-catalog-admin`

#### `skuCode` reservation concurrency

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-skucode-reservation-concurrency`

For two concurrent reserve requests for the same `skuCode` within a tenant, the system **MUST** atomically reserve at **create** time, admit exactly one, and reject the other fail-closed with an audited reason; a `draft` reservation **MUST** block a second draft until released or discarded. A `skuCode` changed while still `draft` **MUST** release the previous code; **discarding a never-published draft MUST also release** its `skuCode`/`productCode` reservation (permanent reservation applies only from first publish).

**Rationale**: Atomic reservation prevents duplicate codes while letting abandoned drafts free the namespace.

**Actors**: `cpt-cf-bss-products-actor-product-manager`

#### Reference-producer registration

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-reference-producer-registration`

Only **registered** producers' signals or silence **MUST** factor into the `referenced` predicate; an unregistered producer's absence **MUST NOT** pin every SKU immutable. **The v1 registered producer set = {plan-price (pricing gear)}** (§15 decision — **P-D-03**; delivery joint with this gear's build); Subscriptions and Contracts register at their own build time, their GA gated on producing the signal. Producer-set membership **MUST** be a governed, audited change **snapshotted symmetrically with the freeze-participant set**, and onboarding a new producer **MUST NOT** retroactively flip historical mutability/retirement decisions.

**Rationale**: Registration prevents a not-yet-onboarded producer from freezing the whole catalog and keeps history stable.

**Actors**: `cpt-cf-bss-products-actor-subscriptions`

#### Grandfathered-snapshot retention coupling

- [ ] `p1` - **ID**: `cpt-cf-bss-products-fr-grandfathered-retention-coupling`

For a `catalogVersionId` referenced by ≥ 1 **live** grandfathered reference, the snapshot **MUST** remain byte-identically resolvable for as long as a live reference exists, **regardless of the statutory-max retention clock**; retention expiry **MUST** be gated on no live references to that `catalogVersionId`. Because per-SKU `SkuReferenceCount` carries no version dimension, version-liveness **MUST** be sourced from per-version freeze-registration records — **acked-and-not-yet-released** — never from the SKU-level count alone and never from a `(catalogVersionId, producer)` producer contract, which **P-D-18 closed** on (a version-scoped producer count would be a second signal with its own freshness and its own producer set). A retention process that would orphan a live reference **MUST** fail closed with an alert.

**Rationale**: Silently GC'ing a snapshot under a live contract breaks reproducibility for every reference frozen to it — a compliance event.

**Actors**: `cpt-cf-bss-products-actor-billing`

#### Pre-publish lint report

- [ ] `p2` - **ID**: `cpt-cf-bss-products-fr-prepublish-lint`

The `validate(lint)` operation before `CatalogVersion` publish **MUST** return a **structured, per-entity report** of every attention condition (uncomposed `bundle` SKUs, missing default-locale attribute values, declarations against a `deprecated` unit) so an operator-initiated publish is an **informed** act and the audit records **what was outstanding**; the uncomposed-bundle two-person override itself is exercised at the bundle's **entity publish** (§15 decision), where the same lint findings **MUST** be presented to the approvers.

**Rationale**: An informed override beats a blind acknowledgment and produces a meaningful audit trail.

**Actors**: `cpt-cf-bss-products-actor-finance-reviewer`

## 7. Non-Functional Requirements

### 7.1 NFR Inclusions

> Numeric targets are binding **design targets** until the program NFR workshop (scheduled within 2 weeks of PRD approval; named DRI = BSS Program Lead). Interim configurable-policy defaults are in §17.1.

#### Read latency

- [ ] `p1` - **ID**: `cpt-cf-bss-products-nfr-read-latency`

Browse/search reads within a tenant partition **MUST** meet **p95 < 100 ms** over a 5-minute window on a warm read model holding 10K SKUs/tenant with ≥ 100 concurrent readers, sustained via cache-first read models and tenant/brand/region partitioning.

**Threshold**: p95 < 100 ms @ 10K SKUs/tenant, ≥ 100 concurrent readers.

**Rationale**: Slow catalog reads degrade portal/sales UX.

#### Read throughput

- [ ] `p1` - **ID**: `cpt-cf-bss-products-nfr-read-throughput`

The cache-first read model **MUST** sustain **≥ 2,000 read QPS per tenant partition** at the read-latency target.

**Threshold**: ≥ 2,000 read QPS/tenant partition at p95 < 100 ms.

**Rationale**: Peak browse/search traffic must not breach latency.

#### Publication propagation

- [ ] `p2` - **ID**: `cpt-cf-bss-products-nfr-publication-propagation`

Downstream event availability (incl. fan-out) after an approved publish **MUST** occur within **< 3 s** — a component preceding freeze acks, distinct from read-model convergence (< 2 s) and end-to-end posting-safe (< 5 s).

**Threshold**: event availability < 3 s after publish (p99).

**Rationale**: Delayed publication yields stale offerings downstream; the three nested budgets must not collapse to one.

#### End-to-end posting-safe budget

- [ ] `p1` - **ID**: `cpt-cf-bss-products-nfr-posting-safe-budget`

From write commit to "posting-safe" (read-model converged **and** all participants' `freezeComplete` acknowledged) **MUST** be **p99 < 5 s**; if the freeze times out the version **MUST** remain non-posting-safe (fail closed). This composite is a **program-level SLO** decomposed into a registry-owned `commit → event-durably-published` budget and per-participant `event → ack` budgets.

**Threshold**: p99 < 5 s commit → posting-safe (fail-closed on freeze timeout).

**Rationale**: Downstream needs a single SLA to design against; freeze acks follow fan-out.

#### Snapshot archival & cold-resolution SLA

- [ ] `p1` - **ID**: `cpt-cf-bss-products-nfr-snapshot-archival-dr`

Cold `catalogVersionId` re-resolution **MUST** remain byte-identical and meet a looser-than-hot target (interim p95 < 2 s). `CatalogVersion` snapshots + version history are **financial records** with a durability class (interim **≥ 11 nines** / replicated storage), backup/restore with **periodic checksum restore verification**, and a cross-region/DR posture with RPO/RTO (set at the NFR workshop). Availability SLOs do **not** substitute for durability.

**Threshold**: cold p95 < 2 s; durability ≥ 11 nines; periodic restore verification; RPO/RTO TBD (workshop).

**Rationale**: Silently losing one snapshot breaks reproducibility for every contract frozen to it — a compliance event.

#### Scale & extensibility limits

- [ ] `p1` - **ID**: `cpt-cf-bss-products-nfr-scale-extensibility`

The system **MUST** support **≥ 10K SKUs per tenant** without breaching read latency, within configured limits (max attributes/entity, max taxonomy depth, max children/node). The scale model **MUST** also bound tenant count, total cardinality, and **`CatalogVersion` growth** (full-snapshot-per-publish is the dominant cost driver) with a publishes/day/tenant target set at the workshop.

**Threshold**: ≥ 10K SKUs/tenant; extensibility limits + publish-frequency target per workshop.

**Rationale**: Full-snapshot economics and extensibility limits bound the design.

#### Graceful degradation & staleness exposure

- [ ] `p2` - **ID**: `cpt-cf-bss-products-nfr-graceful-degradation`

Above the throughput ceiling or read-model lag, the system **MUST** shed or queue excess load **without ever serving cross-scope or unpublished content**, and **MUST** expose staleness via the **same `asOfCatalogVersion` mechanism** (one signal, machine-readable) — no silently-stale degraded response.

**Threshold**: zero cross-scope/unpublished leakage under overload; machine-readable `asOfCatalogVersion` on every stale response.

**Rationale**: Overload must never compromise isolation or hide staleness.

#### Determinism & integrity

- [ ] `p1` - **ID**: `cpt-cf-bss-products-nfr-determinism-integrity`

Version immutability, taxonomy acyclicity, SKU identity uniqueness, and metering-unit validity **MUST** be enforced fail-closed, and posted-period `CatalogVersion` snapshots **MUST** remain immutable.

**Threshold**: 100% fail-closed enforcement of the registry invariants.

**Rationale**: The registry is the integrity foundation all monetization binds to.

#### Backward-compatible schema evolution

- [ ] `p1` - **ID**: `cpt-cf-bss-products-nfr-backward-compatible-evolution`

A consumer pinned to schema `vN` **MUST** successfully deserialize a `vN+1` payload (new fields optional with defined defaults); a CI contract test **MUST** assert backward compatibility on every schema change.

**Threshold**: 100% `vN`→`vN+1` deserialization; CI-guarded on every schema change.

**Rationale**: New product categories/fields must not break published content or downstream contracts.

#### Availability & audit completeness

- [ ] `p1` - **ID**: `cpt-cf-bss-products-nfr-availability-audit`

The cache-first **read** path **MUST** meet **99.9%** availability and the **write/publish** path **99.5%** (reads must not block downstream when writes degrade); write paths **MUST** be fully audited even during partial failures.

**Threshold**: read 99.9% / write 99.5% availability; 100% write-path audit.

**Rationale**: Reads feeding portals/sales must stay up independently of write degradation.

### 7.2 NFR Exclusions

- **Pricing/rating performance** — owned by plan-price / Tariffs / Rating; the registry only serves published primitives.
- **Usage collection/normalization throughput** — OSS metering / Usage Collector.
- **Event-bus transport SLOs (delivery latency, DLQ retention)** — owned by the common event system (Common Core); the registry states only its own emission/projection obligations.
- **Storefront UX performance / accessibility (WCAG) / i18n rendering** — Presentation layer / frontend DESIGN.
- **Marketplace listing/search performance** — Marketplace PRD (§4.8).

## 8. Five Quality Vectors Analysis

| **Quality Vector** | **Show-Stopper Requirements** | **Rationale** |
|--------------------|-------------------------------|---------------|
| **Efficiency** | Product/SKU/category/attribute changes MUST be operator self-service with automated versioning and approvals — no engineering involvement for routine registry change. | Manual catalog management blocks catalog growth and time-to-market. |
| **Reliability** | Immutable Product/SKU versioning, byte-identical `CatalogVersion` snapshots, 100% audited write paths, fail-closed publish validation, and a CI contract test guarding the registry↔plan-price seam. | The registry is the foundation all monetization binds to; silent drift or lost history breaks downstream snapshots and compliance. |
| **Performance** | Browse/search p95 < 100 ms and ≥ 2,000 read QPS/tenant partition via cache-first read models; nested propagation budgets (convergence < 2 s, propagation < 3 s, posting-safe < 5 s, cold-version < 2 s); graceful degradation. | Slow catalog reads degrade portal/sales UX; delayed publication yields stale offerings. |
| **Security** | Complete tenant/brand/region isolation (deny-by-default), RBAC, the configured approver quorum for material changes (default 2, floor 0 — its predicates fixed), time-boxed break-glass for cross-tenant access, retention/erasure reconciled with immutable audit, minimal PII in events. | Cross-tenant leakage is a commercial risk; unauthorized publish is a fraud risk; standing super-access and unbounded retention are compliance risks. |
| **Versatility** | Extensible attributes/taxonomy and a type-agnostic SKU model (product/service/bundle) with backward-compatible schema evolution (`vN` deserializes `vN+1`). | New product categories must be added without breaking published content or downstream contracts. |

## 9. Public Library Interfaces

> The registry is a backend service, not a client library. Interfaces below are high-level contracts; concrete API schemas, endpoints, event payloads, and DDL belong in DESIGN.

### 9.1 Public API Surface

#### Catalog authoring & publish

- [ ] `p1` - **ID**: `cpt-cf-bss-products-interface-authoring-publish`

**Type**: command/authoring + approval-gated publish API (shape in Design)

**Stability**: stable (contract intent), schema unstable (Design owns)

**Description**: Create/update Products, SKUs, categories, attributes, `PlanTier`; declare metering units and accounting codes; lifecycle transitions; quorum-gated publish of **entities** — governance attaches to the entity publish, while a `CatalogVersion` increment is **mechanical and never an approval gate** (P-D-02; the operator lane of a stage-vs-commit conflict rejects, the mechanical lane retries, P-D-09) — plus the S2S/SDK increment-request contract (§9.2); idempotent by key. Requires the resolution caller to **declare intent** (`browse` vs `posted/contractual`).

**Breaking Change Policy**: Major version bump; idempotency-key and intent-declaration semantics are part of the contract.

#### Catalog read model (browse/search)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-interface-read-model`

**Type**: cache-first query/read API (shape in Design)

**Stability**: stable (contract intent)

**Description**: Tenant/brand/region-scoped browse/search/filter of published Products/SKUs/Categories with the per-state visibility contract and an `asOfCatalogVersion` staleness signal; version-history retrieval.

**Breaking Change Policy**: Major version bump for incompatible query/response changes.

### 9.2 External Integration Contracts

#### CatalogVersionPublished + registry events (outbound)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-registry-events`

**Direction**: provided by the registry to the shared event system

**Protocol/Format**: broker-native envelope (event-broker ADR-0003; §15 decision) with versioned schema refs (semver), correlation/causation, per-aggregate ordering keys, pseudonymous actor refs; includes `CatalogVersionPublished` and the full Product/SKU/Category/Attribute/governance event set (Design owns names/schemas).

**Compatibility**: `vN` consumer deserializes `vN+1`; bootstrap path (latest `CatalogVersion` + tail); no direct PII.

#### `SkuReferenceCount` signal (inbound)

*All four inbound machine contracts below are consumed as **`products-sdk` clients resolved from
`ClientHub`**, in-process, with their REST/S2S doors as the out-of-process binding — **P-D-15**,
stated once here over the set rather than repeated per block, since only two blocks carried it
until while the decision speaks for all of them.*

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-sku-reference-count`

**Direction**: required from the registered producer set — **v1 = plan-price (pricing gear)** (§15 decision — **P-D-03**); Subscriptions and Contracts join at their own build

**Protocol/Format**: per-producer watermark ("as of `T`, complete live-reference set is {…}"); freshness on the watermark; registered producers only (Design owns shape).

**Compatibility**: absence under a fresh watermark ⇒ zero; boolean OR across producers; stale/never-received ⇒ conservatively referenced + alert. Producer-set decision recorded in §15, mirrored in the pricing PRD.

#### `CatalogVersion` increment request (inbound)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-increment-request`

**Direction**: required from any consumer needing a resolvable catalog coordinate, consumed as a `products-sdk` client resolved from `ClientHub` rather than an out-of-process REST door (**P-D-15**) — **v1 = plan-price (pricing gear)**, whose pending `pricingSnapshotRef` needs a `catalogVersion` segment (joint contract D-47); operator publishes enter the same queue on the interactive lane. *(Added: this contract triggers the whole `CatalogVersion` machine and its absence stalls pricing's pending refs, yet §9 named only its sibling `SkuReferenceCount` — the two inbound machine contracts from the same counterparty had been documented asymmetrically.)*

**Protocol/Format**: a **typed SDK client** (`products-sdk`), not a transport — per manifest §3.3.2/§3.4.1/§9.1 gear contracts are transport-agnostic, the consumer resolves the interface from `ClientHub`, in-process composition is the default mode, and `runtime.type: local | oop` switches bindings without code changes; the REST/S2S endpoint is the out-of-process binding of the same contract. Payload `{source, lane ∈ {interactive, bulk}, request_key, operation_key?}`; idempotent per `(source, request_key)`; a keyed bulk operation coalesces into exactly one version (Design owns shape).

**Compatibility**: request acceptance is decoupled from publication — the caller gets an accepted request, not a version, and observes completion via `CatalogVersionPublished`. Lane SLO p95 ≤ 60 s requested→published (max 5 min, D-47); a request past its lane deadline raises `catalog_version_overdue`, the registry-side mirror of pricing's `commit_overdue`. **A request is never dropped**: stage-vs-commit re-validation retries it fresh rather than rejecting it (AC #40, mechanical lane). The client's error taxonomy MUST separate "registry not wired" from "unreachable" from "unusable answer" — an in-process binding cannot produce the middle one, and collapsing the three is what makes a degraded platform look like an empty catalog.

#### Freeze acknowledgment (inbound)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-freeze-ack`

**Direction**: required from the registered freeze-participant set — **v1 = plan-price (pricing gear)** (P-D-48, the P-D-03 pattern; the ack and release clients are built jointly with this gear); Contracts and Billing register at their own build time

**Protocol/Format**: per-`catalogVersionId` freeze acknowledgment feeding `freezeComplete`, **and its release half** — a participant that holds no more live references to a `CatalogVersion` records that through `catalog_version × release` (S2S, the participant's own identity, idempotent). Version-liveness is **acked-and-not-yet-released** (P-D-18; Design owns both shapes).

**Compatibility**: bounded timeout fails closed; participant set snapshotted per `catalogVersionId`; force-completion records missing participants as not-frozen. **The release is a duty, not a courtesy**: snapshot GC is gated on every freeze registration satisfying the pair — `state = released`, or `not_frozen(forced)` with `released_at` stamped by force-completion (second arm added: a forced participant never acked and cannot use the S2S release door, and a later recovery moves `state`, so a stale stamp frees nothing), so a participant that never releases pins that version's storage indefinitely.

#### Bundle composition-completed signal (inbound)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-contract-bundle-composition-signal`

**Direction**: required from plan-price, consumed as a `products-sdk` client resolved from `ClientHub` rather than an out-of-process REST door (**P-D-15**)

**Protocol/Format**: signal that a `bundle` SKU has been composed, clearing `compositionPending` — the clearing publish runs as a `system_signal` approval subject (**P-D-14**), never an exemption from the gate — on a **dirty** head the clear is deferred, never refused (P-D-48) (Design owns shape).

**Compatibility**: clearing produces a new published version and emits `SkuCompositionCleared` (this gear's outbound event; the inbound plan-price signal that drives it keeps the name `BundleCompositionCompleted`); MUST NOT mutate a prior frozen `CatalogVersion`.

## 10. Use Cases

#### Author products and SKUs

- [ ] `p1` - **ID**: `cpt-cf-bss-products-usecase-product-sku-editor`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Preconditions**:
- Established tenant/brand/region context; CatalogAdmin or ProductManager role.

**Main Flow**:
1. Create/select a Product (name, category, description, brand/region scope); `productId`/`skuId` system-generated, `skuCode` operator-entered with inline format check.
2. Add a SKU; pick type product/service/bundle.
3. For a usage SKU, declare the metering unit (validated); set `PlanTier` from the taxonomy.
4. Set tax/GL codes; save as draft (each save bumps the internal revision).

**Postconditions**:
- A draft Product/SKU exists with reserved codes and an audit entry, pending gated publish.

**Alternative Flows**:
- **Incomplete at publish**: publish rejected fail-closed (missing required fields / `PlanTier` / accounting codes).

#### Approve and publish registry changes

- [ ] `p1` - **ID**: `cpt-cf-bss-products-usecase-approval-publish`

**Actor**: `cpt-cf-bss-products-actor-finance-reviewer`

**Preconditions**:
- Pending material change(s) in the approval queue.

**Main Flow**:
1. Open the pending-approval queue; review the diff and, for a bundle carrying the uncomposed-publish override, that entity publish's lint findings.
2. Approve/reject with reason; the configured approver quorum applies above threshold (≥ 1 FinanceReviewer for finance-material fields).
3. On full sign-off the **entity** publish proceeds and emits its events. The subsequent `CatalogVersion` increment is **mechanical** over already-governed content and is never itself an approval gate (P-D-02); the `validate(lint)` report before a `CatalogVersion` publish is **informational** and is read outside this queue.

**Postconditions**:
- Content published (or returned to draft with reason); approval pinned to the approved revision.

**Alternative Flows**:
- **Edit after approval**: approval invalidated, change re-queued with diff re-presented.

#### Deprecate and retire safely

- [ ] `p1` - **ID**: `cpt-cf-bss-products-usecase-lifecycle-deprecation`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Preconditions**:
- A `published` SKU with (possibly) active references.

**Main Flow**:
1. Mark `deprecated` (blocks new adoption; existing continue) or un-deprecate (two-person).
2. Initiate retirement; the system shows the active-reference count and requires confirmation. (EOL with `mustMigrateBy` is post-v1 and refused `EOL_DISABLED` — `fr-retirement-eol`, slice 04 C3.)
3. Confirm; snapshots preserved; retirement event emitted with lead-time and optional `replacedBy`.

**Postconditions**:
- Scheduled transition set; existing references grandfathered on frozen snapshots.

**Alternative Flows**:
- **Cascade with EOL-requiring children**: those children listed and left un-retired; parent stays non-`retired` with deferred-retire intent tracked.

#### Browse, search, and inspect history

- [ ] `p2` - **ID**: `cpt-cf-bss-products-usecase-catalog-browser-history`

**Actor**: `cpt-cf-bss-products-actor-auditor`

**Preconditions**:
- Published catalog content and version history exist for the scope.

**Main Flow**:
1. Filter/search (category, status, brand/region) on the cache-first read model.
2. Open an item; view attributes/classification.
3. Open the version timeline with diffs and audit entries (actor, time, correlation ID).

**Postconditions**:
- Offerings found; change lineage traced (tenant-scoped).

**Alternative Flows**:
- **Cross-tenant inspection**: requires time-boxed break-glass (read/audit-export only), individually audited.

#### Bulk import/export at scale

- [ ] `p1` - **ID**: `cpt-cf-bss-products-usecase-bulk-operations`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Preconditions**:
- A CSV/JSON batch of Products/SKUs/Categories.

**Main Flow**:
1. Upload/import; rows land as **draft** with per-row validation and per-row idempotency.
2. Review the aggregated change report (counts, per-type summary, sample, lint findings).
3. Submit the batch for gated approval (two-person on the batch); track per-row success/failure.

**Postconditions**:
- Rows applied consistently (no orphan/partial publish); a coalesced `CatalogBulkOperationCompleted` emitted.

**Alternative Flows**:
- **Dependent-row failure**: dependent rows fail with a distinct per-row error; no orphan committed.

#### Promote a catalog between environments

- [ ] `p2` - **ID**: `cpt-cf-bss-products-usecase-environment-promotion`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Preconditions**:
- A source environment holding a published `CatalogVersion` to promote; a target environment (e.g. staging → prod).

**Main Flow**:
1. Export the deterministic snapshot at the source `catalogVersionId`.
2. Import into the target: identity carries via stable codes (`skuCode`; `productCode` else `(brandId, canonical internal name)` for Products), system ids re-minted, rows land as `draft` with per-row idempotency.
3. Review the aggregated change report — staged content against the target's current heads. (This, not the catalog-version diff, is the pre-approval view: before step 4 the target has published nothing new, so AC #20a's version-to-version diff has no second operand yet.)
4. Submit for gated approval (two-person on the batch); publish.
5. Verify with the catalog-version diff, previous target version vs the new one (AC #20a).

**Postconditions**:
- Target catalog matches the promoted content through the same governance as any authoring; nothing bypassed.

**Alternative Flows**:
- **Identity collision**: a `skuCode`/`productCode` already bound in the target is resolved by the P-D-17 classification — matching content is a no-op, **different content lands as an update against the existing entity in `draft`** (the modal promotion row), and an incompatible kind/type, a `retired` holder or a dirty head fails per-row as a conflict. Never silently merged: nothing publishes without the batch's own gated approval.

#### Inspect and recover a stuck freeze

- [ ] `p2` - **ID**: `cpt-cf-bss-products-usecase-freeze-monitoring`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Preconditions**:
- A `CatalogVersion` whose `freezeComplete` has not been reached past the timeout.

**Main Flow**:
1. View per-`catalogVersionId` `freezeComplete` status and each non-acknowledging participant.
2. Idempotently re-trigger the freeze fan-out.
3. Force-complete under the two-person rule (records missing participants as not-frozen; emits `FreezeForceCompleted`).

**Postconditions**:
- Freeze resolved or force-completed with missing participants pinned fail-closed for posted use.

## 11. User Interaction and Design

| **Interface Name** | **Role** | **Steps** | **Mockup Screen** |
|--------------------|----------|-----------|-------------------|
| Product & SKU editor | As a Product Manager, I define products and SKUs so the catalog foundation is accurate | 1. Create/select Product (name, category, description, scope); `productId`/`skuId` system-generated, `skuCode` operator-entered with format check<br>2. Add SKU; pick type<br>3. For usage SKU declare metering unit; set PlanTier<br>4. Set tax/GL codes<br>5. Save as draft | — |
| Category & taxonomy manager | As a Product Manager, I organize products into a taxonomy so browse/search and listings are coherent | 1. Open taxonomy tree<br>2. Create/re-parent Category (uniqueness + cycle checks)<br>3. Assign products to categories | — |
| Attribute & localization editor | As a Product Manager, I add localized attributes so products read correctly per brand/region | 1. Open attribute editor<br>2. Add key/value; enable i18n; add locale values<br>3. Set brand/region visibility; configure fallback | — |
| Catalog approval & publish console | As a Finance Reviewer, I review and approve registry changes before publication | 1. Open pending-approval queue<br>2. Review diff (+ an override-carrying entity publish's lint findings)<br>3. Approve/reject with reason; the configured quorum above threshold<br>4. On sign-off the **entity** publish proceeds and emits events; the `CatalogVersion` increment follows mechanically and is never itself a gate (P-D-02) | — |
| Lifecycle & deprecation manager | As a Product Manager, I deprecate and retire SKUs safely so existing references are unaffected | 1. Select published SKU<br>2. Deprecate / un-deprecate (two-person)<br>3. Initiate retirement; system shows active-reference count (EOL with `mustMigrateBy` is **post-v1** — the field exists in the schema and the door refuses it, `EOL_DISABLED`)<br>4. Confirm; snapshots preserved; retirement event with lead-time | — |
| Catalog browser & version history | As a Partner/Auditor, I browse/search and inspect version history to find offerings and trace changes | 1. Filter/search (category, status, brand/region) on cache-first read model<br>2. Open item; view attributes/classification<br>3. Open version timeline with diffs and audit entries | — |
| Bulk operations console | As a CatalogAdmin, I import/export catalog entities in bulk so onboarding and mass edits scale | 1. Upload/import (CSV/JSON); rows land as draft with per-row validation<br>2. Review aggregated change report (counts, summary, sample, lint)<br>3. Submit for gated approval; track per-row status<br>4. Export a deterministic snapshot for a `catalogVersionId` | — |
| Freeze monitoring & recovery console | As an Operator, I inspect and recover stuck cross-module freezes so posting is never blocked silently | 1. View per-`catalogVersionId` `freezeComplete` and non-acknowledging participants<br>2. Idempotently re-trigger the fan-out<br>3. Force-complete under two-person (emits `FreezeForceCompleted`) | — |
| Operational console (clone, break-glass, deferred retire) | As an Operator/Platform owner, I want clone, break-glass, and deferred-cascade actions in one place | 1. Clone a Product/SKU (incl. a retired source) to a new draft<br>2. Break-glass cross-tenant read/audit-export under time-boxed elevation<br>3. Resume a deferred cascade-retire once blocked children clear | — |

## 12. Acceptance Criteria

**As a** Product Manager, Catalog Admin, or Finance Reviewer, **I want** an authoritative, governed, versioned catalog registry **so that** plan/price authoring, subscriptions, contracts, rating, and billing build on stable, reproducible product definitions.

### Identifiers & Integrity

**1. Identifier contract**
- **Given** a Product or SKU being created
- **When** the system assigns identifiers
- **Then** `productId`/`skuId` MUST be server-generated immutable identifiers (never operator-supplied); `skuCode` MUST be short, fixed-format, tenant-unique, reserved **atomically at create**, and immutable after first publish
- **And** downstream consumers MUST bind to `skuId`; a reused/malformed `skuCode` MUST be rejected with an audited reason; once first published a `skuCode` is permanently reserved within the tenant
- **And** a Product MAY carry an optional `productCode` under the same rules; when unset, product-level external mapping is `productId`-only

**2. Product/SKU field-mutability matrix**
- **Given** a published Product or SKU
- **When** an operator edits it
- **Then** mutability MUST be classified by lifecycle state: structural identity immutable (remedied only by retire + clone); `type`/metering-unit immutable-but-correctable via the fresh-zero path; material-but-mutable (`PlanTier`/`taxCategory`/`glCode`/`sellable`) via a new published version under governance; other fields via a new version
- **And** an illegal change MUST be rejected fail-closed with an audited reason
- **And** the active-reference count MUST be sourced from `SkuReferenceCount` as the 3-state predicate; never treat an entity as unreferenced absent a fresh watermark

**2a. Sellable flag (offering eligibility)**
- **Given** a SKU with `sellable = false` (composition/metering-only; default is `true`)
- **When** it is published and referenced
- **Then** publish MUST succeed and bundle/plan **component** references MUST remain valid, while any **standalone** offer of the SKU MUST fail the plan-price sellability gate (predicate 6)
- **And** flipping `sellable` MUST follow the material-but-mutable path (new published version, governed) and the value MUST be frozen per `CatalogVersion`

**3. Reference-signal sourcing, freshness & counting**
- **Given** the `SkuReferenceCount` signal (per-producer watermark; owner/delivery date is a pre-approval gate)
- **When** the registry evaluates mutability/correction/retirement
- **Then** absence of a `skuId` under a fresh watermark ⇒ zero for that producer; `referenced` MUST be a boolean OR across registered producers; the registry MUST NOT sum across producers
- **And** a fresh-zero across all producers ⇒ unreferenced; stale ⇒ conservatively referenced + alert; never-received ⇒ conservative + distinct flag
- **And** only registered producers count; Contracts MUST declare whether draft/quote refs count, identically across AC #2/#4/#18; until the signal ships, #2/#4/#18 run fail-safe

**4. Immutable-field correction (zero-reference & break-glass)**
- **Given** a published SKU whose correctable immutable field (`type` or metering-unit) was set wrong
- **When** a CatalogAdmin requests a correction
- **Then** if `SkuReferenceCount` is fresh-zero across all producers the system MUST allow a governed re-publish (two-person), bump the version, and emit `SkuImmutableFieldCorrected`; absent a fresh-zero signal it MUST reject fail-closed
- **And** while the signal is entirely unavailable, correction MAY proceed only via break-glass (two-person + reason + `SkuCorrectionOverride` recording signal-unavailability), feature-flag OFF by default
- **And** when the declared `usageTypeRef` no longer resolves (not-found, never a timeout) a meter-declaration correction MAY proceed regardless of the reference predicate, under the same ceremony, with `SkuCorrectionOverride` recording `unresolvable-target` (P-D-16)

### Product & Taxonomy Definition

**5. Create a product**
- **Given** a CatalogAdmin/ProductManager in a tenant/brand/region context
- **When** they create a Product (primary category optional at draft, required at publish)
- **Then** the system MUST generate `productId`, persist as `draft` (version 0) with isolation + audit
- **And** name uniqueness MUST hold on `(tenantId, brandId, normalized(name))` absolutely — region-independent (§15 decision); a same-named create under one tenant+brand is rejected regardless of region scope
- **And** the canonical internal name is a quasi-code; localized display names are attributes and MAY repeat — regional variants use distinct internal names with identical display names

**6. Manage taxonomy (create, rename, re-parent, retire, delete)**
- **Given** a CatalogAdmin managing the taxonomy
- **When** they create/rename/re-parent/retire/delete a Category
- **Then** the system MUST validate uniqueness within parent (re-checked on rename/re-parent), reject cycles, and reject exceeding max depth/children
- **And** a Product carries exactly one primary + zero-or-more secondary categories; the read model MUST make it filterable under every assigned category
- **And** categories are governed live entities (in-place, two-person-gated, audited); retire/delete MUST be blocked while any active Product references it or active children exist; each op emits a `Category*` event

### SKU Definition & Classification

**7. Define a SKU**
- **Given** an existing Product
- **When** a ProductManager defines a SKU typed `product`/`service`/`bundle`
- **Then** the system MUST link it, assign `skuId`/`skuCode`, and validate the per-type required-field set
- **And** a `bundle` SKU persists only type flag + identity (composition in plan-price; the uncomposed-publish two-person override is exercised at the bundle's entity publish, §15 decision)
- **And** promotional/$0/"Free" SKUs follow identical registry rules; no separate promo entity

**8. Declare metering unit (defines a "usage SKU")**
- **Given** a SKU to be metered (declaring a unit is what defines a usage SKU)
- **When** a ProductManager declares its metering unit
- **Then** the system MUST validate against the configured recognized-unit set and reject an unrecognized unit unless elevated approval marks a new validated unit
- **And** a usage SKU declares exactly one unit — the counted identity only; pricing dimensions (`dimensionKey`) are not units: the declared dimension set over a meter is plan-price-owned (plan-level meter binding), and multi-dimension usage on one unit does NOT require separate SKUs (separate SKUs remain available where variants differ commercially); a composite (derived) meter declares its **output** unit only
- **And** *(UC3 seam adoption; veto round — confirmed as amended, mirrors the FR)* the declaration MUST carry a `usageTypeRef`; publish MUST validate that the referenced UsageType **exists** (resolves in the platform-global catalog; a UsageType has no lifecycle state to check); the dimension-set cross-validation is NOT performed here — it lives at the plan-price meter binding (pricing's meter-binding rule: priced `dimensionKey` ⊆ the UsageType's `metadata_fields`) — **specified there, not built**, so no gate enforces it today on either side
- **And** a draft whose unit was `deprecated` before first publish MUST be treated as a new declaration and rejected; the declared unit MUST be carried on publish

**9. Metering-unit de-listing**
- **Given** a recognized unit referenced by ≥ 1 published SKU
- **When** an operator attempts to de-list it
- **Then** the system MUST reject removal while live references exist and instead support marking it `deprecated` (no new declarations), with full removal only once unreferenced
- **And** a unit's identity/semantics are immutable (no silent GB→GiB); a correction is a new unit + deprecation; de-listing MUST be audited and MUST NOT mutate any frozen snapshot

**10. PlanTier classification**
- **Given** a SKU being authored
- **When** a ProductManager assigns its `PlanTier`
- **Then** the system MUST validate against the taxonomy and carry it on the published SKU
- **And** a `PlanTier` value has a stable tier code; rename affects the display label only; taxonomy management is governed (two-person), emits `PlanTierUpdated`, and a value MUST NOT be retired while any published SKU carries it; seeded with a neutral value
- **And** `PlanTier` MUST NOT be conflated with OrgTier; plan-publish presence enforcement is delegated to plan-price

**11. Stable accounting codes on SKU**
- **Given** a SKU
- **When** a ProductManager sets tax-category and GL codes
- **Then** the system MUST persist them as stable codes and validate each against a configured recognized set (owner Finance)
- **And** the codes are required at publish for `product`/`service`-type SKUs; a SKU published without a required code MUST be rejected
- **And** the system MUST NOT compute tax or post to GL (codes only)

### Attributes & Localization

**12. Localized attributes, well-known display fields, and definition lifecycle**
- **Given** a Product/SKU with attributes
- **When** a ProductManager adds i18n values with brand/region visibility
- **Then** the system MUST validate against the attribute definition, require the default-locale value, and resolve locale via `(locale, region, brand) → (locale, brand) → (default-locale, brand) → global`
- **And** the registry MUST seed well-known display attribute definitions (localized display name/description for Product/SKU/Category; plus `imageUri`, `unitDisplayLabel`, `marketingFeatures[]` — industry parity)
- **And** managing definitions is a governed live-entity op emitting `AttributeDefinitionUpdated`; changes MUST be backward-compatible with a deprecate-then-remove lifecycle
- **And** the registry MUST provide an ungoverned, size-bounded, non-localized, search-excluded, PII-prohibited metadata map for machine metadata

### Versioning, Lifecycle & Deprecation

**13. Revision vs published version**
- **Given** a Product/SKU
- **When** an operator saves a draft edit
- **Then** the system MUST bump the internal revision (every save) and reject stale-revision writes, while the published version bumps only on publish
- **And** consumers and `CatalogVersion` MUST reference the published version; historical versions retained with a diff, non-modifiable
- **And** version binding MUST be explicit; a bound-but-not-yet-frozen reference re-resolving to a different version at freeze MUST surface a version-change diff, not silently swap

**14. Lifecycle transitions & reversibility**
- **Given** the state machine `draft → published [↔ deprecated] → retired` (+ `draft → discarded`)
- **When** an operator requests a transition
- **Then** the system MUST allow only defined transitions, treat `retired`/`discarded` as terminal, and allow referencing only for `published`/`deprecated`
- **And** entity publish MAY be scheduled (`publishAt`, UTC) with approval pinned at scheduling and re-validated fail-closed at activation
- **And** there is no `unpublish` and no in-place rollback — retraction/reversion is `deprecate`/`retire` + a new version (forward-only)

**15. Parent-child (Product↔SKU) lifecycle integrity**
- **Given** the Product↔SKU hierarchy
- **When** an operator publishes or retires across the hierarchy
- **Then** a SKU MUST NOT reach `published` under a non-`published` parent, and MUST NOT be orphaned under a `retired` Product; a SKU's scope MUST be contained within its parent's
- **And** retiring a Product with non-`retired` SKUs MUST require confirmed cascade-retire (partial by design, recording `direct` vs `cascaded`); EOL-requiring children are listed and left un-retired; never-published children auto-`discarded`
- **And** when a partial cascade leaves children, the parent MUST remain non-`retired` with deferred-retire intent tracked/queryable

**16. Deprecation (governed sub-state)**
- **Given** a published SKU referenced by active plans/subscriptions/contracts
- **When** an operator marks it `deprecated`
- **Then** the system MUST move it to the `deprecated` sub-state, mark it so consumers block new adoption while existing references continue, and emit `SkuDeprecated`
- **And** `deprecated` MUST be a tracked, queryable state recording provenance `direct` (vs `cascaded`)

**17. Un-deprecation**
- **Given** a `deprecated` SKU
- **When** an authorized operator un-deprecates it
- **Then** `deprecated → published` MUST be allowed under the two-person rule — `N`-governed, and the record + `SkuUndeprecated` MUST carry `quorumReduced` when the effective count is below 2 (P-D-13) — re-open new adoption, and emit `SkuUndeprecated`
- **And** un-deprecating a Product reverses only `cascaded` child deprecations, never a `direct` one; a `retired` entity MUST NOT be reversible

**18. Retirement / EOL consumer handoff**
- **Given** a `published`/`deprecated` SKU with active references
- **When** an operator retires it (optionally EOL with `mustMigrateBy`)
- **Then** the system MUST require confirmation with the active-reference count shown, then run a scheduled transition: force `deprecated` at initiation, preserve snapshots, emit `SkuRetired`/`ProductRetired` with `{ skuId, fromVersion, reason, replacedBy?, mustMigrateBy?, effectiveAt }` — `fromVersion` pinned at the initiation instant, with **no** publish freeze owed during the lead window and a **re-emission** of `SkuRetired` on any publish that moves it (P-D-20) — honoring the ≥ 30-day lead-time, then flip to `retired` at `effectiveAt`
- **And** the registry is SoR for `replacedBy` (a successor published SKU)
- **And** v1 = plain retirement + grandfathering only; EOL-with-`mustMigrateBy` is post-v1, disabled until the subscriptions-lifecycle AC exists and is referenced by number, and requires a consumer ack contract (lapsed ack ⇒ suspend fail-closed + `SkuEolSuspended`)

### Catalog Versioning & Snapshots

**19. Publish an immutable catalog version**
- **Given** approved catalog changes
- **When** a CatalogAdmin publishes a `CatalogVersion`
- **Then** the system MUST persist a full snapshot (published Product/SKU set + versions + current categories/attributes), assign a monotonic `catalogVersionId`, generate a checksum, record timestamps, capture the freeze-participant set, and make it immutable
- **And** an uncomposed `bundle` SKU enters the snapshot flagged `compositionPending = true`; its two-person override was exercised at its entity publish (§15 decision) — `N`-governed with the reduction recorded, and the `OverrideCeremony`'s acknowledgment-of-findings-by-name is performed by the **author** at `N = 0` rather than skipped, since informedness and not head-count is what that ceremony buys (P-D-13) — an increment, operator- or system-initiated, is never itself a new approval gate
- **And** it MUST emit `CatalogVersionPublished` and expose per-version `freezeComplete`; a published version cannot be withdrawn/rolled back (roll-forward N+1 only); publishes serialize per tenant

**20. Snapshot reproducibility**
- **Given** a posted invoice or active contract that referenced a `catalogVersionId`
- **When** the catalog later changes
- **Then** re-resolving that `catalogVersionId` MUST yield a byte-identical checksum and unchanged registry content
- **And** `CatalogVersion` MUST be exposable as one component of a downstream `pricingSnapshotRef` without asserting it equals the full snapshot

**20a. Catalog-version diff**
- **Given** two `catalogVersionId`s of one tenant
- **When** an operator or approver requests their diff
- **Then** the system MUST return a structured, deterministic diff over **every snapshot member** (entities added/removed, per-entity published-version deltas, and the captured live content: categories + display values, attribute definitions, recognized sets, metadata maps) computed **read-only** from the two frozen snapshots — byte-stable for a given pair
- **And** the diff is presentational: it MUST NOT mutate, re-freeze, or extend the retention of either version

**21. Cross-module snapshot freeze atomicity**
- **Given** a `CatalogVersionPublished` consumed by the registered freeze-participants
- **When** a consumer resolves `catalogVersionId` before all have frozen
- **Then** the system MUST expose `freezeComplete` and reject resolution for posted/contractual use until all participants ack, with a bounded timeout that fails closed
- **And** read-only browse MAY proceed during the freeze window
- **And** the resolution API MUST require the consumer to declare intent (`browse` vs `posted/contractual`) so it cannot post against a not-yet-`freezeComplete` version by mislabeling

**22. Freeze recovery & force-completion**
- **Given** a `CatalogVersion` past the freeze timeout
- **When** an operator inspects it
- **Then** the system MUST identify each non-acknowledging participant, support an idempotent re-trigger, and support force-completion (two-person — `N`-governed, with `quorumReduced` on the record and on `FreezeForceCompleted` when the effective count is below 2; no fixed floor, because one would leave a solo tenant's timed-out version permanently un-resolvable — P-D-13) that records each missing participant as not-frozen and emits `FreezeForceCompleted`
- **And** force-completion MUST NOT mark missing content as frozen; the default is pinned fail-closed for that participant's content, enforced at the registry's own resolver — a `complete(forced)` version is refused for `posted` resolution until every forced participant freezes or releases — P-D-19, P-D-47 (the per-version auto-fallback opt-in is an off-by-default later enhancement, not v1)

**23. Freeze-participant set governance**
- **Given** the set of freeze-participants
- **When** the set changes
- **Then** membership MUST be a governed (two-person), audited change, and each `CatalogVersion` MUST snapshot the participant set at publish time
- **And** a participant removed after publish MUST NOT retroactively flip that version's `freezeComplete`

**24. Grandfathering invariant**
- **Given** a reference grandfathered onto a frozen snapshot after its SKU is deprecated/retired
- **When** the catalog subsequently changes
- **Then** the registry MUST guarantee the grandfathered snapshot is never mutated
- **And** grandfathering eligibility policy is owned by plan-price / subscriptions-lifecycle; this AC makes the delegation auditable

**25. Uncomposed-bundle adoption guard**
- **Given** a `bundle` SKU published with the uncomposed override
- **When** the read model and events expose it
- **Then** it MUST carry `compositionPending = true` until composed, and consumers MUST treat it as not-yet-adoptable for new references
- **And** clearing it MUST be driven by a plan-price composition signal (`BundleCompositionCompleted`, inbound) and emitted as `SkuCompositionCleared` (outbound) (new version, never mutating a prior frozen `CatalogVersion`)

### Approval, Publishing & Eventing

**26. Materiality-gated publish**
- **Given** a Product/SKU change or a material Category/attribute-definition op
- **When** the change is material (touches `PlanTier`/metering-unit/`taxCategory`/`glCode`, a lifecycle transition, a Category create/rename/re-parent/retire/delete, a material attribute-definition change, or exceeds the configured affected-entity count)
- **Then** the system MUST enforce the tenant's configured approver quorum `N` (typed policy, default 2, floor 0 — P-D-11): `N` distinct approvers, each distinct from the author and holding CatalogAdmin or FinanceReviewer; a finance-material field MUST include ≥ 1 FinanceReviewer among them at every `N ≥ 1`; at `N = 0` the predicate MUST NOT be imposed (no principal could satisfy it, and the gate would then refuse forever) and the record MUST carry an explicit unsatisfiable-predicate marker instead
- **And** self-approval MUST be refused at every `N ≥ 1`; `N = 0` MUST still write the approval record (author, pinned content snapshot, audit row, `quorum {required: 0, satisfied: 0}`) and MUST be reachable only by explicit configuration — an absent value falls back to the default; the initial value is set at tenant provisioning and every later change to it is itself material under the then-current quorum
- **And** an approval MUST be pinned to the internal revision; any subsequent edit invalidates it and re-queues with the diff re-presented
- **And** a publish whose sole content is a system-owned flag cleared by an inbound governed signal MUST be recorded under subject kind `system_signal` — auto-satisfied against the signal reference, outside the configured `N`, and never exempt from the record (P-D-14). On a dirty head the clear is deferred, never refused (P-D-48)
- **And** the rule MUST be a typed configurable policy with an enforceable interim default (§17.1); a rejection returns the entity to `draft` with reason; v1 uses a single two-person step

**27. Idempotent authoring boundary**
- **Given** a retried create/update/publish carrying an idempotency key
- **When** the system processes the retry
- **Then** the same key + identical payload MUST NOT create duplicate entities/versions/events; the key MUST be scoped per tenant + endpoint + client key and retained ≥ 24h and ≥ the max freeze timeout
- **And** reuse with a different payload MUST be rejected as a conflict (no silent no-op)

**28. Registry eventing & audit**
- **Given** any state-changing registry mutation that completes
- **When** the write commits
- **Then** the registry MUST publish the corresponding event in the broker-native envelope (§15 envelope decision) onto the event-broker with correlation/causation + idempotency key + ordering keys `(tenant, aggregate)`; every state-changing AC maps to exactly one named event (or an explicit "no event" in Design)
- **And** payloads MUST carry pseudonymous actor references only (never direct operator PII); the mutation MUST be recorded in an immutable, queryable audit trail
- **And** Plan/Price/Bundle-composition events MUST NOT be emitted here (owned by plan-price)

**29. Event schema versioning & replay**
- **Given** the events of AC #28
- **When** the schema evolves or a consumer must rebuild state
- **Then** every event MUST carry a versioned (semver) schema reference (broker-native `dataschema` equivalent, §15 envelope decision); a consumer pinned to `vN` MUST deserialize `vN+1`; out-of-order/duplicate delivery beyond the idempotency window MUST be detectable via `(tenant, aggregate, sequence)`
- **And** the system MUST provide a bootstrap path (latest `CatalogVersion` + event tail) for published-scope consumers and MUST fail loudly when a consumer checkpoint predates the available event tail

### Multi-Tenancy & Read Models

**30. Tenant/brand/region isolation & break-glass**
- **Given** a user scoped to one tenant/brand/region
- **When** they query/mutate outside their scope
- **Then** the system MUST deny by default at the gateway and audit the cross-scope attempt
- **And** privileged cross-tenant access MUST use time-boxed, reason-required, alertable break-glass, itself two-person-approved or post-hoc-reviewed — and here "two-person" is a **fixed floor of two distinct platform principals**, outside the tenant's configured `N` entirely: the acting principal is a platform owner and the subject is another tenant's data, so no tenant configuration has standing over it (P-D-13). The post-hoc-review arm is the escape the floor needs, so the floor blocks no one; standing cross-tenant access MUST NOT be granted

**31. Break-glass action scope**
- **Given** a platform owner under break-glass elevation
- **When** they access a foreign tenant's catalog
- **Then** break-glass MUST permit read and audit-export only; any write/publish MUST be separately gated (two-person + distinct alert) or disallowed in v1
- **And** every break-glass action MUST be individually audited with the reason and correlation ID

**32. Cache-first browse/search with bounded convergence**
- **Given** published Products/SKUs/Categories
- **When** a partner/customer browses/searches/filters
- **Then** the system MUST serve from cache-first read models scoped to the caller's tenant/brand/region, converging within its own budget (interim p99 < 2 s)
- **And** stale reads during the window MUST be safe (never expose unpublished/cross-scope content) and carry the `asOfCatalogVersion` staleness signal
- **And** the per-state visibility contract MUST hold: `published` browsable; `deprecated` browsable + flagged + excludable; `retired` excluded from default browse, retrievable via explicit history query

### Bulk Operations

**33. Bulk import/export**
- **Given** a CatalogAdmin importing/exporting in bulk
- **When** the batch is processed
- **Then** the system MUST apply per-row idempotency, report per-row success/failure (no hidden partial failure), and never leave a partially-inconsistent published state
- **And** dependent rows MUST apply two-phase or dependency-ordered, never committing an orphan; idempotency operates at batch + per-row levels; a coalesced `CatalogBulkOperationCompleted` is emitted (no event storm)
- **And** bulk import lands entities in `draft`; publication remains gated, approved against an aggregated change report; export MUST be deterministic for a given `catalogVersionId`

**33a. Environment promotion via export/import**
- **Given** a deterministic export produced at a `catalogVersionId` in one environment
- **When** it is imported into another environment
- **Then** identity MUST carry via stable codes — `skuCode` for SKUs; `productCode`, else `(brandId, canonical internal name)`, for Products (total under P-D-04's absolute name uniqueness) — with system ids re-minted by the target; rows land in `draft` with per-row idempotency, and publication passes the same gated approval as any bulk import — promotion is never a governance bypass
- **And** an identity collision is classified exhaustively (P-D-17): unknown ⇒ create; matching content ⇒ no-op; different content ⇒ update-as-draft; incompatible kind/type, a `retired` holder, or a dirty head ⇒ per-row conflict — **never a silent merge**, since the update lands in `draft` under the batch's own quorum; the catalog-version diff (AC #20a) is the reviewer's post-commit verification view of what the promotion changed

### Cloning

**34. Clone a product/SKU**
- **Given** a source Product/SKU (draft, published — `deprecated` included — or retired, the sanctioned revival path)
- **When** a ProductManager clones it
- **Then** the system MUST create a new `draft` with new `productId`/`skuId` and a new `skuCode`/optional `productCode`, copying structure/attributes/scoping/category/`PlanTier`/metering-unit, resetting lifecycle and version counters, and never copying pricing/plan content
- **And** the cloned metering unit, `PlanTier`, and category assignment MUST be re-validated against live registries; the clone MUST fail or force re-selection if any was de-listed/deprecated/retired; a `clonedFrom` reference is recorded and the source unaffected

### Data Retention & Erasure

**35. Retention & right-to-erasure**
- **Given** retired entities, historical versions, and audit records
- **When** a retention or erasure (GDPR/CCPA) request applies
- **Then** the system MUST retain financial/version/audit records for the configured duration and satisfy erasure of actor PII by pseudonymizing it across audit, entity version fields, and the actor identity-reference map — not deleting immutable records
- **And** attribute/description free-text MUST NOT contain personal data — enforced by a validation block at write (hard prohibition, no carve-out, fail-closed on uncertainty, curated allow-list); Legal sign-off recorded in the approval artifact
- **And** erasure MUST NOT break `CatalogVersion` reproducibility or audit completeness

### Cross-PRD Consistency

**36. Registry ↔ plan-price seam**
- **Given** the shared contract, whose pinned membership is **the rule of P-D-12 and not a list**: exactly the operands the consumer obligations below are enforced on — `skuId`, `type` (the `bundle` discriminator), the metering-unit declaration **and `usageTypeRef`**, `PlanTier`, `status` **together with its value vocabulary**, `sellable`, `compositionPending`, with `CatalogVersion` pinned as a **surface**, not a field (this `Given` previously enumerated the five-item list `fr-plan-price-seam` records as one where only three could ever be compared; P-D-12's propagation list named the FR, slice 12 and `DESIGN.md` and so nothing pointed at this AC, leaving a suite built from it with `status`, `sellable`, `compositionPending` and `usageTypeRef` unpinned — item 22 of the review)
- **When** registry or plan-price schemas change
- **Then** there MUST be a shared schema-version pin and a CI contract test that fails on divergence; a runtime divergence MUST fail closed (reject the dependent plan publish)
- **And** the same suite MUST assert consumer-side obligations: reject adoption of `compositionPending`/`deprecated` SKUs; reject a usage binding with no declared unit (and reject/warn on a `deprecated` unit) — where usage-completeness is enforced; consume `mustMigrateBy` (post-v1); resolve grandfathered refs against the frozen snapshot; re-validate on `SkuImmutableFieldCorrected`; declare intent before `freezeComplete`
- **And** each consumer-side assertion is authorable only once the referenced counterpart AC exists; (d) `mustMigrateBy` is deferred with the post-v1 EOL capability

**37. Monetization-model traceability**
- **Given** the deliberate decision that the registry carries no monetization-model marker (only `usage` leaves a footprint)
- **When** a reader asks which models are supported and where
- **Then** the PRD MUST expose a traceability map (§17.2): flat/per-seat/tiered/volume/hybrid/commitment → plan-price + Tariffs; usage → metering-unit declaration here + binding/rating downstream
- **And** absence of a model marker on a SKU MUST be treated as intentional, not a missing field

### Error & Negative Paths

**38. Expected failure behavior**
- **Given** an invalid or conflicting authoring request
- **When** the system processes it
- **Then** it MUST fail closed with an audited reason and MUST NOT partially apply, for each of the enumerated cases (see the `expected-failure-behavior` FR): stale-revision write, duplicate idempotency key with a different body, taxonomy cycle, unrecognized unit without elevation, publish of an incomplete entity, immutable-field change without a valid correction path, reissue/collision of a reserved `skuCode`, EOL without an acknowledged migration consumer (post-v1), SKU under a non-`published` parent, SKU scope outside its parent, authoring/cloning against a **de-listed** unit, authoring/cloning against a **deprecated** unit *(**P-D-44** split this row: the two conditions raise different codes)*, a bulk row whose in-batch dependency failed, adopting a `compositionPending` bundle, and a retention process that would orphan a live grandfathered reference

### Operational Resilience & Concurrency

**39. Event delivery resilience**
- **Given** a registry event fanned out to ≥ 1 consumer
- **When** a delivery fails, a consumer never acks, or an event is poison
- **Then** the shared event system MUST provide bounded-backoff retry, per-consumer delivery state, and an audited dead-letter path with alerting (transport owned there)
- **And** the registry's own obligations are: not reporting emission success until durably accepted, surfacing per-consumer delivery/DLQ state as a projection, and never mutating registry state on delivery failure; during a bus outage mutations MAY commit with events to a durable outbox, with the propagation clock starting at durable bus acceptance

**40. CatalogVersion publish concurrency**
- **Given** two staged sets of changes targeting publication within one tenant
- **When** publishes are submitted concurrently, or a publish races a `deprecate`/`retire` on an entity it enumerates
- **Then** publishes MUST serialize per tenant, `catalogVersionId` MUST be allocated monotonically without gaps/collisions, and a staged entity whose published version **or lifecycle state** moved between collect and commit MUST cause that run to re-validate fail-closed — the **operator lane** rejecting and naming the changed entity, the **mechanical lane** (D-47 demand-driven increments, no operator to reject to) re-collecting and retrying within its lane SLO without losing the request (P-D-09). In neither lane may stale content be frozen

**41. Fail-safe duration tripwire**
- **Given** the registry operating in `SkuReferenceCount`-unavailable fail-safe mode
- **When** break-glass immutable-field corrections exceed the configured rate (interim > 5 in 30 days)
- **Then** the system MUST raise an escalation alert and reclassify `SkuReferenceCount` delivery as a release blocker, so degraded operation is escalated, not normalized

**42. `skuCode` reservation concurrency**
- **Given** two concurrent create/reserve requests for the same `skuCode` within one tenant
- **When** both are processed
- **Then** the system MUST atomically reserve at create, admit exactly one, and reject the other fail-closed with an audited reason; a `draft` reservation MUST block a second draft until released/discarded
- **And** a `skuCode` changed while still `draft` MUST release the previous code; discarding a never-published draft MUST also release its `skuCode`/`productCode` reservation

**43. Reference-producer registration**
- **Given** the set of registered `SkuReferenceCount` producers (v1 = plan-price, §15 decision; Subscriptions and Contracts register at their own build)
- **When** the registry evaluates `referenced`
- **Then** only registered producers' signals or silence MUST factor in; an unregistered producer's absence MUST NOT pin SKUs conservatively-referenced; membership MUST be a governed, audited change snapshotted symmetrically with the freeze-participant set
- **And** onboarding a new producer MUST NOT retroactively flip historical mutability/retirement decisions

**44. Grandfathered-snapshot retention coupling**
- **Given** a `catalogVersionId` referenced by ≥ 1 live grandfathered reference
- **When** retention/erasure would expire, tier, or GC that snapshot
- **Then** the snapshot MUST remain byte-identically resolvable for as long as a live reference exists, regardless of the statutory-max clock; retention expiry MUST be gated on no live references to that `catalogVersionId`
- **And** version-liveness MUST be sourced from per-version freeze-registration records — **acked-and-not-yet-released**, the release arriving through the `catalog_version × release` half of the freeze-participant contract (P-D-18, closing the §15 choice in favour of freeze-registration over a `(catalogVersionId, producer)` count) — never the SKU-level count alone; a process that would orphan a live reference MUST fail closed with an alert

**45. Pre-publish lint report**
- **Given** the `validate(lint)` operation before `CatalogVersion` publish
- **When** an admin runs it (or publish triggers it)
- **Then** the lint MUST return a structured, per-entity report of every attention condition (uncomposed bundles, missing default-locale attribute values, declarations against a `deprecated` unit) so an operator publish is informed and the audit records what was outstanding; the uncomposed-bundle override itself is exercised at the bundle's entity publish (§15 decision), with the same lint findings presented to its approvers

### Non-Functional Requirements (Show-Stoppers)

**1. Read latency**
- **Given** a warm read model holding 10K SKUs/tenant with ≥ 100 concurrent readers
- **When** browse/search reads execute within a tenant partition
- **Then** p95 latency MUST be < 100 ms over a 5-minute window, sustained via cache-first read models and partitioning

**2. Read throughput**
- **Given** the cache-first read model under load
- **When** browse/search traffic peaks
- **Then** the system MUST sustain ≥ 2,000 read QPS per tenant partition at the AC-1 latency target

**3. Publication propagation**
- **Given** an approved publish
- **When** the publish completes
- **Then** downstream event availability (incl. fan-out) MUST occur within < 3 s — distinct from read-model convergence (< 2 s) and the end-to-end posting-safe budget (< 5 s)

**4. End-to-end posting-safe budget**
- **Given** a write commit that must become safe for Contracts/Billing to post against
- **When** measured from commit to "posting-safe" (read converged AND all `freezeComplete` acks)
- **Then** the composite MUST be p99 < 5 s; if the freeze times out the version MUST remain non-posting-safe (fail closed)

**5. Snapshot archival & cold-resolution SLA**
- **Given** accumulating immutable snapshots under statutory-bounded retention at ≥ 10K SKUs/tenant
- **When** an archived ("cold") `catalogVersionId` is re-resolved
- **Then** re-resolution MUST remain byte-identical and meet a looser-than-hot target (interim p95 < 2 s)
- **And** snapshots + version history are financial records with a durability class (≥ 11 nines / replicated), periodic restore verification, and a cross-region/DR posture with RPO/RTO

**6. Scale & extensibility limits**
- **Given** a large tenant
- **When** the catalog grows
- **Then** the system MUST support ≥ 10K SKUs/tenant without breaching read latency, within configured extensibility limits, and MUST bound tenant count, cardinality, and `CatalogVersion` growth

**7. Graceful degradation & staleness exposure**
- **Given** read load above the throughput ceiling or read-model lag above budget
- **When** the system serves browse/search
- **Then** it MUST shed/queue excess load without ever serving cross-scope or unpublished content, and MUST expose staleness via the same machine-readable `asOfCatalogVersion` signal (no silently-stale response)

**8. Determinism & integrity**
- **Given** the registry invariants
- **When** authoring/publish runs
- **Then** version immutability, taxonomy acyclicity, SKU identity uniqueness, and metering-unit validity MUST be enforced fail-closed, and posted-period snapshots MUST remain immutable

**9. Backward-compatible schema evolution**
- **Given** a consumer pinned to schema `vN`
- **When** the registry publishes a `vN+1` payload
- **Then** the consumer MUST deserialize it (new fields optional with defaults); a CI contract test MUST assert backward compatibility on every schema change

**10. Availability & audit completeness**
- **Given** the catalog service
- **When** measured over the SLO window
- **Then** the cache-first read path MUST meet 99.9% availability and the write/publish path 99.5%; write paths MUST be fully audited even during partial failures

## 13. Dependencies

| Dependency | Description | Criticality |
|------------|-------------|-------------|
| Tenant identity & hierarchy (OSS/AMS + IdP) | `tenantId`, brand/region claims, OrgTier projection targets, role claims (registry MUST NOT mutate tenant topology) | `p1` |
| Plan & Price Modeling | Consumes published SKU identity/type, metering-unit declaration, `PlanTier`, `CatalogVersion`; produces `SkuReferenceCount`, `freezeComplete` ack, bundle composition-completed signal | `p1` |
| Rating (evaluation core + pipeline) | The one rating gear (post ADR-0002; absorbs the former "Tariffs / Pricing Logic" consumer): consumes published SKU refs + `CatalogVersion` (price resolution) and the metering-unit declaration (usage rating) | `p1` |
| OSS metering | Emits usage values (external); consumes the metering-unit declaration | `p1` |
| Subscriptions (lifecycle & entitlements) | Produces `SkuReferenceCount`; consumes SKU refs + `PlanTier` + `replacedBy` + `mustMigrateBy` (post-v1); owns live-subscription migration | `p1` |
| Contracts & Agreements | Produces `SkuReferenceCount` (incl. draft/quote refs per contract) + `freezeComplete` ack; consumes `CatalogVersion` snapshots for quotes | `p1` |
| Billing & Invoicing | Produces `freezeComplete` ack; consumes SKU refs + `CatalogVersion` (descriptors authored in plan-price, frozen into `CatalogVersion`) | `p1` |
| Marketplace & Vendor Portal | References published SKUs; vendor ops remain in the Marketplace PRD (§4.8) | `p2` |
| Presentation / Portals | Consumes catalog read models for browse/search cache warming | `p2` |
| Events & Audit (Common Core) | Shared event system: durable acceptance, per-consumer delivery/DLQ state, retry; transport owned there | `p1` |
| BSS Architecture Manifest | §4.1 (registry), §4.4 (posting immutability), §4.2/§4.3/§4.6/§4.8 (consumers), §2.1.3, §7.2 | `p1` |

> **Registry is upstream of all commercial modeling.** A SKU MUST be published before a Plan/Price can reference it. The registry MUST NOT require any downstream consumer to re-interpret mutable catalog state for **posted** periods; the `CatalogVersion` snapshot contract is authoritative (manifest §4.4).

## 14. Assumptions

- The `SkuReferenceCount` signal's **v1 registered producer set = {plan-price (pricing gear)}** (§15 decision), built jointly with this gear's development and delivered before its v1 GA; Subscriptions and Contracts register as producers at their own build time (their GA gated on producing). Until the pricing watermark ships, AC #2/#4/#18 run fail-safe, bounded by the break-glass path and the fail-safe tripwire.
- Interim configurable-policy defaults (§17.1) are enforceable at launch; each final value is owned by another function and changes are governed/audited.
- Numeric NFR targets are binding **design targets** until the NFR workshop (within 2 weeks of approval; DRI = BSS Program Lead).
- The shared event system (Common Core) provides ordering/at-least-once/DLQ transport; the registry states only its own emission/projection obligations.
- Recognized-set owners (metering units → Product + Rating; tax/GL codes → Finance; `PlanTier` → Product) seed and govern their sets; the registry validates against them.
- `PRD-plan-price-modeling` and this PRD stay consistent on the shared fields via a CI seam test; the combined predecessor PRD is refactored to Marketplace-only after approval.

## 15. Open Questions

> **Pre-approval gates.** All four launch-governing gates are **closed as of** (see the struck rows below): `SkuReferenceCount` v1 producer set, name-uniqueness/region-algebra scope reduction, event-envelope conformance, and the D-47/governance composition. Event delivery resilience and `CatalogVersion` publish concurrency are build conditions captured as FRs/ACs.

| **Question** | **Answer** | **Date Answered** |
|--------------|------------|-------------------|
| ~~GATE — `SkuReferenceCount` signal owner + delivery date~~ | **Answered (product call): v1 registered producer set = {plan-price (pricing gear)}** — the only coded counterpart, and it already holds the data (live plan→SKU references). Built **jointly with this gear's development**, delivered before products v1 GA (mirrored in the pricing PRD §15). Subscriptions/Contracts register as producers at their own build time; per `fr-reference-producer-registration`, their unregistered silence pins nothing and their late onboarding never re-flips history. Until the pricing watermark ships, AC #2/#4/#18 run fail-safe (break-glass + tripwire). Propagated: §9.2, §14, `fr-reference-producer-registration`, AC #43. | **** |
| ~~GATE — Region-set algebra~~ (was: overlap semantics for AC #5) | **Answered (product call): same-named Products are forbidden outright** — name uniqueness on `(tenantId, brandId, normalized(name))` is absolute, region-independent. Rationale: the sales-facing name is a localized display attribute and repeats freely, so the canonical internal name is a quasi-code; and strict→loose is a compatible later widening while loose→strict is a breaking migration. The overlap/disjointness algebra thereby **disappears from AC #5**; region-set semantics remain only for **parent-child scope containment** (`fr-parent-child-integrity`) — pinned in Design, interim conservative subset-check fail-closed. Propagated: glossary (Region), `fr-create-product`, `fr-expected-failure-behavior`, AC #5, AC #38, §16. *(Owner of the containment rule: Design.)* | **** |
| ~~GATE — Event-envelope conformance~~ (CloudEvents 1.0 vs the built event-broker) | **Answered (product call): the registry adopts the event-broker's broker-native envelope** (ADR-0003 — no CloudEvents conformance, no `dataschema` field). Semantic obligations (versioned schemas, `vN`→`vN+1`, correlation/causation, ordering keys, pseudonymous actors) bind unchanged on the broker-native envelope. Residue owed: **manifest §7.2 amendment** re-scoping the CloudEvents mandate. Propagated: §2, §4.1 (the **Eventing** constraint bullet), §5.1, FRs `fr-registry-eventing-audit`/`fr-event-versioning-replay`, §9.2, AC #28/#29 — §1.3 struck: it is Goals and never mentions the envelope, and `DECISIONS.md` P-D-01's own list never claimed it, so the two registers disagreed (item 31 of the review). *(Owner of the manifest amendment: Architecture / Common Core.)* | **** |
| ~~GATE — D-47 demand-driven increments vs publish governance~~ | **Answered (product call): a `CatalogVersion` increment is mechanical, never an approval gate.** All governance attaches to the **entity publish** that introduces the exception: the uncomposed-bundle two-person override moves from `CatalogVersion` publish to the bundle's entity publish (lint findings presented there); a system-initiated D-47 increment never waits on a human; the `CatalogVersion`-publish lint becomes an informational report for operator publishes. Propagated: §4.1/§5.1 (lint row), §5.2, FRs `fr-define-sku`/`fr-catalog-version-publish`/`fr-bundle-adoption-guard`/`fr-prepublish-lint`, AC #7/#19/#25/#45. *(Design owns the entity-publish override surface.)* | **** |
| `compositionPending` clearing signal unregistered on the pricing side | `fr-bundle-adoption-guard` requires a plan-price composition signal (`BundleCompositionCompleted`), but the shipped pricing bundles design (Slice 8) registers no such outbound signal. Pricing must adopt the counterpart or the guard needs a different clearing mechanism. *(Owner: pricing + Product; raised by the cross-review.)* | TBD |
| `freezeComplete` ack counterparts silent | All three named freeze-participants are silent: pricing and Contracts docs never mention producing an ack, and Billing has no gear at all — where does its ack obligation live? Rating SEAMS independently tracks the freeze-protocol composition as open. *(Owner: Architecture + participants; raised.)* | **The registry-side half is CLOSED (P-D-48)**: the v1 registered freeze-participant set is **{plan-price}**, its ack and release clients built jointly with this gear as P-D-03 builds the watermark; Contracts and Billing register at their own build time, so no v1 duty is booked on a gear that does not exist. Whether pricing's design accepts the ack and the release is the cross-gear half and stays open. |
| Feature/entitlement vocabulary on SKUs | Industry binds a feature vocabulary to catalog items (Stripe Entitlements: Feature objects on Products; Zuora Product Features); here nothing owns "which features a SKU includes". Probable split: registry owns the governed vocabulary + SKU binding (it describes *what is sold*), subscriptions-entitlements enforces. Decide owner + v1/post-v1. *(Owner: Product + Subscriptions; raised by the industry comparison.)* | TBD |
| `resourceTypeRef` — SKU → provisioned-resource GTS binding | When a fulfillment requirement appears ("what does this SKU provision"), the governed binding follows the `usageTypeRef` pattern: an optional GTS-typed ref to the infrastructure resource type (infrastructure-resource-manager / serverless-runtime), validated for resolvability at publish. Until then the ungoverned metadata map carries such refs. **SKUs/Products themselves are never GTS instances** — DESIGN §2.2 records the boundary (tenant-scoped business data vs platform-global types; a third identity would break the identifier contract). *(Owner: Product + Architecture; raised.)* | TBD |
| UsageType deletion vs published declarations | The collector's delete-RESTRICT counts only its own usage records — a UsageType referenced by a **published** metering-unit declaration can be deleted, leaving a sold-but-unrateable meter (rating quarantines, correctly but indefinitely). Negotiate with usage-collector: (a) extend the delete guard to registry-published declarations (needs a reference signal from us), or (b) a deletion event consumed here + by pricing's `meter_binding_divergent` remediation path. Until decided, quarantine is the fail-safe and the pre-publish lint warns on `deprecated`/dangling units. *(Owner: Product + usage-collector + pricing; raised by PR #14 review.)* | **The registry-side half is CLOSED (P-D-16)**: an unresolvable `usageTypeRef` is a third correction-admission arm, so a wedged SKU has an exit. The negotiation with usage-collector over (a) vs (b) stays open and is now about **prevention**, not about whether the state is recoverable. |
| ~~UC3(c) registry-side operand undefined~~ | **Answered by the veto round : the cross-validation moved to where the operand lives.** Registry publish validates only that `usageTypeRef` **resolves** ("is active" dropped too — a UsageType carries no lifecycle state); the dimension check is pricing's meter-binding rule at plan publish (priced `dimensionKey` ⊆ `metadata_fields` — subset, not equality: pricing fewer dimensions than the source emits is harmless, pricing one it never emits is the hazard). Rating SEAMS UC3 row updated in the same round. Propagated: `fr-metering-unit-declaration`, AC #8, SEAMS UC3 + ownership matrix, pricing design/02 premise phrase. | **** |
| Contracts draft/quote references: count toward `referenced`? + re-resolve-at-freeze behavior | Producer contract must declare both, identical across AC #2/#4/#18; recorded at sign-off. **Note :** the contracts gear PRD explicitly positions a Contract as "not a quote", excludes CPQ, and never cites `CatalogVersion` — the quote-snapshot delegation in §3.2/§13 may have no taker and needs renegotiation with Contracts. *(Owner: Contracts + Architecture.)* | TBD |
| EOL `mustMigrateBy`: pull into v1 or confirm the post-v1 deferral? | Registry side deferred; needs a date to pull in or confirmation. Gates EOL child cascade. *(Owner: Subscriptions.)* | TBD |
| Finance materiality threshold production value + date | Dimension + interim default resolved; needs a committed production value + date. *(Owner: Finance.)* | TBD |
| Legal content-PII prohibition sign-off | Normative position = hard prohibition, no carve-out; Legal to confirm sufficiency + detector posture, recorded at approval. *(Owner: Legal.)* | TBD |
| Data retention durations per record class + PII pseudonymization age | Interim set (financial/version/audit → statutory max). Final durations per jurisdiction. *(Owner: Legal/Finance.)* | TBD |
| Recognized metering-unit set owner + add-unit / de-list workflow | Interim seed ships; de-list governed. Owner + workflow. *(Owner: Product + Rating.)* | TBD |
| Recognized tax-category / GL-code sets: owner + add / de-list workflow | Interim configured sets ship; validated + required at publish for product/service types. Owner + workflow. *(Owner: Finance.)* | TBD |
| PlanTier taxonomy governance ownership confirmation | Taxonomy + SKU value owned here; plan enforces presence at plan publish. Confirm with plan-price. *(Owner: Product + plan-price.)* | TBD |
| Catalog taxonomy/category scheme (IaaS/PaaS/SaaS/…) | To be defined. *(Owner: Product.)* | TBD |
| Media/binary asset ownership (does the registry hold asset URIs?) | Out of scope as binaries; registry may carry URIs. Confirm owner — Presentation / Marketplace / DAM. *(Owner: Product + Presentation/Marketplace.)* | TBD |
| Catalog-relationship block beyond `bundle`/parent-child/`replacedBy` | Registry owns `replacedBy`/supersedes; other types out of scope v1. Confirm need vs plan-price/Subscriptions. *(Owner: Product + Architecture.)* | TBD |
| Per-seat monetization: truly zero registry footprint? | Confirm no seat-as-unit artifact; metered seats would collide with the single-unit rule. *(Owner: Product.)* | TBD |
| Monetization-model coverage: plan-price pointer accepted in lieu of registry fields? | Traceability map provided (§17.2). *(Owner: Finance/Program.)* | TBD |
| Event-bus transport contract owner + home (ordering, at-least-once, DLQ retention) | Registry states its requirements; the contract is owned by Common Core / Events & Audit. **The two ordering specifics the registry could not pin (raised by the slice-01 review) are the broker's since P-D-47:** design §4.4 no longer derives a partition of its own — the gear sets no `partition_key`, so the broker's ADR-0002 default applies (MurmurHash3-32 over `tenant_id`, modulo `topic.partitions`, which is fixed at topic creation, so no partition-count change can re-route a tenant mid-stream — the reordering AC #28 forbids). What stays open here is what the row always named: the transport contract's owner and home. *(Owner: Architecture/Eng — Common Core.)* | TBD |
| Platform audit **sealing** capability: owner + home + delivery | **Decided here (P-D-08): tamper-evidence is not built per gear.** The registry ships the complete append-only trail plus a reserved, unwritten sealing seam (`seal_state`/`chain_id`/`seq`/`prev_hash`/`row_hash`) and states the requirements the platform capability MUST satisfy — P-D-08 **S1–S9** (construction, segmentation, never-on-the-mutation-path, verification cadence, WORM anchoring, residency, erasure compatibility, retention, coverage). No such platform gear exists today, and the ledger and pricing each built their own chain; that replication stops here. Until the capability ships, audit immutability is the trigger whitelist on both engines and nothing cryptographic (**P-D-46**) — an edited audit row is undetectable (see §16). *(Owner: Architecture / Common Core — Events & Audit.)* | TBD |
| Event-log retention/TTL value | MUST be ≥ the bootstrap gap implied by AC #29. *(Owner: Eng/Common Core.)* | TBD |
| `CatalogVersion` archival economics: storage growth + publishes/day/tenant target | Tiering allowed while byte-identical; needs the publish-frequency target before storage design. *(Owner: Eng/Finance.)* | TBD |
| Snapshot durability / DR targets (RPO/RTO + restore-verification cadence) | Snapshots are financial records: interim ≥ 11 nines + periodic checksum restore verification; RPO/RTO ratified at the NFR workshop. *(Owner: Eng/Program.)* | TBD |
| Snapshot-GC version-liveness source | Per-SKU count has no version dimension; source from per-version freeze-registration or a `(catalogVersionId, producer)` contract. *(Owner: Architecture + freeze participants.)* | **CLOSED (P-D-18)**: freeze-registration, with liveness = **acked-and-not-yet-released** and the release added to §9.2 as the second half of the freeze-participant contract. A version-scoped producer count would have been a second signal with its own freshness and producer set. |
| Cross-PRD seam contract-suite owner + repo/pipeline | Proposed: BSS Catalog/Architecture in `api-contracts` CI. Final owner sign-off. *(Owner: needs assignment.)* | TBD |
| Disposition of `PRD-product-catalog-marketplace-202601120119` (refactor to Marketplace-only) | Keep now; refactor to §4.8-only after this + plan-price + Tariffs approved. *(Owner: Product/Program.)* | TBD |
| **Where do `partition_id`/`seq` ride on the wire?** | Design 01 §4.4 places them **on the envelope**, 12 `inst-rc-dedup` says the same, and **P-D-27** decided it — but the broker's own contract refuses that slot: `gears/system/event-broker/docs/schemas/event.v1.schema.json` marks `partition`, `sequence` and `sequence_time` **`readOnly`** ("server-stamped on read and rejected with 400 BadRequest if supplied on publish"), and `meta` is `writeOnly` ("accepted on publish and stripped on read"), so a consumer never sees it. The broker also derives `partition` itself from an optional `partition_key`, not from `hash(tenant_id, aggregate_id) mod N` as the design then asserted. The operand `fr-event-versioning-replay` and 12's dedup rest on therefore has no admitted slot: it must ride the **payload body** (which §4.5 declares a closed core), or the gear must set `partition_key` and consume the broker's own stamped `sequence`. P-D-27's premise is false as written and the decision needs re-taking. Raised by the slice-01 fourth lens wave. *(Owner: event-broker / Common Core, with Design and slice 12.)* | **CLOSED (P-D-47)**: the second path, as the broker's own contract shapes it — the gear publishes through the broker SDK's outbox-backed producer, sets no `partition_key` (so ADR-0002's tenant default gives per-tenant order), and the `(tenant, aggregate, sequence)` operand is the broker's read-side `sequence`; the toolkit's `partition_id`/`seq` stay inside the pipeline as the producer chain's `meta.sequence`. Nothing is asked of the broker. |
| **AC #26's "returns the entity to `draft`" against the head-row model** | AC #26 says a rejection "returns the entity to `draft` with reason", but `fr-lifecycle-transitions` forbids the edge that would take a published head there ("There is **no `unpublish`** and **no in-place rollback**" — forward-only). Design 01 §2 and 05 both read it as *no state flip either way* — a first-publish entity stays `draft`, a published head keeps its pending edits unpublished — so the two design slices agree with each other and both diverge from the AC's literal text. The design set cannot amend an AC; either the sentence is amended or the design reading is ratified. Raised by the slice-01 sixth-pass review. *(Owner: PRD owner.)* | TBD |
| **Do `taxCategory` and `glCode` belong to this registry?** | §2.1 lists "billing descriptors (invoice line template / tax category / GL code)" among the things "owned elsewhere and **MUST NOT** be re-specified here", while `fr-accounting-codes` requires the registry to persist them as stable codes and validate each against a recognized set, and design §4.2 carries `tax_category_ref`/`gl_code_ref`. The PRD contradicts itself and only its owner can say which sentence governs; if §2.1 wins, slices 01 and 03 lose two columns. Raised by the slice-01 review. *(Owner: PRD owner.)* | TBD |
| **`sellable` missing from pricing's `CatalogSku`** | The registry publishes `sellable` and P-D-12 pins it in the shared surface, but the consumer shape on the pricing side has no member for it, so the pin cannot be asserted end to end. The fix is a consumer-side addition, not a registry change. Raised, re-confirmed by the slice-01 review. *(Owner: plan-price / pricing gear.)* | TBD |
| **Broker schema-version pinning: one worked example owed** | `fr-event-versioning-replay` requires a versioned schema ref and `vN`→`vN+1` tolerance, but the mechanics on the broker side have never been walked through end to end for this gear; slice 12 freezes the replay contract against them. *(Owner: Common Core / event-broker.)* | TBD |
| **Who measures the < 3 s propagation budget, and against which meter?** | `nfr-publication-propagation` is claimed by slices 01 and 06 and `DESIGN.md` §1.2's coverage table assigns it to both; **no slice §5 measures it either way**. Slice 08's convergence probe already instruments the commit→durable-acceptance segment but asserts it against 08's own budget and expressly refuses to collapse the two. Whether one meter may be asserted against two thresholds, or a second probe is owed, decides the split. *(Owner: BSS Program Lead, with slices 01/06/08.)* | TBD |
| NFR workshop: named DRI, held within 2 weeks of approval, SLO table ratified | Targets are binding design targets until then. *(Owner: BSS Program Lead.)* | TBD |
| **Does browse need a separate serving store at all?** | Raised on inspection: `fr-cache-first-browse`'s rationale rested on two **uncalibrated** numbers, and the audience is a UI. NFR #1's p95 < 100 ms @ **10K SKUs/tenant** is a scale a direct multi-way query plausibly serves; NFR #2's **≥ 2,000 read QPS per tenant partition** is the only figure that would demand a projection, and it is not a portal number — a browse surface with human users does not reach it, so either the figure is uncalibrated or an unnamed machine consumer exists. The FR's rationale has been re-derived onto the two properties that do not depend on the numbers (availability split, structural stale-but-safe). **The workshop must answer two questions, not ratify a number:** (a) must browse survive a write-path outage — i.e. is the 99.9/99.5 split real? (b) does any named consumer exceed what a direct query serves at 10K SKUs/tenant? **Two "no"s make the projection removable** — and cheaply, since slice 08 is documents with no code: it would delete `asOfCatalogVersion`/`projectedAt` and **P-D-07** with it, NFR #7's staleness half, the convergence budget, the projector and its rebuild path, and `READ_MODEL_OVERLOADED`. `products_read_delivery_state` survives either way (it polls the broker's delivery/DLQ state, which is not in this gear's database). *(Owner: BSS Program Lead + Product; input from Presentation/Portals and Marketplace.)* | TBD |
| **The envelope's idempotency key has no source** | Design 01 §4.4 names an idempotency key on the broker envelope and nothing writes it: it cannot be the request's `Idempotency-Key`, which 01 §2 makes optional, so `fr-event-versioning-replay`'s within-window consumer dedup has no operand at all. Needs the producer-side source for the slot, alongside the `partition_id`/`seq` row above. *(Owner: event-broker / Common Core, with Design and slice 12. Filed from design 01 §6.)* | **CLOSED (P-D-47)**: the event `id` — a UUID the broker SDK mints once at enqueue and stores in the outbox row, so every delivery attempt of one event carries the same value; the request's `Idempotency-Key` was never the operand. |
| **Where does the audit row's correlation id come from?** | Design 01 §4.4 lists a correlation id on every audit row without the `nullable` marker its optional columns carry, and §6 repeats it as an envelope operand, while 01 §2's door contract names only `If-Match` and `Idempotency-Key` and no rule mints or reads one. §6 above requires it for break-glass audit. Needs: request header, W3C trace context, or gear-minted — and what a request carrying none stores. *(Owner: Platform, with Design. Filed from design 01 §6.)* | TBD |

## 16. Risks

| Risk | Impact | Mitigation |
|------|--------|------------|
| `SkuReferenceCount` signal slips | AC #2/#4/#18 stuck in fail-safe (no immutable-field correction on referenced SKUs) | Pre-approval gate on owner + date; break-glass bounds per-operation debt; the fail-safe tripwire (> 5/30d) bounds it in time |
| Region-containment semantics undefined | False rejects on parent-child scope checks (`fr-parent-child-integrity`) | Name uniqueness made region-independent (§15) — the algebra survives only as the containment subset-check; interim conservative fail-closed, pinned in Design |
| Snapshot lost/corrupted | Breaks byte-identical reproducibility for every contract frozen to it — a compliance event | Snapshots as financial records: ≥ 11 nines durability, periodic checksum restore verification, retention gated on live references (AC #44; NFR show-stopper #5) |
| Audit trail tamper-evidence deferred to a platform capability that does not yet exist (P-D-08) | A privileged in-database edit of an audit row is **undetectable**; rows written before activation are never retroactively provable, and the erasure-completeness argument (AC #35) rests on an unprovable immutability claim | Physical append-only (the trigger whitelist on both engines — **P-D-46**) as the interim control; the reserved seam makes activation a migration-free step; `seal_state = 'unsealed'` marks the unproven era in the data; requirements S1–S9 stated now so the platform build cannot land an incompatible field list (§15 open, owner Architecture) |
| Registry ↔ plan-price schema drift | Silent divergence breaks downstream binding/posting | Shared schema pin + CI seam contract test that fails closed (AC #36; NFR show-stopper #9) |
| Stuck cross-module freeze | Posting blocked or unsafe | `freezeComplete` fail-closed + idempotent re-trigger + governed force-completion (AC #21/#22) |
| Full-snapshot-per-publish cost growth | `CatalogVersion` storage/economics at 10K+ SKUs × frequent publishes | Batching-as-policy (FR `cpt-cf-bss-products-fr-catalog-version-publish`, D-47 increment-trigger taxonomy); publishes/day/tenant target + archival economics at the NFR workshop (NFR show-stoppers #5/#6) |
| Combined predecessor PRD left authoritative for registry | Duplicate/divergent catalog requirements | Refactor `PRD-product-catalog-marketplace` to Marketplace-only after approval (§15) |

## 17. Reference Materials

| **Material** | **Link** | **Comments** |
|--------------|----------|--------------|
| BSS Architecture Manifest | `docs/bss/manifest/vz-arch-manifest-bss-only.md` | §4.1 (registry) incl. the Decomposition (BSS realization) note = normative home of the §4.1 split; §4.4 posting immutability; §4.2/§4.3/§4.6/§4.8 consumers; §2.1.3; §7.2 events |
| Plan & Price Modeling | `docs/bss/prd/PRD-plan-price-modeling-202605281200/PRD-plan-price-modeling-202605281200.md` | Owns Plan/Price/PriceWindow/PriceList/Bundle composition/add-ons/billing descriptors/plan lifecycle; builds on this registry |
| Rating (evaluation core) | `gears/bss/rating/docs/PRD.md` | Price evaluation over the primitives plan-price authors |
| Rating (pipeline) | `gears/bss/rating/docs/PRD.md` | Consumer of metering-unit declaration and published SKU refs |
| Subscriptions — Lifecycle | `docs/bss/prd/PRD-subscriptions-lifecycle-202604021200/PRD-subscriptions-lifecycle-202604021200.md` | Consumes SKU refs + PlanTier + CatalogVersion; owns live-subscription migration |
| Product Catalog & Marketplace (predecessor) | `docs/bss/prd/PRD-product-catalog-marketplace-202601120119/PRD-product-catalog-marketplace-202601120119.md` | Combined §4.1+§4.8 predecessor; registry scope superseded here, Marketplace retained there |
| Project glossary | `docs/project-glossary.md` | Canonical terms |
| BSS ownership matrix | `gears/bss/rating/docs/SEAMS.md` (§ "Ownership matrix") | The cross-gear register of contested/adjacent responsibilities (e.g. `pricingSnapshotRef` → Tariffs). Products rows added: CatalogVersion+freeze, snapshot component, PlanTier, `sellable`, `SkuReferenceCount` |
| Trace chain | `AGENTS.md` (repository root) | Manifest → PRD → ADR → Design → Stories |

### 17.1 Configurable-Policy Interim Defaults

| Policy | Interim default (fail-safe) | Final owner |
|--------|-----------------------------|-------------|
| Materiality threshold | Any material-field change requires the tenant's configured approver quorum `N` (**default 2**, **floor 0** — P-D-11); affected-entity-count trigger ≥ 10; a single-entity non-material change requires `min(N, 1)`. `N` is reachable only by explicit configuration (absent ⇒ default, so `0` is never reached by omission), its initial value comes from tenant provisioning, and every later change to it is material under the then-current quorum. The FinanceReviewer predicate is **not** part of this policy — it governs who, not how many | Finance |
| Recognized tax-category & GL-code sets | Configured enum + GL chart; unknown codes rejected at authoring; new codes require elevated approval; de-list blocked while referenced; required at publish for product/service types | Finance |
| PlanTier taxonomy seed | Seeded with a neutral value (`standard`/`none`) + operator-defined tiers; tier identity is a stable code, rename = display-only | Product |
| Idempotency-key retention | ≥ 24h and ≥ the maximum freeze timeout | Eng/Design |
| `SkuReferenceCount` freshness threshold | 15 min; staler → conservative + alert | Architecture |
| Fail-safe break-glass tripwire | > 5 break-glass corrections / 30 days → escalate + signal delivery becomes release blocker | Architecture |
| Break-glass elevation window (§6.8 read-only sessions) | Time-boxed, interim **4 hours**, no renewal without a new session (design-interim, raised by the slice-05 review L-5) | Security/Architecture |
| Retirement / EOL lead-time | ≥ 30 days between event and effective hide | Subscriptions + Product |
| Recognized metering-unit set | Seeded with platform base units (`vCPU-hours`, `GB-storage`, `GB-egress`, `request-count`); new units require elevated approval; de-list blocked while referenced | Product + Rating |
| Retention — financial/version/audit | Retain to statutory maximum (not "indefinite") | Legal/Finance |
| Retention — operator PII | Pseudonymize at erasure request or a defined max age, whichever first | Legal/Finance |
| Read-model convergence | p99 < 2 s after write commit | Eng |
| Event propagation / fan-out | p99 < 3 s after publish | Eng |
| End-to-end posting-safe budget | p99 < 5 s (read converged and `freezeComplete`) | Eng |
| Cold `catalogVersionId` resolution | p95 < 2 s (looser than hot reads) | Eng |
| Snapshot durability & DR / RPO-RTO | ≥ 11 nines / replicated storage; periodic restore verification; RPO/RTO at the NFR workshop | Eng/Program |
| Numeric NFRs | Binding design targets until the NFR workshop (approval + 2 weeks); DRI = BSS Program Lead | Program/Eng |

### 17.2 Monetization-Model Traceability

| Monetization model | Where authored / evaluated |
|--------------------|----------------------------|
| flat, per-seat, tiered, volume, hybrid, commitment | `PRD-plan-price-modeling-202605281200` (authoring) + `gears/bss/rating/docs/PRD.md` (evaluation; post-ADR-0002 home of the former tariffs-pricing-logic PRD) |
| usage | Metering-unit **declaration** here (registry) + plan-level meter binding (plan-price) + rating (Rating) |

Absence of a monetization-model marker on a SKU is **intentional**, not a missing field.

---

*Child artifacts: ADR(s) for versioning/snapshot strategy and lifecycle/deprecation modeling; the gear's DESIGN (`gears/bss/products/docs/DESIGN.md` — canonical index + `design/` slice set + `DECISIONS.md` P-D register, started) for entity schemas, APIs, events, and read-model design; STORY documents per scope item. The §4.1 registry↔commercial decomposition is recorded in the manifest §4.1 Decomposition (BSS realization) note, not a separate ADR.*

