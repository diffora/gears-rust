<!-- CONFLUENCE_TITLE: [BSS]: Subscriptions — Consumer & Integration Contracts (Design) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../SEAMS.md | Upstream: Contracts, Policy Engine, Payments, Registry | Downstream: Rating, Billing, OSS | Owners: BSS Subscriptions team -->

# DESIGN — Consumer & Integration Contracts (Slice 9)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-design-consumer-contracts`

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles and Constraints](#2-principles-and-constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions and Sequences](#36-interactions-and-sequences)
  - [3.7 Database Schemas and Tables](#37-database-schemas-and-tables)
  - [3.8 Deployment Topology](#38-deployment-topology)
- [4. Additional Context](#4-additional-context)
  - [4.1 Control-Plane Operations (normative)](#41-control-plane-operations-normative)
  - [4.2 Rating Read-Model Contract (normative)](#42-rating-read-model-contract-normative)
  - [4.3 Billing Handoff Contract (normative)](#43-billing-handoff-contract-normative)
  - [4.4 Contracts Input Contract (normative)](#44-contracts-input-contract-normative)
  - [4.5 Policy and OSS Contracts (normative)](#45-policy-and-oss-contracts-normative)
  - [4.6 Payments and Notifications Contracts (normative)](#46-payments-and-notifications-contracts-normative)
  - [4.7 Registry Overlap-Key Contract (normative)](#47-registry-overlap-key-contract-normative)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

This slice is the **boundary surface**: the control-plane operations clients invoke and the external
integration contracts every neighbour consumes or supplies. It **assembles** the contracts the
capability slices produce — it introduces no new lifecycle policy. The PRD's §6 content boundary
holds: business operations + protocol/format intent live here; concrete REST paths, methods,
idempotency/ETag header bindings, OpenAPI, event field matrices, and error taxonomies are the Design
detail this slice frames ([`../PRD.md`](../PRD.md) §9).

Nearly every seam surfaces at this boundary: **SUB-R1** (rating read-model), **SUB-B1** (Billing
handoff), **SUB-C1/C5** (Contracts input + `PriceOverride`), **SUB-E1/E2/E3** (Policy gate, OSS
provisioning, entitlement check), **SUB-F1/F2** (Payments signals, Notifications triggers), and
**SUB-G1** (the registry overlap key).

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-subscriptions-interface-control-plane` | The business-operations contract (create/get/list/activate/suspend/resume/cancel/changePlan/add-on/entitlement) with idempotency + optimistic-concurrency requirements; REST mapping is Design (§4.1). |
| `cpt-cf-bss-subscriptions-contract-rating-read-model` | Composition read models (effective `PlanLink`/`AddOn`, `PlanTier` @ `t`, active phase @ `t`, `(changeEffectiveAt, changeMode)`) on the shared ordering key (§4.2). |
| `cpt-cf-bss-subscriptions-contract-billing-handoff` | `BillableItemCreated(recurring)` idempotent per `(subscriptionId, billing period)` + traceability tuple; proration only as new/adjusting artifacts (§4.3). |
| `cpt-cf-bss-subscriptions-contract-contracts-input` | Signed terms, `Renewal`, grace/regional templates, `PriceOverride` via events + read models; evaluated fields stored at evaluation time (§4.4). |
| `cpt-cf-bss-subscriptions-contract-policy-gate` / `cpt-cf-bss-subscriptions-contract-oss-provisioning` | Pre-commit allow/deny + `reasonCodes`, fail-closed; provision/deprovision/pause confirmed by events (§4.5). |
| `cpt-cf-bss-subscriptions-contract-payments-signals` | Payment pre-check + retry-exhaustion consumed by the grace ladder; PSP webhooks + dunning payloads are Design (§4.6). |

#### NFR Allocation

| NFR theme | Allocated To | Design Response | Verification / Status |
|-----------|--------------|-----------------|-----------------------|
| `cpt-cf-bss-subscriptions-nfr-operational-baselines` | Read/handoff contracts | Subscription query p95 < 200ms; event delivery p95 < 30s; bulk read models for roll-ups | Load test; baseline (workshop-pending) |
| `cpt-cf-bss-subscriptions-nfr-entitlement-check-latency` | Check contract (slice 05) | p95 < 100ms surfaced through this boundary | Load test before GA |

#### Key ADRs

No slice-local ADR; the contracts realise the seams frozen in [`../SEAMS.md`](../SEAMS.md).

### 1.3 Architecture Layers

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-tech-stack-con`

| Layer | Responsibility | Technology |
|-------|----------------|------------|
| Presentation | REST control-plane + read surfaces behind the inbound gateway; RFC 9457 problems; OAuth 2.0; idempotency + ETag | Rust, REST/OpenAPI, inbound API gateway |
| Application | Contract assembly over the capability slices; event/read-model projection surfaces | Rust module in the `subscriptions` gear |
| Domain | The integration contract value objects (handoff, read-model, gate, signals) | Rust; GTS + Rust domain structs |
| Infrastructure | Read-model projections + the outbox (Foundation) | PostgreSQL, SecureORM |

## 2. Principles and Constraints

### 2.1 Design Principles

#### Assemble, never re-author

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-assemble-con`

This slice assembles the contracts the capability slices produce; it introduces no new lifecycle
policy — the boundary is a projection of committed state + emitted events ([`../PRD.md`](../PRD.md) §9).

#### Protocol intent here, wire detail in Design

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-intent-not-wire-con`

The slice fixes protocol/format **intent** (what crosses, idempotency/ordering guarantees); REST
paths, headers, OpenAPI, and error taxonomies are the Design detail it frames ([`../PRD.md`](../PRD.md)
§6 content boundary, §9.1).

### 2.2 Constraints

#### Fail-closed at every gated boundary

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-fail-closed-boundary-con`

The Policy gate fails closed unconditionally (no allow on deny/unavailability); the entitlement
check fails closed **beyond the SUB-D-10 staleness budget** (last-known-good decisions are served
inside it — slice 05 §4.3); the Billing handoff never mutates posted state
([`../PRD.md`](../PRD.md) §9.2; SEAMS **SUB-E1**, **SUB-B1**).

## 3. Technical Architecture

### 3.1 Domain Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-domain-model-con`

The boundary value objects: `ControlPlaneOperation`, `CompositionReadModel` (rating input),
`RecurringHandoff` (Billing), `ContractsInput` (terms + `PriceOverride`), `PolicyDecision`,
`OssWorkOrder`, `PaymentsSignal`, `CheckDecision` (slice 05). Each projects committed state / emitted
events; none owns lifecycle policy.

### 3.2 Component Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-component-consumer-contracts-con`

- **`ControlPlaneApi`** — the REST business-operations surface (idempotency + ETag).
- **`ReadModelProjector`** — the rating composition read model + roll-up projections.
- **`BillingHandoff`** — the recurring `BillableItem` + traceability handoff.
- **`ContractsAdapter`** / **`PolicyGateAdapter`** / **`OssAdapter`** / **`PaymentsAdapter`** — the inbound/outbound neighbour adapters.

### 3.3 API Contracts

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-interface-boundary-con`

Owns the concrete shape of `cpt-cf-bss-subscriptions-interface-control-plane` (§9.1) and the six §9.2
external contracts (`-contract-billing-handoff`, `-contract-rating-read-model`,
`-contract-contracts-input`, `-contract-policy-gate`, `-contract-oss-provisioning`,
`-contract-payments-signals`), plus the point-of-use check contract surfaced from
[`05-entitlements.md`](./05-entitlements.md). Concrete REST/OpenAPI/error taxonomy is Design.

### 3.4 Internal Dependencies

Depends on every slice: [`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) (transition +
outbox), [`02-composition-versioning.md`](./02-composition-versioning.md) (read model),
[`03-plan-changes.md`](./03-plan-changes.md) (change events), [`04-suspension-renewal-grace.md`](./04-suspension-renewal-grace.md)
(Payments/Billing), [`05-entitlements.md`](./05-entitlements.md) (check contract),
[`06-trials.md`](./06-trials.md) (conversion event), [`07-tenancy-transfer.md`](./07-tenancy-transfer.md)
(transfer events), [`08-events-billing.md`](./08-events-billing.md) (producer inventory + handoff).

### 3.5 External Dependencies

| Dependency | Direction | Contract |
|------------|-----------|----------|
| Rating | out (read model + events) | SEAMS **SUB-R1** |
| Billing | out (recurring handoff) | SEAMS **SUB-B1** |
| Contracts | in (terms, `PriceOverride`) | SEAMS **SUB-C1**, **SUB-C5** |
| Policy Engine | out (gate) / in (decision) | SEAMS **SUB-E1** |
| OSS | out (work orders) / in (check calls, confirmations) | SEAMS **SUB-E2**, **SUB-E3** |
| Payments | in (signals) | SEAMS **SUB-F1** |
| Registry | in (overlap key, published SKU) | SEAMS **SUB-G1**, **SUB-G2** |

### 3.6 Interactions and Sequences

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-flow-boundary-con`

The boundary flows are the per-contract sequences owned by the source slices (transition commit —
slice 01; change boundary — slice 03; renewal/grace — slice 04; check — slice 05; emit — slice 08).
This slice composes them into the external surface; no new sequence beyond assembly.

### 3.7 Database Schemas and Tables

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-storage-boundary-con`

No owned store; the boundary reads the projected read models + the outbox owned by the Foundation +
capability slices. Concrete projection DDL is Design.

### 3.8 Deployment Topology

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-deployment-con`

The control-plane + read surfaces run behind the inbound gateway; read models are projected off the
commit path. **Failure postures are per surface (2026-07-15 review fix):** only the entitlement
**check** surface carries the bounded-staleness/fail-closed posture (SUB-D-10, slice 05); the
composition read model and roll-ups serve committed state under normal eventual consistency
([`../PRD.md`](../PRD.md) §14) — a projection outage there degrades freshness, it does not block
rating/Billing reads of already-served state ([`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md)
§3.8).

## 4. Additional Context

### 4.1 Control-Plane Operations (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-control-plane-con`

- The business operations (create/get/list; activate/suspend/resume/cancel incl. void-from-draft; changePlan; add-on; `updateQuantity`; `convertTrial`/`extendTrial`; manual `renew`; `unschedule`; `pauseCollection`/`resumeCollection`; `confirmAcceptance`; `transfer`; entitlement issue/revoke — the full PRD §9.1 table incl. the SUB-D-08 set) carry idempotency keys + optimistic concurrency on `version`; resources are `Subscription`, `Entitlement`, `AddOn`, `PlanLink`, `TransitionRequest`, `Approval` ([`../PRD.md`](../PRD.md) §9.1). REST paths/methods/header bindings/OpenAPI are Design.

### 4.2 Rating Read-Model Contract (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-rating-contract-con`

- Composition read models expose effective `PlanLink`/`AddOn` intervals, `PlanTier` @ `t`, active plan phase @ `t`, the plan-change `(changeEffectiveAt, changeMode)`, the **committed seat quantity @ `t`** (effective-dated — pricing `quantitySource = subscription_seat_count`), the **`priceEligibility` inputs** (`activatedAt`, bound `cohort`), and the per-sale **`brandId`** context (source discrepancy with rating open — SEAMS **SUB-R5**); ordering is shared on the pinned `(orderingTenantId, subscriptionId)` (SUB-D-06) — rating PRD §9.2 "Subscriptions input contract" is the counterpart, and the two field lists MUST stay mirror-aligned (SEAMS **SUB-R1**, downgraded to Joint until reconciled) ([`../PRD.md`](../PRD.md) §9.2).

### 4.3 Billing Handoff Contract (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-billing-contract-con`

- `BillableItemCreated(kind=recurring)` — the **money-free period fact** idempotent per `(subscriptionId, billing period)`, carrying `{subscriptionId, skuId, planId, priceId}` + `pricingSnapshotRef` + the pause/intent posture; the rating gear prices it and the priced line inherits the key before Billing posts (SUB-D-07; SEAMS **SUB-R6**); proration materialises only as new billable/adjusting artifacts; posted invoices immutable ([`../PRD.md`](../PRD.md) §9.2, §6.8; SEAMS **SUB-B1**). Billing additionally exposes the **`billedThroughAt`** watermark this gear's backdating guard consumes (SEAMS **SUB-B6**).

### 4.4 Contracts Input Contract (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-contracts-contract-con`

- Signed terms, `Renewal` (`autoRenew`, term windows), grace ladder / regional-template values, `PriceOverride` windows — consumed via events (`ContractSigned`, `ContractRenewed`, …) + read models; Subscriptions stores **evaluated fields** at renewal-evaluation time ([`../PRD.md`](../PRD.md) §9.2, §6.5; SEAMS **SUB-C1**, **SUB-C5**).
- **Risk:** the upstream Contracts PRD does not yet author the grace/regional-template SoR — until it does, the platform defaults govern (slice 04 §4.4).

### 4.5 Policy and OSS Contracts (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-policy-oss-contract-con`

- Policy: pre-commit allow/deny + `reasonCodes` for every resource-affecting transition; fail-closed on deny or unavailability; post-change confirmations per integration Design ([`../PRD.md`](../PRD.md) §9.2; SEAMS **SUB-E1**).
- OSS: provision/deprovision/pause work orders confirmed by events; entitlement issue/revoke aligned to committed transitions; BSS never mutates OSS topology directly ([`../PRD.md`](../PRD.md) §9.2; SEAMS **SUB-E2**). The point-of-use check contract (slice 05, p95 < 100ms) is OSS's enforcement input (**SUB-E3**).

### 4.6 Payments and Notifications Contracts (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-payments-notif-contract-con`

- Payments: pre-check outcomes + retry-exhaustion declarations consumed by the renewal/grace ladder; PSP webhooks + dunning handoff payloads are Design scope ([`../PRD.md`](../PRD.md) §9.2; SEAMS **SUB-F1**).
- Notifications: notice + win-back **triggers** are owned here; **delivery** is Notifications/Comms (SEAMS **SUB-F2**).

### 4.7 Registry Overlap-Key Contract (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-registry-contract-con`

- `catalogSubscriptionProductKey` (the `overlapScopeKey` half) is registry-owned; Design binds the stored field to a published SKU/product key; published `skuId`/`PlanTier`/`CatalogVersion` are read-only inputs ([`../PRD.md`](../PRD.md) §6.3; SEAMS **SUB-G1**, **SUB-G2**). Engage upstream PR #4177 before the key shape is frozen.

## 5. Traceability

- **PRD**: [`../PRD.md`](../PRD.md) §9.1 (`interface-control-plane`), §9.2 (`contract-billing-handoff`, `contract-rating-read-model`, `contract-contracts-input`, `contract-policy-gate`, `contract-oss-provisioning`, `contract-payments-signals`), §6 content boundary, §7.1 (NFRs), §16 (Contracts-grace risk).
- **Seams**: **SUB-R1**, **SUB-B1**, **SUB-C1**, **SUB-C5**, **SUB-E1**, **SUB-E2**, **SUB-E3**, **SUB-F1**, **SUB-F2**, **SUB-G1**, **SUB-G2** — [`../SEAMS.md`](../SEAMS.md).
- **Slices**: all — this is the assembled boundary surface over [`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) … [`08-events-billing.md`](./08-events-billing.md).
