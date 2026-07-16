<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Price Structure & Model Kinds (Design, Slice 3) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ./01-foundation.md | Owners: BSS Product Catalog team -->

# DESIGN — Price Structure & Model Kinds (Slice 3)

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
  - [Author Price Rows](#author-price-rows)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Model-Kind Validation](#model-kind-validation)
  - [Tier-Band Validation](#tier-band-validation)
  - [Package Pricing Validation](#package-pricing-validation)
  - [Conformance Fixture Gate](#conformance-fixture-gate)
- [4. States (CDSL)](#4-states-cdsl)
  - [Price Row State Machine](#price-row-state-machine)
- [5. API Surface](#5-api-surface)
- [6. Data Model](#6-data-model)
- [7. Events & Alarms](#7-events--alarms)
- [8. Definitions of Done](#8-definitions-of-done)
  - [Explicit Model Kind](#explicit-model-kind)
  - [Tier Bands](#tier-bands)
  - [Package Pricing](#package-pricing)
  - [Conformance Gate](#conformance-gate)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Non-Functional Considerations](#10-non-functional-considerations)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice owns the **structure of a price row**: the explicit `modelKind`
(`flat | per_unit | graduated | volume | package`), tier-band validation under the
`[fromQty, toQty)` convention, package (block) pricing fields, the usage evaluation-policy
placement rules (`tierAggregationWindow`, `billingGranularity`), and the **joint
golden-conformance-fixture gate** that blocks publish of any `modelKind` Tariffs cannot
provably evaluate. Rows live on the Foundation's canonical scope key and publish through the
Foundation pipeline; the **math is never computed here** — Tariffs applies the formula per
the §17.2 conformance mapping.

**Traces to**: `cpt-cf-bss-pricing-fr-model-kind`, `cpt-cf-bss-pricing-fr-tier-validation`,
`cpt-cf-bss-pricing-fr-package-pricing`, `cpt-cf-bss-pricing-fr-model-kind-conformance`,
`cpt-cf-bss-pricing-fr-price-amount-validation`, `cpt-cf-bss-pricing-fr-per-seat`

### 1.2 Purpose

Guarantee that every published price row is **unambiguously evaluable**: the model kind is
explicit and frozen (no rating-time default), tier bands cannot overlap, gap, or close the top (any quantity is always rateable — D-17), package fields are structurally exclusive with tier fields, and no kind
publishes without a version-controlled joint fixture proving catalog↔Tariffs agreement —
eliminating the class of silent mispricing bugs at band edges and kind mismatches.

### 1.3 Actors

| Actor | Role in Slice |
|-------|---------------|
| `cpt-cf-bss-pricing-actor-finance-manager` | Authors amounts, model kinds, tier bands |
| `cpt-cf-bss-pricing-actor-rating` | Consumes `modelKind` + bands + evaluation policy; resolves policy fields deterministically; co-owns golden fixtures (consolidated gear — rating ADR-0002) |
| `cpt-cf-bss-pricing-actor-subscriptions` | Supplies `subscription_seat_count` for `per_unit` rows at rating time |

### 1.4 References

- **PRD**: [PRD.md](../PRD.md) — §6.2, §17.1 (structure kinds), §17.2 (conformance mapping), §17.4 (validation rules)
- **Design**: [01-foundation.md](./01-foundation.md) — scope key (§4.1), publish contract (§4.2), money constraint
- **Dependencies**: Foundation (Slice 1); co-required with plan-definition (Slice 2) — the cycle shape decides which `chargeKind` rows a plan needs; reserved-capacity attributes extend the usage row in Slice 10.

### 1.5 Scope

**In scope**: `modelKind` enum + explicitness; `[fromQty, toQty)` band validation (ascending,
non-overlapping, contiguous, always-open top — D-17); `package` fields + structural exclusivity;
`per_unit` `quantitySource` persistence; evaluation-policy placement (usage-only fields);
amount/currency/precision validation (delegating the shared checks to the Foundation);
the conformance-fixture publish gate.

**Out of scope**: formula evaluation, graduated-vs-volume math, round-up math (Tariffs);
tier resets / `Q` derivation semantics (Tariffs; catalog persists the window enum only);
reserved-rate attributes and derived meters (Slice 10); price overlays/overlays (Slice 9);
windows (Slice 7).

### 1.6 Constraints & Assumptions

Inherits Foundation C-set. Slice-3-specific:

| # | Topic | Assumption (default) | Source |
|---|-------|----------------------|--------|
| Q1 | Band convention | Half-open `[fromQty, toQty)`; the top band is **always open** (`toQty = null`) — a closed top fails publish (`TIER_TOP_CLOSED`, D-17): quantity capping is owned by entitlement **quotas** (Subscriptions enforces), per-period fee caps are Tariffs Future | PRD §6.2; D-17 |
| Q2 | Aggregation derivation | `tierAggregationWindow` defines **when** `Q` resets; derivation is the row's authorable `aggregationFunction ∈ {sum (default), peak, time_weighted}` (**D-44**, launch): non-`sum` folds gauge samples per `aggregationGranularity {hour, day}` granule (max / step-integral) and `Q` = Σ granule folds — **additive**, so band math, supersession continuity, and `bandOffsetQ` are untouched; frozen in `pricingSnapshotRef`; `last`/`unique` Future; no composite co-occurrence at launch | PRD §1.4; D-44; rating T-D-17 |
| Q3 | Volume semantics | Catalog `volume` maps to Tariffs **Variant A only** (single rate on total `Q`); Variant B (per-tier block fee) is dropped and not authorable | PRD §17.2 |
| Q4 | Fixture repo | Golden fixtures are version-controlled in a shared catalog+Tariffs repo **before code**; the publish gate reads a per-tenant-independent fixture registry | PRD §13 |
| Q5 | Zero amounts | `0` is a valid amount (free tier, `trial`/`intro`, first graduated band); negatives rejected (typed credit rows are Future) | PRD §17.4 |
| Q6 | Included allowance | Authored `includedAllowance {quantity, rolloverPolicy}` (D-45) **compiles at publish**: `none` → `$0` first band `[0, N)` + frozen first-class marker (band math unchanged); `carry` → D-43 per-period promotional grant (Billing executes; no catalog balance). `sum` rows only; never combined with an authored `$0` first band (double-free, publish-blocked) | PRD §1.4/§6.10; D-45 |

### 1.7 Naming & Design-Introduced Names

Reuses the PRD glossary; inherits Foundation mechanics. Not restated.

Design-introduced names (Slice 3):

| Name | Meaning |
|------|---------|
| `ModelKindValidator` | Registered rules: explicit kind, kind-specific required/forbidden fields |
| `TierBandValidator` | Registered rules: ordering, non-overlap, contiguity, top-band policy under Q1 |
| `PackageValidator` | Registered rules: `packageSize`/`packagePrice` presence + structural exclusivity with tier-band fields |
| `FixtureGate` | The publish-time check that the row's `modelKind` (and reservation variant) has a green joint golden fixture |

### 1.8 Context & Dependencies

```mermaid
flowchart TB
    subgraph s3["Slice 3 — Price Structure"]
        MKV["ModelKindValidator"]
        TBV["TierBandValidator"]
        PKV["PackageValidator"]
        FXG["FixtureGate"]
    end
    FIX[("Joint golden fixtures<br/>(shared catalog + Tariffs repo)")]
    FND["Foundation (Slice 1)<br/>ScopeKey · ValidationPipeline · ReadModelProjector"]
    TRF["Tariffs / PLAL<br/>formula evaluation (§17.2)"]
    FIX --> FXG
    MKV --> FND
    TBV --> FND
    PKV --> FND
    FXG --> FND
    FND --> TRF
```

**Consumed:** the fixture registry (Q4). **Produced:** the price-structure portion of the read
model — `modelKind`, ordered bands, `packageSize`/`packagePrice`,
`quantitySource`, evaluation-policy fields — frozen in `pricingSnapshotRef` for Tariffs.

## 2. Actor Flows (CDSL)

### Author Price Rows

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-flow-price-author`

**Actor**: `cpt-cf-bss-pricing-actor-finance-manager`

**Success Scenarios**:
- A draft price row is authored on the canonical scope key with an explicit `modelKind` and its kind-specific fields; `PriceCreated` emits per row
- Tiered rows carry ordered `[fromQty, toQty)` bands; usage rows carry `billingGranularity` (+ `tierAggregationWindow` when tiered); a tiered row MAY additionally carry the Slice-10 `tierQualificationWindow` primitive (`current` \| `trailing_month`, D-40 / [`design/10-advanced-primitives.md`](./10-advanced-primitives.md)) — a **third, orthogonal** window that qualifies the rate tier from the prior period and locks it, distinct from `tierAggregationWindow` (counter reset) and `billingGranularity` (billing cadence)

**Error Scenarios**:
- Duplicate active scope key without supersession → `DUPLICATE_SCOPE_KEY` (409, Foundation)
- Tiered row without a `modelKind` → `MODEL_KIND_MISSING` (422)
- Precision above the currency's ISO 4217 minor unit → `PRECISION_EXCEEDED` (422, Foundation)

**Steps**:
1. [ ] - `p1` - API: POST /v1/pricing/plans/{planId}/prices (draft row; idempotency key honored; scope-key axes defaulted by the Foundation `ScopeKey`) - `inst-pr-create`
2. [ ] - `p1` - Persist `modelKind` + kind-specific fields (bands / package / `quantitySource`); shared amount/currency/precision checks run in the Foundation - `inst-pr-fields`
3. [ ] - `p1` - PATCH while `draft`; published rows are append-only (change = supersession, Foundation §4.3) - `inst-pr-mutate`
4. [ ] - `p1` - **RETURN** 201 (draft row, ETag). **Validation split (D-21):** all **row-local** checks run at save *and* re-run at publish — model-kind shape (explicit kind, kind×chargeKind matrix, required/forbidden fields), band-set geometry (ordering, overlap, gap/contiguity, zero-width, open top), precision, evaluation-policy placement, scope-key duplication (PRD AC #12's "save/publish MUST fail" for band geometry is satisfied at save). **Aggregate/cross-entity** checks run at publish only: fixtures, window coverage, phase coverage, hybrid completeness, meter injectivity - `inst-pr-return`

## 3. Processes / Business Logic (CDSL)

### Model-Kind Validation

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-model-kind`

**Input**: a draft price row at publish
**Output**: pass, or enumerated fail-closed violations

**Steps**:
1. [ ] - `p1` - `modelKind ∈ {flat, per_unit, graduated, volume, package}` MUST be explicit; a tiered row with no kind MUST NOT publish ("tiered (unspecified)" is not publishable, §17.1); no implicit default exists at rating time - `inst-mk-explicit`
2. [ ] - `p1` - **Kind-specific required fields**: `per_unit` → unit price + `quantitySource` (`subscription_seat_count | manual`, and the fixed quantity for `manual`); `graduated`/`volume` → ≥ 1 tier band; `package` → `packageSize`/`packagePrice`; `flat` → single amount - `inst-mk-required`
3. [ ] - `p1` - **Kind-specific forbidden fields**: tier-band fields absent on `flat`/`per_unit`/`package`; `tierAggregationWindow`/`billingGranularity` are **usage-row only** — presence on `flat` non-usage or `per_unit` rows fails publish (§17.4 evaluation-policy placement) - `inst-mk-forbidden`
3a. [ ] - `p1` - **Kind×chargeKind matrix (D-18)**: `graduated`/`volume`/`package` are valid **only on `chargeKind = usage`** rows — a tiered/package model on a `recurring`/`one_time`/`one_time_setup` row fails publish (`MODEL_KIND_CHARGEKIND_MISMATCH`): the tier machinery presupposes a metered quantity stream, and no `Q` semantics exist for non-usage rows. Tiered per-seat pricing (bands over seat count on recurring rows) is Future scope (§17.8) - `inst-mk-chargekind`
4. [ ] - `p1` - The catalog computes **no** charge: kinds are flags Tariffs maps to formulas one-to-one per §17.2; catalog `volume` = Variant A only (Q3) - `inst-mk-nocompute`

### Tier-Band Validation

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-tier-bands`

**Input**: a `graduated`/`volume` row's band set
**Output**: an ordered, gapless, non-overlapping band set in the read model (+ top-band policy)

**Steps**:
1. [ ] - `p1` - Bands sorted ascending by `fromQty`; any overlap fails; any gap fails (contiguity under `[fromQty, toQty)`: next `fromQty` = previous `toQty`); each band MUST satisfy `toQty > fromQty` when `toQty` is non-null (`TIER_BAND_EMPTY`); an advisory (non-blocking) warning is emitted when any band's effective unit price exceeds the previous band's (non-volume-discount pattern) — carried in the Foundation validation report's `warnings[]` channel - `inst-tb-order`
2. [ ] - `p1` - First band starts at the row's quantity origin (`fromQty = 0`); a `$0` first band is valid (Q5 — the included-allowance representation) - `inst-tb-first`
3. [ ] - `p1` - **Top band is always open (D-17)**: `toQty = null` REQUIRED on the top band; a closed top fails publish (`TIER_TOP_CLOSED`) — "price undefined above X" is never the commercial intent: quantity capping is an entitlement **quota** (grant set; Subscriptions enforces), per-period fee caps are Tariffs Future (§17.8), and a different price above X is simply another band. Any quantity is therefore always rateable on a tiered row — "sold but unrateable" is impossible by construction - `inst-tb-top`
4. [ ] - `p1` - Tiered usage rows MUST carry `tierAggregationWindow` (`calendar_month | invoice_period | subscription_lifetime | per_event`); derivation is window-sum only at launch (Q2) - `inst-tb-window`
5. [ ] - `p1` - **Band units (normative):** `fromQty`/`toQty` are expressed in **billable units after `billingGranularity` quantization** (e.g. `per_hour` → band quantities are hours, never raw seconds); the read model documents the unit so catalog and Tariffs cannot diverge on it - `inst-tb-units`
6. [ ] - `p1` - **Window continuity across supersession (normative):** the tier counter `Q` is derived per **`(subscription, meter, window)`** — it belongs to the subscription's usage history, **not** to a price-row version. Superseding a row (new bands, new price) does **NOT** reset an in-window counter, and `subscription_lifetime` `Q` in particular survives every supersession/versioning; the new row's bands are simply applied to the continued `Q`. Requires its own joint golden fixture (a supersession mid-window scenario) in the Slice 3 conformance registry - `inst-tb-window-continuity`

### Package Pricing Validation

- [ ] `p2` - **ID**: `cpt-cf-bss-pricing-algo-package`

**Input**: a `modelKind=package` row
**Output**: `packageSize`/`packagePrice` in the read model; structural exclusivity enforced

**Steps**:
1. [ ] - `p2` - `packageSize > 0` (units per block) and `packagePrice ≥ 0` (per block) MUST be present; tier-band fields MUST be absent; publish rejects otherwise - `inst-pk-fields`
2. [ ] - `p2` - The round-up math (`blocks = ceil(used / packageSize)`, `charge = blocks × packagePrice`) is **Tariffs-owned**; the read model exposes the two fields only - `inst-pk-math`

### Conformance Fixture Gate

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-fixture-gate`

**Input**: a publishing row's `modelKind` (and, from Slice 10, its reservation variant)
**Output**: pass, or a publish block naming the missing fixture

**Steps**:
1. [ ] - `p1` - The `FixtureGate` resolves the row's kind against the **joint golden conformance fixture registry** (Q4); publish of any `modelKind` lacking a green joint fixture is **blocked** - `inst-fx-gate`
2. [ ] - `p1` - `package` (repeating-block) and `per_unit` (external-quantity) each require their own joint fixture before first publish; the reservation variant of a usage row requires its own fixture (Slice 10 registers it into this gate) - `inst-fx-kinds`
3. [ ] - `p1` - The §17.2 table is the **single kind-to-formula source of truth**; catalog and Tariffs MUST NOT diverge from it — the gate is the enforcement point on the catalog side - `inst-fx-sot`

## 4. States (CDSL)

### Price Row State Machine

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-state-price-row`

**States**: draft, published, superseded
**Initial State**: draft (mutable, deletable)

**Transitions**:
1. [ ] - `p1` - **FROM** draft **TO** published **WHEN** the Foundation pipeline (incl. this slice's validators + the `FixtureGate`) passes and the publish commits; the row becomes append-only - `inst-ps-publish`
2. [ ] - `p1` - **FROM** published **TO** superseded **WHEN** a supersession within the same canonical scope key closes this row's window and opens the successor's (Foundation §4.3; no in-place mutation, no overlap) - `inst-ps-supersede`
3. [ ] - `p1` - There is no deleted state for published rows; only never-published `draft` rows are deletable - `inst-ps-nodelete`

## 5. API Surface

| Method | Path | Purpose | Idempotency |
|--------|------|---------|-------------|
| `POST` | `/v1/pricing/plans/{planId}/prices` | Create a draft price row on the scope key | client idempotency key |
| `PATCH` | `/v1/pricing/plans/{planId}/prices/{priceId}` | Update a draft row | ETag |
| `DELETE` | `/v1/pricing/plans/{planId}/prices/{priceId}` | Delete a **draft** row (published rows: 409) | — |
| `GET` | `/v1/pricing/plans/{planId}/prices` | List rows (draft for authors; published via read model) | — |

**Problem responses (RFC 9457):** `MODEL_KIND_MISSING` (422), `TIER_BANDS_OVERLAP` /
`TIER_BANDS_GAP` (422), `TIER_BAND_EMPTY` (422 — `toQty ≤ fromQty` on a non-open band),
`TIER_TOP_CLOSED` (422 — the top band must be open; capping belongs to quotas / per-period caps, D-17), `PACKAGE_FIELDS_INVALID` (422),
`EVAL_POLICY_MISPLACED` (422), `MODEL_KIND_CHARGEKIND_MISMATCH` (422 — `graduated`/`volume`/`package` on a non-usage row, D-18), `EVAL_POLICY_MISSING` (422 — `tierAggregationWindow` unset on a
tiered usage row, or `billingGranularity` unset on a usage row; the error references the
allowed values per the PRD Glossary), `QUANTITY_SOURCE_MISSING` (422), `FIXTURE_MISSING` (422),
`DUPLICATE_SCOPE_KEY` (409 — Foundation-owned, referenced here), `PRECISION_EXCEEDED` (422).
The publish-time report enumerates all violations.

## 6. Data Model

This slice populates the Foundation-owned `pricing_price` and owns `pricing_price_tier_band` (tenant-scoped,
SecureORM; published rows append-only per Foundation §4.3):

**`pricing_price` (Foundation-owned; Slice-3 columns)**:

| Column | Type | Notes |
|--------|------|-------|
| `model_kind` | `enum` | `flat \| per_unit \| graduated \| volume \| package`; NOT NULL on publish |
| `quantity_source` | `enum` | `subscription_seat_count \| manual`; required for `per_unit` |
| `manual_quantity` | `bigint` | required when `quantity_source = manual`; frozen in snapshot |
| `package_size` | `bigint` | `> 0`; `package` only |
| `package_price_minor` | `bigint` | `≥ 0`; `package` only |
| `tier_aggregation_window` | `enum` | `calendar_month \| invoice_period \| subscription_lifetime \| per_event`; tiered usage rows only |
| `billing_granularity` | `enum` | `per_second \| per_minute \| per_hour \| per_day \| whole_unit`; all usage rows |
| `meter` | `ref` | the published `meteringUnit` a usage row prices; feeds the Slice-2 injectivity rule |
| `dimension_key` | `text` | nullable dimension discriminator on the `(meter, dimensionKey)` line (Slice-2 injectivity) |

**`pricing_price_tier_band`** (FK `price_id`; `graduated`/`volume` rows only):

| Column | Type | Notes |
|--------|------|-------|
| `band_id` | `uuid` | PK |
| `price_id` | `uuid` | FK |
| `from_qty` | `bigint` | inclusive; ascending, contiguous |
| `to_qty` | `bigint` | exclusive; NULL = open top |
| `unit_price_minor` | `bigint` | `≥ 0` (`0` valid — Q5); unit prices only, no per-band flat fee (Q3) |

**`pricing_conformance_fixture_registry`** (read-side of the shared fixture repo, Q4): `model_kind` /
`variant`, `fixture_ref`, `status` (`green | missing | stale`). The `FixtureGate` reads it at
publish. The `variant` axis also keys **cross-cutting scenario fixtures** (e.g.
`variant = supersession_continuity` on the tiered kinds, per `inst-tb-window-continuity`);
the continuity fixture **gates the first publish of any tiered usage kind** (alongside that
kind's own fixture) — ratified, D-22.

Key constraints: `CHECK (package_size > 0)`; `CHECK (unit_price_minor >= 0)`;
`CHECK (to_qty IS NULL OR to_qty > from_qty)` (no zero-width bands); structural exclusivity —
band rows forbidden unless `model_kind IN ('graduated','volume')`, enforced by a trigger or a
composite FK on `(price_id, model_kind)` (cross-table, not expressible as a row CHECK); the
package-fields-forbidden-with-bands half is a row CHECK on `pricing_price`; per-row band
contiguity/non-overlap enforced by the `TierBandValidator` at publish (order-dependent, not
expressible as a row CHECK); unique `(price_id, from_qty)`.

## 7. Events & Alarms

No new event names: `PriceCreated` on row authoring, `PriceUpdated` on supersession
(Foundation frozen set). A publish blocked by the `FixtureGate` is a synchronous 422; a
**stale** fixture (registry drift after publish) raises the operational alarm
`pricing.conformance.fixture_stale` (Warn) — the mispricing-risk signal that the §17.2
source-of-truth table and the fixture repo have diverged.

## 8. Definitions of Done

### Explicit Model Kind

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-model-kind`

Every price row **MUST** persist an explicit `modelKind` with its kind-specific
required/forbidden field sets enforced at publish (per-unit `quantitySource`; usage-only
evaluation-policy placement); a tiered row without a kind **MUST NOT** publish; the catalog
computes no charge.

**Implements**: `cpt-cf-bss-pricing-algo-model-kind`, `cpt-cf-bss-pricing-flow-price-author`

**Touches**:
- API: `POST/PATCH /v1/pricing/plans/{planId}/prices`
- DB: `pricing_price` (model-kind columns)
- Entities: `ModelKindValidator`

### Tier Bands

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-tier-bands`

Tier bands **MUST** validate ascending, non-overlapping, contiguous under `[fromQty, toQty)`
with an **always-open top** (`toQty = null`; a closed top fails publish — `TIER_TOP_CLOSED`,
D-17: capping is owned by entitlement quotas / per-period caps) and no zero-width bands
(`toQty > fromQty`); tiered usage rows **MUST** carry `tierAggregationWindow`; band quantities are expressed in
billable units after `billingGranularity` quantization (the read model documents the unit).

**Implements**: `cpt-cf-bss-pricing-algo-tier-bands`

**Touches**:
- DB: `pricing_price_tier_band`
- Entities: `TierBandValidator`

### Package Pricing

- [ ] `p2` - **ID**: `cpt-cf-bss-pricing-dod-package`

A `package` row **MUST** persist `packageSize > 0` and `packagePrice ≥ 0` with tier-band
fields absent (publish rejects otherwise); the read model exposes the two fields and Tariffs
owns the round-up math.

**Implements**: `cpt-cf-bss-pricing-algo-package`

**Touches**:
- DB: `pricing_price` (package columns)
- Entities: `PackageValidator`

### Conformance Gate

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-conformance`

Publish of any `modelKind` lacking a green joint golden fixture **MUST** be blocked;
`package` and `per_unit` each carry their own fixture before first publish, and the
**supersession-continuity** scenario fixture (`inst-tb-window-continuity`) is registered on
the tiered kinds; the §17.2 mapping is the single kind-to-formula source of truth.

**Implements**: `cpt-cf-bss-pricing-algo-fixture-gate`

**Touches**:
- DB: `pricing_conformance_fixture_registry`
- Entities: `FixtureGate`

## 9. Acceptance Criteria

Delta over the Foundation testing architecture.

Unit:

- [ ] Band-edge cases: adjacent bands share the boundary exactly once (`[0,100) [100,null)`); overlap and gap both fail; a zero-width band (`toQty = fromQty`) fails (`TIER_BAND_EMPTY`); a closed top band fails (`TIER_TOP_CLOSED`); `$0` first band passes; a band whose effective unit price exceeds the previous band's emits the advisory warning (publish succeeds)
- [ ] Band units follow `billingGranularity` quantization (a `per_hour` row's bands count hours; a raw-seconds band definition is rejected/normalized per the documented unit)
- [ ] Kind field matrices: each kind's required/forbidden set; `per_unit` without `quantitySource` fails; `manual` without quantity fails; eval-policy fields on a `flat` non-usage row fail; `graduated` on a `recurring` row fails (`MODEL_KIND_CHARGEKIND_MISMATCH`)
- [ ] Package exclusivity: bands + package fields together fail; `packageSize = 0` fails

Integration (testcontainers):

- [ ] A graduated 3-band row publishes and its ordered bands + window appear in the read model exactly as authored
- [ ] A `volume` row publishes as Variant A (no per-band fee field exists to author)
- [ ] Publish with a `package` row while the registry lacks the package fixture is blocked (`FIXTURE_MISSING`); flipping the registry green unblocks
- [ ] Superseding a published row creates a new row + closes the window; UPDATE/DELETE of the published row is rejected by the DB role/trigger
- [ ] The supersession-continuity fixture (`variant = supersession_continuity`) is registered green for the tiered kinds before their first publish; a mid-window supersession scenario keeps the tier counter `Q` continuous

API:

- [ ] RFC 9457 mapping for the §5 codes; the publish report enumerates all violations

## 10. Non-Functional Considerations

- **Performance**: band validation is O(n log n) per row at publish (authoring path); read-path exposure is a pre-sorted band array in the projected read model (no sort at rating time). Max bands per row is part of the plan/tier size caps — **provisional NFR** to ratify ([`../PRD.md`](../PRD.md) §14).
- **Observability / metrics**: `pricing_fixture_gate_blocks_total{model_kind}`, `pricing_tier_validation_failures_total{rule}`, fixture-registry staleness gauge.
- **Security & AuthZ**: price-row mutation requires the catalog-authoring scope; amount changes flow through the governance slice's materiality check.
- **Risks & open items**: fixture repo/process is cross-team (catalog + Tariffs) and MUST exist **before code** (Q4; PRD §13 gate); Tariffs sign-off that its non-overlap and formula matrix use the identical §17.2/§2.2 keys (ADR `cpt-cf-bss-pricing-adr-canonical-scope-key` confirmation item).
