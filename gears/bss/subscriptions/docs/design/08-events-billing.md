<!-- CONFLUENCE_TITLE: [BSS]: Subscriptions — Event Model & Billing Alignment (Design) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../SEAMS.md | Upstream: Contracts (PriceOverride) | Downstream: Rating, Billing, OSS, Policy Engine, Analytics | Owners: BSS Subscriptions team -->

# DESIGN — Event Model & Billing Alignment (Slice 8)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-design-events-billing`

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
  - [4.1 Producer Inventory and Payload Sufficiency (normative)](#41-producer-inventory-and-payload-sufficiency-normative)
  - [4.2 Ordering (normative)](#42-ordering-normative)
  - [4.3 Recurring Idempotency and No Retro-Edit (normative)](#43-recurring-idempotency-and-no-retro-edit-normative)
  - [4.4 Traceability (normative)](#44-traceability-normative)
  - [4.5 Dataset Separation and PriceOverride Consumption (normative)](#45-dataset-separation-and-priceoverride-consumption-normative)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

This slice is the **outbound integration substrate**: the lifecycle **event producer inventory**, the
ordering guarantee, the recurring `BillableItem` idempotency + no-retro-edit invariants, and the
charge-to-catalog traceability tuple. The PRD fixes **sufficiency, not schema** — every lifecycle
event carries enough identity/tenancy/correlation/time context to route, deduplicate, and replay;
composition-changing events additionally carry enough snapshot-oriented commercial context for rating
and Billing to stay aligned and idempotent ([`../PRD.md`](../PRD.md) §6.7, §6.8). **This slice owns
the event naming registry and the field matrix** (§4.1, SUB-D-09);
[`09-consumer-contracts.md`](./09-consumer-contracts.md) owns only the wire mappings.

Two seams meet here: **SUB-B1** (recurring idempotency `(subscriptionId, billing period)`,
posted-invoice immutability, `{subscriptionId, skuId, planId, priceId}` + `pricingSnapshotRef`
traceability) and the ordering half of **SUB-R1** (the pinned `(orderingTenantId, subscriptionId)` key shared
with rating); it also consumes **SUB-C5** (Contract `PriceOverride` windows).

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-subscriptions-fr-event-producers` | The frozen producer set (`SubscriptionCreated`/`Activated`/`Suspended`/`Resumed`/`Cancelled`/`PlanChanged`, `BillableItemCreated(recurring)`, `EntitlementIssued`/`Revoked`, `OwnershipTransfer*`) emitted via the outbox (§4.1). |
| `cpt-cf-bss-subscriptions-fr-event-payload-completeness` | Sufficiency rule (not schema): identity/tenancy/correlation/time for route/dedup/replay; snapshot-oriented commercial context on composition-changing events (§4.1). |
| `cpt-cf-bss-subscriptions-fr-event-ordering` | Ordered within the pinned `(orderingTenantId, subscriptionId)` (SUB-D-06) — the key shared with rating partition ordering (§4.2). |
| `cpt-cf-bss-subscriptions-fr-recurring-idempotency` / `cpt-cf-bss-subscriptions-fr-no-retro-edit` | `BillableItem(kind=recurring)` idempotent per `(subscriptionId, billing period)`; posted lines never rewritten — corrections are new billable/adjustment artifacts (§4.3). |
| `cpt-cf-bss-subscriptions-fr-billing-traceability` / `cpt-cf-bss-subscriptions-fr-dataset-separation` | Every item traces to `{subscriptionId, skuId, planId, priceId}` + `pricingSnapshotRef`; subscription state ≠ posted invoice state (§4.4, §4.5). |

#### NFR Allocation

| NFR theme | Allocated To | Design Response | Verification / Status |
|-----------|--------------|-----------------|-----------------------|
| `cpt-cf-bss-subscriptions-nfr-operational-baselines` | Event outbox | Event delivery to consumers p95 < 30s; at-least-once + dedupable | Load test; baseline (workshop-pending) |
| `cpt-cf-bss-subscriptions-nfr-recurring-cut` | Recurring emission | Daily generation cut; zero duplicates via the idempotency key | Reconciliation §17.1 |

#### Key ADRs

No slice-local ADR; the ordering key + recurring idempotency are manifest invariants shared with
rating/Billing (SEAMS **SUB-R1**, **SUB-B1**).

### 1.3 Architecture Layers

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-tech-stack-evt`

| Layer | Responsibility | Technology |
|-------|----------------|------------|
| Application | Producer emission + payload assembly; recurring idempotency keying; traceability stamping | Rust module in the `subscriptions` gear |
| Domain | Event envelopes, `BillableItem` recurring key, traceability tuple | Rust; GTS + Rust domain structs |
| Infrastructure | Transactional event outbox (committed with the transition) | PostgreSQL, SecureORM |

## 2. Principles and Constraints

### 2.1 Design Principles

#### Sufficiency, not schema

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-sufficiency-evt`

The PRD contract is payload **sufficiency** (route/dedup/replay; snapshot-oriented context on
composition changes); the field matrix + wire format are Design, owned with the consumer contracts
([`../PRD.md`](../PRD.md) §6.7).

#### One ordering key

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-one-ordering-key-evt`

Commit + emission preserve order within the pinned `(orderingTenantId, subscriptionId)` (SUB-D-06) so rating consumes
composition changes without reorder hazards ([`../PRD.md`](../PRD.md) §6.7; SEAMS **SUB-R1**).

### 2.2 Constraints

#### Recurring is idempotent; posted is immutable

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-recurring-idempotent-evt`

At most one recurring `BillableItem` per `(subscriptionId, billing period)` even under bill-run
retries; posted invoice lines are never rewritten ([`../PRD.md`](../PRD.md) §6.8 AC 5).

#### Outbox is committed with the transition

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-transactional-outbox-evt`

Events are written to the outbox **in the same commit** as the state change (slice 01) — no event
without a committed transition, no committed transition without its events.

## 3. Technical Architecture

### 3.1 Domain Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-domain-model-evt`

- **`LifecycleEvent`** — the producer envelope (type, identity, tenancy, correlation, time, sequence); CloudEvents 1.0, tenant-scoped, minimal PII.
- **`RecurringBillableItem`** — the recurring handoff keyed `(subscriptionId, billing period)` with the traceability tuple + `pricingSnapshotRef`.
- **`TraceabilityTuple`** — `{subscriptionId, skuId, planId, priceId}` + `pricingSnapshotRef`.

### 3.2 Component Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-component-events-evt`

- **`EventPublisher`** — assembles the sufficient payload + writes to the outbox in the transition commit.
- **`OrderingSequencer`** (Foundation) — enforces per-`(orderingTenantId, subscriptionId)` order on emission (the pinned tenant, SUB-D-06).
- **`RecurringEmitter`** — cuts the **money-free recurring period facts** idempotent per `(subscriptionId, billing period)` with the traceability tuple + pause/intent posture; rating prices them (SUB-D-07, §4.3).

### 3.3 API Contracts

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-interface-events-evt`

The producer inventory + the recurring `BillableItemCreated` handoff are the outbound contract; the
concrete event field matrix, CloudEvents extensions, and the Billing handoff payload are owned by
[`09-consumer-contracts.md`](./09-consumer-contracts.md). This slice fixes the producer set + the
sufficiency/ordering/idempotency/traceability rules.

### 3.4 Internal Dependencies

Depends on [`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) (transactional outbox,
`OrderingSequencer`); every capability slice (02–07) produces the events emitted here.

### 3.5 External Dependencies

| Dependency | What crosses the boundary | Contract |
|------------|---------------------------|----------|
| Rating | Consumes composition-changing events on the shared ordering key | SEAMS **SUB-R1** |
| Billing | Consumes recurring `BillableItem`s; posts immutable invoices | SEAMS **SUB-B1** |
| OSS / Policy / Analytics | Consume lifecycle facts + confirmations | [`../PRD.md`](../PRD.md) §6.7 |
| Contracts | `PriceOverride` windows consumed into composition/renewal | SEAMS **SUB-C5** |

### 3.6 Interactions and Sequences

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-flow-emit-evt`

**Emit**: on a committed transition, `EventPublisher` writes the sufficient-payload event(s) to the
outbox in the same commit; `OrderingSequencer` assigns the per-aggregate sequence; the outbox delivers
at-least-once, dedupable, in order within the pinned `(orderingTenantId, subscriptionId)`. `RecurringEmitter`
cuts the recurring `BillableItem` idempotent per `(subscriptionId, billing period)` with the
traceability tuple + `pricingSnapshotRef`.

### 3.7 Database Schemas and Tables

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-storage-events-evt`

Uses the Foundation `event_outbox` (committed with the transition); the recurring idempotency key
`(subscriptionId, billing period)` is a unique index on the recurring handoff record. No separate
owned store. Concrete DDL is Design.

### 3.8 Deployment Topology

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-deployment-evt`

Outbox delivery + the recurring-generation cut run as coordinated singletons **per tenant partition**
(one lease per `orderingTenantId` shard, shard-parallel — the same sharding rule as slice 04's jobs),
with the same **intra-tenant sub-sharding by hash of `subscriptionId`** (slice 04 §3.8) so a single
large tenant's daily 00:00 cut is not serialised through one worker, so the cut and the p95 < 30s
delivery target are not funnelled through one global instance
([`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) §3.8).

## 4. Additional Context

### 4.1 Producer Inventory and Payload Sufficiency (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-producers-evt`

- Subscriptions emits (CloudEvents 1.0, tenant-scoped, minimal PII): `SubscriptionCreated`, `SubscriptionActivated`, `SubscriptionSuspended`, `SubscriptionResumed`, `SubscriptionCancelled`, `SubscriptionPlanChanged`, `BillableItemCreated(kind=recurring)`, `EntitlementIssued`, `EntitlementRevoked`, `OwnershipTransferRequested`/`Approved`/`Completed` ([`../PRD.md`](../PRD.md) §6.7).
- **Secondary producer set (SUB-D-09 — naming is normative here; this closes the PRD's "naming per Design" obligations):**

  | Event | Source / AC |
  |-------|-------------|
  | `SubscriptionIntentScheduled` / `SubscriptionIntentUnscheduled` | Scheduled intents, slice 01 §4.3 (AC 22); un-schedule voids a previously announced boundary (slice 03 event-once convention) |
  | `SubscriptionQuantityChanged` | The composition-changing quantity event, slice 03 (AC 23; consumed like `SubscriptionPlanChanged`) |
  | `SubscriptionRenewalSucceeded` / `SubscriptionRenewalFailed` | Renewal job outcome, slice 04 §4.3 (AC 7) |
  | `SubscriptionGraceEntered` / `SubscriptionGraceExited` | Grace ladder, slice 04 §4.4 (AC 7; exit carries the resolution: renewed / suspended / cancelled) |
  | `SubscriptionRenewalNoticeDue` | Notice trigger, slice 04 §4.5 (AC 19; delivery = Notifications) |
  | `SubscriptionCollectionPaused` / `SubscriptionCollectionResumed` | Pause window, slice 04 §4.2 (AC 24) |
  | `SubscriptionTrialConverted` / `SubscriptionTrialExtended` / `SubscriptionTrialExpired` | Trials, slice 06 (AC 16–18; `TrialExpired` doubles as the win-back hook; `TrialExtended` carries the moved boundary on the shared channel) |
  | `SubscriptionAcceptanceConfirmed` | `confirmAcceptance`, slice 01 §4.4 (AC 25) |
  | `EntitlementQuotaWarning` / `EntitlementQuotaExhausted` / `EntitlementQuotaRestored` | Quota crossings, slice 05 §4.4 (AC 14) |

- **Sufficiency, not schema**: every lifecycle event carries enough identity/tenancy/correlation/time for route/dedup/replay **without** an undocumented side channel; composition-changing events carry enough snapshot-oriented commercial context that rating + Billing stay aligned on the effective offer and process idempotently (AC 11). The **field matrix is owned by this slice together with §4.1's registry** (per-event required-context groups: identity, tenancy incl. the pinned `orderingTenantId`, correlation, time, commercial snapshot context for composition changes); slice 09 owns only the wire mappings (SUB-D-09 closes the earlier circular deferral).

### 4.2 Ordering (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-ordering-evt`

- Order MUST be preserved within `(tenantId, aggregateId)` with `aggregateId = subscriptionId`; `tenantId` = the **pinned `orderingTenantId`** (= `resourceTenantId` at creation, immutable across transfers — SUB-D-06, AC 26) so subscription command ordering + downstream rating partition ordering share one **stable** key ([`../PRD.md`](../PRD.md) §6.7 AC 3; SEAMS **SUB-R1**).

### 4.3 Recurring Idempotency and No Retro-Edit (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-recurring-evt`

- `BillableItem(kind=recurring)` MUST be idempotent on `(subscriptionId, billing period)` — at most one recurring item per key even under bill-run retries ([`../PRD.md`](../PRD.md) §6.8 AC 5).
- **What the emitter cuts is the money-free period fact (SUB-D-07):** period identity from the billing anchor, the traceability tuple, `pricingSnapshotRef`, and the pause/intent posture — **no monetary column** (the Foundation store has none). The **rating gear prices** the recurring component from the frozen snapshot and the priced line **inherits the fact's key** before Billing posts (AC 27; SEAMS **SUB-R6**). This removes the double-producer collision with rating's recurring lines.
- **Pause marker:** during a `collectionPaused` window the fact is still emitted, marked, so Billing owns the suppress-vs-defer treatment (SUB-D-03/12, AC 24 — "not posted" is Billing's act, emission is ours).
- **Period-key stability:** the period identity is the anchor-derived canonical id frozen at emission; a cycle-length change (e.g. monthly→annual at a boundary) starts a **new period sequence** at that boundary — no retroactive re-keying of already-cut facts.
- **Cut-vs-intent race:** the daily cut reads the pending-intent set as of its run; an `unschedule` committed after the cut suppressed a period re-triggers a targeted re-cut via `SubscriptionIntentUnscheduled` (idempotent on the same key), with the §17.1 charge-coverage reconciliation as the backstop.
- Posted invoice lines MUST NOT be rewritten; subscription corrections emit **new** billable or adjustment paths ([`../PRD.md`](../PRD.md) §6.8; SEAMS **SUB-B1**).

### 4.4 Traceability (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-traceability-evt`

- Items MUST trace to `subscriptionId`, `skuId`/`planId`/`priceId`, and `pricingSnapshotRef` (manifest §4.4 itemization) — the charge-to-catalog lineage partners + auditors reconcile against ([`../PRD.md`](../PRD.md) §6.8).

### 4.5 Dataset Separation and PriceOverride Consumption (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-dataset-priceoverride-evt`

- Subscription state ≠ invoice posted state; late usage adjustments remain a Rating→Billing concern; this slice states **Billing invariants** for coordinated artifacts, not client control-plane operations (REST paths/methods/errors are Design) ([`../PRD.md`](../PRD.md) §6.8).
- Contract `PriceOverride` windows are consumed into composition/renewal via events/read models; in rating these are the step-5 contract overlay — Subscriptions references the binding, never evaluates the override (SEAMS **SUB-C5**).

## 5. Traceability

- **PRD**: [`../PRD.md`](../PRD.md) §6.7 (`fr-event-producers`, `fr-event-payload-completeness`, `fr-event-consumers`, `fr-event-ordering`), §6.8 (`fr-recurring-idempotency`, `fr-no-retro-edit`, `fr-billing-traceability`, `fr-dataset-separation`), AC 3/5/11, §7.1 (delivery NFR).
- **Seams**: **SUB-B1** (recurring/immutability/traceability), **SUB-R1** (ordering), **SUB-C5** (PriceOverride) — [`../SEAMS.md`](../SEAMS.md).
- **Slices**: [`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) (outbox, sequencer), capability slices 02–07 (event sources), [`09-consumer-contracts.md`](./09-consumer-contracts.md) (payload + Billing handoff contract).
