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

**Delivery:** phase 4 for reads, with quote not built (D-415); 2c for core events; 3 for added events. Every checkbox is an implementation obligation, not an assertion about the legacy code. Core event payloads are built with phase 2 approvals; dependencies on plans/promotions apply only to their later reads and events.

Deliver reproducible resolution matrices, pinned-price reads and Studio quote, plus typed transactional events and consumer goldens. The Studio quote is not built (D-415).

Requirements: `cpt-cf-bss-pricing-fr-resolve`, `cpt-cf-bss-pricing-fr-price-read`, `cpt-cf-bss-pricing-fr-quote`, `cpt-cf-bss-pricing-fr-events`. Architecture: `cpt-cf-bss-pricing-component-read-contract`, `cpt-cf-bss-pricing-component-events`, `cpt-cf-bss-pricing-component-prices`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-constraint-two-backends`.
[FEATURE](../features/read-contract-events.md) owns the executable flow/algorithm/DoD identifiers; this slice defines no duplicate DoDs.
Dependencies: `cpt-cf-bss-pricing-feature-plans`, `cpt-cf-bss-pricing-feature-promotions-migrations`, `cpt-cf-bss-pricing-feature-approvals`.
Source: PriceBook spec §2.2, §5–§8, §12–§13 and [DECISIONS](../DECISIONS.md) D-384–D-423.

## 2. Actor Flows (CDSL)

### Resolve a renewal and preserve invoice inputs

Actors: `cpt-cf-bss-pricing-actor-rating`, `cpt-cf-bss-pricing-actor-subscriptions`, `cpt-cf-bss-pricing-actor-products`, `cpt-cf-bss-pricing-actor-finance-manager`. Feature flow: `cpt-cf-bss-pricing-flow-read-contract-events`.

1. [ ] - `p1` - Rating or Subscriptions sends the revision id, period start and optional current pins. - `inst-read-contract-events-flow-1`
2. [ ] - `p1` - Load immutable revision structure and the full chain matrix, scoped to the tenant. - `inst-read-contract-events-flow-2`
3. [ ] - `p1` - For existing pins walk eligible all successors, stopping before the first new price; for signup select in-force prices. - `inst-read-contract-events-flow-3`
4. [ ] - `p1` - Read Products versions?as_of for the period start, bind descriptors/timing/meter/unit with their source (entry, SKU or tenant, D-421) and return the whole dimension matrix. - `inst-read-contract-events-flow-4`
5. [ ] - `p1` - A later replay reads the pinned price by id (GET /bss-pricing/v1/prices/{id}, D-422): the approved money is served forever, whatever its window; binding usage lazily per value and keeping the complete inputs are the consumer's part. - `inst-read-contract-events-flow-5`
6. [ ] - `p1` - Return the active promotion (id, version) with the matrix: deferred with promotions (D-409), this step stays unticked until they return. - `inst-read-contract-events-flow-6`

## 3. Processes / Business Logic (CDSL)

### renewal-walk

Feature algorithm: `cpt-cf-bss-pricing-algo-read-contract-events-renewal-walk`.

1. [ ] - `p1` - Validate the supplied pin belongs to the tenant, revision item and entry chain. - `inst-read-contract-events-renewal-walk-1`
2. [ ] - `p1` - From the pinned price walk successors with eligibility all, bounded by the relevant date; stop before the first new successor. - `inst-read-contract-events-renewal-walk-2`
3. [ ] - `p1` - Without a pin choose the in-force price per value, falling back to default where the value has no active price. - `inst-read-contract-events-renewal-walk-3`
4. [ ] - `p1` - Return uncovered rather than inventing a price when neither chain covers; preserve historical keep_for_bound prices. - `inst-read-contract-events-renewal-walk-4`

### period-slices-and-quote

Feature algorithm: `cpt-cf-bss-pricing-algo-read-contract-events-period-slices-and-quote`. Not built (D-415): the owner dropped quote and the Studio wiring; consumers read resolve and GET /pricing/v1/prices/{id}, and Rating owns the minimum-fee floor arithmetic.

1. [ ] - `p1` - Split the period at every price boundary inside the bound chain, including a temporary end. - `inst-read-contract-events-period-slices-and-quote-1`
2. [ ] - `p1` - Prorate recurring slices by calendar days; rate usage by reading timestamp with tier counters per slice. - `inst-read-contract-events-period-slices-and-quote-2`
3. [ ] - `p1` - Deduct included quantities, aggregate per price/subscription/period and apply the coverage-prorated min_fee once per price; not built in pricing, Rating applies the floor (D-415). - `inst-read-contract-events-period-slices-and-quote-3`
4. [ ] - `p1` - Apply period-start promotion after floors, then the bound rounding/currency policy; quote returns totals while resolve never does; quote is not built (D-415). - `inst-read-contract-events-period-slices-and-quote-4`

### typed-events

Feature algorithm: `cpt-cf-bss-pricing-algo-read-contract-events-typed-events`.

1. [ ] - `p1` - Implement PricesPublished, ApprovalUnitDecided and PriceBookEntryReferenceLost through broker TypedEvent in phase 2. - `inst-read-contract-events-typed-events-1`
2. [ ] - `p1` - Add PlanRevisionPublished, PlanReferenceLost, PlanRetired, PromotionPublished and SubscriptionMigrationRequested as their phase 3 acts become real; PromotionPublished (D-409), PlanRetired and SubscriptionMigrationRequested (D-410) are deferred. - `inst-read-contract-events-typed-events-2`
3. [ ] - `p1` - Append event and audit through the same mutation transaction; encode the tenant and stable subject identities in the durable envelope, the correlation id staying on the audit rows of the same transaction. - `inst-read-contract-events-typed-events-3`
4. [ ] - `p1` - Deliver from the toolkit dispatcher after commit; test restart/retry and prevent domain publish on reject/withdraw/refresh. - `inst-read-contract-events-typed-events-4`

## 4. States (CDSL)

A consumer binding is created for a period and stays immutable for replay. New all prices affect later renewal binding; a new price blocks forward renewal walking until explicit migration. Closed/superseded/keep_for_bound prices remain readable. Event states belong to toolkit delivery; Pricing does not maintain a second outbound state machine.

State definition: `cpt-cf-bss-pricing-state-read-contract-events` in the FEATURE.

## 5. API Surface

The spec consumer paths GET /pricing/v1/resolve and GET /pricing/v1/prices/{id} are mounted below the gear's base; phase 4 registers them with golden snake_case request/response contracts:

- GET /bss-pricing/v1/resolve?plan_revision_id=&date=&item_id=&pins= (label plan, action read; D-419). plan_revision_id and date (YYYY-MM-DD) are required; item_id resolves that one item only; pins is comma-separated, each pin price_id (the price's own chain) or price_id:dim_value (a default-chain price that value was bound to), at most 1 000. Only a published or superseded revision resolves. Refusals: 409 REVISION_NOT_PUBLISHED (a draft or pending revision); 400 DATE_INVALID; 400 PIN_FOREIGN for the whole request (a pin that names no approved price of an entry an item of this revision names, or a :dim_value pin on a price that is not a default-chain price; the value itself is not checked against today's registry); 400 PIN_DUPLICATE (two pins for one item and value); 400 PINS_TOO_MANY; 404 for an unknown or another tenant's revision, before any Products read, and for an item_id the revision does not have. Each item's SKU version is read as of date as the caller (products read): 503 REGISTRY_UNAVAILABLE when Products cannot answer, Products' own status and code on a definite refusal, and sku_version null for a SKU Products does not know (D-421). A chain that no price covers is not a refusal: it is uncovered (D-420, PRD AC #18).
- GET /bss-pricing/v1/prices/{id} (label price, action read; D-422): an approved price of the tenant, whatever its window (closed, followed by a later price, keep_for_bound), with its entry's SKU, charge kind, period, book and currency; stored facts only, no status or other value computed from today, no authoring internals (version, pending_unit_id, note, created_by). A draft, pending or rejected price, an unknown id and another tenant's id are 404 with one body.

GET /pricing/v1/quote is a Studio preview with quantities and optional-item choices; it is not built, and the Studio is not wired to the API (D-415). Both reads are tenant-scoped and deny-by-default, and write nothing: no binding, no audit row, no idempotency key.

[DESIGN §3.3](../DESIGN.md#33-api-contracts) fixes canonical errors and route prefixes.
Each mounted route must appear in all four censuses with authz and precondition expectations.

## 6. Data Model

Resolution is a per-item matrix of default and value chains (D-420) with each item's SKU version and resolved invoice inputs (D-421); it carries no totals, and the active promotion (id, version) is deferred with promotions (D-409). The resolve response, field by field (the golden contracts freeze it):

| Level | Field | Content |
| --- | --- | --- |
| revision | plan_revision_id, plan_id, rev_no, state | The revision resolved; state is published or superseded. |
| revision | book_id, currency, currency_minor_digits | The revision's book, its currency and that currency's scale (domain::book::minor_digits). |
| revision | rounding_policy | The tenant default_rounding. |
| revision | date | The date resolved (YYYY-MM-DD). |
| revision | items | One per item of the revision, or the one item_id names. |
| item | item_id, sku_id, treatment, included_qty, qty_min | The item as stored; included_qty is exact decimal text or null. |
| item | price_book_entry_id, charge_kind, period | The item's entry and its key; null for an included item without an entry, which has no chains. |
| item | sku_version | { published_version, effective_from } of the SKU version in force on date; null when Products has no version on that date or does not know the SKU. |
| item | invoice_line_template | { value, source }: the entry's invoice_line_override (source entry), else the SKU version's template (sku), else the tenant template for the charge kind (tenant; an item without an entry takes its SKU version's type); { null, null } when none. |
| item | gl_code | { value, source }: the SKU version's (sku), else the tenant default_gl (tenant), else { null, null }. |
| item | tax_category | { value, source }: the SKU version's (sku), else the tenant default_tax_category (tenant), else { null, null }. |
| item | billing_timing | { value, source }: the SKU version's (sku), else the tenant default_timing (tenant); PRD AC #13. |
| item | meter | { usage_type_ref, unit } of that SKU version; both null without a version. |
| item | chains | The default chain first, then each value registered today for the entry's dimension key in the registry's order, then any other value a pin names (a removed value still resolves for its pin). |
| chain | dim_value | The value; null for the default chain. |
| chain | uncovered | True when neither the value's own chain nor the default binds on date; binding is then null. Never a refusal, never an invented price. |
| chain | binding | The price bound for the period, or null. |
| binding | price_id | The bound price. |
| binding | dim_used | The chain the bound price belongs to: the value, or null for the default chain. |
| binding | pinned_from | The pin the renewal walk started from; null for a signup. |
| binding | model, price, min_fee | The price's model and money, exact decimal text; min_fee null when the price has none. |
| binding | eligibility | all or new. |
| binding | effective_from, effective_to, temporary_until | The stored window and the temporary end, if any. |
| binding | keep_for_bound | Whether the price is kept for pinned subscriptions (the predecessor of a new price). |

The pinned price read returns one approved price's stored facts with its entry's SKU, charge kind, period, book and currency (D-422). Pricing prices are read forever; consumer pins persist outside this gear. Events use toolkit outbox envelopes, not a second pricing schema.

Tenant-scoped parent validation is required even where foreign keys use entity ids. Never substitute a
cross-gear read for transactional local ownership/version guards. Approved money and historical pins survive.

## 7. Events & Alarms

Core payloads (camelCase on the wire): PricesPublished { book_id, unit_id, prices[] { price_id, price_book_entry_id, dim_value (null is the default chain), effective_from, effective_to, eligibility }, actor_ref }, ApprovalUnitDecided { unit_id, kind, state (approved, rejected or withdrawn), generation, actors[] }, PriceBookEntryReferenceLost { price_book_entry_id, sku_id, reservation_id, actor_ref }, and in phase 3 PlanRevisionPublished { plan_id, revision_id, rev_no, book_id, superseded_revision_id (null for a first publication), unit_id, actor_ref } about the plan; each also names its tenant_id. Later payloads name the published revision, retired plan, promotion id/version or migration request/target/subscriptions. The tenant is an envelope fact. The envelope carries no correlation id (trace_parent is unset); an event joins its audit rows through the subject ids it names (unit_id, price_book_entry_id), which the same transaction audits with the request's correlation id. No SkuChanged subscription is required by phase 2.

Every event is a broker TypedEvent of source bss-pricing on the topic `gts.cf.core.events.topic.v1~cf.bss.pricing.catalog.v1`, and the bound producer prepares every type at bind (a broker that lacks one fails the boot). Type ids are `gts.cf.core.events.event.v1~cf.bss.pricing.<name>.v1~`; subject types are `gts.cf.core.events.subject.v1~cf.bss.pricing.<subject>.v1`.

| Event | `<name>` | Subject | Trigger | Phase |
| --- | --- | --- | --- | --- |
| PricesPublished | `prices_published` | `price_book` (the book) | A `prices` unit is applied: quorum 0 at submit or publish-changes, or the approving vote. | 2 |
| ApprovalUnitDecided | `approval_unit_decided` | `approval_unit` (the unit) | Every terminal transition of a unit of any kind: approved (applied), rejected or withdrawn. | 2 |
| PriceBookEntryReferenceLost | `price_book_entry_reference_lost` | `price_book_entry` (the entry) | A rereserve of an entry whose reservation Products released ends refused (the SKU is fenced, retiring or retired). | 2 |
| PlanRevisionPublished | `plan_revision_published` | `plan` (the plan) | A `plan_revision` unit is applied: the revision is published, its predecessor superseded and `published_rev` advanced. | 3 |
| PlanReferenceLost | `plan_reference_lost` | `plan_item` (the item) | A plan item's attach (a copied item, D-413) or rereserve ends refused. | 3 |
| PromotionPublished | — | — | Deferred with promotions (D-409). | — |
| PlanRetired, SubscriptionMigrationRequested | — | — | Deferred with retirement and migration requests (D-410). | — |

Audit and outbox inserts use the same mutation transaction; retry is lifecycle-managed and observes shutdown.

## 8. Definitions of Done

The sole definitions live in [features/read-contract-events.md](../features/read-contract-events.md):

- `cpt-cf-bss-pricing-dod-resolve-matrix` — Full chain resolution matrix.
- `cpt-cf-bss-pricing-dod-renewal-all-new` — Renewal walk and eligibility.
- `cpt-cf-bss-pricing-dod-binding-sku-version` — Descriptors from the dated SKU version.
- `cpt-cf-bss-pricing-dod-price-read-forever` — Durable pinned-price read.
- `cpt-cf-bss-pricing-dod-period-slices` — Period boundary semantics. (not built, D-415)
- `cpt-cf-bss-pricing-dod-quote-totals` — Studio quote calculation (not built, D-415).
- `cpt-cf-bss-pricing-dod-events-typed-outbox` — Typed transactional domain events.
- `cpt-cf-bss-pricing-dod-consumer-golden-contracts` — Frozen consumer golden responses.

## 9. Acceptance Criteria

1. PRD AC #18 / `cpt-cf-bss-pricing-dod-resolve-matrix`: Given one item with EU/default prices, when resolve runs then both inputs are returned; an uncovered chain is explicit and no quantity total appears.
2. PRD AC #18 / `cpt-cf-bss-pricing-dod-renewal-all-new`: Given pinned 10 → all 12 → new 15, when renewal resolves then it chooses 12 and signup 15; a forged foreign-chain pin is refused.
3. PRD AC #18 / `cpt-cf-bss-pricing-dod-binding-sku-version`: Given an October 1 GL change already applied to the current SKU, when September resolves then it binds the earlier version; October binds the new one and prior pins do not change.
4. PRD AC #19 / `cpt-cf-bss-pricing-dod-price-read-forever`: Given a closed price id from an old invoice, when read then its original money is returned; unknown/foreign ids reveal no price.
5. PRD AC #20 / `cpt-cf-bss-pricing-dod-period-slices` (not built, D-415): Given a temporary price ending October 11 inside October 5–November 5, when preview runs then two slices appear; their common-price floors are not charged twice.
6. PRD AC #20 / `cpt-cf-bss-pricing-dod-quote-totals` (not built, D-415): Given valid quantities and a promotion, when quote runs then totals apply included quantities before prorated floor and promotion afterward; invalid quantities fail without changing pins.
7. PRD AC #14 / `cpt-cf-bss-pricing-dod-events-typed-outbox`: Given approve/reject/withdraw/quorum-zero outcomes, when committed then each has its terminal event and only successful apply has its domain publication; rollback has neither.
8. PRD AC #19 / `cpt-cf-bss-pricing-dod-consumer-golden-contracts`: Given stored contract fixtures including negative tenant/uncovered cases, when either backend serves the public paths then responses match; a shape drift fails the contract gate.

## 10. Non-Functional Considerations

All paths enforce tenant isolation, deny-by-default authz and append-only attribution. Mutations use conditional
versions, required POST replay and PATCH/PUT preconditions; PostgreSQL serializable chain changes and SQLite
writer serialization preserve the same invariants. A failed audit/outbox write cannot leave a committed act.
No timeout releases a live reference. Review and apply retain typed database failures for bounded retry.
Implementation gates cover both backends and route censuses; document gates cover toc, language and identifier ownership.
