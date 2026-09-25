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
  - [Item and sold-as references](#item-and-sold-as-references)
  - [Minimal grants and included items](#minimal-grants-and-included-items)
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
- [DECISIONS](../DECISIONS.md), D-384–D-400; spec means `docs/superpowers/specs/2026-09-24-pricebook-model-design.md` in the main checkout.
- Source: spec §2 decisions 4–8, 13–17, §2.2, §5–§8, §10, §12–§13; the phase 2 plan supplies delivery boundaries and D-399/D-400.

## 2. Actor Flows (CDSL)

### Prepare and publish a revision

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-flow-plans`

1. [ ] - `p1` - Product Manager copies published structure into a new draft revision, or starts a new plan. - `inst-plans-flow-1`
2. [ ] - `p1` - Select one book, items, included quantities, availability and grants; reserve any new SKU item or sold-as reference before writing. - `inst-plans-flow-2`
3. [ ] - `p1` - Read checks for the sale date and all dimension values; show ITEM_UNCOVERED and computed blocked_by row units when coverage is missing. - `inst-plans-flow-3`
4. [ ] - `p1` - After checks pass, submit a separate plan_revision unit; revalidate on apply. - `inst-plans-flow-4`
5. [ ] - `p1` - On approval publish the revision, supersede the previous published revision and advance plan.published_rev atomically; existing subscription pins remain unchanged. - `inst-plans-flow-5`

## 3. Processes / Business Logic (CDSL)

### revision-checks

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-plans-revision-checks`

1. [ ] - `p1` - Check every item SKU is allowed, non-bundle and not newly deprecated; validate receipt and charge treatment. - `inst-plans-revision-checks-1`
2. [ ] - `p1` - Enforce one recurring frequency, unique usage meter and usage-only included_qty; reject foreign-book prices. - `inst-plans-revision-checks-2`
3. [ ] - `p1` - For every registered dimension value, verify sale-date coverage and an open tail through its own or the default chain; check book validity. - `inst-plans-revision-checks-3`
4. [ ] - `p1` - When uncovered, compute blocking pending row unit ids from current rows; return checks, never persist blocked_by or create a unit while red. - `inst-plans-revision-checks-4`

### revision-apply

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-plans-revision-apply`

1. [ ] - `p1` - Claim the unit version and verify generation, SoD and business-content fingerprint through slice 05. - `inst-plans-revision-apply-1`
2. [ ] - `p1` - Revalidate book, SKU lifecycle and complete coverage inside apply; changed environment refuses apply without partial publication. - `inst-plans-revision-apply-2`
3. [ ] - `p1` - Publish the selected revision, supersede the previous one and update published_rev, audit and PlanRevisionPublished in one transaction. - `inst-plans-revision-apply-3`
4. [ ] - `p1` - Keep all historical revision/book bindings and subscription pins intact. - `inst-plans-revision-apply-4`

### clone-and-retire

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-plans-clone-and-retire`

1. [ ] - `p1` - Clone creates a new uniquely coded draft with copied structure and fresh reference attempts; it does not clone approved identity or decisions. - `inst-plans-clone-and-retire-1`
2. [ ] - `p1` - Retirement requires an explicit migration proposal against an eligible published target. - `inst-plans-clone-and-retire-2`
3. [ ] - `p1` - Persist and approve the migration request through slice 06, retaining references required by live or historical bindings. - `inst-plans-clone-and-retire-3`
4. [ ] - `p1` - Wait for the separately implemented Subscriptions completion contract before claiming retirement completion; release references only after durable cancellation/removal is valid. - `inst-plans-clone-and-retire-4`

## 4. States (CDSL)

### Plans states

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-state-plans`

Revision states are draft → pending → published → superseded, with retired retained for history. Rejected/withdrawn units unlock proposals without publishing them. A blocked draft has no unit; blocked_by is a computed check result. A retirement request does not mean all subscriptions have moved.

## 5. Definitions of Done

Every DoD below is required for this feature's delivery phase. Constraints: `cpt-cf-bss-pricing-constraint-no-row-locks`.

### Immutable revision book binding

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-revision-book`

Each published revision owns one book and immutable item structure. Copying to draft allocates a new revision number; publishing cannot rewrite a historical book binding (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Item and frequency rules

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-item-rules`

Validate recurring frequency, duplicate usage meters, treatment and usage-only included quantities. Reject bundle items, deprecated SKUs in new revisions and foreign-book prices using the named spec errors (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Coverage for every value

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-coverage`

Sale-date checks include each dimension value via own chain or default plus open-tail coverage and book validity. Missing default alone is not an error when every value is covered (spec §5, §14).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Computed blocking units

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-blocked-by`

Checks derive blocked_by from current pending rows of uncovered prices. A red draft creates no plan_revision unit and becomes eligible only after a fresh successful check (spec §6, §8).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Independent revision approval

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-revision-unit`

plan_revision uses the shared generation/SoD/quorum rules and revalidates coverage on apply. It publishes structure independently of approved money and never migrates pins (spec §6, §12).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Item and sold-as references

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-reference-protocol`

New plan_item and sold_as references use reserve, SKU re-read, local receipt/work commit and confirm. Removal/cancellation precedes durable release, with the same lost-receipt handling as prices (spec §13).

Requirement: `cpt-cf-bss-pricing-fr-reference-protocol`; PRD AC #11.

### Minimal grants and included items

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-grants`

Revision grants remain structural entitlement inputs. Included items have no price charge, and usage included_qty is applied before money floors at quote/rating time; no phase schedule or prepaid-grant system returns (spec §3 item 25, §5).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Clone into a fresh draft

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-clone`

Clone preserves authorable structure while allocating new plan/revision ids and reference attempts. It does not inherit approval decisions, published state or subscription pins (spec §3 item 28).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

### Retirement requires migration

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-plan-retire-migration`

Retirement is coupled to an explicit migration request and cannot silently strand subscribers. Pricing records request approval; Subscriptions executes movement and confirms completion in its own plan (spec §11).

Requirement: `cpt-cf-bss-pricing-fr-plans`; PRD AC #15.

## 6. Acceptance Criteria

| DoD | PRD criterion | Given / When / Then |
| --- | --- | --- |
| `cpt-cf-bss-pricing-dod-plan-revision-book` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given published rev 4 in EUR book A, when rev 5 chooses book B then rev 4 keeps A; direct published PATCH is refused. |
| `cpt-cf-bss-pricing-dod-plan-item-rules` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given otherwise valid items, when a second recurring period or foreign-book price is added then FREQUENCY_MIXED or ITEM_BOOK_FOREIGN blocks submit; the valid set passes. |
| `cpt-cf-bss-pricing-dod-plan-coverage` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given EU coverage but uncovered US, when checks run then ITEM_UNCOVERED identifies US; complete own-value chains pass without a default, while invalid book dates fail. |
| `cpt-cf-bss-pricing-dod-plan-blocked-by` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given pending row unit ap-12 covering a gap, when revision checks run then they name ap-12; rejection or withdrawal changes the next check rather than leaving a stored dependency. |
| `cpt-cf-bss-pricing-dod-plan-revision-unit` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given an approved repricing and rejected revision, when both outcomes are read then the old revision uses the new book money and the rejected revision is not published. |
| `cpt-cf-bss-pricing-dod-plan-reference-protocol` | AC #11; `cpt-cf-bss-pricing-fr-reference-protocol` | Given a sold-as reservation and confirmation outage, when the draft commits then the reference stays protective and retryable; bundle items remain forbidden although sold-as bundles are allowed. |
| `cpt-cf-bss-pricing-dod-plan-grants` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given a revision with an included usage item, when structure is read then grant and included quantity survive; an included quantity on recurring is refused. |
| `cpt-cf-bss-pricing-dod-plan-clone` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given a published source, when clone succeeds then the destination is a separate draft; duplicate tenant code is refused and changing the clone leaves the source unchanged. |
| `cpt-cf-bss-pricing-dod-plan-retire-migration` | AC #15; `cpt-cf-bss-pricing-fr-plans` | Given subscriptions pinned to a retiring plan, when only the request is approved then movement is not reported complete; an invalid target blocks the request. |

Verification uses domain tests, scoped repository tests on both backends and REST positive/denial/precondition probes as applicable. Phase 2 checks must not mark later-phase behavior implemented. Golden consumer contracts belong to phase 4.
