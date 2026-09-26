<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Plans (Feature) -->
<!-- Related: ../DECOMPOSITION.md, ../DESIGN.md, ../PRD.md | Owners: BSS Pricing team -->

# Feature: Plans

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-featstatus-plans-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-pricing-feature-plans`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Prepare and publish a revision](#prepare-and-publish-a-revision)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [revision-checks](#revision-checks)
  - [revision-apply](#revision-apply)
  - [clone-and-retire](#clone-and-retire)
- [4. States (CDSL)](#4-states-cdsl)
  - [Plans states](#plans-states)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Immutable revision book binding](#immutable-revision-book-binding)
  - [Item and frequency rules](#item-and-frequency-rules)
  - [Coverage for every value](#coverage-for-every-value)
  - [Computed blocking units](#computed-blocking-units)
  - [Independent revision approval](#independent-revision-approval)
  - [Item references](#item-references)
  - [Included items](#included-items)
  - [Clone into a fresh draft](#clone-into-a-fresh-draft)
  - [Retirement requires migration](#retirement-requires-migration)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

**Delivery:** phase 3. Every checkbox is an implementation obligation, not an assertion about the legacy code. This complete phase 3 design remains unchecked during phase 2.

This feature implements [slice 04](../design/04-plans.md) — `cpt-cf-bss-pricing-design-slice-04`.
[DECOMPOSITION](../DECOMPOSITION.md) records integration order; [DESIGN §3](../DESIGN.md#3-technical-architecture)
is the schema and transaction authority. Unchecked phase 3/4 work is not part of the phase 2 core gate.

### 1.2 Purpose

Publish independent revision structure against book coverage, preserving existing pins; author items, clone and retirement prerequisites.

Requirements: `cpt-cf-bss-pricing-fr-plans`, `cpt-cf-bss-pricing-fr-reference-protocol`.

Architecture: `cpt-cf-bss-pricing-component-plans`, `cpt-cf-bss-pricing-component-approvals`, `cpt-cf-bss-pricing-component-reservations-client`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-principle-reserve-before-write`, `cpt-cf-bss-pricing-constraint-no-row-locks`, `cpt-cf-bss-pricing-seq-blocked-revision`, `cpt-cf-bss-pricing-seq-reserve-write-confirm`.

### 1.3 Actors

`cpt-cf-bss-pricing-actor-product-manager`, `cpt-cf-bss-pricing-actor-products`, `cpt-cf-bss-pricing-actor-subscriptions`. Every operation authenticates and derives tenant scope before storage, replay or cross-gear calls.
Holding multiple permissions never bypasses separation of duties.

### 1.4 References

- [PRD](../PRD.md), especially the numbered acceptance criteria referenced below.
- [DESIGN](../DESIGN.md), §3 model, API contracts, transaction sequences and DDL.
- [Slice 04](../design/04-plans.md), including API, data and event obligations.
- [DECISIONS](../DECISIONS.md), D-384–D-426; spec means `docs/superpowers/specs/2026-09-24-pricebook-model-design.md` in the main checkout.
- Source: spec §2 decisions 4–8, 13–17, §2.2, §5–§8, §10, §12–§13; the phase 2 plan supplies delivery boundaries and D-399/D-400.

## 2. Actor Flows (CDSL)

### Prepare and publish a revision

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-flow-plans`

1. [x] - `p1` - Product Manager copies published structure into a new draft revision, or starts a new plan. - `inst-plans-flow-1`
2. [x] - `p1` - Select one book, items, included quantities and availability; add each item through the item sub-resource, which reserves its reference before the write (D-407), while a copied item attaches its reference after the copy is written (D-413). Grants and the sold-as bundle SKU are deferred by the owner (D-411). - `inst-plans-flow-2`
3. [x] - `p1` - Read checks for the sale date and all dimension values; show ITEM_UNCOVERED and computed blocked_by price units when coverage is missing. - `inst-plans-flow-3`
4. [x] - `p1` - After checks pass, submit a separate plan_revision unit; revalidate on apply. - `inst-plans-flow-4`
5. [x] - `p1` - On approval publish the revision, supersede the previous published revision and advance plan.published_rev atomically; existing subscription pins remain unchanged. - `inst-plans-flow-5`

## 3. Processes / Business Logic (CDSL)

### revision-checks

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-algo-plans-revision-checks`

