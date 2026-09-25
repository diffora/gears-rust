<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Read Contract & Events (Feature) -->
<!-- Related: ../DECOMPOSITION.md, ../DESIGN.md, ../PRD.md | Owners: BSS Pricing team -->

# Feature: Read Contract & Events

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-featstatus-read-contract-events-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-pricing-feature-read-contract-events`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Resolve a renewal and preserve invoice inputs](#resolve-a-renewal-and-preserve-invoice-inputs)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [renewal-walk](#renewal-walk)
  - [period-slices-and-quote](#period-slices-and-quote)
  - [typed-events](#typed-events)
- [4. States (CDSL)](#4-states-cdsl)
  - [Read Contract & Events states](#read-contract--events-states)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Full chain resolution matrix](#full-chain-resolution-matrix)
  - [Renewal walk and eligibility](#renewal-walk-and-eligibility)
  - [Descriptors from the dated SKU version](#descriptors-from-the-dated-sku-version)
  - [Durable pinned-price read](#durable-pinned-price-read)
  - [Period boundary semantics](#period-boundary-semantics)
  - [Studio quote calculation](#studio-quote-calculation)
  - [Typed transactional domain events](#typed-transactional-domain-events)
  - [Frozen consumer golden responses](#frozen-consumer-golden-responses)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

**Delivery:** phase 4 for reads, with quote not built (D-415); 2c for core events; 3 for added events. Every checkbox is an implementation obligation, not an assertion about the legacy code. Core event payloads are built with phase 2 approvals; dependencies on plans/promotions apply only to their later reads and events.

This feature implements [slice 07](../design/07-read-contract-events.md) — `cpt-cf-bss-pricing-design-slice-07`.
[DECOMPOSITION](../DECOMPOSITION.md) records integration order; [DESIGN §3](../DESIGN.md#3-technical-architecture)
is the schema and transaction authority. Unchecked phase 3/4 work is not part of the phase 2 core gate.

### 1.2 Purpose

Deliver reproducible resolution matrices, pinned-price reads and Studio quote, plus typed transactional events and consumer goldens. The Studio quote is not built (D-415).

Requirements: `cpt-cf-bss-pricing-fr-resolve`, `cpt-cf-bss-pricing-fr-price-read`, `cpt-cf-bss-pricing-fr-quote`, `cpt-cf-bss-pricing-fr-events`.

Architecture: `cpt-cf-bss-pricing-component-read-contract`, `cpt-cf-bss-pricing-component-events`, `cpt-cf-bss-pricing-component-prices`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-constraint-two-backends`.

### 1.3 Actors

`cpt-cf-bss-pricing-actor-rating`, `cpt-cf-bss-pricing-actor-subscriptions`, `cpt-cf-bss-pricing-actor-products`, `cpt-cf-bss-pricing-actor-finance-manager`. Every operation authenticates and derives tenant scope before storage, replay or cross-gear calls.
Holding multiple permissions never bypasses separation of duties.

### 1.4 References

- [PRD](../PRD.md), especially the numbered acceptance criteria referenced below.
- [DESIGN](../DESIGN.md), §3 model, API contracts, transaction sequences and DDL.
- [Slice 07](../design/07-read-contract-events.md), including API, data and event obligations.
- [DECISIONS](../DECISIONS.md), D-384–D-415; spec means `docs/superpowers/specs/2026-09-24-pricebook-model-design.md` in the main checkout.
- Source: spec §2 decisions 4–8, 13–17, §2.2, §5–§8, §10, §12–§13; the phase 2 plan supplies delivery boundaries and D-399/D-400.

## 2. Actor Flows (CDSL)

### Resolve a renewal and preserve invoice inputs

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-flow-read-contract-events`

1. [ ] - `p1` - Rating or Subscriptions sends the revision id, period start and optional current pins. - `inst-read-contract-events-flow-1`
2. [ ] - `p1` - Load immutable revision structure and the full chain matrix, scoped to the tenant. - `inst-read-contract-events-flow-2`
3. [ ] - `p1` - For existing pins walk eligible all successors, stopping before the first new price; for signup select in-force prices. - `inst-read-contract-events-flow-3`
4. [ ] - `p1` - Read Products versions?as_of for the period start, bind descriptors/timing/meter/unit and return the whole dimension matrix plus promotion version (deferred with promotions, D-409). - `inst-read-contract-events-flow-4`
5. [ ] - `p1` - Consumers lazily bind usage per value and retain the complete inputs; later replay reads the pinned price and stored binding without choosing new descriptors. - `inst-read-contract-events-flow-5`

## 3. Processes / Business Logic (CDSL)

### renewal-walk

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-read-contract-events-renewal-walk`

