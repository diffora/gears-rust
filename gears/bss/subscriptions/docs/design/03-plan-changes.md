<!-- CONFLUENCE_TITLE: [BSS]: Subscriptions — Plan & Quantity Changes (Design) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../SEAMS.md | Upstream: Pricing (plan-change classification), Contracts (ramps), Registry (overlap key) | Downstream: Rating (proration math + usage slicing), Billing (proration artifacts) | Owners: BSS Subscriptions team -->

# DESIGN — Plan & Quantity Changes (Slice 3)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-design-plan-changes`

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
  - [4.1 The WHEN/MATH Split (normative)](#41-the-whenmath-split-normative)
  - [4.2 Up/Down Asymmetry and Seat Provenance (normative)](#42-updown-asymmetry-and-seat-provenance-normative)
  - [4.3 Plan-Change Classification (normative)](#43-plan-change-classification-normative)
  - [4.4 Overlap Cardinality (normative)](#44-overlap-cardinality-normative)
  - [4.5 Ramp Execution (normative)](#45-ramp-execution-normative)
  - [4.6 Backdating Guard (normative)](#46-backdating-guard-normative)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

This slice owns the **change boundary and mode** — the WHEN of every commercial mutation
(`changePlan`, `addAddOn`/`removeAddOn`, `updateQuantity`) — and nothing about the money. It sets
`changeEffectiveAt` and `changeMode`, opens/closes composition intervals at that instant (slice 02),
and emits `SubscriptionPlanChanged` (or the composition-changing quantity event) carrying the
boundary inputs; the **rating gear** prorates the recurring component and slices usage at the same
instant ([`../PRD.md`](../PRD.md) §6.3). One boundary owner + one math owner is the split that keeps
replay deterministic.

Five seams meet here: **SUB-R1** (the WHEN/MATH split + shared boundary), **SUB-R3** (seat-count
provenance and the mid-period seat boundary), **SUB-P1** (plan-change classification adopted from
pricing), **SUB-C2** (ramp execution of Contract-authored schedules), and **SUB-G1** (the overlap
cardinality key from the registry).

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-subscriptions-fr-plan-change-boundary` | Subscriptions sets `changeEffectiveAt`/`changeMode ∈ {immediate, next-cycle, end-of-term}` and emits `SubscriptionPlanChanged`; the boundary opens/closes `PlanLink` intervals (§4.1). |
| `cpt-cf-bss-subscriptions-fr-proration-ownership` / `cpt-cf-bss-subscriptions-fr-proration-triggers` | Rating prorates `planA` over `[periodStart, changeEffectiveAt)` and `planB` over `[changeEffectiveAt, periodEnd)`; Subscriptions never computes day-count math — it fixes the trigger + `effectiveFrom` + "no posted-invoice mutation" (§4.1). |
| `cpt-cf-bss-subscriptions-fr-update-quantity` | `updateQuantity` is a first-class transition with the change envelope; seat counts rating reads originate only from **committed** transitions; up increases MAY be immediate, decreases default `next-cycle` (§4.2; SUB-D-02). |
| `cpt-cf-bss-subscriptions-fr-ramp-execution` | A Contract-authored ramp materialises as a sequence of scheduled `changePlan`/`updateQuantity` intents executed via the slice-01 `IntentScheduler` (§4.5; SUB-D-04). |
| `cpt-cf-bss-subscriptions-fr-overlap-cardinality` | On `activate`, evaluate `overlapScopeKey` + `maxConcurrentActive`; reject/queue fail-closed where the rule would break idempotent billing (§4.4). |
| `cpt-cf-bss-subscriptions-fr-backdated-changes` | A backdated `effectiveFrom` inside a posted invoice period is rejected → adjustment path; operational backdating emits an explicit audit reason (§4.6). |

#### NFR Allocation

| NFR theme | Allocated To | Design Response | Verification / Status |
|-----------|--------------|-----------------|-----------------------|
| `cpt-cf-bss-subscriptions-nfr-lifecycle-latency` | Change commit path | `changePlan`/`updateQuantity` in the synchronous commit class (p95 < 1s) | Load test; baseline (workshop-pending) |
| `cpt-cf-bss-subscriptions-nfr-proration-accuracy` | Boundary + rating | 100% end-to-end alignment vs policy; math owned by rating, boundary here — tested on the joint fixture | Joint proration fixture |

#### Key ADRs

| ADR ID | Decision Summary |
|--------|------------------|
| [`../ADR/0002`](../ADR/0002-cpt-cf-bss-subscriptions-adr-when-not-math-split.md) `cpt-cf-bss-subscriptions-adr-when-not-math-split` | Subscriptions owns only the change boundary/mode; rating owns all proration math (§4.1). |

### 1.3 Architecture Layers

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-tech-stack-chg`

| Layer | Responsibility | Technology |
|-------|----------------|------------|
| Application | Boundary/mode setting, classification enforcement, quantity provenance, ramp intent execution, overlap detection | Rust module in the `subscriptions` gear |
| Domain | Change envelope, `overlapScopeKey`, seat-provenance record | Rust; GTS + Rust domain structs |
| Infrastructure | None beyond the Foundation stores + `scheduled_intent` (slice 01) | PostgreSQL, SecureORM |

## 2. Principles and Constraints

### 2.1 Design Principles

#### WHEN, never math

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-when-not-math-chg`

Subscriptions sets `changeEffectiveAt`/`changeMode` and emits them; it computes no proration, tax, or
FX. Rating owns the arithmetic at the same boundary ([`../PRD.md`](../PRD.md) §6.3; SEAMS **SUB-R1**).

#### Provenance before the number

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-provenance-chg`

Any quantity rating reads (`quantitySource = subscription_seat_count`) originates from a **committed
`updateQuantity` transition** — never an untyped attribute edit — so the seat count has an auditable
boundary and Policy gate (§4.2; SEAMS **SUB-R3**).

### 2.2 Constraints

#### No posted-invoice mutation

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-no-posted-mutation-chg`

Proration surfaces as **new billable or adjusting artifacts** (Billing), never an edit of a posted
invoice line ([`../PRD.md`](../PRD.md) §6.3, §6.8).

#### Classification is adopted, not derived

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-classification-adopted-chg`

Comparability / allowed change targets come from the pricing consumer contract; Subscriptions
**enforces** the classification, it does not re-derive it. Cross-currency/region/frequency =
cancel+new (§4.3; SEAMS **SUB-P1**).

## 3. Technical Architecture

### 3.1 Domain Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-domain-model-chg`

- **`ChangeEnvelope`** — `changeEffectiveAt` (UTC), `changeMode ∈ {immediate, next-cycle, end-of-term}`, the up/down asymmetry policy, source `TransitionRequest`.
- **`QuantityInterval`** — the **effective-dated committed seat count** (`quantity`, `effectiveFrom`, `effectiveTo`, source `updateQuantity` transition): the only source rating may read, resolvable as `quantity @ t` for replay (2026-07-15 review fix — a single mutable value cannot serve the replay contract; stored in slice 02's `quantity_interval`).
- **`overlapScopeKey`** — default `(payerTenantId, catalogSubscriptionProductKey)` + optional extra dimensions; `maxConcurrentActive` from Catalog/Contract (default 1); **`supersedesSubscriptionId`** — the cancel+new handover linkage (§4.3/§4.4).
- **Ramp step** — a `ScheduledIntent` (slice 01) of `changePlan`/`updateQuantity` kind authored by Contracts.

### 3.2 Component Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-component-plan-changes-chg`

- **`ChangeBoundaryPlanner`** — resolves `changeEffectiveAt`/`changeMode` (incl. up/down asymmetry) and drives the interval open/close via slice 02.
- **`ChangeClassifier`** — enforces the pricing-published comparability/target rules; routes cross-boundary changes to cancel+new.
- **`QuantityProvenanceRecorder`** — writes the effective-dated `QuantityInterval`, binding each committed seat count to its `updateQuantity` transition.
- **`OverlapDetector`** — evaluates `overlapScopeKey` + `maxConcurrentActive` at `activate`.
- **`RampExecutor`** — materialises Contract-authored ramps as scheduled intents via the Foundation `IntentScheduler`.

### 3.3 API Contracts

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-interface-plan-change-chg`

The `changePlan` / `addAddOn` / `removeAddOn` / `updateQuantity` operations carry the change envelope;
the emitted `SubscriptionPlanChanged` (and the composition-changing quantity event) carry the
boundary inputs for rating/Billing. Preview of proration impact is surfaced via the preview owner
named in Design — the **calculation authority is rating** ([`../PRD.md`](../PRD.md) §10 plan-change UC).
Wire mappings + event field matrix are owned by [`08-events-billing.md`](./08-events-billing.md) /
[`09-consumer-contracts.md`](./09-consumer-contracts.md).

### 3.4 Internal Dependencies

Depends on [`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) (commit path, `IntentScheduler`,
`Approval`) and [`02-composition-versioning.md`](./02-composition-versioning.md) (interval open/close).
Feeds [`08-events-billing.md`](./08-events-billing.md) (change events).

### 3.5 External Dependencies

| Dependency | What crosses the boundary | Contract |
|------------|---------------------------|----------|
| Pricing | Plan-change classification (`allowedChangeTargets`/`comparabilityRank`) | SEAMS **SUB-P1** |
| Rating | Consumes `(changeEffectiveAt, changeMode)` + `quantity @ t`; owns proration + usage slicing | SEAMS **SUB-R1**, **SUB-R3** |
| Contracts | Authors the committed ramp schedule | SEAMS **SUB-C2** |
| Registry | `catalogSubscriptionProductKey` for `overlapScopeKey` | SEAMS **SUB-G1** |
| Billing | `billedThroughAt` posted-period watermark consumed by the backdating guard (§4.6) | SEAMS **SUB-B6** |

### 3.6 Interactions and Sequences

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-change-boundary-chg`

**Plan/quantity change** (refines `cpt-cf-bss-subscriptions-seq-change-boundary`): classify the
target (`ChangeClassifier`; cross-boundary ⇒ cancel+new) → resolve `changeEffectiveAt`/`changeMode`
(up/down asymmetry) → for immediate: close prior / open new interval at `now`; for next-cycle/
end-of-term: persist a `ScheduledIntent` **and write the future interval** (slice 02 future-replaceable
rule) → emit `SubscriptionPlanChanged` with the boundary inputs → rating slices + prorates.
`updateQuantity` additionally writes the `QuantityInterval` and emits `SubscriptionQuantityChanged`.

**Event-once convention (2026-07-15 review fix):** the boundary is announced **exactly once** —
`SubscriptionPlanChanged`/`SubscriptionQuantityChanged` is emitted at commit time (immediate) or at
scheduling time (deferred, carrying the future `changeEffectiveAt`); the successful intent **firing
emits no second boundary event** (the transition commit references the announced boundary). An
`unschedule` before the boundary emits `SubscriptionIntentUnscheduled`, which **voids the announced
boundary** for consumers — by construction it always precedes the boundary instant on the ordered
stream.

**Firing-failure retraction (2026-07-15 review fix).** A deferred boundary is announced *before* it
is committed, so a firing that **fails its guard set at `effectiveAt`** (Policy deny, `guard_violation`,
`oss_unconfirmed` — slice 01 §4.3/§4.5 leave state unchanged) would otherwise strand rating on a
boundary that never materialised. The `IntentScheduler` therefore emits `SubscriptionIntentUnscheduled`
(reason = `firing_failed`, referencing the same intent) on any terminal firing failure, which **voids
the announced boundary** exactly like an operator `unschedule`. This is the one case where the
voiding event lands **at** the boundary instant rather than strictly before it; consumers MUST treat
a boundary as retractable up to and including its `effectiveAt` on the ordered stream, and re-rate/
un-slice on receipt. The §17.1 charge-coverage reconciliation is the backstop.

### 3.7 Database Schemas and Tables

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-storage-plan-change-chg`

No new owned store beyond the Foundation's `scheduled_intent` (ramps + deferred changes) and the
slice-02 interval tables — the committed quantity history lives in slice 02's `quantity_interval`
(written by this slice's `updateQuantity`); `overlapScopeKey` (+ `supersedesSubscriptionId`) rides
the aggregate with an `active`-status partial index per key. Concrete DDL is Design.

### 3.8 Deployment Topology

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-deployment-chg`

No slice-specific topology; scheduled changes + ramp steps fire via the Foundation `IntentScheduler`
singleton ([`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) §3.8).

## 4. Additional Context

### 4.1 The WHEN/MATH Split (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-when-math-chg`

- Subscriptions owns `changeEffectiveAt` + `changeMode`; it emits them on `SubscriptionPlanChanged` and never computes proration day-count or override resolution ([`../PRD.md`](../PRD.md) §6.3).
- Rating rates `planA` over `[periodStart, changeEffectiveAt)` and `planB` over `[changeEffectiveAt, periodEnd)` (half-open UTC), prorates on the frozen `prorationBasis`, and slices usage at the same boundary ([rating PRD](../../../rating/docs/PRD.md) §6.11; SEAMS **SUB-R1**).
- Immediate ⇒ delta recurring / one-time true-up (Billing); next-cycle ⇒ first recurring under the new plan at the new period; mid-cycle ⇒ credit/debit notes if an invoice is already posted — **never** an edited posted line ([`../PRD.md`](../PRD.md) §6.3).

### 4.2 Up/Down Asymmetry and Seat Provenance (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-asymmetry-chg`

- Default asymmetry (owned here, like plan changes): **increases MAY be immediate** (prorated by rating at the boundary); **decreases default `next-cycle`** ([`../PRD.md`](../PRD.md) §6.3; SUB-D-02).
- The seat count rating reads (`quantitySource = subscription_seat_count`, pricing D-18) MUST come from **committed** `updateQuantity` transitions only — auditable provenance, never an untyped edit — and is stored **effective-dated** (`QuantityInterval`) so `quantity @ t` is resolvable in the read model for any charge instant and replay ([`../PRD.md`](../PRD.md) §6.3, §9.2).
- **Edge guards:** a decrease below the quantity already consumed by committed assignments is rejected unless policy explicitly forces revocation (the entitlement effect routes through slice 05); `quantity = 0` is not a quantity change — submit a `cancel`. **Source of "consumed" (2026-07-15 review fix):** the guard reads the count of **committed entitlement bindings owned here** (slice 05 `GrantSetAssignment` / seat-scoped `Entitlement` rows) — a local, transactionally-consistent number — **not** an external OSS/AMS seat-to-user mapping; where finer per-user seat placement lives in OSS, that enforcement is OSS's (SUB-E3) and the decrease guard binds only to what this gear committed. So the guard never depends on a cross-gear read that could be stale or unavailable.
- **Open (mirrors rating's seat-change boundary transport, rating `design/09` §4.3):** a mid-period seat change is transported as a **Subscriptions-driven change boundary** (default) that rating prorates, not Subscriptions-side proration; pin the default with rating at Design (SEAMS **SUB-R3**).

### 4.3 Plan-Change Classification (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-classification-chg`

- The pricing consumer contract publishes `allowedChangeTargets` / `comparabilityRank` / the boundary class; Subscriptions **classifies** upgrade/downgrade/cross and **enforces** the boundary — it does not re-derive comparability ([pricing `design/06`]; SEAMS **SUB-P1**).
- **Cross-currency / cross-region / cross-frequency changes = cancel+new**, not an in-place change (pricing §15 cross-boundary sign-off; the overlap default protects against double `active`, §4.4).
- **Cancel+new mechanics (2026-07-15 review fix).** The replacement is a **saga over two aggregates**, not a transaction: (1) create the successor in `draft` with **`supersedesSubscriptionId`** set; (2) validate its full guard set (Policy, sellability, overlap-with-exemption §4.4) **before** touching the predecessor; (3) schedule the predecessor's cancel and the successor's activation on the **same boundary instant** (entitlement continuity across the handover — slice 05 re-issue at one boundary); (4) on any successor failure the predecessor is untouched (compensation = void the successor draft). The successor re-freezes the `(currency, region)` segment (slice 02 §4.2). The customer is never left without the predecessor until the successor has passed its guards.

### 4.4 Overlap Cardinality (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-overlap-chg`

- Default: at most **one `active`** per `overlapScopeKey = (payerTenantId, catalogSubscriptionProductKey)`; `catalogSubscriptionProductKey` is **registry-owned** — Design binds the stored field to a published SKU/product key (SEAMS **SUB-G1**, PR #4177).
- Multiple concurrent `active` are allowed when they differ on `overlapScopeKey` (extra dimensions) or when Catalog/Contract sets `maxConcurrentActive > 1`; else `maxConcurrentActive = 1` ([`../PRD.md`](../PRD.md) §6.3).
- **Detection runs on every entry into `active`** — `activate` **and** `resume` — **and on every committed change that mutates the key**: a `changePlan` that alters `catalogSubscriptionProductKey` and an ownership `transfer` that alters `payerTenantId` (slice 07) re-evaluate before commit (2026-07-15 review fix; detection-on-activate-only left three bypasses). The rule **rejects or queues fail-closed** when it would break idempotent billing.
- **Supersedes exemption:** a successor carrying `supersedesSubscriptionId` (§4.3 cancel+new) is exempt from the rule against exactly its predecessor, only until the predecessor's scheduled end — the handover window is not a violation.
- The `overlapScopeKey` is indexed (`active`-status partial index per key) so the check is a point lookup at 100K+ subscriptions/tenant, not a scan.

### 4.5 Ramp Execution (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-ramp-chg`

- A committed multi-step ramp is a **Contract term**: Contracts authors + owns the schedule; it materialises here as a sequence of scheduled `changePlan`/`updateQuantity` intents, each executed with the normal envelope + guards + idempotency (§4.1; slice 01 `IntentScheduler`) ([`../PRD.md`](../PRD.md) §6.3; SUB-D-04).
- **Mid-ramp failure:** a step that fails its guard set at firing (e.g. Policy deny) **halts the remaining steps** — they park `suspended pending re-authoring`, an auditable failure event is emitted, and the Contracts owner is signalled to amend or re-confirm the schedule; the executed prefix stands (committed transitions are not rolled back).
- **No native schedule aggregate** at launch; atomic multi-action submission (Zuora-Orders-style) is a Contracts/Design follow-up (SEAMS **SUB-C2**, [`../PRD.md`](../PRD.md) §15).

### 4.6 Backdating Guard (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-backdating-chg`

- A commercial `effectiveFrom` in the past is allowed only when contract/catalog rules allow and **no posted invoice** would be contradicted; a backdated boundary inside a posted period is **rejected** with a clear reason → the adjustment path ([`../PRD.md`](../PRD.md) §6.3 AC 6).
- **Data source (2026-07-15 review fix):** "posted" is evaluated against the Billing-supplied per-subscription **`billedThroughAt`** watermark (read model/event — SEAMS **SUB-B6**); the guard is **fail-closed**: an unknown/stale watermark is treated as posted and the change rejected toward the adjustment path.
- Operational backdating (e.g. an entitlement start in the past) emits an explicit audit reason and MAY require Policy re-evaluation.

## 5. Traceability

- **PRD**: [`../PRD.md`](../PRD.md) §6.3 (`fr-plan-change-boundary`, `fr-proration-triggers`, `fr-proration-ownership`, `fr-backdated-changes`, `fr-overlap-cardinality`, `fr-update-quantity`, `fr-ramp-execution`), §6.8 (no-retro), §10 (plan-change UC), §7.1 (NFRs), §15 (seat-boundary + ramp opens).
- **Seams**: **SUB-R1**, **SUB-R3**, **SUB-P1**, **SUB-C2**, **SUB-G1** — [`../SEAMS.md`](../SEAMS.md).
- **Decisions**: SUB-D-02 (`updateQuantity`), SUB-D-04 (ramps) — [`../DECISIONS.md`](../DECISIONS.md).
- **ADR**: [`../ADR/0002`](../ADR/0002-cpt-cf-bss-subscriptions-adr-when-not-math-split.md) (WHEN/MATH split).
- **Slices**: [`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) (envelope, scheduler), [`02-composition-versioning.md`](./02-composition-versioning.md) (intervals), [`08-events-billing.md`](./08-events-billing.md) (change events), [`09-consumer-contracts.md`](./09-consumer-contracts.md) (rating read-model).
