<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./02-taxonomy-attributes.md | Owners: BSS Product Catalog team -->

# DESIGN — SKU Classification (Slice 3)

<!-- toc -->

- [1. Context](#1-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
  - [1.5 Scope](#15-scope)
  - [1.6 Constraints & Assumptions](#16-constraints--assumptions)
  - [1.7 Naming & Design-Introduced Names](#17-naming--design-introduced-names)
  - [1.8 Context & Dependencies](#18-context--dependencies)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Type a SKU / evolve its classification](#type-a-sku--evolve-its-classification)
  - [Declare a metering unit](#declare-a-metering-unit)
  - [Govern the recognized-unit set](#govern-the-recognized-unit-set)
  - [Govern the PlanTier taxonomy & assign tiers](#govern-the-plantier-taxonomy--assign-tiers)
  - [Set accounting codes](#set-accounting-codes)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 `RecognizedSet` mechanics (shared by units, codes, tiers)](#31-recognizedset-mechanics-shared-by-units-codes-tiers)
  - [3.2 Error taxonomy (slice-owned codes)](#32-error-taxonomy-slice-owned-codes)
  - [3.3 The publish-time collector dependency](#33-the-publish-time-collector-dependency)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice owns everything that makes a SKU **classified and downstream-bindable**: the type
(`product`/`service`/`bundle`) with per-type required-field sets, the `sellable`
offering-eligibility flag (D-46), the `PlanTier` taxonomy and the SKU-level tier value, the
stable accounting codes (`taxCategory`, `glCode`) against their Finance-owned recognized sets,
and the **metering-unit declaration** — the one thing that makes a SKU a usage SKU — with its
`usageTypeRef` binding (P-D-05) and the recognized-unit set's own governed lifecycle. The three
vocabularies this slice governs (PlanTier taxonomy, recognized units, recognized codes) are
**governed live entities** reusing slice 02's `GovernedLiveOp` pattern verbatim.

### 1.2 Purpose

Downstream binds without re-validation (PRD §1.3) exactly when classification is enforced at
authoring: an unpostable SKU (missing code), an unrateable meter (unrecognized unit, dangling
`usageTypeRef`), or an unclassifiable plan (unknown tier) must be impossible to publish — not
discovered weeks later at ERP export or rating time.

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-product-manager` | Types SKUs, declares meters, assigns tiers |
| `cpt-cf-bss-products-actor-finance-reviewer` | Owns the recognized code sets; second approver on finance-material fields |
| `cpt-cf-bss-products-actor-plan-price` | Consumes type/`sellable`/tier/meter via the SDK read shape; enforces predicate 6 and plan-publish tier presence |
| `cpt-cf-bss-products-actor-oss-metering` | The usage-collector: SoR of the UsageType catalog `usageTypeRef` resolves against |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.3 (all six FRs); AC #2a, #7–#11; AC #38 (unit rows)
- [`../DECISIONS.md`](../DECISIONS.md) P-D-02 (bundle override at entity publish), P-D-05
  (`usageTypeRef` resolvability only)
- Pricing D-46 (`sellable` → sellability-gate predicate 6) and `inst-cmp-usagetype`
  (the UC3(c) dimension check's home) — pricing register/design
- [`./02-taxonomy-attributes.md`](./02-taxonomy-attributes.md) §3.1 — the `GovernedLiveOp`
  pattern this slice reuses

### 1.5 Scope

**In**: type + per-type required-field validators; `sellable`; `PlanTier` taxonomy + SKU value;
recognized code sets + code validators; recognized unit set + metering-unit declaration +
`usageTypeRef` resolution; unit de-listing; the uncomposed-bundle publish override registration
(P-D-02); mutability-bucket registration for every field this slice owns.

**Out**: bundle composition and `compositionPending` clearing (06); the fresh-zero correction
door for bucket-ii fields (07); plan-side enforcement — predicate 6, tier presence, dimension
subset (pricing); usage collection (collector); the approval machinery (05).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | A usage SKU is **defined**, not detected: declaring a unit is what makes it one; no separate flag exists | PRD glossary "Usage SKU" |
| C2 | Exactly one unit per declaration — the counted identity; dimension sets are plan-price's (P-D-05); a composite meter declares its **output** unit | PRD `fr-metering-unit-declaration` |
| C3 | Unit identity/semantics immutable (no silent GB→GiB); correction = new unit + deprecate old | PRD `fr-metering-unit-delisting` |
| C4 | Codes only, never computation: no tax math, no GL posting here | PRD `fr-accounting-codes` |
| C5 | `PlanTier` ≠ OrgTier; tier identity is the stable code, rename is display-only | PRD `fr-plantier-classification` |
| C6 | Bucket registration (PRD mutability matrix): `type`, metering-unit declaration (incl. `usageTypeRef`) → **bucket ii** (immutable-but-correctable, slice 07); `PlanTier`, `taxCategory`, `glCode`, `sellable` → **bucket iii** (material-mutable) | PRD `fr-field-mutability-matrix` |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `TypeProfile` | The per-type required-field set the define/publish validators run (`product`/`service`: accounting codes required at publish; `bundle`: exempt from codes, subject to the override gate) |
| `MeterDeclaration` | The value object `(unit, usageTypeRef)` — always both or neither |
| `RecognizedSet` | The generic governed vocabulary (units; tax categories; GL codes) with `active|deprecated` states and reference-guarded removal |
| `UsageTypeResolver` | The publish-time port to the usage-collector's `get_usage_type` (P-D-05) |

### 1.8 Context & Dependencies

**Consumed**: Foundation doors/pipeline (01); `GovernedLiveOp` (02); slice-05 gate (materiality
of bucket-iii fields; elevated approval for new units; the bundle override); usage-collector SDK
(`get_usage_type`). **Produced**: `PlanTierUpdated`, `RecognizedUnitUpdated`,
`RecognizedCodeUpdated` events; the classification validators registered on SKU save/publish;
the SDK read shape fields (`type`, `sellable`, `plan_tier`, `metering_unit`, `usage_type_ref`,
`tax_category_ref`, `gl_code_ref`) — including **`sellable`, the member pricing's `CatalogSku`
currently lacks** (consumer-side addition owed there).

## 2. Actor Flows (CDSL)

### Type a SKU / evolve its classification

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-classify-sku`

1. [ ] - `p1` - `TypeProfile` validators register on SKU save and publish: `type` present and in the closed set; per-type required fields at publish — `product`/`service` require both accounting codes (`ACCOUNTING_CODE_REQUIRED` naming the missing one), `bundle` requires neither (composition is pricing's; a bundle is commercially incomplete by design) - `inst-cl-type-profile`
2. [ ] - `p1` - Promotional/$0/"Free" offerings are ordinary SKUs — no separate entity, no special validator path (PRD `fr-define-sku`) - `inst-cl-no-promo-entity`
3. [ ] - `p1` - `sellable` defaults `true`; flipping it is a bucket-iii edit — a head-row save re-published as version N+1 (01's head-row model) under slice-05 materiality; the SDK read shape exposes it per `CatalogVersion` so pricing's predicate 6 has its operand - `inst-cl-sellable`
4. [ ] - `p1` - **Uncomposed-bundle publish override (P-D-02)**: publishing a `bundle` that plan-price has not composed requires the explicit two-person override at THIS entity publish — `N`-governed with `quorumReduced` recorded, the author performing the acknowledgment at `N = 0` (P-D-13) — this slice registers the gate condition, slice 05 executes the override ceremony (lint findings presented to approvers). **The condition is registered on the publish, not on the lane**: every lane that publishes a `bundle` carries it, bulk included (09 `inst-bk-override`), and the published SKU enters flagged `compositionPending = true` (cleared by slice 06's inbound signal) - `inst-cl-bundle-override`

### Declare a metering unit

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-declare-meter`

1. [ ] - `p1` - A `MeterDeclaration` is atomic: `unit` and `usageTypeRef` together or not at all (`METER_DECLARATION_INCOMPLETE`); exactly one unit (C2) - `inst-mt-atomic-pair`
2. [ ] - `p1` - The unit **MUST** be in the recognized-unit set and `active`: unknown fails `UNRECOGNIZED_UNIT` (the path to a new unit is `RecognizedSet` elevated approval, never inline); a `deprecated` unit fails new declarations (`UNIT_DEPRECATED`) — including a draft whose unit was deprecated before its first publish (PRD: treated as a new declaration and rejected) - `inst-mt-recognized`
3. [ ] - `p1` - At publish, `UsageTypeResolver` **MUST** resolve `usageTypeRef` in the collector's platform-global catalog (P-D-05 — resolvability only, no lifecycle check, no dimension check): unresolvable fails `USAGE_TYPE_UNRESOLVED`; **collector unavailable fails closed** with the distinct retryable `USAGE_TYPE_UNAVAILABLE` — a publish never proceeds on an unverified binding - `inst-mt-resolve`
4. [ ] - `p2` - The declaration is bucket ii: immutable after publish, correctable only through slice 07's fresh-zero door; the draft plane edits freely - `inst-mt-bucket`

### Govern the recognized-unit set

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-unit-set`

1. [ ] - `p1` - The set is a `RecognizedSet` (governed live entity via `GovernedLiveOp`): seeded per PRD §17.1 (`vCPU-hours`, `GB-storage`, `GB-egress`, `request-count`); adding a unit = **elevated approval** (slice-05 gate, FinanceReviewer not required — owner is Product + Rating per §15) - `inst-us-governed`
2. [ ] - `p1` - De-listing: removal refused while ≥ 1 **published** SKU declares the unit (`UNIT_DELIST_BLOCKED`, holders sampled); the admitted path is `deprecated` (no new declarations, existing publishes unaffected) then removal once unreferenced — where "referenced" means **non-terminal published heads** (published/deprecated SKUs); frozen version content is self-contained and never blocks removal (operand narrowed with slice 02's — M2 fix, 2026-08-25 review) - `inst-us-delist`
3. [ ] - `p1` - Unit semantics are immutable (C3): there is no rename/redefine op at all on this set — the absence of the door is the enforcement; a correction is a new unit + deprecation, and the audit trail ties them via the `GovernedLiveOp` payload - `inst-us-immutable`
4. [ ] - `p1` - De-listing/deprecation never mutates any frozen snapshot (append-only posture, 01 C5) - `inst-us-snapshots`

### Govern the PlanTier taxonomy & assign tiers

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-plantier`

1. [ ] - `p1` - The taxonomy is a `RecognizedSet` variant with a display label: identity = the **stable tier code**; rename touches the label only (display-only by construction — the code column has no update path); seeded with the neutral value (`standard`, §17.1) - `inst-pt-stable-code`
2. [ ] - `p1` - Taxonomy ops (add/rename/retire) are governed (`GovernedLiveOp`, two-person) and emit `PlanTierUpdated`; retiring a value is refused while any `published` SKU carries it (`PLAN_TIER_RETIRE_BLOCKED`) — deprecate-then-retire, same shape as units - `inst-pt-governed`
3. [ ] - `p1` - The SKU-level value validates against the taxonomy at save: unknown fails `PLAN_TIER_UNKNOWN`, a **`deprecated` tier blocks NEW assignment** (`PLAN_TIER_DEPRECATED` — parity with `UNIT_DEPRECATED`; existing published carriers unaffected — added by the slice-11 review H1); it is bucket iii (material-mutable, finance-material per PRD — FinanceReviewer in the approval); presence enforcement at **plan** publish is pricing's, not re-checked here - `inst-pt-assign`

### Set accounting codes

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-accounting-codes`

1. [ ] - `p1` - `taxCategory` and `glCode` each validate against their `RecognizedSet` (owner Finance; unknown fails `ACCOUNTING_CODE_UNKNOWN`); the sets follow the same governed lifecycle (elevated add, deprecate-then-remove, reference-guarded) - `inst-ac-recognized`
2. [ ] - `p1` - Required at publish for `product`/`service` types (via `TypeProfile`, flow 1); both are bucket iii finance-material — ≥ 1 FinanceReviewer in the `N`-governed approval (slice 05 role predicate). **At `N = 0` the predicate is recorded `predicateUnsatisfiable` rather than blocking (P-D-11)**: this very rule is the operand P-D-11's amendment names — `taxCategory` being required at publish for `product`/`service` types is what would otherwise have left the one-person tenant unable to publish their first such SKU **forever**, which is the block that decision exists to remove - `inst-ac-required`
3. [ ] - `p1` - No computation: the columns are opaque codes to this gear (C4) - `inst-ac-codes-only`

## 3. Processes / Business Logic

### 3.1 `RecognizedSet` mechanics (shared by units, codes, tiers)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-recognized-set`

1. [ ] - `p1` - One generic shape: `(tenant_id, set_kind, member_code, display_label?, state ∈ {active, deprecated}, seeded_by?)`; mutations ride `GovernedLiveOp` (02 §3.1); membership checks are the classification validators' single lookup - `inst-rs-shape`
2. [ ] - `p1` - Removal operand is uniform: **non-terminal published heads** — a member is removable when no live head references it; frozen versions are self-contained copies, neither blocking removal nor touched by it (M2 fix); the pre-publish lint (P-D-02: informational) surfaces `deprecated`-member usage so operators see debt before refusal teaches them - `inst-rs-removal-operand`
3. [ ] - `p2` - Seeded members (`seeded_by` set) are deprecatable but not removable — the platform baseline survives tenant edits, mirroring slice 02's `WellKnownSeed` rule - `inst-rs-seeded`

### 3.2 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-classification-errors`

`SKU_TYPE_UNKNOWN` (raised by `inst-cl-type-profile` when `type` is absent or outside the closed set — named 2026-08-26; the rule described the check and named no code), `ACCOUNTING_CODE_REQUIRED`, `ACCOUNTING_CODE_UNKNOWN`,
`METER_DECLARATION_INCOMPLETE`, `UNRECOGNIZED_UNIT`, `UNIT_DEPRECATED`, `USAGE_TYPE_UNRESOLVED`,
`USAGE_TYPE_UNAVAILABLE` (retryable, fail-closed), `UNIT_DELIST_BLOCKED`, `PLAN_TIER_UNKNOWN`, `PLAN_TIER_DEPRECATED`,
`PLAN_TIER_RETIRE_BLOCKED`, `BUNDLE_OVERRIDE_REQUIRED` (the interactive refusal of `inst-cl-bundle-override`, the P-D-02 gate's API behaviour — named 2026-08-26; the bulk analogue already had `BULK_OVERRIDE_UNACKNOWLEDGED`). Registered into 01 §3.3; the AC #38
rows "unrecognized metering unit without elevation" and "authoring/cloning against a
de-listed/deprecated unit" map here.

**Problem responses (RFC 9457):** `UNIT_DELIST_BLOCKED`, `PLAN_TIER_RETIRE_BLOCKED` (409); `SKU_TYPE_UNKNOWN`, `ACCOUNTING_CODE_REQUIRED`, `ACCOUNTING_CODE_UNKNOWN`, `METER_DECLARATION_INCOMPLETE`, `UNRECOGNIZED_UNIT`, `UNIT_DEPRECATED`, `USAGE_TYPE_UNRESOLVED`, `PLAN_TIER_UNKNOWN`, `PLAN_TIER_DEPRECATED`, `BUNDLE_OVERRIDE_REQUIRED`, `BULK_OVERRIDE_UNACKNOWLEDGED` (422); `USAGE_TYPE_UNAVAILABLE` (503).

*Statuses added 2026-08-26, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, 2026-08-02, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"* — the citation was right the first time;
a 2026-08-26 pass re-pointed it at D-186 and was wrong to, D-186 being a later amendment scoped to
one config route) and where an earlier pass here wrongly wrote
412 and called that pricing's convention — **403** where the caller may not perform the act at
all, **404** only where a path segment names a resource this tenant has none of, **503** where retry
is the remedy. **The 422s here are architectural, not wire** — see 01 §3.3, which quotes the
platform rule: no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 in `OpenAPI`. Proposed per
row and open to correction; the requirement is that every code carries one.
  Codes listed here for the response map but **declared elsewhere**: `BULK_OVERRIDE_UNACKNOWLEDGED` (slice 09) — the status is repeated, not a second declaration, so the one-declaration rule stands.*

### 3.3 The publish-time collector dependency

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-collector-dependency`

1. [ ] - `p1` - `UsageTypeResolver` is the gear's only synchronous cross-gear call inside a publish pipeline; it runs **once per publish** (not per validator), with a short timeout and no retry inside the transaction — on timeout the publish fails `USAGE_TYPE_UNAVAILABLE` and the operator retries the publish (idempotent by 01 §3.2). **On the scheduled lane there is no operator, so the code is explicitly retryable there too** (item 37 of the 2026-08-26 review): 04 `inst-ar-failure` makes anything but `STALE_REVISION`/`APPROVAL_REQUIRED` terminal, which burned a pinned approval on a transient collector blip. `USAGE_TYPE_UNAVAILABLE` therefore joins the runner's **`deferred`** set, not its `failed` set — re-evaluated on the runner's own cadence, bounded by the transition's own attempt budget before it lands `failed` - `inst-cd-once`
2. [ ] - `p2` - The resolved `(gts_id, kind, metadata_fields)` snapshot is stamped into the audit row of the publish — the record of *what the binding resolved to* at approval time, which is what the §15 deletion-negotiation and pricing's `meter_binding_divergent` remediation will reference - `inst-cd-stamp`

## 4. Data / Storage (normative shape; DDL in migrations)

- **Columns on `products_sku`** (carried by 01 §4.2, rules owned here): `type`, `sellable`,
  `plan_tier` (FK by code into the tier set), `tax_category_ref`, `gl_code_ref`,
  `metering_unit`, `usage_type_ref` — with a CHECK that `metering_unit` and `usage_type_ref`
  are both null or both non-null (`inst-mt-atomic-pair`'s physical floor).
- **`products_recognized_set`** — the generic table of §3.1: PK `(tenant_id, set_kind,
  member_code)` with `set_kind ∈ {metering_unit, tax_category, gl_code, plan_tier}`;
  `display_label` used by `plan_tier` (and ignored elsewhere); `state`; `seeded_by`.
  Append-only discipline: no UPDATE of `member_code` ever (trigger whitelist admits `state`
  and `display_label` only).
- **Events**: `PlanTierUpdated` (PRD-named), `RecognizedUnitUpdated`, `RecognizedCodeUpdated` —
  broker-native, ordering key `(tenant, set_kind)`; classification edits on a SKU ride the
  Foundation entity events (explicit "no event" for per-field changes).

## 5. Testing posture (slice-local)

- Every refusal in §2 paired with its positive control; the bundle-exempt-from-codes case is a
  named probe (the exemption is the easy thing to lose).
- `UsageTypeResolver` probed against a stub collector: resolved / unknown / timeout — three
  distinct outcomes, the timeout one asserting the publish is retryable and idempotent.
- Tier-retire and unit-delist guards probed both ways: a **deprecated head** still blocks; a
  value alive only in frozen `products_entity_version` content does **not** block (the
  M2-narrowed operand), and the old snapshot still renders after removal.
- The `(metering_unit, usage_type_ref)` CHECK gets its CorruptRow probe on both engines.
- A `sellable` flip probed end-to-end to the SDK read shape (the pricing `CatalogSku` gap is
  consumer-side, but our shape must already carry the member).

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-fr-define-sku` (typing/classification half — identity carrier is slice 01's), `cpt-cf-bss-products-fr-sku-sellable`, `cpt-cf-bss-products-fr-metering-unit-declaration`,
`cpt-cf-bss-products-fr-metering-unit-delisting`, `cpt-cf-bss-products-fr-plantier-classification`, `cpt-cf-bss-products-fr-accounting-codes`; AC #2a,
#7 (typing clauses; identity/link clauses = 01), #8–#11; AC #38 (unit rows); P-D-02 (override registration), P-D-05 (resolver semantics).

**Risks & open items**:
- **Recognized-set owners + add/de-list workflows** are PRD §15 opens (Finance for codes,
  Product + Rating for units) — the `GovernedLiveOp` machinery is ready either way; only the
  approver-role predicates per set await the owners' sign-off.
- **Collector in the publish path**: a synchronous dependency bounds publish availability by
  collector availability for usage SKUs; acceptable at authoring rates, but slice 08/12 should
  surface resolver latency in the publish SLO breakdown.
- **UsageType deletion** (PRD §15, P-D-05 residue): `inst-cd-stamp` gives the remediation path
  its evidence, but the negotiation with the collector is still open.
- **`sellable` member missing in pricing's `CatalogSku`** — owed consumer-side; our SDK shape
  carries it from day one so the fix stays additive.