1. [ ] - `p1` - Validate the supplied pin belongs to the tenant, revision item and entry chain. - `inst-read-contract-events-renewal-walk-1`
2. [ ] - `p1` - From the pinned price walk successors with eligibility all, bounded by the relevant date; stop before the first new successor. - `inst-read-contract-events-renewal-walk-2`
3. [ ] - `p1` - Without a pin choose the in-force price per value, falling back to default where the value has no active price. - `inst-read-contract-events-renewal-walk-3`
4. [ ] - `p1` - Return uncovered rather than inventing a price when neither chain covers; preserve historical keep_for_bound prices. - `inst-read-contract-events-renewal-walk-4`

### period-slices-and-quote

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-read-contract-events-period-slices-and-quote`

Not built (D-415): the owner dropped quote and the Studio wiring; consumers read resolve and GET /pricing/v1/prices/{id}, and Rating owns the minimum-fee floor arithmetic.

1. [ ] - `p1` - Split the period at every price boundary inside the bound chain, including a temporary end. - `inst-read-contract-events-period-slices-and-quote-1`
2. [ ] - `p1` - Prorate recurring slices by calendar days; rate usage by reading timestamp with tier counters per slice. - `inst-read-contract-events-period-slices-and-quote-2`
3. [ ] - `p1` - Deduct included quantities, aggregate per price/subscription/period and apply the coverage-prorated min_fee once per price; not built in pricing, Rating applies the floor (D-415). - `inst-read-contract-events-period-slices-and-quote-3`
4. [ ] - `p1` - Apply period-start promotion after floors, then the bound rounding/currency policy; quote returns totals while resolve never does; quote is not built (D-415). - `inst-read-contract-events-period-slices-and-quote-4`

### typed-events

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-read-contract-events-typed-events`

1. [ ] - `p1` - Implement PricesPublished, ApprovalUnitDecided and PriceBookEntryReferenceLost through broker TypedEvent in phase 2. - `inst-read-contract-events-typed-events-1`
2. [ ] - `p1` - Add PlanRevisionPublished, PlanReferenceLost, PlanRetired, PromotionPublished and SubscriptionMigrationRequested as their phase 3 acts become real; PromotionPublished (D-409), PlanRetired and SubscriptionMigrationRequested (D-410) are deferred. - `inst-read-contract-events-typed-events-2`
3. [ ] - `p1` - Append event and audit through the same mutation transaction; encode the tenant and stable subject identities in the durable envelope, the correlation id staying on the audit rows of the same transaction. - `inst-read-contract-events-typed-events-3`
4. [ ] - `p1` - Deliver from the toolkit dispatcher after commit; test restart/retry and prevent domain publish on reject/withdraw/refresh. - `inst-read-contract-events-typed-events-4`

## 4. States (CDSL)

### Read Contract & Events states

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-state-read-contract-events`

A consumer binding is created for a period and stays immutable for replay. New all prices affect later renewal binding; a new price blocks forward renewal walking until explicit migration. Closed/superseded/keep_for_bound prices remain readable. Event states belong to toolkit delivery; Pricing does not maintain a second outbound state machine.

## 5. Definitions of Done

Every DoD below is required for this feature's delivery phase. Constraints: `cpt-cf-bss-pricing-constraint-two-backends`.

### Full chain resolution matrix

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-resolve-matrix`

Resolve returns every item's default/value inputs and active promotion version (deferred with promotions, D-409) without totals. Usage consumers bind lazily by value rather than choosing one value for the plan (spec §7.1).

Requirement: `cpt-cf-bss-pricing-fr-resolve`; PRD AC #18.

### Renewal walk and eligibility

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-renewal-all-new`

Existing pins advance through all successors and stop before the first new price. Signup chooses the in-force price and keep_for_bound preserves the predecessor needed by renewals (spec §7.1, §12).

Requirement: `cpt-cf-bss-pricing-fr-resolve`; PRD AC #18.

### Descriptors from the dated SKU version

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-binding-sku-version`

Binding reads versions?as_of at period start, not the latest mutable SKU. Preserve version, unit/meter, descriptors, timing, rounding and currency scale in replay inputs (spec §2.2, §7.1).

Requirement: `cpt-cf-bss-pricing-fr-resolve`; PRD AC #18.

