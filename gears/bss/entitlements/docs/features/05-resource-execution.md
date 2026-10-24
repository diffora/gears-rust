---
description: "Provision, pause, restore, stop, export and delete work orders with generation fencing, hold-versus-delete arbitration and truthful physical outcomes."
---

# Feature: Resource Execution

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Resource work order](#resource-work-order)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Generation fencing](#generation-fencing)
- [4. States (CDSL)](#4-states-cdsl)
  - [Work order state](#work-order-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [5.1 Truthful execution per resource generation](#51-truthful-execution-per-resource-generation)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-featstatus-resource-execution-implemented`
- [ ] `p1` - `cpt-cf-bss-entitlements-feature-resource-execution`

**Status:** design set only, created 2026-10-23 by the split of the application layer out of Subscriptions. No adapter, SDK or runtime is delivered.

## 1. Feature Context

### 1.1 Overview

Entitlements translates posture into resource work orders with operation identity and immutable resource generation and submits them through the selected resource adapter. The resource provider owns execution and reports actual effective times and pending, unknown or partial outcomes; Entitlements never backdates an outcome and never applies an old order to a replacement resource with a recycled name.

### 1.2 Purpose

**Requirements:** `cpt-cf-bss-entitlements-fr-resource-execution`.

### 1.3 Actors

| Actor | Role |
| --- | --- |
| `cpt-cf-bss-entitlements-actor-resource-provider` | Executes work orders and reports physical outcomes per resource generation. |
| `cpt-cf-bss-entitlements-actor-subscriptions` | Owns commercial intent, the confirmed charging cutoff and disposition eligibility. |
| `cpt-cf-bss-entitlements-actor-platform-operator` | Inspects partial and unknown outcomes and hold protection. |

### 1.4 References

- **PRD:** [PRD.md](../PRD.md); **Design:** [adapter capability profile](../DESIGN.md#333-adapter-capability-profile).
- **Build prerequisites:** as allocated in [DECOMPOSITION.md](../DECOMPOSITION.md); decisions in [DECISIONS.md](../DECISIONS.md).
- **Provider prerequisites / integration acceptance:** gate E-U05 in [UPSTREAM_REQS.md](../UPSTREAM_REQS.md); the owner asks are in the [proposals](../proposals/README.md).

## 2. Actor Flows (CDSL)

### Resource work order

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-flow-resource-work-order`

**Actor:** Subscriptions, through `CommercialPostureV1` for a committed revision; the selected resource adapter as the executor. **Success:** every work order derived from the revision is submitted with its operation identity and immutable resource generation, and a correlated physical outcome with the actual effective time is stored for each resource. **Errors:** no resource member in the revision, unsupported work order kind at the selected adapter, fence refused, hold protection pending, adapter refused, unknown outcome after a lost response, partial completion, revision superseded before application.

The adapter boundary is provider independent (ENT-D-01): the platform's own resource service, a third-party service system and a customer integration are alternative adapters of one capability profile. Entitlements keeps the desired work, the correlated outcomes and the end-to-end idempotency, stale-action and recovery guarantees; the adapter maps them to its native operations. A deployment selects the adapter per capability profile; an explicitly inapplicable resource capability differs from an unavailable required adapter, and an absent required adapter is never fabricated success. Subscriptions owns commercial intent, the confirmed charging cutoff and disposition eligibility; the resource provider owns execution and the authoritative physical outcome. Inventory and evidence is a separate role even when the same deployment service supplies both.

1. **Derive.** Entitlements accepts a posture revision under [posture intake](01-posture-intake.md) and selects the members whose capability references name a resource capability. For each member it maps the posture and the lifecycle transition carried by the revision to work order kinds under [generation fencing](#generation-fencing). The mapping below is the complete set of kinds; a transition that needs no resource effect derives no order.

| Posture / transition | Work order kinds | Boundary carried |
| --- | --- | --- |
| `Granted`, initial activation | `provision` | Pinned service profile and target; readiness is reported through [initial readiness](02-initial-readiness.md), not through this flow. |
| `Granted` after `Restricted` (resume) | `restore` | The retained resource generation; the same resource is restored, no second resource is created. |
| `Restricted` (suspension) | `pause` | Data and restoration identity preserved; a pause is never deletion. |
| `Revoked` with a confirmed charging cutoff (cancellation, trial expiry) | `service-stop`, then `export` where the frozen disposition terms grant it | The confirmed cutoff as a boundary only; the actual stop time is reported by the adapter. |
| `Revoked`, retention window closed and eligible | `delete` | Exact resource and disposition identity, current deletion schedule generation, expected hold policy version and required prerequisite evidence, as supplied by Subscriptions. |

2. **Persist.** Each work order carries the operation identity, the kind, the immutable resource generation, the boundary from the table and the adapter and profile it is routed to. The orders, the desired revision and the delivery intent are written with the intake in one local transaction, before any remote call. Entitlements keeps the association from the posture identity to the resource internally; the adapter receives the authorized scope, the resource generation and the operation identity, never a subscription identifier.
3. **Fence and submit.** The outbox dispatches each order to the selected adapter under [generation fencing](#generation-fencing). Retry and backoff change neither identity nor content. A `delete` order is submitted only while the proven hold fence of gate E-U05 is available; without it automatic deletion stays disabled and the order stays `requested` with a finding.
4. **Record the outcome.** The adapter reports the physical outcome per resource and generation with the actual effective time and an explicit `pending`, `applied`, `partial`, `refused` or `unknown` result. Entitlements stores it as it was reported. A stop at 10:05 after a 10:00 cutoff stays 10:05; Entitlements never backdates an outcome to the cutoff and never advances a cutoff to the stop. A dispatch log, a queued command or a broker acknowledgement is not an outcome. The outcome is reported to Subscriptions as materialization state; the ordinary customer tariff ends at the confirmed cutoff regardless of the technical delay, and that decision stays with Subscriptions and Billing.
5. **Retention and export.** After a `service-stop`, retained data follows the frozen accepted disposition terms. The adapter exposes the accepted read-only export capability and returns the retention state with effective timing evidence; whether retained data is charged is decided by the financial owner from that evidence, not by the provider and not by Entitlements. Data presence, hold, export or delayed deletion alone creates no fee and does not continue the former service tariff.
6. **Unknown and partial.** A timeout is an unknown outcome, not completion and not absence. Provider inventory absence alone does not prove that this specific deletion completed. A partial multi-resource completion records each resource with its own outcome and keeps the remaining work visible. Unknown and partial outcomes stay open until [reconciliation](06-reconciliation-recovery.md) resolves them by the original identity.

## 3. Processes / Business Logic (CDSL)

### Generation fencing

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-algo-generation-fencing`

**Input:** one work order with its operation identity, kind, resource generation, boundary and current hold evidence; the current desired head for the resource. **Output:** the order is submitted, deferred or refused before any effect, and every late outcome is applied to the right generation or kept as evidence only.

1. **Bind the generation.** Every order names the immutable resource generation it targets. A resource name is not an identity: a later independent purchase can create a new generation under the same name. The adapter must refuse or ignore an order whose generation does not match the resource it finds, and must never apply an old action to a replacement resource with a recycled name (RS-P05).
2. **Order against the desired head.** A lower revision than the applied head is stale and is refused. An equal revision with the same content returns the existing outcome. A higher revision replaces only this source's work after validation. A terminal `delete` outcome closes the generation; a later `provision` for the same acquisition needs a new generation, never the old one.
3. **Pause is not deletion.** A `pause` preserves data and the restoration identity and implements the accepted restricted posture (RS-P02). A `restore` targets the retained generation. An old `restore` already in flight cannot override a newer `pause` or `service-stop`; the fence is at the adapter's effect boundary, not only at a callback filter.
4. **Hold versus delete.** A `delete` carries the current schedule generation and the expected hold policy version. The adapter orders destructive execution against hold placement and withdrawal at its own authoritative execution boundary, or implements acknowledged generation and hold fencing (RS-P04). A cached negative hold read plus an idempotent delete is insufficient. When a hold arrives, Entitlements blocks new destructive dispatch at once and distinguishes a requested or propagating hold from confirmed protection at the adapter boundary. If the hold wins there, the old delete is durably refused or unapplied. If irreversible deletion already won, the adapter reports its true applied, in-progress or partial state; an already-started destructive winner is reported truthfully, and no one claims that the hold protected or restored the data. The hold stays in force for the remaining protected scope and each resource is reconciled. Unknown ordering blocks any claim of protected or completed disposition until authoritative recovery.
5. **Release is not delete.** When the final applicable hold is released, Subscriptions supplies a fresh schedule generation with a future deadline and the current notice evidence. An old expired deadline, an old notice or an old worker is never reused; a stale worker carrying an earlier schedule generation cannot delete. A newer hold invalidates the new eligibility through the same execution fence.
6. **Recover by identity.** Retry and recovery use the original operation identity and generation (RS-P05). A lost response is read back as the same operation; no second resource is provisioned and no second delete is issued. An old stop success may establish historical runtime evidence, but cannot complete a newer unrelated operation or undo the current posture.

## 4. States (CDSL)

### Work order state

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-state-work-order`

Each work order carries one current outcome: `requested`, `applied`, `partial`, `refused` or `unknown`. These are execution observations per operation and resource generation, not subscription statuses; the commercial status stays in Subscriptions.

| From | Event / to | Mandatory guard |
| --- | --- | --- |
| none | order persisted / requested | Written with the desired revision and delivery intent in one local transaction; adapter and profile routing retained. |
| requested | correlated outcome `applied` for the same operation and generation / applied | Actual effective time recorded as reported; a result for another generation does not move this row. |
| requested | some resources applied, others not / partial | Per-resource outcomes recorded; the remaining work stays visible and recoverable by the original identity. |
| requested or partial | definitive adapter refusal, stale generation or fence refused / refused | Recorded separately from an unknown result; a finding is opened. |
| requested or partial | lost response or timeout / unknown | Not completion, not absence; resolved only by readback or a later correlated outcome. |
| unknown | readback finds the same operation applied or refused / applied, partial or refused | The same operation identity; no second effect was issued. |
| requested, partial or unknown | higher desired revision supersedes the order / refused | The newer revision owns the resource head; a late outcome for this order is historical evidence only. |

A hold arriving while a `delete` is `requested` or `unknown` keeps the order open until the adapter confirms which side won; the order never becomes `applied` on a local receipt alone. A lower revision never moves any row. Each state is readable by resource, generation, operation and revision after restart.

## 5. Definitions of Done

### 5.1 Truthful execution per resource generation

- [ ] `p1` - **ID**: `cpt-cf-bss-entitlements-dod-resource-execution`

The implementation must derive the complete set of work orders of a revision from the posture and the lifecycle transition, persist them with the desired revision and the delivery intent in one local transaction, dispatch only through the outbox to the selected adapter, bind every order and outcome to the immutable resource generation, and record outcomes with the actual effective time as reported, including explicit pending, partial, refused and unknown results. A pause never deletes; a delete is dispatched only under the proven hold fence of gate E-U05 and is otherwise disabled; a hold racing a delete reaches the fenced outcome and an already-started destructive winner is reported truthfully. No subscription identifier is written to any adapter payload; the adapter receives the authorized scope, the resource generation and the operation identity (ENT-D-01). Enabling the profile requires the ratified adapter mapping and the executed cases named in the [resource provider proposal](../proposals/resource-provider-proposal.md).

**Implements:** `cpt-cf-bss-entitlements-flow-resource-work-order`, `cpt-cf-bss-entitlements-algo-generation-fencing`, `cpt-cf-bss-entitlements-state-work-order`.

## 6. Acceptance Criteria

- [ ] A confirmed charging cutoff of 10:00 with a service stop at 10:05 stores 10:05 as the actual stop and reports it to Subscriptions unchanged; the adapter is never asked to backdate, and the tariff boundary stays with Subscriptions (RS-C01).
- [ ] A hold that reaches the adapter before a stale delete prevents the deletion; the reverse order reports the true applied, in-progress or partial deletion without claiming protected or restored data. An unknown order of events keeps the disposition pending until authoritative recovery (RS-C02).
- [ ] A delayed delete for generation A of a resource name does not delete generation B created later under the same name; the new purchase adopts neither A's data nor its continuity (RS-C03).
- [ ] A lost delete response, or a delete that completed only some resources, is recovered by the original operation identity with per-resource outcomes; no second delete is issued and no false completion is recorded (RS-C04).
- [ ] A pause preserves data and the restoration identity; a restore targets the same generation; an old restore in flight cannot override a newer pause or stop. A delete without the proven hold fence stays `requested` with a finding.