1. [x] - `p1` - Read every item SKU fresh and check it is allowed, non-bundle and not newly deprecated (D-408); validate every item reference's receipt (D-413) and the charge treatment. - `inst-plans-revision-checks-1`
2. [x] - `p1` - Enforce one recurring frequency, unique usage meter and usage-only included_qty; reject foreign-book entries. - `inst-plans-revision-checks-2`
3. [x] - `p1` - For every registered dimension value, verify sale-date coverage and an open tail through its own or the default chain; check book validity. - `inst-plans-revision-checks-3`
4. [x] - `p1` - When uncovered, compute blocking pending price unit ids from current prices; return checks, never persist blocked_by or create a unit while red. - `inst-plans-revision-checks-4`

### revision-apply

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-algo-plans-revision-apply`

1. [x] - `p1` - Claim the unit version and verify generation, SoD and business-content fingerprint through slice 05. - `inst-plans-revision-apply-1`
2. [x] - `p1` - Revalidate book, SKU lifecycle and complete coverage inside apply; changed environment refuses apply without partial publication. - `inst-plans-revision-apply-2`
3. [x] - `p1` - Publish the selected revision, supersede the previous one and update published_rev, audit and PlanRevisionPublished in one transaction. - `inst-plans-revision-apply-3`
4. [x] - `p1` - Keep all historical revision/book bindings and subscription pins intact. - `inst-plans-revision-apply-4`

### clone-and-retire

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-plans-clone-and-retire`

Retirement (steps 2 to 4) is deferred by the owner (D-410, 2026-09-25) and not built in phase 3; clone stays.

1. [x] - `p1` - Clone creates a new uniquely coded draft with copied structure and fresh reference attempts; it does not clone approved identity or decisions. - `inst-plans-clone-and-retire-1`
2. [ ] - `p1` - Retirement requires an explicit migration proposal against an eligible published target. - `inst-plans-clone-and-retire-2`
3. [ ] - `p1` - Persist and approve the migration request through slice 06, retaining references required by live or historical bindings. - `inst-plans-clone-and-retire-3`
4. [ ] - `p1` - Wait for the separately implemented Subscriptions completion contract before claiming retirement completion; release references only after durable cancellation/removal is valid. - `inst-plans-clone-and-retire-4`

## 4. States (CDSL)

### Plans states

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-state-plans`

Revision states are draft → pending → published → superseded; submit locks the draft under its unit (pending_unit_id, conditional), a rejected or withdrawn unit returns its revision to an editable draft without publishing it, and apply supersedes the published revision before it publishes this one. Plan retirement is deferred by the owner (D-410). A blocked draft has no unit; blocked_by is a computed check result. A retirement request does not mean all subscriptions have moved.

## 5. Definitions of Done

Every DoD below is required for this feature's delivery phase. Constraints: `cpt-cf-bss-pricing-constraint-no-row-locks`.

### Immutable revision book binding

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-revision-book`