### Durable pinned-price read

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-price-read-forever`

The public price-id read serves original approved money after closure, supersession or keep_for_bound. Tenant scope remains enforced and no retention deletes a pinned fact (spec §7.1).

Requirement: `cpt-cf-bss-pricing-fr-price-read`; PRD AC #19.

### Period boundary semantics

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-period-slices`

Not built (D-415): the owner dropped quote and the Studio wiring; consumers read resolve and GET /pricing/v1/prices/{id}, and Rating owns the minimum-fee floor arithmetic. This DoD stays unticked.

Recurring slices prorate by calendar days and usage follows timestamp-selected prices, with counters per slice. Temporary boundaries inside a period produce multiple slices and price-level floor aggregation (spec §7.1).

Requirement: `cpt-cf-bss-pricing-fr-quote`; PRD AC #20.

### Studio quote calculation

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-quote-totals`

Not built (D-415): the owner dropped quote and the Studio wiring; consumers read resolve and GET /pricing/v1/prices/{id}, and Rating owns the minimum-fee floor arithmetic. This DoD stays unticked.

Quote adds quantities and optional-item choices to selection, included quantities, price floors and promotions. It is read-only and separate from resolve, with exact tier-edge and rounding goldens (spec §7.1).

Requirement: `cpt-cf-bss-pricing-fr-quote`; PRD AC #20.

### Typed transactional domain events

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-events-typed-outbox`

Core domain events are TypedEvents enqueued on the toolkit outbox in the mutation transaction and delivered by the broker SDK producer when an EventBrokerApi is registered (D-400); later payloads take the same shape. Envelope and payload identity are preserved. Terminal units always event; submission and committed refresh do not falsely publish domain success (spec §6–§7.3, D-400).

Requirement: `cpt-cf-bss-pricing-fr-events`; PRD AC #14.

### Frozen consumer golden responses

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-consumer-golden-contracts`

Golden responses cover resolve matrix, renewal eligibility, descriptor dates, promotion versions and forever-readable prices. Rating and Subscriptions consume these in separate plans; fixtures-crate deletion occurs in phase 4 (spec §10–§12).

Requirement: `cpt-cf-bss-pricing-fr-price-read`; PRD AC #19.

## 6. Acceptance Criteria

| DoD | PRD criterion | Given / When / Then |
| --- | --- | --- |
| `cpt-cf-bss-pricing-dod-resolve-matrix` | AC #18; `cpt-cf-bss-pricing-fr-resolve` | Given one item with EU/default prices, when resolve runs then both inputs are returned; an uncovered chain is explicit and no quantity total appears. |
| `cpt-cf-bss-pricing-dod-renewal-all-new` | AC #18; `cpt-cf-bss-pricing-fr-resolve` | Given pinned 10 → all 12 → new 15, when renewal resolves then it chooses 12 and signup 15; a forged foreign-chain pin is refused. |
| `cpt-cf-bss-pricing-dod-binding-sku-version` | AC #18; `cpt-cf-bss-pricing-fr-resolve` | Given an October 1 GL change already applied to the current SKU, when September resolves then it binds the earlier version; October binds the new one and prior pins do not change. |
| `cpt-cf-bss-pricing-dod-price-read-forever` | AC #19; `cpt-cf-bss-pricing-fr-price-read` | Given a closed price id from an old invoice, when read then its original money is returned; unknown/foreign ids reveal no price. |
| `cpt-cf-bss-pricing-dod-period-slices` | AC #20; `cpt-cf-bss-pricing-fr-quote` | Not built (D-415). Given a temporary price ending October 11 inside October 5–November 5, when preview runs then two slices appear; their common-price floors are not charged twice. |
| `cpt-cf-bss-pricing-dod-quote-totals` | AC #20; `cpt-cf-bss-pricing-fr-quote` | Not built (D-415). Given valid quantities and a promotion, when quote runs then totals apply included quantities before prorated floor and promotion afterward; invalid quantities fail without changing pins. |
| `cpt-cf-bss-pricing-dod-events-typed-outbox` | AC #14; `cpt-cf-bss-pricing-fr-events` | Given approve/reject/withdraw/quorum-zero outcomes, when committed then each has its terminal event and only successful apply has its domain publication; rollback has neither. |
| `cpt-cf-bss-pricing-dod-consumer-golden-contracts` | AC #19; `cpt-cf-bss-pricing-fr-price-read` | Given stored contract fixtures including negative tenant/uncovered cases, when either backend serves the public paths then responses match; a shape drift fails the contract gate. |

Verification uses domain tests, scoped repository tests on both backends and REST positive/denial/precondition probes as applicable. Phase 2 checks must not mark later-phase behavior implemented. Golden consumer contracts belong to phase 4.
