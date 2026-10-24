---
description: "Decision to make the application of commercial posture to quota, license and resource providers its own BSS gear, Entitlements, with derivation and adapter contracts owned there."
status: accepted
date: 2026-10-23
decision-makers: "BSS Subscriptions and Entitlements owners"
---

**Current applicability, 2026-10-23:** this ADR records the founding boundary of the Entitlements design set. It applies to every document in `gears/bss/entitlements/docs` and to the Subscriptions documents that publish `CommercialPostureV1` and consume `ReadinessObservationV1`. It moves no commercial decision and reopens no deferred Subscriptions capability. Runtime, SDK types, storage schemas and adapter implementations are outside it.

Created:  2026-10-23 by Virtuozzo International GmbH
Updated:  2026-10-23 by Virtuozzo International GmbH

# ADR-0001: The Application Layer Is Its Own Gear

<!-- toc -->

- [Context and Problem Statement](#context-and-problem-statement)
- [Decision Drivers](#decision-drivers)
- [Considered Options](#considered-options)
- [Decision Outcome](#decision-outcome)
  - [Consequences](#consequences)
  - [Confirmation](#confirmation)
- [Pros and Cons of the Options](#pros-and-cons-of-the-options)
  - [A: thin adapter gear, derivation stays in Subscriptions](#a-thin-adapter-gear-derivation-stays-in-subscriptions)
  - [B: gear owns derivation and application (chosen)](#b-gear-owns-derivation-and-application-chosen)
  - [C: fold into the system gears](#c-fold-into-the-system-gears)
- [More Information](#more-information)
- [Traceability](#traceability)

<!-- /toc -->

**ID**: `cpt-cf-bss-entitlements-adr-application-boundary`

> **Status: accepted.** Records the rationale for [`../DECISIONS.md`](../DECISIONS.md)
> ENT-D-01, ENT-D-02 and ENT-D-03; accepted by the owner on 2026-10-23 as SUB-D-56 in the Subscriptions design set.

## Context and Problem Statement

The Subscriptions design set carried two subjects with different owners, different cadence and different consumers. The first is the commercial lifecycle of a subscription: statuses, operations, lots, facts, seals, renewal and retention, consumed by Orders, Rating and Billing. The second is the application of what was bought to enforcement and execution systems: quota caps, license grants and resource work orders, consumed by platform enforcement and deployment-selected adapters.

The second subject was already half outside Subscriptions. Point-of-use checks and quota counters were retired to license-resolver, license-enforcement and quota-enforcement on 2026-10-15. What remained was materialization, and it had grown into half of a lifecycle feature, a quota projection draft with eight open decisions, two adapter proposals and the readiness contract of 2026-10-18. The review of 2026-10-23 found the adapter boundary unowned: the quota draft passed `subscriptionId` across the adapter boundary while the readiness contract forbade it, and the rule semantics had three owners and no owner of meaning.

Where does the derivation of provider-desired state, the readiness manifest and the adapter contract live, so that one owner answers for the boundary and Subscriptions keeps only commercial posture?

## Decision Drivers

* One owner for the adapter boundary, so that identity, fencing and recovery rules are stated once and do not drift between drafts.
* Adapters must address work by scope, assignment and revision, never by subscription, so that a native, third-party or customer integration can serve the same contract.
* Rules must be frozen once, in the acceptance that also freezes prices, so that replay never reads a live plan.
* Subscriptions must keep publishing posture from its atomic lifecycle commit and must keep its consumer-side readiness gate, so that T stays a Subscriptions fact.
* The split must be a documentation change: no runtime, no SDK, no migration of identifiers.

## Considered Options

1. A: a thin adapter gear; derivation stays in Subscriptions.
2. B: a gear that owns derivation and application (chosen).
3. C: fold derivation and application into the system gears (quota-enforcement, license-enforcement, resource provider).

## Decision Outcome

Chosen option: **B, a gear that owns derivation and application.** The new design set `gears/bss/entitlements/docs` owns the derivation of provider-desired state from committed commercial posture, the readiness manifest and observation envelope, the rule schema and semantics, stale-action fencing, reconciliation and findings, and the adapter specifications for Quota Enforcement, the selected license adapter (LE, native LM or third party), the resource provider and customer integrations. Subscriptions keeps the commercial posture as a versioned fact of its aggregate, committed atomically with the lifecycle transition and the outbox, and the consumer-side initial readiness gate. Pricing stores quota and grant rules on the immutable PlanRevision and freezes them in the same acceptance receipt as the price bindings. Products supplies metric, unit and subject semantics and the versioned mapping to the enforcement registry. Providers own counters, grants, point-of-use decisions and physical resources.

Two contracts connect the gears. Downward, `CommercialPostureV1` is published by Subscriptions with the identity `(orderingTenantId, subscriptionId, revision)`, a digest, tenant axes, members and provenance; this is the only place in the Entitlements set where `subscriptionId` appears. Upward, `ReadinessObservationV1` is published by Entitlements with scope, assignment, desired revision, producer, sequence and status; no `subscriptionId` crosses it. The quota source key at the provider becomes `(authenticated namespace, resource tenant, assignmentId)`.

### Consequences

* Three functional requirements and one non-functional requirement leave Subscriptions: posture issuance, grant assignment, license state and the handoff latency budget. Subscriptions keeps one requirement, the publication of commercial posture and the exposure of application state as reported by Entitlements. The retired ids are recorded in the Subscriptions PRD §17.2; nothing is copied with its old id.
* Two cross-gear contracts exist and are versioned bilaterally: `CommercialPostureV1` down and `ReadinessObservationV1` up. Their versions are a gate, E-U06, in the Entitlements upstream requirements, and S-U04 in Subscriptions points at Entitlements as its provider.
* The eight open quota decisions and the five readiness gates become the Entitlements open-question register, ENT-Q01 to ENT-Q13. A profile that depends on an open question stays unavailable.
* Owner proposals to Quota Enforcement, License Enforcement, Pricing, Products and the resource provider are issued from Entitlements, not from Subscriptions.
* Both design sets must pass link, structure, language, allocation census and spec-check gates; a census confirms that `subscriptionId` appears in no Entitlements adapter contract.

### Confirmation

* The subscription identifier appears only in the Subscriptions-facing contract (`CommercialPostureV1` intake) and never in any adapter contract.
* An allocation census: eight functional and one non-functional requirement in the Entitlements PRD, each allocated exactly once in DECOMPOSITION; the Subscriptions census drops by the four retired ids and gains one.
* A fixture per adapter contract: issue N delivered after revoke N+1 with different request keys cannot restore access at the provider; a lost reply is recovered from the provider's source head.

## Pros and Cons of the Options

### A: thin adapter gear, derivation stays in Subscriptions

* Good: the smallest new document set; Subscriptions keeps its existing derivation text.
* Bad: the boundary blurs again at once, because the identity carried to the adapter is decided where the derivation lives; the review's finding that `subscriptionId` crossed the boundary would recur.

### B: gear owns derivation and application (chosen)

* Good: one owner for identity, fencing, readiness and recovery; Subscriptions shrinks to commercial posture and a consumer gate; rules frozen once in Pricing acceptance; adapters serve one contract regardless of vendor.
* Bad: one more design set to keep consistent; two versioned contracts to agree bilaterally; the quota and readiness gates move but do not close.

### C: fold into the system gears

* Good: no BSS gear between posture and enforcement.
* Bad: contradicts the 2026-10-14 provider-independence decision; the derivation of what was bought would live in each platform gear and in no third-party or customer integration; Pricing acceptance evidence would be read by enforcement code.

## More Information

The split was accepted in the Subscriptions decisions log as SUB-D-56 on 2026-10-23 and is adopted here as ENT-D-01 to ENT-D-03 in [`../DECISIONS.md`](../DECISIONS.md). The provider-independence decision of 2026-10-14 and the initial-readiness decision SUB-D-55 of 2026-10-18 bind both gears and are kept in both decision logs.

## Traceability

- **PRD**: [`../PRD.md`](../PRD.md) §1.2 (background), §6 (eight functional requirements), §9.2 (the five integration contracts), §17.3 (upstream coverage).
- **Decisions**: [`../DECISIONS.md`](../DECISIONS.md) ENT-D-01, ENT-D-02, ENT-D-03; open questions ENT-Q01 to ENT-Q13.
- **Design**: [`../DESIGN.md`](../DESIGN.md) §1.1 (boundary), §3.3.1 and §3.3.2 (the two contracts), §3.3.3 (adapter capability profile).
- **Gates**: [`../UPSTREAM_REQS.md`](../UPSTREAM_REQS.md) E-U01 to E-U06.
