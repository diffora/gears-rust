---
description: "Commercial posture derived from committed terms and lifecycle, published to Entitlements, with the readiness and materialization state Entitlements reports."
---

# Commercial Posture — Feature


<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Initial activation readiness contract](#initial-activation-readiness-contract)
  - [Apply a commercial posture and record its observations](#apply-a-commercial-posture-and-record-its-observations)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Posture by transition](#posture-by-transition)
  - [Financial boundary of posture](#financial-boundary-of-posture)
- [4. States (CDSL)](#4-states-cdsl)
  - [Entitlement delivery state](#entitlement-delivery-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Commercial posture publication and observation intake](#commercial-posture-publication-and-observation-intake)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-featstatus-entitlements-implemented`
- [ ] `p1` - `cpt-cf-bss-subscriptions-feature-entitlements`

**Status:** Subscriptions owns the commercial posture. Entitlements owns manifests, adapters and provider integration; a capability is enabled only after Entitlements accepts the selected adapter.
**Current-profile boundary, 2026-10-15:** apply the [current delivery scope](../PRD.md). Audit subsystem, confirmAcceptance, ramps, co-term, collection pause, notifications, tenant/payer transfer, administrative backdating/correction and optional brand/PlanTier metadata are deferred. Earlier references and scenarios involving those capabilities are future context only, not current acceptance or implementation tasks. Retain authorization, operation/event provenance, reliable effects and current facts/replay.

**Additional current-profile boundary, 2026-10-15:** the [additional scope reduction](../PRD.md#11-purpose) defers trial extension/repeat, acquisition without Orders, scheduled composition/resume, linked subscription replacement and base-plan changes. Retain supported immediate increases/additions, existing lifecycle, future order starts/cancellation and ordinary timers, both trial variants and early conversion to the accepted paid target. Deferred paths are refused even with explicit policy or operator authorization; their earlier examples are future context, not launch gates.

## 1. Feature Context

### 1.1 Overview

Derive the commercial posture from committed terms and lifecycle, commit it atomically with the transition, publish it to Entitlements, and expose the readiness and materialization state Entitlements reports.

Subscriptions supplies the desired commercial rights and records the observations Entitlements reports against the matching revision. Entitlements owns the readiness manifest, the adapters and the provider integration; see the [Entitlements DESIGN](../../../entitlements/docs/DESIGN.md). Subscriptions does not take over materialization or point-of-use enforcement.

### 1.2 Purpose

**Requirements:** `cpt-cf-bss-subscriptions-fr-commercial-posture`.

**Principle:** `cpt-cf-bss-subscriptions-principle-idempotent-ordered`. **Constraint:** `cpt-cf-bss-subscriptions-constraint-tenant-isolation`.

### 1.3 Actors

| Actor | Role |
| --- | --- |
| `cpt-cf-bss-subscriptions-actor-partner-admin` | Inspects the desired revision, the latest observed state and findings. |
| Entitlements | System actor. Receives the commercial posture, publishes readiness and materialization observations. |

### 1.4 References

- **PRD:** [PRD.md](../PRD.md); **Design:** [entitlement contract](../DESIGN.md#335-entitlement-and-retention-contracts); [Entitlements DESIGN](../../../entitlements/docs/DESIGN.md#331-commercial-posture-contract).
- **Build prerequisites:** [foundation](./01-foundation.md) and [composition](./02-composition.md), as allocated in [DECOMPOSITION.md](../DECOMPOSITION.md).
- **Provider prerequisites / integration acceptance:** Entitlements accepts the selected adapter against its capability profile before a capability is enabled. Cross-lifecycle handoff verification is an integration acceptance step, not a build dependency on each lifecycle feature.

## 2. Actor Flows (CDSL)

### Initial activation readiness contract

**Accepted direction — 2026-10-18 (IA-2026-10-18).** Initial activation completes only after all required initial service capabilities are confirmed usable (SUB-D-55). This amends the earlier S-Q01 permission for initial usable access to follow commercial T; T remains the fresh server-assigned instant of the guarded local activation commit. Subscription status stays `draft` while its durable activation operation is pending/approved. Preparation progress is operation state, not a new commercial status. This is an accepted Subscriptions design direction, not delivered SDKs, external-owner ratification or runtime proof. Renewals, resume, trial-to-paid conversion and modifications of an already active subscription retain their existing contracts.

**Consumer gate.** Subscriptions persists the activation operation and the association between that operation and its assignments before any readiness observation can escape. It then waits for a `ReadinessObservationV1` with `status = ready` for every assignment at the exact `desiredRevision`, and only then runs the commit boundary below. An observation for another revision, scope or assignment cannot satisfy the gate.

**Commit boundary.** After the whole manifest is ready, obtain/check final Pricing and financial authority and recheck local operation/association revision, order/attempt and cancellation fences, accepted terms, security/Policy and occupancy. Revalidate readiness under the agreed current-evidence/freshness and revocation contract. Atomically commit active status, own-reservation conversion, fresh local `serviceActivatedAt` T, period start, operation applied result, commercial records and outbox. Do not backdate T to assignment creation, provider `observedAt`, capture or callback time. Publish applied outcome to Orders Workflow from durable state/outbox; transport receipt, a single ready event or an intermediate approved operation is not completion. The initial activation's service-period/accrual boundary is T; separately agreed one-time occurrences retain their existing rules. Prepayment may precede T under existing protected-capacity/terms/capture rules.

All rights essential for initial usable service must be ready before this commit. The post-commit outbox may reconcile the same satisfied assignment or deliver nonessential/ongoing changes; it cannot be the first action that makes required initial access usable. Initial readiness is not an uptime guarantee, and later service failure does not reopen or backdate the completed acquisition.

The manifest, the observation envelope, producer rules, uncertainty handling and cleanup are specified in [Entitlements: readiness manifest and observation envelope](../../../entitlements/docs/features/02-initial-readiness.md#readiness-manifest-and-observation-envelope).

**Trial compatibility gate.** Free-period initial trials follow this readiness boundary with their existing trial-admission and financial guards. The earlier S-U15 staged credit proposal finalizes the same grant from applied T and can postpone credit-dependent access until afterward. It cannot satisfy this initial gate when that final grant is a prerequisite for usable access. Such credit-dependent initial profiles remain disabled pending a joint Billing/entitlement readiness and coverage contract that breaks the dependency cycle. Prepared coverage remains unissued/unspendable; do not invent T, pre-T grant issuance, new bonus or a bypass. Same-grant history, original expiry and late-finalization/correction rules remain intact; this amendment does not redesign conversion or ongoing trial accounting.

### Apply a commercial posture and record its observations

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-entitlement-posture`

For initial activation, apply IA-2026-10-18 above first: required preparation and usable access precede commercial application. The following procedure describes ongoing posture changes and nonessential post-commit reconciliation; it must not postpone an initial prerequisite until after active.

1. The lifecycle handler derives the complete target commercial grant/posture set from immutable accepted product descriptors and committed quantity at the boundary. It does not resolve an old Pricing phase/grant map.
2. The Foundation commit stores desired posture, effective interval, source aggregate revision, tenant axes and the outbox intent atomically with the commercial change.
3. Publish `CommercialPostureV1` to Entitlements through the outbox, identified by `(orderingTenantId, subscriptionId, revision)` and the payload digest, carrying the operation id and provenance (acceptance id, rule/binding snapshot digests, contributing lot ids). Recovery reads expose the same identity and digest, so a replay or a late reader obtains the same revision. A newer revision supersedes the target; earlier outcomes are retained.
4. Record the readiness and materialization observations Entitlements reports against the matching revision only, and surface them through subscription reads: desired revision, latest observed state, last observed time and findings. Show pending or failed delivery explicitly instead of reporting implied access.

## 3. Processes / Business Logic (CDSL)

### Posture by transition

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-entitlement-posture`

| Commercial event | Desired posture and boundary |
| --- | --- |
| Initial activation | Required assignments are prepared and confirmed usable while draft; guarded local application records the accepted active target and actual T without issuing initial prerequisites for the first time afterward. Import/resource adoption is deferred. |
| Composition/quantity change | New seat purchases/increases/additions require an admitted active target (draft changes retain the separate S-U01 gate); suspended requests refuse before commercial effects. For an applied active change, replace the complete target at its committed boundary; no transient union of old/new grants. |
| Suspend | Restricted/paused posture preserving data and restoration identity. |
| Resume | Restore the accepted set only after required lifecycle/occupancy guards. |
| Cancel | Revoke normal service, with separate narrow export capability during permitted retention. |
| Trial conversion | Target paid posture only after S-Q06 model and the applicable authorized settlement route: prescribed payment for prepay, or accepted postpay terms plus authorization to serve on credit. A linked card or immediate payment is not required for authorized postpay. |

**Purchase-state rule accepted, 2026-10-18:** require a separate authorized guarded resume before purchasing seats, increasing quantity or adding components to an existing suspended subscription (SUB-D-39). The purchase itself never resumes or queues a deferred grant. This is compatible with replacement integrations that require an active/usable successor, while keeping the core provider-independent and introducing no new synchronous readiness gate. If an active purchase already committed before suspension, preserve its commercial history and reconcile its pending delivery under the newer restricted revision; the provider fence must prevent stale replacement/issue from restoring access. Renewal recovery and unrelated administration retain their own guards.

Where the product requires no access gap, Entitlements and its selected adapter must demonstrate that continuity guarantee; its replacement or readiness mechanism belongs to the integration specification. Subscriptions cannot promise issue-before-revoke continuity by sending two asynchronous commands. IA-2026-10-18 makes initial usable readiness mandatory before initial commercial activation, subject to the joint freshness/withdrawal contract. It does not add a synchronous gate to resume or active changes. Circular waiting for a post-activation command is prohibited.

### Financial boundary of posture

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-posture-financial-boundary`

Counting, seats and point-of-use decisions belong to the providers behind Entitlements. Prepaid wallet balance/drawdown remains Billing/Rating-owned and launch-gated until that separate contract is accepted. The newly accepted promotional trial-credit target is distinct from paid wallets: Billing now owns its grant/balance/allocation/correction direction, while exact provider/Ledger integration remains OPEN under S-U15. Early valid guarded paid conversion closes the trial episode while retaining unused grant eligibility for paid expenses until original expiry under unchanged restrictions. It grants no access by balance alone and neither renews trial access nor issues a new grant. A financial correction cannot itself restore access or reopen a closed trial. After grant expiry, corrected credit remains unavailable to spend and retains the original expiration; no trial extension follows. A later correction cannot automatically reopen a trial already ended by exhaustion, even before original expiry: restored accounting balance grants no service/spend permission for that closed episode and does not reverse an applied paid conversion. V1 has no reactivation route; new acquisition is a fresh purchase; repeat trials are deferred. Platform liability for eligible accepted-trial value beyond bounded bonus, including positive corrections and delay/error overrun, served before the applied paid boundary is accepted; post-boundary expenses follow accepted paid terms. Financial protection is not a cap applied by Entitlements or at the point of use; asynchronous usage/Rating/access effects can overshoot, so credit launch needs explicit coverage and enforcement contracts. Subscriptions calculates neither remaining balance nor quota, and unknown credit evidence is not spend permission.

## 4. States (CDSL)

### Entitlement delivery state

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-state-entitlement-handoff`

Subscriptions records, per desired revision, the delivery and materialization states Entitlements reports (pending/acknowledged/failed/withdrawn; pending/applied/failed). These are observations reported by Entitlements, not subscription statuses and not a Subscriptions state machine. A later desired revision supersedes the target but retains earlier outcomes. Failed handoff never silently broadens commercial access.

## 5. Definitions of Done

### Commercial posture publication and observation intake

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-dod-entitlements`

Implement atomic posture/outbox persistence, posture publication, observation intake and the read surface.

**Implements:** `cpt-cf-bss-subscriptions-flow-entitlement-posture`, `cpt-cf-bss-subscriptions-algo-entitlement-posture`, `cpt-cf-bss-subscriptions-algo-posture-financial-boundary`, `cpt-cf-bss-subscriptions-state-entitlement-handoff`.

## 6. Acceptance Criteria

- [ ] A crash after commercial commit and before dispatch recovers the same posture publication without losing posture or duplicating materialization.
- [ ] The read surface distinguishes commercially active, delivery pending, materialization failed and observed success without asserting point-of-use access.
- [ ] Suspend preserves resource data and restoration identity; cancel grants only the explicitly accepted export capability. New suspended seat purchases/increases/additions produce no desired purchase grant; a separate guarded resume precedes normal active change. A committed-before-suspend purchase remains recoverable, and its late delivery cannot override the newer restriction.
- [ ] Authorized postpay conversion without a linked card or immediate payment produces the accepted paid posture once; absent required credit-service authorization or unsatisfied prescribed prepayment cannot produce it. Entitlements reports materialization asynchronously, and Subscriptions records it separately from commercial status.
- [ ] A credit-dependent initial trial whose necessary access awaits post-T final grant is explicitly gated under IA-2026-10-18; prepared coverage is not spendable and no active success is emitted. Recovery of previously applied outcomes preserves the same grant/T/expiry and newer cancellation fences.
