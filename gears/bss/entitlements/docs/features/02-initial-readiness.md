---
description: "Readiness manifest, preparation assignments, observation envelope, withdrawal and cleanup on the provider side of IA-2026-10-18."
---

# Feature: Initial Readiness

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Readiness manifest and observation envelope](#readiness-manifest-and-observation-envelope)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Observation acceptance](#observation-acceptance)
- [4. States (CDSL)](#4-states-cdsl)
  - [Readiness state](#readiness-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [5.1 Manifest-gated readiness with recoverable observations](#51-manifest-gated-readiness-with-recoverable-observations)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-featstatus-initial-readiness-implemented`
- [ ] `p1` - `cpt-cf-bss-entitlements-feature-initial-readiness`

**Status:** design set only, created 2026-10-23 by the split of the application layer out of Subscriptions. No adapter, SDK or runtime is delivered.

## 1. Feature Context

### 1.1 Overview

For an initial activation Entitlements persists the manifest of every required assignment, makes the preparation assignments available through the selected adapters while the subscription is still draft, accepts readiness observations against the manifest, and reports ready to Subscriptions only when every assignment is ready for the exact desired revision. Subscriptions performs the guarded commit and owns T.

### 1.2 Purpose

**Requirements:** `cpt-cf-bss-entitlements-fr-initial-readiness`.

**Primary NFRs:** `cpt-cf-bss-entitlements-nfr-handoff-latency`.

### 1.3 Actors

| Actor | Role |
| --- | --- |
| `cpt-cf-bss-entitlements-actor-subscriptions` | Persists the activation operation, consumes `ReadinessObservationV1` and commits active with fresh T. |
| `cpt-cf-bss-entitlements-actor-license-enforcement` | Selected license adapter (LE, native LM or third party); prepares rights assignments and reports readiness. |
| `cpt-cf-bss-entitlements-actor-resource-provider` | Prepares the purchased environment and reports its readiness. |
| `cpt-cf-bss-entitlements-actor-platform-operator` | Inspects pending, failed and withdrawn preparation and its cleanup obligations. |

### 1.4 References

- **PRD:** [PRD.md](../PRD.md); **Design:** [readiness observation contract](../DESIGN.md#332-readiness-observation-contract).
- **Build prerequisites:** as allocated in [DECOMPOSITION.md](../DECOMPOSITION.md); decisions in [DECISIONS.md](../DECISIONS.md).
- **Provider prerequisites / integration acceptance:** the joint freshness and withdrawal contract with Subscriptions and the per-profile readiness gates ENT-Q09 to ENT-Q13 in [DECISIONS.md](../DECISIONS.md#open-questions).

## 2. Actor Flows (CDSL)

### Readiness manifest and observation envelope

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-flow-initial-readiness`

**Actor:** Subscriptions, through `CommercialPostureV1` for the draft revision; the selected adapters as producers of readiness observations. **Success:** every required assignment of the manifest is `ready` for its exact desired revision and Entitlements reports ready to Subscriptions. **Errors:** manifest cannot be built, unsupported `schemaVersion`, unauthorized producer, refused or quarantined observation, preparation `failed`, operation `withdrawn`.

Entitlements is the provider side of IA-2026-10-18 (ENT-D-02): initial activation completes only after all required initial service capabilities are confirmed usable. Subscriptions keeps the subscription `draft` while its durable activation operation is pending or approved. Preparation progress is observation state here and operation state in Subscriptions, not a new commercial status. This is an accepted design direction, not delivered SDKs, external-owner ratification or runtime proof. Renewals, resume, trial-to-paid conversion and modifications of an already active subscription keep their existing contracts.

1. Subscriptions persists its activation operation and the association to the subscription before any preparation request or readiness observation can escape, and publishes `CommercialPostureV1` for the draft revision through [posture intake](01-posture-intake.md). Entitlements accepts the posture and persists the readiness manifest in the same transaction. The manifest enumerates every required scoped assignment, desired revision, service profile and target, and the authoritative producer; optional and not-applicable capabilities are explicit. An empty manifest is permitted only when the selected accepted service profile authoritatively requires no preparation, never because discovery or an adapter failed. Its digest and version are bound to the operation and guard changes to composition or targets while preparation is running.
2. The selected license adapter (LE, native LM or third party) makes the authorized preparation assignment available before commercial activation (ENT-D-01). The selected service adapter reconciles that assignment to the required platform state: for PaaS, creating or linking the account, deploying the environment included in the purchase and establishing necessary usable access. This does not wait for the customer's first login or a later VM purchase. Entitlements does not orchestrate each vendor step. It records the preparation authority and waits for the required readiness evidence. Preparation rights cannot be derived only from `active`, or the cycle would prevent activation. Reuse an existing assignment identity if it supplies the required scope, lifetime and revision semantics; no new license entity is mandated.
3. The [License Resolver](../../../../system/license-resolver/docs/DESIGN.md) is a read/check boundary, not the owner of issuing/revoking rights or a delivered change feed. The selected adapter's contract must supply authoritative assignment state and recovery. Event delivery, polling or another reliable feed is an integration choice (ENT-Q09); no broker or new issuance API is assumed by this document. Entitlements records an assignment/publication intent and its manifest association durably before dispatch, with recovery for external results. A fast callback must never be irretrievably orphaned by a not-yet-persisted association.
4. Each producer reports readiness per assignment with the envelope below. Entitlements accepts or refuses every observation under [observation acceptance](#observation-acceptance), persists it and advances the producer's high-water mark.
5. Entitlements reports `ready` to Subscriptions through `ReadinessObservationV1` only when every assignment of the manifest is ready for `desiredRevision`; Subscriptions performs the guarded commit and owns T (ENT-D-02). Entitlements never assigns T and never treats a ready event, a receipt or an intermediate approved operation as completion. All rights essential for initial usable service are ready before that commit. Post-commit reconciliation may reconcile the same satisfied assignment or deliver nonessential or ongoing changes; it cannot be the first action that makes required initial access usable. Initial readiness is not an uptime guarantee, and later service failure does not reopen the completed acquisition.
6. **Uncertainty, revocation and cleanup.** Entitlements persists external effects and durable lookup/reconciliation keys. Lost/unknown results, absence from a read, or a quiet broker are not proof of failure or non-execution; recover the same assignment and operation without allocating a second account/environment. The status/read contract must recover authoritative state and sequence after missed events, restart or retention gaps; dispatch logs are not completion evidence. A `failed` observation describes preparation failure, not proof all partial effects are gone. Withdrawal starts in Subscriptions, which fences its local activation first; Entitlements then marks the manifest withdrawn, revokes/supersedes preparation authority at the selected adapters, reconciles already running external actions and records cleanup/remedy obligations. Delayed ready for a cancelled/obsolete revision cannot activate; it still contributes historical effect evidence needed for cleanup. Adapter-side ordering must prevent delayed preparation from recreating a withdrawn environment, including work already in flight. Do not erase pending cleanup or release protected purchase rights because a draft disappeared. Cleanup targets only resources owned by this operation under the accepted policy, never an existing customer account merely because it was linked. Operational cleanup completion and refund/financial settlement remain separate outcomes.

**Standard semantic envelope.** The following are candidate names for the common adapter contract, not ratified SDK fields or a vendor payload schema. The adapter need not receive or report `subscriptionId`; only Subscriptions stores that association.

| Semantic field | Assignment / readiness meaning |
| --- | --- |
| `schemaVersion` | Explicit supported contract version; unsupported versions are quarantined/reported, not interpreted as ready. |
| `scopeId`, `assignmentId` | Authorized tenant scope and stable identity of the concrete rights assignment/service instance. A shared product or customer ID alone is insufficient. Native identity must distinguish multiple environments under one broad right. |
| `desiredRevision` | Immutable desired-state generation bound to exact accepted preparation inputs; it is not a transport idempotency key. Retired identities/revisions cannot be silently reused. |
| `serviceProfileRef`, `targetRef` | Pinned versioned readiness requirements and concrete service target, retained in the descriptor/manifest and bound to results. Target allocation discovered during execution is durably associated before it can satisfy readiness. |
| `eventId` | Identity of an emitted observation for delivery deduplication; repeats keep the original identity/content, conflicting content is refused. A reconciled newer observation has its own identity. |
| `producerId`, `statusSequence` | Trusted responsible authority and durable monotonically ordered observation sequence within the agreed scoped assignment/authority stream. Producer replacement or sequence reset needs an explicit fenced generation mapping. |
| `status` | Semantic observation `pending`, `ready`, `failed` or `withdrawn`; these are preparation observations, not subscription states. Pending includes accepted work; ready means the service profile's complete initial readiness criterion. |
| `observedAt`, `evidenceRef`, `reasonCode` | Observation time, recoverable authoritative evidence and structured reason. Timestamp is diagnostic evidence, not ordering, freshness authority or the commercial start. |

**Open integration gates.** Owners must agree rights-feed transport and recovery, native schema/identity mapping, per-profile readiness evidence, timeouts/escalation and cleanup policy. They must also agree evidence freshness and readiness-withdrawal versus final local commit arbitration. A timestamp or last successful health check alone cannot prevent a concurrent remote revocation; no cross-service atomicity is claimed. A profile lacking that contract cannot claim a safe initial activation merely on a cached ready event. Preactivation usable exposure/usage, permitted preparation timing and its financial treatment require explicit provider/Billing agreement; this design neither backdates service accrual nor invents free credit, permission to serve before financial admission or a universal pre-T liability policy. These gates are ENT-Q09 to ENT-Q13 in [DECISIONS.md](../DECISIONS.md#open-questions); a profile stays unavailable until its owners close them.

## 3. Processes / Business Logic (CDSL)

### Observation acceptance

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-algo-observation-acceptance`

**Input:** one `ReadinessObservationV1` from a producer and the saved manifest. **Output:** accepted and persisted, no-op, or refused/quarantined with a structured reason.

1. Authenticate the producer. Authorize its tenant, assignment and profile authority against the saved manifest; an arbitrary publisher's `producerId` is not proof.
2. Check `schemaVersion`. An unsupported version is quarantined and reported, never interpreted as ready.
3. Match exact `scopeId`, `assignmentId`, `desiredRevision`, `targetRef`, `serviceProfileRef` and the current manifest association. A target allocated during execution must be durably associated before an observation against it can satisfy readiness.
4. Compare `eventId` and content with what is stored. An identical delivery is a no-op. A conflicting duplicate, wrong scope, unexpected producer or ambiguous association is refused or quarantined without information leakage.
5. Compare `statusSequence` with the producer's high-water mark for the assignment stream. An older sequence cannot overwrite a newer `ready`, `withdrawn` or `failed` result. A gap requires authoritative reconciliation unless the agreed full-state snapshot contract proves it safely covered. Producer replacement or a sequence reset needs an explicit fenced generation mapping.
6. Persist the accepted observation, its content identity, the producer sequence high-water mark and provenance durably, then advance the assignment status under [readiness state](#readiness-state).
7. Recompute manifest readiness. Receipts, broker acknowledgement and license existence alone never satisfy readiness. One adapter's ready result does not satisfy another required producer or assignment. The manifest is ready only when every required assignment's current accepted observation is `ready` for the exact `desiredRevision`; a ready for a retired, superseded or withdrawn revision is historical evidence only.

## 4. States (CDSL)

### Readiness state

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-state-readiness`

Each required assignment of a manifest carries one current observation status: `pending`, `ready`, `failed` or `withdrawn`. These are preparation observations, not subscription states; the subscription stays `draft` on the Subscriptions side until its own commit. Order is the producer's `statusSequence`, never `observedAt`.

| From | Observation / to | Mandatory guard |
| --- | --- | --- |
| none | manifest persisted / pending | Posture accepted and manifest written in the same transaction; producer authorities saved. |
| pending | newer `ready` / ready | Accepted under observation acceptance; exact `desiredRevision`, target and profile. |
| pending or ready | newer `failed` / failed | Higher sequence from the authorized producer; partial effects stay subject to reconciliation. |
| failed | newer `pending` or `ready` / pending or ready | Higher sequence; the same assignment identity is recovered, no second environment is allocated. |
| pending, ready or failed | withdrawal / withdrawn | Subscriptions has fenced its local activation; Entitlements revokes or supersedes preparation authority and records cleanup obligations. |
| withdrawn | any later observation / withdrawn | Stored as effect evidence for cleanup; never produces ready for that revision. |

The manifest is `ready` only while every required assignment is `ready` for the exact desired revision; any other assignment status makes the manifest `pending`, `failed` or `withdrawn` in that precedence. A lower sequence never moves any row. A new desired revision starts a new manifest row set; it does not reuse retired identities or revisions.

## 5. Definitions of Done

### 5.1 Manifest-gated readiness with recoverable observations

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-dod-initial-readiness`

The implementation must persist the complete manifest before any preparation request or observation can escape, authenticate and authorize every producer against it, keep one durable high-water mark per producer and assignment stream, refuse or quarantine conflicting, stale, wrong-scope and unsupported observations without information leakage, and report ready to Subscriptions only when the whole manifest is ready for the exact desired revision. Readiness, withdrawal and cleanup obligations survive restart and are readable by assignment and operation; dispatch logs are not completion evidence. The same assignment identity is recovered after a lost or unknown result without a second account or environment. No subscription identifier is accepted in or written to any adapter payload; the manifest association is keyed by assignment within `(namespace, resource tenant)`.

**NFR acceptance evidence:** measure first delivery of each preparation assignment to its required adapter against the ratified `cpt-cf-bss-entitlements-nfr-handoff-latency` targets (p95 < 5 s, p99 < 15 s, auditable exception beyond 60 s). Report time to confirmed usable access separately; it is never hidden inside the delivery figure. Fault injection must show a lost ready event recovered through the assignment status read, a restart resuming pending observations from the high-water mark, and a withdrawal racing a ready reaching the fenced outcome with cleanup obligations recorded.

**Implements:** `cpt-cf-bss-entitlements-flow-initial-readiness`, `cpt-cf-bss-entitlements-algo-observation-acceptance`, `cpt-cf-bss-entitlements-state-readiness`.

## 6. Acceptance Criteria

- [ ] Ready is reported only for the entire pinned assignment and producer manifest; duplicate, wrong-tenant, old-revision and lower-sequence observations cannot produce ready or overwrite a newer `withdrawn`.
- [ ] A lost ready event is recovered through the same assignment status read; an unknown or not-found result does not allocate a second environment. A Subscriptions commit failure after readiness finds the same ready state again; Entitlements assigns no T.
- [ ] Withdrawal racing ready is fenced at external execution; partial and late effects retain cleanup obligations. A delayed ready for a withdrawn revision is stored as evidence and never produces ready.
- [ ] PaaS initial readiness covers the account, the purchased environment and required access, without waiting for first login or a later VM creation. Entitlements reports ready; the active commit and period start are owned by Subscriptions with fresh local T, not provider observed time.
- [ ] An observation with an unsupported `schemaVersion`, an unauthenticated producer or an ambiguous manifest association is quarantined with a structured reason and no readiness effect; an identical redelivery is a no-op.
