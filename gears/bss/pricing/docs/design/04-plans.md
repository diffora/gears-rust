<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Plans (Design, Slice 4) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../DECISIONS.md | Owners: BSS Pricing team -->

# DESIGN — Plans (Slice 4)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-design-slice-04`

<!-- toc -->

- [1. Context](#1-context)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Prepare and publish a revision](#prepare-and-publish-a-revision)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [revision-checks](#revision-checks)
  - [revision-apply](#revision-apply)
  - [clone-and-retire](#clone-and-retire)
- [4. States (CDSL)](#4-states-cdsl)
- [5. API Surface](#5-api-surface)
- [6. Data Model](#6-data-model)
- [7. Events & Alarms](#7-events--alarms)
- [8. Definitions of Done](#8-definitions-of-done)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Non-Functional Considerations](#10-non-functional-considerations)

<!-- /toc -->

## 1. Context

**Delivery:** phase 3. Every checkbox is an implementation obligation, not an assertion about the legacy code. This complete phase 3 design remains unchecked during phase 2. 

Publish independent revision structure against book coverage, preserving existing pins; author items, clone and retirement prerequisites.

Requirements: `cpt-cf-bss-pricing-fr-plans`, `cpt-cf-bss-pricing-fr-reference-protocol`. Architecture: `cpt-cf-bss-pricing-component-plans`, `cpt-cf-bss-pricing-component-approvals`, `cpt-cf-bss-pricing-component-reservations-client`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-principle-reserve-before-write`, `cpt-cf-bss-pricing-constraint-no-row-locks`, `cpt-cf-bss-pricing-seq-blocked-revision`, `cpt-cf-bss-pricing-seq-reserve-write-confirm`.
[FEATURE](../features/plans.md) owns the executable flow/algorithm/DoD identifiers; this slice defines no duplicate DoDs.
Dependencies: `cpt-cf-bss-pricing-feature-rows-windows-dimension`, `cpt-cf-bss-pricing-feature-approvals`.
Source: PriceBook spec §2.2, §5–§8, §12–§13 and [DECISIONS](../DECISIONS.md) D-384–D-400.

## 2. Actor Flows (CDSL)

### Prepare and publish a revision

Actors: `cpt-cf-bss-pricing-actor-product-manager`, `cpt-cf-bss-pricing-actor-products`, `cpt-cf-bss-pricing-actor-subscriptions`. Feature flow: `cpt-cf-bss-pricing-flow-plans`.

1. [ ] - `p1` - Product Manager copies published structure into a new draft revision, or starts a new plan. - `inst-plans-flow-1`
2. [ ] - `p1` - Select one book, items, included quantities, availability and grants; reserve any new SKU item or sold-as reference before writing. - `inst-plans-flow-2`
3. [ ] - `p1` - Read checks for the sale date and all dimension values; show ITEM_UNCOVERED and computed blocked_by row units when coverage is missing. - `inst-plans-flow-3`
4. [ ] - `p1` - After checks pass, submit a separate plan_revision unit; revalidate on apply. - `inst-plans-flow-4`
5. [ ] - `p1` - On approval publish the revision, supersede the previous published revision and advance plan.published_rev atomically; existing subscription pins remain unchanged. - `inst-plans-flow-5`

## 3. Processes / Business Logic (CDSL)

### revision-checks

Feature algorithm: `cpt-cf-bss-pricing-algo-plans-revision-checks`.

1. [ ] - `p1` - Check every item SKU is allowed, non-bundle and not newly deprecated; validate receipt and charge treatment. - `inst-plans-revision-checks-1`
2. [ ] - `p1` - Enforce one recurring frequency, unique usage meter and usage-only included_qty; reject foreign-book prices. - `inst-plans-revision-checks-2`
3. [ ] - `p1` - For every registered dimension value, verify sale-date coverage and an open tail through its own or the default chain; check book validity. - `inst-plans-revision-checks-3`
4. [ ] - `p1` - When uncovered, compute blocking pending row unit ids from current rows; return checks, never persist blocked_by or create a unit while red. - `inst-plans-revision-checks-4`

### revision-apply

Feature algorithm: `cpt-cf-bss-pricing-algo-plans-revision-apply`.

1. [ ] - `p1` - Claim the unit version and verify generation, SoD and business-content fingerprint through slice 05. - `inst-plans-revision-apply-1`
2. [ ] - `p1` - Revalidate book, SKU lifecycle and complete coverage inside apply; changed environment refuses apply without partial publication. - `inst-plans-revision-apply-2`
3. [ ] - `p1` - Publish the selected revision, supersede the previous one and update published_rev, audit and PlanRevisionPublished in one transaction. - `inst-plans-revision-apply-3`
4. [ ] - `p1` - Keep all historical revision/book bindings and subscription pins intact. - `inst-plans-revision-apply-4`

### clone-and-retire

Feature algorithm: `cpt-cf-bss-pricing-algo-plans-clone-and-retire`.

1. [ ] - `p1` - Clone creates a new uniquely coded draft with copied structure and fresh reference attempts; it does not clone approved identity or decisions. - `inst-plans-clone-and-retire-1`
2. [ ] - `p1` - Retirement requires an explicit migration proposal against an eligible published target. - `inst-plans-clone-and-retire-2`
3. [ ] - `p1` - Persist and approve the migration request through slice 06, retaining references required by live or historical bindings. - `inst-plans-clone-and-retire-3`
4. [ ] - `p1` - Wait for the separately implemented Subscriptions completion contract before claiming retirement completion; release references only after durable cancellation/removal is valid. - `inst-plans-clone-and-retire-4`

## 4. States (CDSL)

Revision states are draft → pending → published → superseded, with retired retained for history. Rejected/withdrawn units unlock proposals without publishing them. A blocked draft has no unit; blocked_by is a computed check result. A retirement request does not mean all subscriptions have moved.

State definition: `cpt-cf-bss-pricing-state-plans` in the FEATURE.

## 5. API Surface

Under /bss-pricing/v1: POST/GET /plans; POST /plans/{id}/revisions copies published structure to a draft; PATCH /plan-revisions/{id} for items, book_id, sold-as, availability and grants; GET /plan-revisions/{id}/checks; POST /plan-revisions/{id}/submit; POST /plans/{id}/clone and /retire. Migration proposals use slice 06. Draft mutations require author, submission requires submit; POST replay and PATCH If-Match apply.

[DESIGN §3.3](../DESIGN.md#33-api-contracts) fixes canonical errors and route prefixes.
Each mounted route must appear in all four censuses with authz and precondition expectations.

## 6. Data Model

Phase 3 adds pricing_plan(id, tenant_id, code, name, bundle_sku_id, published_rev), unique tenant/code; pricing_plan_revision(id, tenant_id, plan_id, rev_no, book_id, state, available_from, grants, pending_unit_id, approved_by_unit_id), unique plan/rev_no; pricing_plan_item(id, tenant_id, revision_id, sku_id, price_id nullable, treatment paid|optional|included, included_qty, qty_min), unique revision/SKU. Mutable drafts carry versions/timestamps and creator attribution; plan_item and sold_as references retain receipt/pending-confirm/release recovery as in slice 03. A published revision book binding is immutable. A null price_id denotes an included item with no charge.

Tenant-scoped parent validation is required even where foreign keys use entity ids. Never substitute a
cross-gear read for transactional local ownership/version guards. Approved money and historical pins survive.

## 7. Events & Alarms

PlanRevisionPublished shares the successful apply transaction and terminal ApprovalUnitDecided. PlanRetired represents an actual completed retirement, not merely an approved migration request. SubscriptionMigrationRequested belongs to slice 06. Existing pins remain readable after either event.

Audit and outbox inserts use the same mutation transaction; retry is lifecycle-managed and observes shutdown.

## 8. Definitions of Done

The sole definitions live in [features/plans.md](../features/plans.md):

- `cpt-cf-bss-pricing-dod-plan-revision-book` — Immutable revision book binding.
- `cpt-cf-bss-pricing-dod-plan-item-rules` — Item and frequency rules.
- `cpt-cf-bss-pricing-dod-plan-coverage` — Coverage for every value.
- `cpt-cf-bss-pricing-dod-plan-blocked-by` — Computed blocking units.
- `cpt-cf-bss-pricing-dod-plan-revision-unit` — Independent revision approval.
- `cpt-cf-bss-pricing-dod-plan-reference-protocol` — Item and sold-as references.
- `cpt-cf-bss-pricing-dod-plan-grants` — Minimal grants and included items.
- `cpt-cf-bss-pricing-dod-plan-clone` — Clone into a fresh draft.
- `cpt-cf-bss-pricing-dod-plan-retire-migration` — Retirement requires migration.

## 9. Acceptance Criteria

1. PRD AC #15 / `cpt-cf-bss-pricing-dod-plan-revision-book`: Given published rev 4 in EUR book A, when rev 5 chooses book B then rev 4 keeps A; direct published PATCH is refused.
2. PRD AC #15 / `cpt-cf-bss-pricing-dod-plan-item-rules`: Given otherwise valid items, when a second recurring period or foreign-book price is added then FREQUENCY_MIXED or ITEM_BOOK_FOREIGN blocks submit; the valid set passes.
3. PRD AC #15 / `cpt-cf-bss-pricing-dod-plan-coverage`: Given EU coverage but uncovered US, when checks run then ITEM_UNCOVERED identifies US; complete own-value chains pass without a default, while invalid book dates fail.
4. PRD AC #15 / `cpt-cf-bss-pricing-dod-plan-blocked-by`: Given pending row unit ap-12 covering a gap, when revision checks run then they name ap-12; rejection or withdrawal changes the next check rather than leaving a stored dependency.
5. PRD AC #15 / `cpt-cf-bss-pricing-dod-plan-revision-unit`: Given an approved repricing and rejected revision, when both outcomes are read then the old revision uses the new book money and the rejected revision is not published.
6. PRD AC #11 / `cpt-cf-bss-pricing-dod-plan-reference-protocol`: Given a sold-as reservation and confirmation outage, when the draft commits then the reference stays protective and retryable; bundle items remain forbidden although sold-as bundles are allowed.
7. PRD AC #15 / `cpt-cf-bss-pricing-dod-plan-grants`: Given a revision with an included usage item, when structure is read then grant and included quantity survive; an included quantity on recurring is refused.
8. PRD AC #15 / `cpt-cf-bss-pricing-dod-plan-clone`: Given a published source, when clone succeeds then the destination is a separate draft; duplicate tenant code is refused and changing the clone leaves the source unchanged.
9. PRD AC #15 / `cpt-cf-bss-pricing-dod-plan-retire-migration`: Given subscriptions pinned to a retiring plan, when only the request is approved then movement is not reported complete; an invalid target blocks the request.

## 10. Non-Functional Considerations

All paths enforce tenant isolation, deny-by-default authz and append-only attribution. Mutations use conditional
versions, required POST replay and PATCH/PUT preconditions; PostgreSQL serializable chain changes and SQLite
writer serialization preserve the same invariants. A failed audit/outbox write cannot leave a committed act.
No timeout releases a live reference. Review and apply retain typed database failures for bounded retry.
Implementation gates cover both backends and route censuses; document gates cover toc, language and identifier ownership.