Each published revision owns one book and immutable item structure. Copying to draft allocates a new revision number; publishing cannot rewrite a historical book binding (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Item and frequency rules

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-item-rules`

Validate recurring frequency, duplicate usage meters, treatment and usage-only included quantities. Reject bundle items, a deprecated SKU newly added (one carried over from the same plan's published revision stays, D-408) and foreign-book entries using the named spec errors (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Coverage for every value

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-coverage`

Sale-date checks include each dimension value via own chain or default plus open-tail coverage and book validity. Missing default alone is not an error when every value is covered (spec §5, §14).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Computed blocking units

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-blocked-by`

Checks derive blocked_by from current pending prices of uncovered entries. A red draft creates no plan_revision unit and becomes eligible only after a fresh successful check (spec §6, §8).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Independent revision approval

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-revision-unit`

plan_revision uses the shared generation/SoD/quorum rules and revalidates coverage on apply. It publishes structure independently of approved money and never migrates pins (spec §6, §12).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Item references

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-reference-protocol`

A new plan_item reference, added through the item sub-resource, uses reserve, SKU re-read, local receipt/work commit and confirm (D-407). A copied item is written unreserved and attaches after the write (D-413). Removal/cancellation precedes durable release, with the same lost-receipt handling as entries; published and superseded revisions keep their references (D-414) (spec §13). The sold_as reference is deferred with the sold-as bundle (D-411).

Requirement: `cpt-cf-bss-pricing-fr-reference-protocol`; PRD AC #11.

### Included items

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-grants`

Included items have no entry charge, and usage included_qty is applied before money floors at quote/rating time; no phase schedule or prepaid-grant system returns (spec §3 item 25, §5). Revision grants are deferred by the owner (D-411).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Clone into a fresh draft

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-clone`

Clone preserves authorable structure while allocating new plan/revision ids and reference attempts: the new plan's draft rev 1 copies the source's published revision (book, availability, items), each item written unreserved and attached after the write (D-413). It does not inherit approval decisions, published state (approved_by_unit_id, published_at) or subscription pins; a source with no published revision is refused CLONE_SOURCE_UNPUBLISHED, and a carried deprecated SKU is red in the new plan's checks (D-408) (spec §3 item 28).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Retirement requires migration

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-retire-migration`

Deferred by the owner (D-410, 2026-09-25): not built in phase 3. This DoD stays unticked.

Retirement is coupled to an explicit migration request and cannot silently strand subscribers. Pricing records request approval; Subscriptions executes movement and confirms completion in its own plan (spec §11).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

## 6. Acceptance Criteria

| DoD | PRD criterion | Given / When / Then |
| --- | --- | --- |
| `cpt-cf-bss-pricing-dod-plan-revision-book` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given published rev 4 in EUR book A, when rev 5 chooses book B then rev 4 keeps A; direct published PATCH is refused. |
| `cpt-cf-bss-pricing-dod-plan-item-rules` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given otherwise valid items, when a second recurring period or foreign-book entry is added then FREQUENCY_MIXED or ITEM_BOOK_FOREIGN blocks submit; the valid set passes. |
| `cpt-cf-bss-pricing-dod-plan-coverage` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given EU coverage but uncovered US, when checks run then ITEM_UNCOVERED identifies US; complete own-value chains pass without a default, while invalid book dates fail. |
| `cpt-cf-bss-pricing-dod-plan-blocked-by` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given pending price unit ap-12 covering a gap, when revision checks run then they name ap-12; rejection or withdrawal changes the next check rather than leaving a stored dependency. |
| `cpt-cf-bss-pricing-dod-plan-revision-unit` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given an approved repricing and rejected revision, when both outcomes are read then the old revision uses the new book money and the rejected revision is not published. |
| `cpt-cf-bss-pricing-dod-plan-reference-protocol` | AC #11; `cpt-cf-bss-pricing-fr-reference-protocol` | Given a plan_item reservation and confirmation outage, when the draft commits then the reference stays protective and retryable; bundle items remain forbidden. |
| `cpt-cf-bss-pricing-dod-plan-grants` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given a revision with an included usage item, when structure is read then its included quantity survives; an included quantity on recurring is refused. |
| `cpt-cf-bss-pricing-dod-plan-clone` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given a published source, when clone succeeds then the destination is a separate draft; duplicate tenant code is refused and changing the clone leaves the source unchanged. |
| `cpt-cf-bss-pricing-dod-plan-retire-migration` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Deferred (D-410). Given subscriptions pinned to a retiring plan, when only the request is approved then movement is not reported complete; an invalid target blocks the request. |

Verification uses domain tests, scoped repository tests on both backends and REST positive/denial/precondition probes as applicable. Phase 2 checks must not mark later-phase behavior implemented. Golden consumer contracts belong to phase 4.
