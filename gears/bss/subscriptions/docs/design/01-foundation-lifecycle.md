<!-- CONFLUENCE_TITLE: [BSS]: Subscriptions — Lifecycle Foundation (Design) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../SEAMS.md, ../DECISIONS.md | Upstream: Policy Engine (fail-closed gate), OSS Provisioning (work-order confirm), AMS (tenant identity) | Downstream: every capability slice, Rating, Billing | Owners: BSS Subscriptions team -->

# DESIGN — Lifecycle Foundation (Slice 1)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-design-foundation`

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
  - [4.1 Closed Status Machine and Terminality (normative)](#41-closed-status-machine-and-terminality-normative)
  - [4.2 TransitionRequest Envelope, Idempotency, Ordering (normative)](#42-transitionrequest-envelope-idempotency-ordering-normative)
  - [4.3 Scheduled Intents on the Aggregate (normative)](#43-scheduled-intents-on-the-aggregate-normative)
  - [4.4 Activation Instants Trio (normative)](#44-activation-instants-trio-normative)
  - [4.5 Fail-Closed Policy and OSS Gate (normative)](#45-fail-closed-policy-and-oss-gate-normative)
  - [4.6 Manifest Alignment of New Transition Types (normative)](#46-manifest-alignment-of-new-transition-types-normative)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

This slice is the **shared substrate** every capability slice runs *through*: the `Subscription`
aggregate, the **closed manifest status machine**, and the single **transition commit path** that
takes a `TransitionRequest` from validation to a durable, idempotent, versioned, audited, evented
state change. No capability slice commits state itself — renewal (slice 04), entitlements (05), and
trials (06) express *what* the change is and *when*; the Foundation owns *how it commits* so the
correctness-critical guarantees (idempotency, ordering, the fail-closed Policy/OSS gate, terminality)
live in one auditable place ([`../PRD.md`](../PRD.md) §6.1).

Two invariants shape the slice. First, **the status enum is closed and terminal**: manifest §4.3
lists exactly `draft | active | suspended | cancelled | archived`, and this gear never adds a state
— trials, the billing-only pause, and scheduled intents are **attributes, postures, and
pending-intents** on that enum, never new statuses ([`../PRD.md`](../PRD.md) §6.1;
[`../DECISIONS.md`](../DECISIONS.md) SUB-D-03/05). Second, **every resource-affecting transition is
fail-closed**: it passes the Policy Engine pre-check before commit, and on deny *or* Policy
unavailability the state does not change ([`../PRD.md`](../PRD.md) §6.1 AC 1; SEAMS **SUB-E1**). The
slice owns the seams **SUB-E1** (the gate), **SUB-C4** (the activation date-trio), and **SUB-N1**
(the manifest alignment of the new transition types).

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-subscriptions-fr-status-enum` / `cpt-cf-bss-subscriptions-fr-transitions-guards` | A `StatusMachine` with the closed enum, the seven normative edges (incl. the `resume` reverse edge and the `draft→cancelled` void, SUB-D-11), terminality on `cancelled`/`archived`, and a guard per edge; an edge not in the table is rejected, never inferred (§4.1). |
| `cpt-cf-bss-subscriptions-fr-transition-request` | A uniform `TransitionRequest` envelope (`type`, `idempotencyKey`, `status ∈ {pending, approved, applied, failed}`) is the *only* mutation shape; the `TransitionEngine` is the single commit path (§4.2). |
| `cpt-cf-bss-subscriptions-fr-scheduled-intents` | Non-immediate `cancelMode`/`resumeAt` intents are `ScheduledIntent` rows **on the aggregate**, visible to the renewal job, un-schedulable until effective, evented both ways (§4.3; SUB-D-01). |
| `cpt-cf-bss-subscriptions-fr-activation-instants` | Three instants — `contractEffectiveAt` (from Contract), `serviceActivatedAt` (stamped at commit), `customerAcceptedAt` (acceptance confirmation) — as aggregate attributes; no interim statuses (§4.4; SUB-D-05, SUB-C4). |
| `cpt-cf-bss-subscriptions-fr-monotonic-version` | Each committed commercial-meaning change increments `version` and appends an immutable `SubscriptionRevision`; optimistic concurrency rejects a stale submit (§3.1, §3.7). |
| `cpt-cf-bss-subscriptions-fr-event-ordering` | Commit and emission are sequenced per the pinned `(orderingTenantId, subscriptionId)` (SUB-D-06) — the key shared with rating partition ordering (§4.2; SUB-R1). |
| `cpt-cf-bss-subscriptions-fr-trials-not-a-status` | The closed enum is enforced structurally: a `trial`/`pending` value is unrepresentable; trials ride phase attributes (slice 06) on a manifest status (§4.1). |

#### NFR Allocation

| NFR theme | Allocated To | Design Response | Verification / Status |
|-----------|--------------|-----------------|-----------------------|
| `cpt-cf-bss-subscriptions-nfr-lifecycle-latency` | `TransitionEngine` commit path | Synchronous commit class (p95 < 1s) = validate + idempotency + guard + Policy pre-check + single versioned write; **OSS-blocking edges are excluded** — they run async (`pending → approved → applied`, §3.6 async note), the sub-second bound applies to the intent commit | Load test; baseline (workshop-pending [`../PRD.md`](../PRD.md) §7.1, §14) |
| `cpt-cf-bss-subscriptions-nfr-operational-baselines` | `StatusMachine`, `IdempotencyRegistry` | State-transition p95 < 500ms is the same commit path **excluding the external Policy round-trip** (the 1s class includes it); exactly-one durable effect per `(subscriptionId, idempotencyKey)` under retry | Idempotency + concurrency fixtures |
| `cpt-cf-bss-subscriptions-nfr-horizontal-partitioning` | Aggregate store + `OrderingSequencer` | Partition by the pinned `orderingTenantId` (SUB-D-06 — stable across transfers, no row migration); per-aggregate ordering with no cross-partition lock on the commit path | Design + load test |

#### Key ADRs

| ADR ID | Decision Summary |
|--------|------------------|
| [`../ADR/0001`](../ADR/0001-cpt-cf-bss-subscriptions-adr-manifest-closed-status-machine.md) `cpt-cf-bss-subscriptions-adr-manifest-closed-status-machine` | Trials, billing-only pause, and scheduled intents are attributes/postures/pending-intents on the closed manifest enum — no new status (§4.1). |
| [`../ADR/0003`](../ADR/0003-cpt-cf-bss-subscriptions-adr-scheduled-intents-on-aggregate.md) `cpt-cf-bss-subscriptions-adr-scheduled-intents-on-aggregate` | Scheduled cancel/resume/ramp intents live on the aggregate so the renewal job, Billing, and audit can see them — not as portal-side automation (§4.3). |

### 1.3 Architecture Layers

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-tech-stack-fnd`

```text
Control plane (REST)        create / activate / suspend / resume / cancel / changePlan / ...
        │  (idempotency key + ETag/version)   → TransitionRequest
        ▼
TransitionEngine (this slice)   Validate → IdempotencyRegistry → (Approval hold) →
        │                       StatusMachine guard → version check → PolicyGate (fail-closed) →
        │                       OssCoordinator (confirm; async edges) →
        │                       commit + VersionStamper + AuditWriter → OrderingSequencer → outbox
        ▼
Aggregate store + read models   Subscription + SubscriptionRevision (append-only) ·
                                TransitionRequest · ScheduledIntent · Approval · audit · outbox
```

| Layer | Responsibility | Technology |
|-------|----------------|------------|
| Presentation | REST control-plane behind the inbound gateway; RFC 9457 problems; OAuth 2.0; idempotency key + ETag optimistic concurrency (mappings in slice 09 / Design) | Rust, REST/OpenAPI, inbound API gateway |
| Application | The `TransitionEngine` commit path + the capability handlers that invoke it | Rust modules in the `subscriptions` gear |
| Domain | The `Subscription` aggregate, `StatusMachine`, `TransitionRequest`/`ScheduledIntent`/`Approval`, versioning, activation instants | Rust; GTS + Rust domain structs |
| Infrastructure | Aggregate + append-only revision history, projected read models, audit store, event outbox | PostgreSQL, SecureORM |

## 2. Principles and Constraints

### 2.1 Design Principles

#### Single commit path

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-single-commit-path-fnd`

Every state change reaches the store one way: `TransitionRequest` → `TransitionEngine` (validate →
guard → gate → OSS confirm → commit + version + audit + emit). There is no side door that mutates a
committed subscription ([`../PRD.md`](../PRD.md) §6.1).

#### Closed enum, enforced structurally

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-closed-enum-fnd`

`Subscription.status` is a closed domain type; a value outside `{draft, active, suspended,
cancelled, archived}` is unrepresentable. Trial/pause/intent state is modelled as attributes and
pending intents, never as a status ([`../PRD.md`](../PRD.md) §6.1; [`../DECISIONS.md`](../DECISIONS.md)
SUB-D-03/05).

#### Idempotent and ordered

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-idempotent-ordered-fnd`

A duplicate `(subscriptionId, idempotencyKey)` yields exactly one durable effect and replays the
original outcome; commit and emission preserve order within the pinned `(orderingTenantId, subscriptionId)`
([`../PRD.md`](../PRD.md) §6.7, §6.8).

### 2.2 Constraints

#### Fail-closed gate is mandatory

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-fail-closed-fnd`

A resource-affecting transition that cannot obtain a Policy **allow** — deny or Policy unavailable —
does not commit; the state is unchanged and the attempt is audited ([`../PRD.md`](../PRD.md) §6.1 AC
1; SEAMS **SUB-E1**).

#### Terminality is irreversible

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-terminality-fnd`

`cancelled` and `archived` are terminal; the only forward move is `cancelled → archived`. There is
no commercial rebirth — reactivating a cancelled commercial relationship is a **new subscription**
([`../PRD.md`](../PRD.md) §6.1).

#### No money, no catalog authoring

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-no-money-fnd`

The Foundation commits lifecycle state only; it computes no charge/proration/FX and authors no
catalog entity. Monetary effects are downstream (rating math, Billing posting), driven by the events
this slice emits ([`../PRD.md`](../PRD.md) §6.3, §5.2).

## 3. Technical Architecture

### 3.1 Domain Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-domain-model-fnd`

- **`Subscription`** (aggregate root) — `subscriptionId`, `status` (closed enum), `version` (monotonic), tenant axes (`resourceTenantId`, `payerTenantId`, `sellerTenantId`), `brandId` (per-sale, slice 02), the activation instants (`contractEffectiveAt`, `serviceActivatedAt`, `customerAcceptedAt`; §4.4), posture flags (`collectionPaused` window — attribute here, semantics slice 04), and references to its composition (slice 02), pending intents, and entitlements (slice 05).
- **`TransitionRequest`** — `id`, `subscriptionId`, `type ∈ {activate, suspend, resume, cancel, archive, changePlan, addAddOn, removeAddOn, updateQuantity, convertTrial, transfer, renew, unschedule, pauseCollection, resumeCollection, confirmAcceptance, extendTrial}` (SUB-D-08 completes the set so every FR-mandated mutation has a type; `archive` is the `cancelled→archived` edge operation), `idempotencyKey`, `status ∈ {pending, approved, applied, failed}`, the change envelope (`changeEffectiveAt`, `changeMode` / `cancelMode`, `resumeAt`), `correlationId`, actor + delegation-proof reference.
- **`ScheduledIntent`** — a pending non-immediate intent on the aggregate: `kind` (cancel/resume/ramp step), `effectiveAt`, source `TransitionRequest`, `unscheduledAt?`; suppresses renewal / next-term recurring while pending (§4.3).
- **`Approval`** — the maker-checker record required for high-risk types (`transfer`; trial extension slice 06): submitter, approver(s), decision, evidence.
- **`SubscriptionRevision`** — an append-only immutable snapshot per committed version (audit + replay lineage).
- **`AuditRecord`** — append-only, tamper-evident, correlation-keyed record of every transition attempt (incl. denied/failed).

Catalog (`Plan`/`Price`/`PriceWindow`) and registry (`skuId`/`PlanTier`/`CatalogVersion`) entities
are **resolved frozen inputs**, never owned here.

### 3.2 Component Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-component-transition-engine-fnd`

- **`TransitionEngine`** — the single commit path; orchestrates validate → guard → idempotency → gate → OSS confirm → commit → version → audit → emit.
- **`StatusMachine`** — the closed enum + edge/guard table; rejects any edge not in the table (§4.1).
- **`IdempotencyRegistry`** — dedup on `(subscriptionId, idempotencyKey)`; returns the original outcome on replay (§4.2).
- **`OrderingSequencer`** — assigns and enforces per-`(orderingTenantId, subscriptionId)` sequence (the pinned tenant, SUB-D-06) for commit + outbox emission (§4.2).
- **`IntentScheduler`** — persists/cancels `ScheduledIntent`s and surfaces them to the renewal job; fires the real transition at `effectiveAt` with the full guard set (§4.3).
- **`PolicyGate`** — the fail-closed Policy pre-check; deny/unavailable ⇒ abort (§4.5).
- **`OssCoordinator`** — issues provision/deprovision/pause work orders and blocks the commit on confirmation where the edge requires it (§4.5).
- **`VersionStamper` + `AuditWriter`** — increment `version`, append the revision + audit record atomically with the commit.

### 3.3 API Contracts

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-interface-transition-fnd`

The **internal transition contract** (in-process): input = a validated `TransitionRequest`; output =
the committed `Subscription` revision + the emitted event set, or a typed rejection
(`policy_denied`, `guard_violation`, `stale_version`, `duplicate_idempotency` → original outcome,
`oss_unconfirmed`). The **REST control-plane** surface (`cpt-cf-bss-subscriptions-interface-control-plane`,
[`../PRD.md`](../PRD.md) §9.1) — paths, methods, idempotency/ETag header bindings, RFC 9457 problem
mappings — is owned by [`09-consumer-contracts.md`](./09-consumer-contracts.md); this slice fixes the
**semantics** (which rejections exist, which are fail-closed), not the wire mapping ([`../PRD.md`](../PRD.md)
§6 content boundary).

### 3.4 Internal Dependencies

Every capability slice depends on this Foundation for its commit path. Within the slice:
`toolkit-db` for transactional persistence (aggregate + revision + audit + outbox in one commit) and
the **coordination lease library** for the singleton `IntentScheduler` firing and background work.

### 3.5 External Dependencies

| Dependency | What crosses the boundary | Contract |
|------------|---------------------------|----------|
| Policy Engine | Pre-commit allow/deny + `reasonCodes` for resource-affecting transitions | [`../PRD.md`](../PRD.md) §9.2 policy-gate; SEAMS **SUB-E1** |
| OSS Provisioning | Provision/deprovision/pause work orders confirmed by events; BSS never mutates OSS topology | [`../PRD.md`](../PRD.md) §9.2 oss-provisioning; SEAMS **SUB-E2** |
| AMS | Tenant identity + the three axes + delegation-proof backbone (referenced, never invented) | [`../PRD.md`](../PRD.md) §6.6 |
| Contracts | `contractEffectiveAt` booking instant + acceptance clauses (§4.4) | SEAMS **SUB-C4** |

### 3.6 Interactions and Sequences

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-flow-transition-commit-fnd`

**Policy-gated transition** (refines `cpt-cf-bss-subscriptions-seq-policy-gated-transition`):

1. Validate the request; resolve the caller's scope + delegation proof for cross-tenant actions (reject + audit if absent).
2. `IdempotencyRegistry` **first**: a seen `(subscriptionId, idempotencyKey)` short-circuits to the **original outcome** — before any guard or version check, so a retry of an already-applied transition can never surface as `guard_violation`/`stale_version` (2026-07-15 review fix).
3. **Approval hold** (approval-required types: `transfer`, `extendTrial`): the request parks `pending → approved` (maker-checker) before evaluation; on approval the flow continues with a **re-read** of current state; the idempotency key covers the whole envelope across the hold.
4. `StatusMachine` guard: the `(from, to)` edge must exist and its preconditions hold (else `guard_violation`).
5. Optimistic concurrency: the submitted `version` must match (else `stale_version`). System-originated firings (scheduled intents, renewal job) re-read the current `version` instead of carrying a client ETag.
6. `PolicyGate` pre-check for resource-affecting edges: deny **or** unavailable ⇒ abort with no state change (fail-closed).
7. `OssCoordinator`: where the edge provisions/deprovisions, issue the work order and coordinate confirmation (§4.5 — confirmation is event-driven; the OSS-blocking edge classes run asynchronously, see the async note below).
8. Commit atomically: new `status`/attributes + `version` increment + `SubscriptionRevision` + `AuditRecord` + entitlement issue/revoke (slice 05) + outbox events — sequenced by `OrderingSequencer`.

**Async note (OSS-blocking edges).** Edges that require OSS provisioning confirmation (`activate`, `suspend`/`resume` with resource legs, `cancel` deprovision) are **not** in the synchronous sub-second class: the `TransitionRequest` commits its *intent* synchronously (`approved`, work order issued) and the status edge commits when the confirmation event arrives; the client tracks `pending → approved → applied`. On confirmation timeout (Design timers) the request fails `oss_unconfirmed` and the work order is **idempotently cancelled**; a confirmation arriving after the timeout hits the cancelled order and is reconciled (compensating deprovision work order + audit) — no orphaned resource without a committed transition.

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-flow-scheduled-intent-fnd`

**Scheduled-intent lifecycle**: a non-immediate `cancel`/`suspend` (or ramp step) persists a
`ScheduledIntent` and emits `SubscriptionIntentScheduled`; the renewal job (slice 04) observes it (a
pending end-of-term cancel suppresses renewal + next-term recurring); an `unschedule` (SUB-D-08)
before `effectiveAt` voids it and emits `SubscriptionIntentUnscheduled`; at `effectiveAt` the
`IntentScheduler` runs the real transition through the full commit path above (§4.3). The firing is
**idempotent by derivation**: its idempotency key is derived as `intent:{scheduledIntentId}`, so a
crashed-and-retried scheduler can never double-fire; the firing re-reads the current `version`
(step 5 note) and runs the full guard set — a Policy deny at firing leaves state unchanged.

### 3.7 Database Schemas and Tables

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-storage-aggregate-fnd`

Owned here (tenant-partitioned by the pinned `orderingTenantId`, SUB-D-06 — stable across transfers, UTC): `subscription` (aggregate current
state), `subscription_revision` (append-only history; `REVOKE UPDATE, DELETE` + triggers),
`transition_request` (with the `(subscriptionId, idempotencyKey)` unique index), `scheduled_intent`,
`approval`, the append-only `audit_log`, and the `event_outbox`. Capability slices add their own
tables (composition in 02, grace evaluation in 04, entitlements in 05, trial phase state in 06,
transfer approvals in 07). Concrete DDL is Design-owned; money never lives here (no monetary column).

### 3.8 Deployment Topology

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-deployment-fnd`

A stateful control-plane service over a shared `toolkit-db`, partitioned by the pinned
`orderingTenantId` (SUB-D-06 — stable across transfers, no partition migration). The
`IntentScheduler` and other background jobs run as coordinated singletons **per tenant-partition
shard** (one lease per `orderingTenantId`, shard-parallel across partitions) via the coordination
lease library — no single global instance funnels the 100K+/tenant scale (slices 04/08 §3.8). Read models (entitlement check, roll-ups) are projected off the commit path. Deployment is
platform-standard for a BSS gear ([`../DESIGN.md`](../DESIGN.md) §3.8).

## 4. Additional Context

### 4.1 Closed Status Machine and Terminality (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-status-machine-fnd`

- The enum is exactly `draft | active | suspended | cancelled | archived` (manifest §4.3); the edges are `draft→active` (activate), `draft→cancelled` (void — SUB-D-11; not resource-affecting, `cancelMode = immediate` only, no OSS leg), `active→suspended` (suspend), `suspended→active` (resume; a grace-driven suspension additionally requires the blocking payment failure resolved — PRD §6.5), `active→cancelled` / `suspended→cancelled` (cancel), `cancelled→archived` (archive) ([`../PRD.md`](../PRD.md) §6.1). An edge not in the table is rejected — no silent transition.
- `cancelled` and `archived` are terminal; `cancelled→archived` is the only forward move; there is no reverse from either. Reactivation of a cancelled relationship is a new subscription.
- **Archive trigger (2026-07-15 review fix):** `archive` is the operation on the `cancelled→archived` edge — a `TransitionRequest` of type `archive` (not resource-affecting, no OSS leg), submitted either by an operator or by a **retention job** that archives `cancelled` subscriptions past a configurable dwell TTL (Product knob, §15 — same family as the SUB-D-11 draft-void TTL). Archival is a data-lifecycle move (read-model demotion / cold-storage eligibility), never a commercial state change, and rides the audit trail like any transition. `archive` joins the §4.2 `type` list under manifest alignment (§4.6, SUB-N1) — it was missed by the SUB-D-08 completion set even though the edge was already named here.
- Trials, `collectionPaused`, and scheduled intents are attributes/postures/pending-intents on this enum; adding a `trial`/`pending`/`paused` status requires a **manifest change first** ([`../DECISIONS.md`](../DECISIONS.md) SUB-D-03/05).

### 4.2 TransitionRequest Envelope, Idempotency, Ordering (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-envelope-fnd`

- All mutations are `TransitionRequest`s; `type ∈ {activate, suspend, resume, cancel, archive, changePlan, addAddOn, removeAddOn, updateQuantity, convertTrial, transfer, renew, unschedule, pauseCollection, resumeCollection, confirmAcceptance, extendTrial}` (all non-manifest types + the scheduled envelope pending manifest alignment, §4.6; SUB-D-08). High-risk types (`transfer`, `extendTrial`) require an `Approval` ([`../PRD.md`](../PRD.md) §6.1).
- **Idempotency**: exactly one durable effect per `(subscriptionId, idempotencyKey)`; a replay returns the original outcome, never a second effect — the registry lookup runs **before** guard and version evaluation (§3.6 step 2) ([`../PRD.md`](../PRD.md) §6.1, §6.8 AC 5).
- **Ordering**: commit + outbox emission are sequenced within `(orderingTenantId, subscriptionId)` — `aggregateId = subscriptionId`, and the ordering tenant is the **pinned-at-creation** `orderingTenantId` (= `resourceTenantId` at creation, immutable across transfers — SUB-D-06) so subscription command ordering and downstream rating partition ordering share one stable key ([`../PRD.md`](../PRD.md) §6.7; SEAMS **SUB-R1**).
- **Concurrency**: optimistic on `version`; a stale submit is rejected (ETag mapping in slice 09); system-originated firings re-read `version` instead of carrying one.

### 4.3 Scheduled Intents on the Aggregate (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-scheduled-intents-fnd`

- `cancel` accepts `cancelMode ∈ {immediate, end_of_term, at(date)}`; `suspend` MAY carry `resumeAt` ([`../PRD.md`](../PRD.md) §6.1; SUB-D-01).
- A non-immediate intent is a `ScheduledIntent` **on the aggregate**: visible to the renewal job (a pending end-of-term cancel MUST suppress renewal attempts + next-term recurring), auditable, and **un-schedulable** until effective; scheduling and un-scheduling both emit events.
- At `effectiveAt` the real transition runs through the full §3.6 guard set — a scheduled intent is not a pre-authorised bypass of Policy/OSS.
- Ramp steps (slice 03, SUB-D-04) are `ScheduledIntent`s of `changePlan`/`updateQuantity` kind; the Foundation provides the scheduling mechanism, Contracts authors the schedule.

### 4.4 Activation Instants Trio (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-activation-instants-fnd`

- The aggregate records three instants: `contractEffectiveAt` (booking; **referenced from the Contract** — booking semantics stay Contracts/Finance SoR), `serviceActivatedAt` (stamped at the `activate` commit), `customerAcceptedAt` (stamped by an optional **acceptance-confirmation** operation where Contract clauses require it; absent clauses ⇒ equals service activation) ([`../PRD.md`](../PRD.md) §6.1; SUB-D-05, SEAMS **SUB-C4**).
- **No interim statuses**: pending-activation / pending-acceptance are rejected — `draft` covers the pre-activation window.
- All three ride the lifecycle events + ASC input hooks; recognition semantics stay Finance/Billing. The trio is **not** a `pricingSnapshotRef` segment (SEAMS **SUB-R2**) — it travels on events/read-models.
- **Open**: the acceptance-confirmation flow shape (who confirms, evidence) is Design ([`../PRD.md`](../PRD.md) §15).

### 4.5 Fail-Closed Policy and OSS Gate (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-gate-fnd`

- Every **resource-affecting** transition (`activate`, `suspend`, `resume`, `cancel`, Policy-gated plan/add-on/quantity changes) passes the Policy pre-check before commit; **deny or Policy unavailability ⇒ no state change** + audited attempt ([`../PRD.md`](../PRD.md) §6.1 AC 1; SEAMS **SUB-E1**).
- OSS provisioning is coordinated via work orders **confirmed by events**; BSS never mutates OSS resource topology directly ([`../PRD.md`](../PRD.md) §6.4; SEAMS **SUB-E2**). Edges requiring provisioning confirmation run **asynchronously** (§3.6 async note): the intent commits (`approved`, work order issued), the status edge commits on confirmation; on timeout the request fails `oss_unconfirmed` and the work order is idempotently cancelled; a late confirmation is reconciled with a compensating deprovision + audit — never an orphaned resource.
- The gate is the same for a scheduled intent firing at its `effectiveAt` (§4.3) — no pre-authorisation bypass.

### 4.6 Manifest Alignment of New Transition Types (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-manifest-alignment-fnd`

- `updateQuantity` (slice 03, SUB-D-02), `convertTrial` (slice 06, gap G-2), the SUB-D-08 completion set (`renew`, `unschedule`, `pauseCollection`, `resumeCollection`, `confirmAcceptance`, `extendTrial`), and `archive` (the `cancelled→archived` edge operation — §4.1) extend the manifest §4.3 `TransitionRequest.type` list; the scheduled-intent envelope (`cancelMode`, `resumeAt`) extends the change vocabulary. Manifest alignment is tracked in [`../PRD.md`](../PRD.md) §15 (SEAMS **SUB-N1**).
- **Consumer obligation**: downstream consumers keying on `TransitionRequest.type` MUST tolerate the new values (route/ignore, not fail-closed) before the manifest lands — the type set is additive, and this gear is the producer.

## 5. Traceability

- **PRD**: [`../PRD.md`](../PRD.md) §6.1 (`fr-status-enum`, `fr-transitions-guards`, `fr-trials-not-a-status`, `fr-transition-request`, `fr-scheduled-intents`, `fr-activation-instants`), §6.2 (`fr-monotonic-version`), §6.7 (`fr-event-ordering`), §6.1 AC 1, §7.1 (NFRs), §15 (acceptance-flow + manifest-alignment opens).
- **Seams**: **SUB-E1** (fail-closed gate), **SUB-C4** (activation trio), **SUB-N1** (manifest alignment); consumes the ordering half of **SUB-R1** — [`../SEAMS.md`](../SEAMS.md).
- **Decisions**: SUB-D-01 (scheduled intents), SUB-D-05 (activation trio), SUB-D-03 (pause posture attribute) — [`../DECISIONS.md`](../DECISIONS.md).
- **ADR**: [`../ADR/0001`](../ADR/0001-cpt-cf-bss-subscriptions-adr-manifest-closed-status-machine.md) (manifest-closed status machine), [`../ADR/0003`](../ADR/0003-cpt-cf-bss-subscriptions-adr-scheduled-intents-on-aggregate.md) (scheduled intents on the aggregate).
- **Downstream slices**: [`02-composition-versioning.md`](./02-composition-versioning.md) (version + composition), [`03-plan-changes.md`](./03-plan-changes.md) (change envelope + ramps), [`04-suspension-renewal-grace.md`](./04-suspension-renewal-grace.md) (intent visibility, OSS pause), [`05-entitlements.md`](./05-entitlements.md) (issue/revoke at commit), [`06-trials.md`](./06-trials.md) (convertTrial, phase), [`07-tenancy-transfer.md`](./07-tenancy-transfer.md) (delegation, transfer approval), [`08-events-billing.md`](./08-events-billing.md) (outbox, ordering), [`09-consumer-contracts.md`](./09-consumer-contracts.md) (REST + gate contracts).
