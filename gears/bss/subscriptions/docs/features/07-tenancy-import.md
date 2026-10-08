---
description: "Fixed subscription tenant identities, authorization and historical isolation; transfer and import remain deferred."
---

# Feature: Tenant Ownership, Transfer and Controlled Import


<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Request, approve and commit a transfer](#request-approve-and-commit-a-transfer)
  - [Import a legacy subscription](#import-a-legacy-subscription)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Financial responsibility through transfer](#financial-responsibility-through-transfer)
  - [Hierarchy, privacy and audit](#hierarchy-privacy-and-audit)
  - [Adoption completeness](#adoption-completeness)
- [4. States (CDSL)](#4-states-cdsl)
  - [Transfer and import state](#transfer-and-import-state)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Authorized ownership and controlled adoption](#authorized-ownership-and-controlled-adoption)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-featstatus-tenancy-import-implemented`
- [ ] `p1` - `cpt-cf-bss-subscriptions-feature-tenancy-import`

**Status:** tenant identity and authorization design; ownership/payer transfer is deferred. Import/adoption is DEFERRED by the user on 2026-10-14; retained import headings and IDs are future traceability only, with no current implementation or acceptance tasks. Immediate payer splitting and cross-axis transfer retain their applicable contracts.
**Current-profile boundary, 2026-10-15:** apply the [current delivery scope](../PRD.md). Audit subsystem, confirmAcceptance, ramps, co-term, collection pause, notifications, tenant/payer transfer, administrative backdating/correction and optional brand/PlanTier metadata are deferred. Earlier references and scenarios involving those capabilities are future context only, not current acceptance or implementation tasks. Retain authorization, operation/event provenance, reliable effects and current facts/replay.

**Additional current-profile boundary, 2026-10-15:** the [additional scope reduction](../PRD.md#11-purpose) defers trial extension/repeat, acquisition without Orders, scheduled composition/resume, linked subscription replacement and base-plan changes. Retain supported immediate increases/additions, existing lifecycle, future order starts/cancellation and ordinary timers, both trial variants and early conversion to the accepted paid target. Deferred paths are refused even with explicit policy or operator authorization; their earlier examples are future context, not launch gates.

## 1. Feature Context

### 1.1 Overview

Preserve current authorization and historical financial/resource ownership for the subscription throughout its current lifecycle. Tenant/payer transfer and import are deferred.

### 1.2 Purpose

**Requirements:** `cpt-cf-bss-subscriptions-fr-tenant-axes`, `cpt-cf-bss-subscriptions-fr-delegation-proofs`, `cpt-cf-bss-subscriptions-fr-hierarchy-reference`. **Deferred requirement:** `cpt-cf-bss-subscriptions-fr-import`.

**Constraints:** `cpt-cf-bss-subscriptions-constraint-tenant-isolation`, `cpt-cf-bss-subscriptions-constraint-posted-immutability`.

### 1.3 Actors

| Actor | Role |
| --- | --- |
| `cpt-cf-bss-subscriptions-actor-ams` | Owns tenant/account identity, hierarchy and delegation proof. |
| `cpt-cf-bss-subscriptions-actor-partner-admin` | Acts within delegated creation-time tenant scope; ownership transfer is deferred. |
| `cpt-cf-bss-subscriptions-actor-platform-operator` | Resolves authorized tenant-scope findings; transfer and import operations are deferred. |

### 1.4 References

- **PRD:** [PRD.md](../PRD.md); **Design:** [tenant isolation](../DESIGN.md#tenant-isolation-delegation-proofed-cross-tenant).
- **Build prerequisites:** [foundation](./01-foundation.md) and [composition](./02-composition.md), as allocated in [DECOMPOSITION.md](../DECOMPOSITION.md).
- **Integration acceptance:** the tenant/history core must demonstrate applicable [renewal/dwell](./04-renewal-retention.md), [entitlement posture](./05-entitlements.md), [commercial fact](./08-commercial-facts.md) and [attribution/recovery](./09-consumer-contracts.md) contracts for the retained scope. Transfer/import conformance is deferred; these checks do not make consumer features reverse build prerequisites for the tenant/history core.
- **Source decisions:** SUB-D-06 pinned ordering, SUB-D-20 as-of payer and SUB-D-22 dwell lifecycle; user-accepted S-Q02 occupancy including suspended-place retention, with open owner/consumer contract and exact policy/key movement mappings, with transfer-specific Billing integration deferred.

## 2. Actor Flows (CDSL)

### Request, approve and commit a transfer

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-transfer-tnt`

**DEFERRED, 2026-10-15.** Payer/resource-owner transfer is outside the current version. No transfer endpoint, approval flow, claim handover or provider rebind is implemented. Creation-time tenant axes and authorization remain required.

### Import a legacy subscription

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-flow-import`

**DEFERRED, 2026-10-14.** Traceability placeholder only; no import flow is implemented or required by the current feature.

## 3. Processes / Business Logic (CDSL)

### Financial responsibility through transfer

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-transfer-billing`

**DEFERRED, 2026-10-15.** No payer-transfer splitting or financial reallocation algorithm is implemented. Existing facts retain their recorded payer; ordinary financial replay remains unchanged.

### Hierarchy, privacy and audit

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-algo-tenant-reference`

Commercial account and OrgTier roll-ups reference AMS identity/version; Subscriptions never authors tenant topology. Authorize commands, historical reads and recovery ranges using current PDP-derived scope and the fixed creation-time tenant axes. Preserve the ordering partition and lifecycle sequence. Receipts/domain events retain relevant actor, reason, scope proof, source revision and correlation provenance for later audit consumers. No transfer approval flow or audit store/query/export subsystem is implemented.

### Adoption completeness

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-algo-import-cutover`

**DEFERRED, 2026-10-14.** No adoption/cutover algorithm is part of the current delivery.

## 4. States (CDSL)

### Transfer and import state

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-state-tenancy-import`

Tenant axes remain fixed in the current profile. Transfer and import operation states are deferred; current authorization and original tenant provenance remain observable.

## 5. Definitions of Done

### Authorized ownership and controlled adoption

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-dod-tenancy-import`

Implement creation-time tenant-axis authorization, current-scope proof checks, pinned ordering and cross-tenant isolation for commands, history and replay. No transfer, import or audit subsystem is part of this definition of done.

**Implements:** `cpt-cf-bss-subscriptions-algo-tenant-reference`. Transfer and import IDs remain deferred.

## 6. Acceptance Criteria

- [ ] Creation binds independently authorized seller, payer and resource tenant axes and a stable ordering tenant.
- [ ] Commands and historical/recovery reads enforce current authorization, including replay, without leaking another tenant's operation or financial history.
- [ ] Transfer/rebinding and import are unavailable in the current profile; generic metadata updates cannot bypass that boundary.
- [ ] Actor, correlation and applicable reason provenance accompany domain events for a future audit consumer; no dedicated audit service or projection is required.
