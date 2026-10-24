---
description: "Decision to persist supported scheduled intents on the subscription aggregate for durable execution and recovery."
status: proposed
date: 2026-07-15
decision-makers: "BSS Subscriptions team"
---

**Current applicability, 2026-10-15:** the [PRD scope](../PRD.md#11-purpose) governs this ADR. Base-plan changes, scheduled composition/resume, collection pause and other deferred capabilities are not implementation examples or acceptance requirements for this release. The underlying state, fact and scheduling principles apply only to retained operations.

Created:  2026-08-24 by Virtuozzo International GmbH
Updated:  2026-08-24 by Virtuozzo International GmbH

# ADR-0003: Scheduled Intents Live on the Aggregate (Not Portal-Side Automation)

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [Portal-side automation (cron re-keys the transition)](#portal-side-automation-cron-re-keys-the-transition)
  - [Pending intents on the aggregate (chosen)](#pending-intents-on-the-aggregate-chosen)
  - [A native SubscriptionSchedule aggregate](#a-native-subscriptionschedule-aggregate)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-cf-bss-subscriptions-adr-scheduled-intents-on-aggregate`

> **Status: proposed / flagged for veto.** Records the rationale for [`../DECISIONS.md`](../DECISIONS.md)
> SUB-D-01 and SUB-D-04; ratified when Product/Architecture confirms.

> **2026-10-15 applicability.** Scheduled-intent ownership remains applicable to retained cancellation and ordinary renewal/trial timers; future-start Orders retain their acquisition flow. All scheduled composition changes, including a single future `changeComposition`, and scheduled resume are deferred. Only supported immediate increases/additions remain in the composition-change profile. Multi-date changes, ramps and full schedule aggregates are also deferred. Intent alone produces no served interval or commercial fact. See [current decisions](../DECISIONS.md#current-disposition) and [canonical design](../DESIGN.md). Historical rationale below is retained, not a separate current implementation contract.

## Context and Problem Statement

The most common self-service intent is "**cancel at period end**" (Stripe `cancel_at_period_end`,
Zuora cancellation policies); "suspend until a date" and committed multi-step **ramps** (Zuora
Ramps/Orders) are the enterprise equivalents. None of these is an *immediate* transition — they are a
**future** transition the customer commits to **now**. If the future firing is implemented as
portal-side automation (a cron job that re-keys the real transition at the due date), the commitment
is **invisible** to the renewal job, Billing, and audit until it fires: the renewal job may auto-renew
a subscription the customer already asked to cancel, and there is no auditable record of the standing
intent.

Where does a committed-but-future transition live so the renewal job, Billing, and audit can all see
it before it fires?

## Decision Drivers

* The renewal job MUST see a pending end-of-term cancel and **suppress** the renewal + next-term recurring — otherwise it double-acts against the customer's intent.
* Billing and audit need the standing intent as first-class, auditable state — not a hidden scheduler row in a portal.
* A scheduled transition must run the **full guard set** (Policy/OSS) at its effective instant — it is not a pre-authorised bypass.
* Multi-step ramps and future composition/resume are deferred; their authoring and execution are not release requirements.

## Considered Options

1. Portal-side automation (a cron re-keys the transition at the due date).
2. **Pending intents on the aggregate** (supported cancellation and lifecycle timers) (chosen).
3. A native `SubscriptionSchedule` aggregate that owns multi-step schedules.

## Decision Outcome

Chosen option: **pending intents live on the aggregate.** Supported cancellation accepts immediate, end-of-term or explicit future-date intent. Pending cancellation suppresses conflicting renewal/conversion and remains observable independently of worker execution. Only successful guarded application records the actual commercial transition. A confirmed financial cutoff remains effective even if resource work finishes later.

Withdraw/reschedule only supported unapplied work; confirmed cancellation additionally requires a versioned confirmation before its cutoff. After cutoff, withdrawal cannot restore charges retroactively. Timers for future Orders activation, renewal, trial expiry/conversion and configured lifecycle policy remain. Scheduled resume, future composition, ramps and base-plan changes are deferred; no native multi-step schedule aggregate is required.

### Consequences

* Renewal and operation readers see the durable standing intent and cannot silently renew over it.
* One durable scheduler protocol supplies idempotency, current-state guards, ordering and recoverable outcomes for supported intents/timers.
* Worker leases require multi-instance proof; process-local locks and a named singleton do not establish ordering across instances.
* Notification delivery and audit projections are deferred; reliable domain events and operation history remain.

### Confirmation

* A fixture: a subscription with a pending end-of-term cancel is **not** auto-renewed and emits no next-term recurring; a valid confirmed withdrawal before the charging cutoff restores ordinary renewal eligibility; both scheduling and un-scheduling emit events.
* A fixture: a scheduled cancellation fires at `effectiveAt` through the full Policy/OSS guard set (a Policy deny at firing time leaves the state unchanged).

## Pros and Cons of the Options

### Portal-side automation (cron re-keys the transition)

* Good: no aggregate change.
* Bad: the intent is invisible to the renewal job/Billing/audit until it fires; auto-renewal races the cancel; no auditable standing-intent record; each portal reinvents it.

### Pending intents on the aggregate (chosen)

* Good: visible to renewal/Billing/audit; one uniform scheduling mechanism; full guard replay at firing; future capabilities require a separate scope decision.
* Bad: the aggregate carries pending-intent state + a recoverable scheduler with proven worker claims; does not by itself give atomic multi-action orders.

### A native SubscriptionSchedule aggregate

* Good: rich multi-step scheduling, atomic orders.
* Bad: duplicates Contracts' role as the home of negotiated commitments; a large new aggregate for a launch with supported cancellation intents and ordinary lifecycle timers; revisit only if self-service ramps become a product goal.

## More Information

Historical seams SUB-C2 (ramps) and SUB-B2 (collection pause) are deferred; retained references are in [`../SEAMS.md`](../SEAMS.md). Decisions:
[`../DECISIONS.md`](../DECISIONS.md) SUB-D-01, SUB-D-04.

## Traceability

- **PRD**: [`../PRD.md`](../PRD.md) §6.1 (`fr-scheduled-intents`), §6.3 (`fr-ramp-execution`), §6.5 (renewal suppression), §15 (atomic multi-action open).
- **Decisions**: [`../DECISIONS.md`](../DECISIONS.md) SUB-D-01, SUB-D-04.
- **Design**: [Lifecycle Foundation](../features/01-foundation.md) scheduled intents and [Composition Changes](../features/03-changes.md).
