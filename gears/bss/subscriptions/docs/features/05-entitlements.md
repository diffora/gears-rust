---
description: "Commercial entitlement posture and its asynchronous application by deployment-selected fulfillment providers."
---

# Feature: Commercial Entitlement Posture and Provider Handoff


<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Retired entitlement-check compatibility identity](#retired-entitlement-check-compatibility-identity)
  - [Apply a commercial posture and reconcile materialization](#apply-a-commercial-posture-and-reconcile-materialization)
  - [Inspect and resolve a mismatch](#inspect-and-resolve-a-mismatch)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Posture by transition](#posture-by-transition)
  - [Capacity, quotas and prepaid limits](#capacity-quotas-and-prepaid-limits)
  - [Handoff recovery](#handoff-recovery)
- [4. States (CDSL)](#4-states-cdsl)
  - [Entitlement delivery state](#entitlement-delivery-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Versioned Entitlement provider handoff](#versioned-entitlement-provider-handoff)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-featstatus-entitlements-implemented`
- [ ] `p1` - `cpt-cf-bss-subscriptions-feature-entitlements`

**Status:** provider-independent capability boundary. The selected integration must demonstrate its applicable application/recovery guarantees before runtime readiness can be asserted; provider-specific APIs are deferred to adapter specifications.
**Current-profile boundary, 2026-10-15:** apply the [current delivery scope](../PRD.md). Audit subsystem, confirmAcceptance, ramps, co-term, collection pause, notifications, tenant/payer transfer, administrative backdating/correction and optional brand/PlanTier metadata are deferred. Earlier references and scenarios involving those capabilities are future context only, not current acceptance or implementation tasks. Retain authorization, operation/event provenance, reliable effects and current facts/replay.

**Additional current-profile boundary, 2026-10-15:** the [additional scope reduction](../PRD.md#11-purpose) defers trial extension/repeat, acquisition without Orders, scheduled composition/resume, linked subscription replacement and base-plan changes. Retain supported immediate increases/additions, existing lifecycle, future order starts/cancellation and ordinary timers, both trial variants and early conversion to the accepted paid target. Deferred paths are refused even with explicit policy or operator authorization; their earlier examples are future context, not launch gates.

## 1. Feature Context

### 1.1 Overview

Translate committed commercial terms and lifecycle posture into a versioned desired entitlement set, then track the selected provider integration result without taking over materialization or point-of-use enforcement.

This is an abstract entitlement fulfillment boundary, not a specification of our License Manager. A deployment selects an adapter for its native, third-party or customer-managed fulfillment system. Subscriptions supplies desired commercial rights and consumes its own correlated observations. The [provider boundary](../DESIGN.md#11-architectural-vision) defines applicability and guarantees; vendor requests/responses, license states and transport belong to integration-specific documentation. In the procedures below, provider evidence is normalized by that adapter and does not require a vendor to expose the same field names or state machine.

### 1.2 Purpose

**Requirements:** `cpt-cf-bss-subscriptions-fr-entitlement-issue-revoke`, `cpt-cf-bss-subscriptions-fr-entitlement-assignment`, `cpt-cf-bss-subscriptions-fr-entitlement-license-state`.

**Principle:** `cpt-cf-bss-subscriptions-principle-idempotent-ordered`. **Constraint:** `cpt-cf-bss-subscriptions-constraint-tenant-isolation`.

### 1.3 Actors

| Actor | Role |
| --- | --- |
| `cpt-cf-bss-subscriptions-actor-partner-admin` | Inspects desired/observed license state and findings. |
| Entitlement provider | Materializes, replaces, restricts and revokes the requested grant set. |
| Platform license/quota enforcement | Makes point-of-use decisions, manages counters and assignment enforcement. |

### 1.4 References

- **PRD:** [PRD.md](../PRD.md); **Design:** [entitlement contract](../DESIGN.md#335-entitlement-and-retention-contracts).
- **Build prerequisites:** [foundation](./01-foundation.md) and [composition](./02-composition.md), as allocated in [DECOMPOSITION.md](../DECOMPOSITION.md).
- **Provider prerequisites / integration acceptance:** accepted product entitlement descriptors and a selected adapter implementing the abstract capability profile, including the S-U04 ordering/recovery fixture. Cross-lifecycle handoff verification is an integration acceptance step, not a build dependency on each lifecycle feature.
- **Retirement:** legacy entitlement-check API, QuotaTracker, local SeatBinding/LIFO revocation and SUB-D-10 check staleness defaults do not apply to this target.

## 2. Actor Flows (CDSL)

### Retired entitlement-check compatibility identity

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-entitlement-check-ent`

**Retired compatibility identity only.** The former local point-of-use check flow creates no implementation obligation. Its current replacement is `cpt-cf-bss-subscriptions-flow-entitlement-posture` below: Subscriptions supplies commercial posture through the selected entitlement adapter, while platform enforcement performs point-of-use checks and counting. This retained identifier does not reintroduce a local check API, counters or an access-decision service.

### Apply a commercial posture and reconcile materialization

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-entitlement-posture`

1. The lifecycle handler derives the complete target commercial grant/posture set from immutable accepted product descriptors and committed quantity at the boundary. It does not resolve an old Pricing phase/grant map.
2. The Foundation commit stores desired posture, effective interval, source aggregate revision, tenant axes and entitlement provider delivery intent atomically with the commercial change.
3. Dispatch an idempotent materialize/replace/restrict/revoke request using a stable subscription/revision/action identity. Attach correlation, source evidence and the target effective boundary. For credit-dependent issue/replace, first require the correlated final grant/eligibility evidence from the [staged S-U15 startup](../TRIAL_PRICING_PROPOSAL.md#4-candidate-provider-capabilities); prepared coverage alone is not issued credit. Keep that delivery visibly pending while finalization is unknown, without moving the committed T or waiting synchronously for entitlement provider readiness. Restrict/revoke and a newer cancellation fence do not wait for grant finalization; a late grant cannot authorize an obsolete posture. Concrete vendor payloads and transport are outside this design; the selected adapter must satisfy the abstract capability profile before that integration is enabled.
4. The selected adapter/provider integration must prevent an older issue/replace from undoing a newer revoke, including reordered deliveries with different request keys. A provider-side revision fence is one possible implementation, not a required native API. Adapter serialization/reconciliation is sufficient only when it also accounts for already in-flight external actions and proves the same effective ordering. Unsupported ordering blocks the affected profile under S-U04.
5. Record entitlement provider acknowledgement, materialized version and structured findings separately from commercial status. A broker delivery receipt is not materialization success; entitlement provider success is not evidence of platform enforcement propagation.
6. On retry or delayed response, apply a result only to its matching request/revision. Preserve older outcomes for audit while leaving the newer desired posture authoritative.
7. Surface desired state, latest confirmed license state, source revision, last observed time and actionable mismatch reason through subscription reads. Explicitly show pending or failed delivery instead of reporting implied access.

### Inspect and resolve a mismatch

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-flow-entitlement-repair`

1. An authorized operator requests reconciliation for a subscription or scope, reading desired and observed versions with one consistent tenant scope.
2. Missing/mismatched materialization produces a durable finding and retries the existing desired-version command or submits an explicitly authorized successor posture.
3. A commercially invalid desired posture must be changed through the Foundation; the operator cannot directly edit a materialized license or quota counter through Subscriptions.
4. Close a finding only on correlated materialization evidence or an explicit audited disposition. A consumer outage or a newer desired version cannot silently erase the historical failed handoff.

## 3. Processes / Business Logic (CDSL)

### Posture by transition

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-entitlement-posture`

| Commercial event | Desired posture and boundary |
| --- | --- |
| Activation | Full accepted target at actual applied start; import/resource adoption is deferred. |
| Composition/quantity change | Replace the complete target at the same committed change boundary; no transient union of old/new grants. |
| Suspend | Restricted/paused posture preserving data and restoration identity. |
| Resume | Restore the accepted set only after required lifecycle/occupancy guards. |
| Cancel | Revoke normal service, with separate narrow export capability during permitted retention. |
| Transfer | Effective tenant binding changes, with historical financial/resource attribution retained. |
| Trial conversion | Target paid posture only after S-Q06 model and the applicable authorized settlement route: prescribed payment for prepay, or accepted postpay terms plus authorization to serve on credit. A linked card or immediate payment is not required for authorized postpay. |

Where the product requires no access gap, the selected adapter/provider integration must demonstrate that continuity guarantee; its replacement or readiness mechanism belongs to the integration specification. Subscriptions cannot promise issue-before-revoke continuity by sending two asynchronous commands. Any requirement to wait for entitlement provider readiness before commercial activation is part of that joint contract; circular waiting for a post-activation command is prohibited.

### Capacity, quotas and prepaid limits

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-entitlement-boundaries`

Quota usage, hard/soft limits, seat binding, reservation atomicity, resets and point-of-use staleness are owned by the enforcement system, with entitlement provider materializing the accepted limit definition. Subscriptions may forward committed commercial quantity/period boundaries and report findings; it does not calculate remaining quota or reset counters when a recurring fact emits. A quantity-decrease guard needing occupied-seat evidence requires the owner's authoritative guard contract, not a cached count in this gear. Prepaid wallet balance/drawdown remains Billing/Rating-owned and launch-gated until that separate contract is accepted. The newly accepted promotional trial-credit target is distinct from paid wallets: Billing now owns its grant/balance/allocation/correction direction, while exact provider/Ledger integration remains OPEN under S-U15. Early valid guarded paid conversion closes the trial episode while retaining unused grant eligibility for paid expenses until original expiry under unchanged restrictions. It grants no access by balance alone and neither renews trial access nor issues a new grant. A financial correction cannot itself restore access or reopen a closed trial. After grant expiry, corrected credit remains unavailable to spend and retains the original expiration; no trial extension follows. A later correction cannot automatically reopen a trial already ended by exhaustion, even before original expiry: restored accounting balance grants no service/spend permission for that closed episode and does not reverse an applied paid conversion. V1 has no reactivation route; new acquisition is a fresh purchase; repeat trials are deferred. Platform liability for eligible accepted-trial value beyond bounded bonus, including positive corrections and delay/error overrun, served before the applied paid boundary is accepted; post-boundary expenses follow accepted paid terms. Financial protection is not an entitlement provider or point-of-use cap; asynchronous usage/Rating/access effects can overshoot, so credit launch needs explicit coverage and enforcement contracts. Subscriptions calculates neither remaining balance nor quota, and unknown credit evidence is not spend permission.

### Handoff recovery

**RO-2026-10-15 integration.** [RO-2026-10-15](04-renewal-retention.md#7-renewal-and-offboarding-integration-contract) specifies frozen term/default evaluation and Contract-end handling, one renewal/dwell episode, durable cutoff versus actual stop/cancel, and independent retention/hold/delete/archive generations. Terms or payment events do not mutate running terms or reopen terminal state. Resource/hold adapters must prove stale-action suppression and hold-versus-delete arbitration at the actual execution boundary. Deferred notification delivery is not assumed complete. Exact provider mapping and RO-C01-RO-C16 execution remain open; no native LM/OSS dependency is introduced.


- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-entitlement-delivery`

Outbox replay uses the same request identity and retained adapter/profile routing. Reconciliation obtains correlated evidence of the applied desired operation through the adapter; that evidence may derive from native provider versions, operation status, authoritative state reads or a combination backed by its durable journal. An adapter's dispatch log alone is not proof of external application. Vendor-specific mappings are deferred to the adapter specification. S-Q07 measures handoff acknowledgement and observed materialization separately for the enabled profile. No inherited local entitlement-check latency or cache policy becomes a Subscriptions guarantee.

## 4. States (CDSL)

### Entitlement delivery state

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-state-entitlement-handoff`

A desired revision has delivery pending/acknowledged/failed and observed materialization pending/applied/failed. These are operation observations, not subscription statuses or a replacement for entitlement provider's license state machine. A later desired revision supersedes the target but retains earlier outcomes. Failed handoff never silently broadens commercial access.

## 5. Definitions of Done

### Versioned Entitlement provider handoff

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-dod-entitlements`

Implement atomic desired-posture/outbox persistence, versioned idempotent entitlement provider delivery, correlated outcomes and mismatch reads/repair. Accept the entitlement provider contract and joint failure/continuity fixture before enablement. Remove local check/counter/SeatBinding ownership from the target and ensure dependent quantity/trial flows fail explicitly when missing owner capabilities.

**Implements:** `cpt-cf-bss-subscriptions-flow-entitlement-posture`, `cpt-cf-bss-subscriptions-flow-entitlement-repair`.

## 6. Acceptance Criteria

- [ ] A crash after commercial commit and before dispatch recovers the same entitlement provider command without losing posture or duplicating materialization.
- [ ] Deliver issue request N after revoke request N+1, using different command keys: the adapter/provider integration prevents access restoration and recovers evidence that the newer posture is applied, without requiring a vendor-native source-revision field.
- [ ] A delayed old result cannot restore the old plan after a newer replacement or cancellation.
- [ ] The read surface distinguishes commercially active, delivery pending, materialization failed and observed success without asserting point-of-use access.
- [ ] Suspend preserves resource data and restoration identity; cancel grants only the explicitly accepted export capability.
- [ ] Quantity reduction cannot bypass the enforcement owner's occupied-capacity guard by consulting a retired local counter.
- [ ] No local point-of-use check endpoint, quota reset job or wallet drawdown is required by this feature.
- [ ] An unavailable required adapter yields an explicit gated capability and visible finding; a product profile that does not require that capability reports not applicable without fabricated application success.
- [ ] Authorized postpay conversion without a linked card or immediate payment produces the accepted paid posture once; absent required credit-service authorization or unsatisfied prescribed prepayment cannot produce it. entitlement provider materialization remains asynchronous and separately observed.
- [ ] Credit-trial T committed with Billing finalization delayed leaves credit-dependent issuance pending under the same revision; recovering the finalized grant permits only the still-current authorized posture. A newer cancellation/revocation proceeds without that grant and cannot be undone by late finalization. Neither path moves T or waits synchronously for entitlement provider readiness.
