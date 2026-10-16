---
description: "Immediate component additions and quantity increases with atomic composition validation and preserved commercial terms."
---

# Feature: Composition Changes and Commercial Boundaries


<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Apply one atomic changeComposition](#apply-one-atomic-changecomposition)
  - [Schedule or withdraw a future change](#schedule-or-withdraw-a-future-change)
  - [Replace across unsupported commercial boundaries](#replace-across-unsupported-commercial-boundaries)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Commercial boundary and correction](#commercial-boundary-and-correction)
  - [Ramps and manual supersession](#ramps-and-manual-supersession)
  - [Backdated amendments and migrations](#backdated-amendments-and-migrations)
- [4. States (CDSL)](#4-states-cdsl)
  - [Change operation state](#change-operation-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Atomic changes and recoverable boundaries](#atomic-changes-and-recoverable-boundaries)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-featstatus-changes-implemented`
- [ ] `p1` - `cpt-cf-bss-subscriptions-feature-changes`

**Status:** proposed implementation. S-Q01 activation and S-Q02 occupancy, including suspended-place retention and confirmed-order reservation before capture, are user-accepted; their owner contracts/runtime evidence remain open. Reservation/financial protocols, special replacement exemptions and exact policy/key movement mappings remain gated under S-Q02/S-U10; entitlement provider-dependent quantity restrictions require the external contract.
**Current-profile boundary, 2026-10-15:** apply the [current delivery scope](../PRD.md). Audit subsystem, confirmAcceptance, ramps, co-term, collection pause, notifications, tenant/payer transfer, administrative backdating/correction and optional brand/PlanTier metadata are deferred. Earlier references and scenarios involving those capabilities are future context only, not current acceptance or implementation tasks. Retain authorization, operation/event provenance, reliable effects and current facts/replay.

**Additional current-profile boundary, 2026-10-15:** the [additional scope reduction](../PRD.md#11-purpose) defers trial extension/repeat, acquisition without Orders, scheduled composition/resume, linked subscription replacement and base-plan changes. Retain supported immediate increases/additions, existing lifecycle, future order starts/cancellation and ordinary timers, both trial variants and early conversion to the accepted paid target. Deferred paths are refused even with explicit policy or operator authorization; their earlier examples are future context, not launch gates.

## 1. Feature Context

### 1.1 Overview

Apply one complete resulting subscription composition, preserving quantity provenance, order correlation and a single commercial boundary.

### 1.2 Purpose

**Requirements:** `cpt-cf-bss-subscriptions-fr-plan-change-boundary`, `cpt-cf-bss-subscriptions-fr-proration-triggers`, `cpt-cf-bss-subscriptions-fr-proration-ownership`, `cpt-cf-bss-subscriptions-fr-backdated-changes`, `cpt-cf-bss-subscriptions-fr-overlap-cardinality`, `cpt-cf-bss-subscriptions-fr-update-quantity`, `cpt-cf-bss-subscriptions-fr-ramp-execution`, `cpt-cf-bss-subscriptions-fr-composite-change`.

**Principles:** `cpt-cf-bss-subscriptions-principle-when-not-math`, `cpt-cf-bss-subscriptions-principle-effective-dated-composition`.

### 1.3 Actors

| Actor | Role |
| --- | --- |
| `cpt-cf-bss-subscriptions-actor-customer` | Requests supported immediate additions/increases through its authorized channel. |
| `cpt-cf-bss-subscriptions-actor-contracts` | Supplies applicable accepted terms; ramp execution is deferred. |
| Orders Workflow | Submits all order change items and expected revision as one intent. |

### 1.4 References

- **PRD:** [PRD.md](../PRD.md); **Design:** [components](../DESIGN.md#32-component-model).
- **Build prerequisites:** [foundation](./01-foundation.md) and [composition](./02-composition.md), as allocated in [DECOMPOSITION.md](../DECOMPOSITION.md).
- **Provider prerequisites / integration acceptance:** accepted target compatibility from Products/Pricing; committed change publication with [commercial facts](./08-commercial-facts.md) and affected entitlement/resource providers must pass their joint fixtures before enablement. Those integration checks do not add feature build dependencies.
- **Order profile:** [Orders Changes PRD](../../../orders-changes/docs/PRD.md) defines increases/additions and a non-terminal target. Draft application additionally requires S-U01 agreement on combining immutable acquisition and delta acceptances before first activation; S-Q01 governs any affected first hold.
- **Source decisions:** SUB-D-02/04/15/19/20/21/27 amended by PriceBook and the reconciliation decisions.

## 2. Actor Flows (CDSL)

### Apply one atomic changeComposition

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-change-boundary-chg`

**Initial quantity-change scope accepted, 2026-10-14:** support quantity increases and component additions under the existing Orders contract. Customer-requested quantity decreases and component removals are deferred, including through direct/operator wrappers; such requests refuse before effects. Transfers of purchased quantity or resources between commercial purchase lots are outside this profile. Ordinary whole-subscription cancellation, current-operation full fact replacements and external entitlement user assignment remain; administrative past edits and tenant/payer transfer are deferred. The accepted reduction rule is retained only for a future phase.

**Success:** every requested item applies at one boundary and revision. **Errors:** changed replay content, stale revision, unsupported date mixture, invalid resulting bounds, occupancy conflict or missing external capability.

1. Authenticate and perform replay lookup; bind the authorized purchase-business identity, order/version/line and request correlation. A repeated purchase with another transport key recovers the same result/lot; conflicting purchase content refuses. A new attempt/version must follow S-U01 association/settlement and cannot implicitly duplicate the purchase. Require expected aggregate revision, matching order/subscription tenant axes, no conflicting change in flight and a non-terminal target. Apply the source-state matrix in §4 at submission and inside the completing transaction; non-terminality does not implicitly activate or resume the target.
2. Select the declared operation profile before normalization. The order-originated launch profile permits only `add_component` and `increase_quantity` for eligible recurring/usage components; reject one-time/setup rows, decreases, removals and plan replacement before effects. Derive add-versus-increase from the target composition and reject a caller kind mismatch. `increase_quantity` carries a strictly positive added quantity; it is not an unqualified replacement total. Direct/operator wrappers must enforce the same initial-profile prohibition on decreases, component removals and transfers between purchase lots. Other separately scoped operations retain their own declared capability gates and shared validator. Preserve logical item IDs on changes and assign new IDs only for new items; reject duplicate or contradictory item operations.
3. Resolve accepted **delta** evidence and provider-supported compatibility, retaining every item's order-line and binding provenance alongside unchanged predecessor evidence. Validate base/optional-item constraints and min/max/step/required/eligible limits on the **result**, not on each operation against the old composition. Validate currency/market compatibility and delta purchase floors. A component addition does not fail occupancy by counting its own subscription twice. Never mutate the acquisition receipt or treat accepted delta money as a repricing of the existing quantity. If an existing logical item's added quantity has a different accepted binding from its existing quantity, refuse before effects until S-U08 supplies the versioned quantity-lot allocation described in [composition](./02-composition.md#interval-and-identity-discipline). One total quantity and one temporal binding cannot preserve both purchases. This gate does not disable same-binding increases or separately identified add-on items.
4. Require a supported immediate change; refuse base-plan changes and future requested boundaries or policy requiring delayed application. Do not coerce a future request to immediate. Derive the actual applied instant through the normal guards; the next-cycle decrease default is retained for a future phase and cannot enable a decrease in this profile. Order-driven requests reject past boundaries. If items require different effective instants, reject the batch before effects; multi-date atomic commands require a separately accepted design.
5. Refuse decreases, removals and transfers between commercial purchase lots in the initial profile before effects. **Future-phase procedure only:** for an expressly supported decrease, select and retain the latest-purchase-first per-lot allocation and validate its contractual restrictions under [composition](./02-composition.md#interval-and-identity-discipline). Refuse a prohibited allocation before effects without silently choosing different lots. A decrease cannot silently revoke occupied capacity. The old local SeatBinding/LIFO algorithm is retired with local enforcement ownership. An authoritative entitlement provider/enforcement capacity guard or approved revocation workflow is required; without that contract, the dependent decrease refuses explicitly. Zero quantity does not mean cancellation; use the explicit removal/cancel operation required by the product model.
6. Stage only the resource/posture effects permitted by the source-state matrix through the Foundation. At application lock aggregate and affected source/target policy occupancy keys, revalidate revision/resulting bounds, source state, explicit versioned limit and all local guards, then atomically commit every item, quantity history, one revision, order correlations and applicable posture/fact changes. Capacity includes actual active, retained suspended and explicit order reservations; exclude the same right once. A key-changing active or suspended update moves its claim atomically, and target conflict preserves the old composition and claim. An unchanged key retains the same place. A suspended target remains suspended with restricted posture and its accepted suspension-billing policy; retention is not a billing waiver. Policy key/limit changes and current policy/key movement require owner-agreed handling under S-Q02/S-U10. A draft remains unactivated and creates no service facts or automatic reservation; an already confirmed order's reservation/commitment cannot be silently invalidated by its edit.
7. Commit the complete lot allocation and its full fact replacement plus successor commercial seal with the composition, durable outcome and outbox under [feature 08](./08-commercial-facts.md#33-derive-stable-facts-and-complete-accepted-term-slices). A same-component increase under unchanged financial coordinates retains the fact ID. Return per-order-line and created-lot references in one applied result. A validation/commit failure applies no subset. Remote effects staged before a failed commit remain recoverable compensation obligations, not partial commercial success.

### Schedule or withdraw a future change

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-scheduled-change`

**DEFERRED, 2026-10-15.** Reject future composition requests before effects; do not silently execute them immediately. Cancellation schedules, order starts, renewal and trial timers remain in their own flows.

### Replace across unsupported commercial boundaries

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-flow-replacement-saga`

**DEFERRED, 2026-10-15.** No linked A-to-B replacement, atomic occupancy handover, resource reuse/transfer or base-plan change is implemented. Retain the earlier atomic design as historical direction in DECISIONS, not current implementation work. Independent cancellation/new purchase follows separate retention and provisioning; it cannot promise continuity or automatically adopt existing data/resources. Technical attempt succession for the same unapplied acquisition is distinct and remains supported under Foundation recovery gates.

## 3. Processes / Business Logic (CDSL)

### Commercial boundary and correction

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-change-facts`

For an active target, the commit closes prior served slices and opens accepted successor slices at the same boundary. Quantity changes preserve occurrence identity and publish a replacement version with the complete quantity timeline. New items create their own facts for the served remainder. A suspended target preserves its suspension intervals and accepted pause-or-continue billing posture in every affected fact version; changing composition creates no resumed service or full grant. A draft edit records a new proposed-to-activate composition revision with separate immutable delta evidence and emits no activated service or billable facts. In a future phase that explicitly enables removals, removed items produce explicit zero-obligation/corrected served-extent versions where required; no removal is encoded as silence. Enclosing period geometry remains fixed until the next period when anchor policy changes. Rating calculates all proration; Billing applies immutable-document correction rules.

### Ramps and manual supersession

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-algo-ramp-execution`

**DEFERRED, 2026-10-15.** Multi-step Contracts schedules are not implemented in this version. Single-boundary future composition changes are also deferred.

### Backdated amendments and migrations

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-algo-change-backdating`

**DEFERRED, 2026-10-15.** Administrative historical correction/backdating is not implemented. Ordinary commands reject past-effective edits; this does not disable immutable replacement facts required by normal current lifecycle operations or their authoritative late outcomes. Migration is excluded; Pricing bulk migrations remain outside scope.

## 4. States (CDSL)

### Change operation state

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-state-changes`

| Source status | Order-originated composition application | Preserved effects and capability conditions |
| --- | --- | --- |
| active | Apply accepted increases/additions with the complete commit guards. | Status remains active; actual composition boundary drives applicable posture and fact versions. |
| suspended | Apply accepted increases/additions without an implicit resume. | Preserve suspension, restricted entitlement provider posture, paused resources and accepted suspension-billing policy. New capacity cannot become usable before a separate guarded resume. Refuse a resource leg whose provider cannot preserve that posture; do not require activation as a substitute. |
| draft | Non-terminal candidate; application is gated on the explicit draft-change acceptance contract under S-U01. | The joint Orders/Workflow/Pricing contract must bind original acquisition and delta receipts, define the resulting activation composition and first-hold/withdrawal behavior, and retain every original receipt unchanged. Once enabled, edit only the draft composition/revision; no service activation, full entitlement provider grants or commercial facts occur before activation. Until agreed, return a specific unsupported draft-change capability before effects. |
| cancelled or archived | Refuse. | No composition mutation or commercial rebirth. |

The draft gate is an explicit missing source contract, not an active-only interpretation of the upstream non-terminal requirement. S-U01 must demonstrate acquisition/change races and recovery after a draft delta has been accepted, including preservation or explicit agreed amendment of a confirmed order's reservation, future commercial commitment and financial evidence. A changed order version or attempt cannot inherit or duplicate those rights implicitly. S-Q01 additionally governs final activation authorization; changed composition cannot extend an expired hold/permit or silently reprice paid terms. Existing in-flight resource/activation operations are fenced rather than merged with a change implicitly.

A supported immediate composition request follows the Foundation operation lifecycle. Commercial application and external completion remain distinct; pending/failed effects stay visible after the local commit. Scheduled composition, ramps and linked replacement have no current operation-state implementation; reject them before effects.

## 5. Definitions of Done

### Atomic changes and recoverable boundaries

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-dod-changes`

Implement an explicit operation profile and source-state matrix around one resulting-composition validator and one transaction for single-item and composite paths; carry expected revision, per-line correlation, accepted delta provenance and effective instant consistently. Preserve suspension and draft posture; publish facts only for the accepted committed obligation/service posture. Refuse deferred plan, future-composition and linked-replacement operations before effects. Current provider capabilities determine which compatibility/migration paths may be enabled; draft changes require the S-U01 acceptance/activation fixture.

**Implements:** `cpt-cf-bss-subscriptions-flow-change-boundary-chg`.

## 6. Acceptance Criteria

- [ ] A batch with one invalid add-on bound changes no quantities/items and publishes no partial fact/posture result.
- [ ] An order batch containing a decrease, removal, plan replacement or one-time/setup item refuses before effects; a mismatched `add_component`/`increase_quantity` assertion also refuses. Direct/operator wrappers also refuse initial-profile decreases, component removals and transfers between commercial lots without emitting partial facts or posture changes.
- [ ] A valid increase/addition against a suspended target commits one revision while preserving suspension, restricted grants and its accepted suspension-billing posture; no implicit resume or usable extra capacity occurs.
- [ ] Draft changes return an explicit capability-gate refusal until S-U01 is agreed. The required fixture then proves that a valid draft delta preserves original and delta receipts, increments only draft composition revision and emits no activated service/grants/facts before separately guarded activation.
- [ ] Increasing an existing item from 10 at its old binding by 2 at a new binding refuses before effects until the S-U08 lot contract is enabled. The future fixture must preserve both accepted price/quantity lots, total quantity 12 and immutable purchase lineage without repricing the original 10; replay and corrections use the agreed allocation contract. Decreases are a future-phase capability and do not block delivery of the increase profile.
- [ ] A stale expected revision conflicts even when individual operations would otherwise pass.
- [ ] Any requested future composition boundary, base-plan change or linked replacement refuses before effects; no resource deletion, handover or partial commercial change follows.
- [ ] A self-exempt active/suspended composition change does not consume another place. A key-changing update that conflicts at the target leaves its original claim and composition intact; admission counts active, retained suspended and explicit order reservations under the same fence.
- [ ] Quantity and price changes preserve occurrence identity and reproduce both pre/post-boundary quantities without duplicate obligations.
- [ ] An unsupported entitlement provider capacity guard, bulk migration or historical correction refuses explicitly rather than using retired local counters or live catalog assumptions.
