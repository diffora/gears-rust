<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Read Contract & Events (Design, Slice 7) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../DECISIONS.md | Owners: BSS Pricing team -->

# DESIGN — Read Contract & Events (Slice 7)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-design-slice-07`

<!-- toc -->

- [1. Context](#1-context)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Resolve a renewal and preserve invoice inputs](#resolve-a-renewal-and-preserve-invoice-inputs)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [renewal-walk](#renewal-walk)
  - [period-slices-and-quote](#period-slices-and-quote)
  - [typed-events](#typed-events)
- [4. States (CDSL)](#4-states-cdsl)
- [5. API Surface](#5-api-surface)
- [6. Data Model](#6-data-model)
- [7. Events & Alarms](#7-events--alarms)
- [8. Definitions of Done](#8-definitions-of-done)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Non-Functional Considerations](#10-non-functional-considerations)

<!-- /toc -->

## 1. Context

**Delivery:** phase 4 for reads/quote; 2c for core events; 3 for added events. Every checkbox is an implementation obligation, not an assertion about the legacy code. Core event payloads are built with phase 2 approvals; dependencies on plans/promotions apply only to their later reads and events.

Deliver reproducible resolution matrices, pinned-price reads and Studio quote, plus typed transactional events and consumer goldens.

Requirements: `cpt-cf-bss-pricing-fr-resolve`, `cpt-cf-bss-pricing-fr-price-read`, `cpt-cf-bss-pricing-fr-quote`, `cpt-cf-bss-pricing-fr-events`. Architecture: `cpt-cf-bss-pricing-component-read-contract`, `cpt-cf-bss-pricing-component-events`, `cpt-cf-bss-pricing-component-prices`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-constraint-two-backends`.
[FEATURE](../features/read-contract-events.md) owns the executable flow/algorithm/DoD identifiers; this slice defines no duplicate DoDs.
Dependencies: `cpt-cf-bss-pricing-feature-plans`, `cpt-cf-bss-pricing-feature-promotions-migrations`, `cpt-cf-bss-pricing-feature-approvals`.
Source: PriceBook spec §2.2, §5–§8, §12–§13 and [DECISIONS](../DECISIONS.md) D-384–D-406.

## 2. Actor Flows (CDSL)

### Resolve a renewal and preserve invoice inputs

Actors: `cpt-cf-bss-pricing-actor-rating`, `cpt-cf-bss-pricing-actor-subscriptions`, `cpt-cf-bss-pricing-actor-products`, `cpt-cf-bss-pricing-actor-finance-manager`. Feature flow: `cpt-cf-bss-pricing-flow-read-contract-events`.

1. [ ] - `p1` - Rating or Subscriptions sends the revision id, period start and optional current pins. - `inst-read-contract-events-flow-1`
2. [ ] - `p1` - Load immutable revision structure and the full chain matrix, scoped to the tenant. - `inst-read-contract-events-flow-2`
3. [ ] - `p1` - For existing pins walk eligible all successors, stopping before the first new price; for signup select in-force prices. - `inst-read-contract-events-flow-3`
4. [ ] - `p1` - Read Products versions?as_of for the period start, bind descriptors/timing/meter/unit and return the whole dimension matrix plus promotion version. - `inst-read-contract-events-flow-4`
5. [ ] - `p1` - Consumers lazily bind usage per value and retain the complete inputs; later replay reads the pinned price and stored binding without choosing new descriptors. - `inst-read-contract-events-flow-5`

## 3. Processes / Business Logic (CDSL)

### renewal-walk

Feature algorithm: `cpt-cf-bss-pricing-algo-read-contract-events-renewal-walk`.

1. [ ] - `p1` - Validate the supplied pin belongs to the tenant, revision item and entry chain. - `inst-read-contract-events-renewal-walk-1`
2. [ ] - `p1` - From the pinned price walk successors with eligibility all, bounded by the relevant date; stop before the first new successor. - `inst-read-contract-events-renewal-walk-2`
3. [ ] - `p1` - Without a pin choose the in-force price per value, falling back to default where the value has no active price. - `inst-read-contract-events-renewal-walk-3`
4. [ ] - `p1` - Return uncovered rather than inventing a price when neither chain covers; preserve historical keep_for_bound prices. - `inst-read-contract-events-renewal-walk-4`

### period-slices-and-quote

Feature algorithm: `cpt-cf-bss-pricing-algo-read-contract-events-period-slices-and-quote`.

1. [ ] - `p1` - Split the period at every price boundary inside the bound chain, including a temporary end. - `inst-read-contract-events-period-slices-and-quote-1`
2. [ ] - `p1` - Prorate recurring slices by calendar days; rate usage by reading timestamp with tier counters per slice. - `inst-read-contract-events-period-slices-and-quote-2`
3. [ ] - `p1` - Deduct included quantities, aggregate per price/subscription/period and apply the coverage-prorated min_fee once per price. - `inst-read-contract-events-period-slices-and-quote-3`
4. [ ] - `p1` - Apply period-start promotion after floors, then the bound rounding/currency policy; quote returns totals while resolve never does. - `inst-read-contract-events-period-slices-and-quote-4`

### typed-events

Feature algorithm: `cpt-cf-bss-pricing-algo-read-contract-events-typed-events`.

1. [ ] - `p1` - Implement PricesPublished, ApprovalUnitDecided and PriceBookEntryReferenceLost through broker TypedEvent in phase 2. - `inst-read-contract-events-typed-events-1`
2. [ ] - `p1` - Add PlanRevisionPublished, PlanRetired, PromotionPublished and SubscriptionMigrationRequested as their phase 3 acts become real. - `inst-read-contract-events-typed-events-2`
3. [ ] - `p1` - Append event and audit through the same mutation transaction; encode the tenant and stable subject identities in the durable envelope, the correlation id staying on the audit rows of the same transaction. - `inst-read-contract-events-typed-events-3`
4. [ ] - `p1` - Deliver from the toolkit dispatcher after commit; test restart/retry and prevent domain publish on reject/withdraw/refresh. - `inst-read-contract-events-typed-events-4`

## 4. States (CDSL)

A consumer binding is created for a period and stays immutable for replay. New all prices affect later renewal binding; a new price blocks forward renewal walking until explicit migration. Closed/superseded/keep_for_bound prices remain readable. Event states belong to toolkit delivery; Pricing does not maintain a second outbound state machine.

State definition: `cpt-cf-bss-pricing-state-read-contract-events` in the FEATURE.

## 5. API Surface

The spec consumer paths are GET /pricing/v1/resolve?plan_revision_id&date with optional pins, and GET /pricing/v1/prices/{id}. GET /pricing/v1/quote is a Studio preview with quantities and optional-item choices. Phase 4 explicitly registers these public paths and golden snake_case request/response contracts. All reads require tenant-scoped pricing:read and create no binding mutation.

[DESIGN §3.3](../DESIGN.md#33-api-contracts) fixes canonical errors and route prefixes.
Each mounted route must appear in all four censuses with authz and precondition expectations.

## 6. Data Model

Resolution is a per-item matrix of default and value chains: price_id, dim_value, dim_used, model, price, min_fee, eligibility, window, sku_version, descriptors, billing_timing, rounding_policy, currency scale and active promotion id/version. A binding additionally retains the SKU version's meter/unit. Pricing prices are read forever; consumer pins persist outside this gear. Events use toolkit outbox envelopes, not a second pricing schema.

Tenant-scoped parent validation is required even where foreign keys use entity ids. Never substitute a
cross-gear read for transactional local ownership/version guards. Approved money and historical pins survive.

## 7. Events & Alarms

Core payloads (camelCase on the wire): PricesPublished { book_id, unit_id, prices[] { price_id, price_book_entry_id, dim_value (null is the default chain), effective_from, effective_to, eligibility }, actor_ref }, ApprovalUnitDecided { unit_id, kind, state (approved, rejected or withdrawn), generation, actors[] }, PriceBookEntryReferenceLost { price_book_entry_id, sku_id, reservation_id, actor_ref }; each also names its tenant_id. Later payloads name the published revision, retired plan, promotion id/version or migration request/target/subscriptions. The tenant is an envelope fact. The envelope carries no correlation id (trace_parent is unset); an event joins its audit rows through the subject ids it names (unit_id, price_book_entry_id), which the same transaction audits with the request's correlation id. No SkuChanged subscription is required by phase 2.

Audit and outbox inserts use the same mutation transaction; retry is lifecycle-managed and observes shutdown.

## 8. Definitions of Done

The sole definitions live in [features/read-contract-events.md](../features/read-contract-events.md):

- `cpt-cf-bss-pricing-dod-resolve-matrix` — Full chain resolution matrix.
- `cpt-cf-bss-pricing-dod-renewal-all-new` — Renewal walk and eligibility.
- `cpt-cf-bss-pricing-dod-binding-sku-version` — Descriptors from the dated SKU version.
- `cpt-cf-bss-pricing-dod-price-read-forever` — Durable pinned-price read.
- `cpt-cf-bss-pricing-dod-period-slices` — Period boundary semantics.
- `cpt-cf-bss-pricing-dod-quote-totals` — Studio quote calculation.
- `cpt-cf-bss-pricing-dod-events-typed-outbox` — Typed transactional domain events.
- `cpt-cf-bss-pricing-dod-consumer-golden-contracts` — Frozen consumer golden responses.

## 9. Acceptance Criteria

1. PRD AC #18 / `cpt-cf-bss-pricing-dod-resolve-matrix`: Given one item with EU/default prices, when resolve runs then both inputs are returned; an uncovered chain is explicit and no quantity total appears.
2. PRD AC #18 / `cpt-cf-bss-pricing-dod-renewal-all-new`: Given pinned 10 → all 12 → new 15, when renewal resolves then it chooses 12 and signup 15; a forged foreign-chain pin is refused.
3. PRD AC #18 / `cpt-cf-bss-pricing-dod-binding-sku-version`: Given an October 1 GL change already applied to the current SKU, when September resolves then it binds the earlier version; October binds the new one and prior pins do not change.
4. PRD AC #19 / `cpt-cf-bss-pricing-dod-price-read-forever`: Given a closed price id from an old invoice, when read then its original money is returned; unknown/foreign ids reveal no price.
5. PRD AC #20 / `cpt-cf-bss-pricing-dod-period-slices`: Given a temporary price ending October 11 inside October 5–November 5, when preview runs then two slices appear; their common-price floors are not charged twice.
6. PRD AC #20 / `cpt-cf-bss-pricing-dod-quote-totals`: Given valid quantities and a promotion, when quote runs then totals apply included quantities before prorated floor and promotion afterward; invalid quantities fail without changing pins.
7. PRD AC #14 / `cpt-cf-bss-pricing-dod-events-typed-outbox`: Given approve/reject/withdraw/quorum-zero outcomes, when committed then each has its terminal event and only successful apply has its domain publication; rollback has neither.
8. PRD AC #19 / `cpt-cf-bss-pricing-dod-consumer-golden-contracts`: Given stored contract fixtures including negative tenant/uncovered cases, when either backend serves the public paths then responses match; a shape drift fails the contract gate.

## 10. Non-Functional Considerations

All paths enforce tenant isolation, deny-by-default authz and append-only attribution. Mutations use conditional
versions, required POST replay and PATCH/PUT preconditions; PostgreSQL serializable chain changes and SQLite
writer serialization preserve the same invariants. A failed audit/outbox write cannot leave a committed act.
No timeout releases a live reference. Review and apply retain typed database failures for bounded retry.
Implementation gates cover both backends and route censuses; document gates cover toc, language and identifier ownership.
