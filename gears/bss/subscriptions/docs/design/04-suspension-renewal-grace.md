<!-- CONFLUENCE_TITLE: [BSS]: Subscriptions — Suspension, Renewal & Grace (Design) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../SEAMS.md | Upstream: Contracts (renewal/grace SoR), Payments (pre-check/retry-exhaustion), OSS (pause) | Downstream: Billing (dunning, collection artifacts), Notifications (notice delivery) | Owners: BSS Subscriptions team -->

# DESIGN — Suspension, Renewal & Grace (Slice 4)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-design-suspension-renewal-grace`

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
  - [4.1 Suspend / Resume and OSS Pause (normative)](#41-suspend--resume-and-oss-pause-normative)
  - [4.2 Billing-Only Pause Posture (normative)](#42-billing-only-pause-posture-normative)
  - [4.3 Renewal Evaluation, Auto vs Manual (normative)](#43-renewal-evaluation-auto-vs-manual-normative)
  - [4.4 Grace Ladder and Policy (normative)](#44-grace-ladder-and-policy-normative)
  - [4.5 Notices and Opt-Out (normative)](#45-notices-and-opt-out-normative)
  - [4.6 Dunning Handoff (normative)](#46-dunning-handoff-normative)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

This slice owns the **posture and time-driven** transitions: suspend/resume, the billing-only
`collectionPaused` posture, and the renewal → grace ladder. All of them route through the Foundation
commit path and are **governed, reversible, auditable** — never soft deletes or silent state drift
([`../PRD.md`](../PRD.md) §6.4, §6.5). The renewal job is Contract-driven: Subscriptions executes and
audits; the commercial terms (grace length, ladder, regional templates) are Contract SoR.

The load-bearing risk (**SUB-C1**) is that the upstream Contracts PRD does **not yet author** the
renewal/grace SoR; until it does, the **platform defaults govern** (7-day grace, 30/14/7/1 notices,
hybrid exit). The slice also owns **SUB-B2** (`collectionPaused` artifact treatment with Billing),
**SUB-F1** (Payments signals), **SUB-B5** (dunning handoff), and **SUB-E2** (OSS pause on suspend).

### 1.2 Architecture Drivers

#### Functional Drivers

| Requirement | Design Response |
|-------------|-----------------|
| `cpt-cf-bss-subscriptions-fr-suspend-resume` | `suspend`/`resume` through the Foundation gate; revoke/freeze then re-issue entitlements (slice 05); OSS deprovision/pause / reprovision confirmed by events (§4.1). |
| `cpt-cf-bss-subscriptions-fr-suspension-billing-posture` | Suspension-vs-billing (pause recurring vs continue for reserved capacity) is explicit product policy in subscription attributes + contract clauses (§4.1). |
| `cpt-cf-bss-subscriptions-fr-collection-pause` | `collectionPaused` is an auditable window (start/end/limit/reason) **posture on `active`** — service untouched, recurring emission suppressed/deferred per policy (§4.2; SUB-D-03). |
| `cpt-cf-bss-subscriptions-fr-renewal-evaluation` / `cpt-cf-bss-subscriptions-fr-renewal-auto-manual` | A renewal job evaluates term/`endDate`, extends on success, triggers the failed path on failure; auto requires a valid payment method + contract allow; manual is an explicit `TransitionRequest`; attempts keyed against double extension (§4.3). |
| `cpt-cf-bss-subscriptions-fr-failed-renewal-ladder` / `cpt-cf-bss-subscriptions-fr-grace-policy` | A testable grace ladder: 7-day default, paused next-term recurring, evaluated fields stored for replay, hybrid exit (elapsed OR retry-exhausted) (§4.4). |
| `cpt-cf-bss-subscriptions-fr-renewal-notices` | Notice triggers at 30/14/7/1 days (Contract/template override within Legal bounds); opt-out = scheduled non-renewal at term end; delivery = Notifications (§4.5). |

#### NFR Allocation

| NFR theme | Allocated To | Design Response | Verification / Status |
|-----------|--------------|-----------------|-----------------------|
| `cpt-cf-bss-subscriptions-nfr-lifecycle-latency` | Suspend/resume commit path | Synchronous commit class (p95 < 1s) | Load test; baseline (workshop-pending) |
| `cpt-cf-bss-subscriptions-nfr-recurring-cut` | Grace recurring suppression | Blocked next-term recurring is not emitted until renewal succeeds or grace fails | Reconciliation §17.1 |

#### Key ADRs

No slice-local ADR; the renewal/grace SoR split is governed by SEAMS **SUB-C1** (Contracts SoR;
platform default until authored) and the pause posture by SUB-D-03.

### 1.3 Architecture Layers

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-tech-stack-rnw`

| Layer | Responsibility | Technology |
|-------|----------------|------------|
| Application | Suspend/resume + pause posture handlers; the renewal job; notice + grace ladder | Rust module in the `subscriptions` gear |
| Domain | `collectionPaused` window, renewal-evaluation record (evaluated fields), grace-ladder state, notice schedule | Rust; GTS + Rust domain structs |
| Infrastructure | Grace-evaluation table; renewal job coordinated via the lease library | PostgreSQL, SecureORM |

## 2. Principles and Constraints

### 2.1 Design Principles

#### Governed, reversible posture

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-governed-posture-rnw`

Suspension and pause are governed, reversible posture changes — not soft deletes; every transition
is Policy-gated, evented, and audited ([`../PRD.md`](../PRD.md) §6.4).

#### Contract is the terms SoR; Subscriptions executes

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-principle-contract-sor-rnw`

Renewal terms, grace length, ladder, and regional templates are Contract SoR; Subscriptions runs the
job, stores **evaluated fields** at evaluation time, and audits — it invents no commercial term
([`../PRD.md`](../PRD.md) §6.5; SEAMS **SUB-C1**).

### 2.2 Constraints

#### Grace defaults are platform-testable

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-grace-defaults-rnw`

Until Contracts authors the SoR, the **7-day grace / 30-14-7-1 notices / hybrid exit** platform
defaults govern and MUST be product-testable; Contract/template override only within Legal bounds
([`../PRD.md`](../PRD.md) §6.5; SEAMS **SUB-C1**).

#### Idempotent renewal attempts

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-constraint-renewal-idempotent-rnw`

Renewal attempts are keyed to prevent **double term extension**; a retry-driven job never extends
twice ([`../PRD.md`](../PRD.md) §6.5).

## 3. Technical Architecture

### 3.1 Domain Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-domain-model-rnw`

- **`CollectionPauseWindow`** — `start`, `end`/`limit`, `reason`; posture on `active`, collection-scoped only.
- **`RenewalEvaluation`** — the evaluated fields at term-end (grace length, ladder variant, billing posture, `graceEndsAt`) frozen for replay + idempotent jobs.
- **`GraceLadderState`** — grace start, elapsed/retry status, exit trigger.
- **`NoticeSchedule`** — the 30/14/7/1 trigger set + opt-out flag.

### 3.2 Component Model

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-component-renewal-rnw`

- **`SuspendResumeHandler`** — the posture transitions through the Foundation gate; coordinates OSS pause/reprovision + entitlement freeze/re-issue (slice 05).
- **`CollectionPauseHandler`** — sets/clears the `collectionPaused` window; signals Billing the artifact treatment.
- **`RenewalJob`** — the coordinated singleton evaluating term/`endDate`, extending on success, entering the failed path on failure.
- **`GraceLadder`** — drives the 7-day (or Contract) window, the paused next-term recurring, and the hybrid exit.
- **`NoticeScheduler`** — emits notice triggers + processes opt-out as scheduled non-renewal.

### 3.3 API Contracts

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-interface-renewal-rnw`

`suspend`/`resume`/`renew` operations + the `collectionPaused` set/clear operation; renewal-monitoring
read models (failing renewals, `graceEndsAt`, ladder variant) power the Finance UC
([`../PRD.md`](../PRD.md) §10 renewal-monitoring). Payments failure-signal + Billing dunning wire
contracts are owned by [`09-consumer-contracts.md`](./09-consumer-contracts.md).

### 3.4 Internal Dependencies

Depends on [`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) (commit path, gate,
`IntentScheduler` for scheduled non-renewal) and [`05-entitlements.md`](./05-entitlements.md)
(freeze/re-issue on suspend/resume). Feeds [`08-events-billing.md`](./08-events-billing.md)
(suspend/resume/collection events).

### 3.5 External Dependencies

| Dependency | What crosses the boundary | Contract |
|------------|---------------------------|----------|
| Contracts | Renewal terms, grace ladder, regional templates, `PriceOverride` | SEAMS **SUB-C1**, **SUB-C5** |
| Payments | Pre-check outcomes + retry-exhaustion declarations | SEAMS **SUB-F1** |
| Billing | Dunning execution; `collectionPaused` artifact treatment | SEAMS **SUB-B2**, **SUB-B5** |
| OSS | Deprovision/pause on suspend; reprovision on resume | SEAMS **SUB-E2** |
| Notifications | Notice + win-back delivery (triggers owned here) | SEAMS **SUB-F2** |

### 3.6 Interactions and Sequences

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-flow-renewal-grace-rnw`

**Renewal + grace** (refines `cpt-cf-bss-subscriptions-seq-renewal-grace`): `RenewalJob` at term end
→ payment pre-check → **success**: extend term + fresh snapshot refs (keyed against double extension);
**failure**: `GraceLadder` starts (7-day default), the blocked next-term recurring is **paused**
(not emitted), notices fire, hybrid exit (interval elapsed OR Payments retry-exhausted) → `suspended`/
`cancelled` per Contract ladder; all transitions run through the Foundation gate. `RenewalEvaluation`
stores evaluated fields for replay.

### 3.7 Database Schemas and Tables

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-storage-renewal-rnw`

Owned here: `renewal_evaluation` (evaluated fields, `graceEndsAt`), `grace_ladder_state`, and the
`collection_pause_window` rows on the aggregate. Scheduled non-renewal rides the Foundation
`scheduled_intent`. Concrete DDL is Design.

### 3.8 Deployment Topology

- [ ] `p3` - **ID**: `cpt-cf-bss-subscriptions-deployment-rnw`

The `RenewalJob` + `NoticeScheduler` run as coordinated singletons **per tenant partition** via the
lease library — one lease per `orderingTenantId` shard, shard-parallel across partitions, so the
100K+/tenant scale target is not funnelled through one global instance (2026-07-15 review fix);
within a partition the per-aggregate ordering holds. **Intra-tenant parallelism (2026-07-15 review
fix):** because a single large tenant is one `orderingTenantId` shard, the daily-cut-class work
inside it is further sub-sharded by a stable hash of `subscriptionId` into N worker leases, so a
100K+/tenant renewal/notice sweep is not serialised through one worker; per-aggregate ordering is
preserved because a given `subscriptionId` always maps to the same sub-shard. N is a deploy-time
capacity knob, not a commercial one. Suspend/resume are control-plane transitions;
edges with OSS legs follow the Foundation async note ([`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md)
§3.6/§3.8).

## 4. Additional Context

### 4.1 Suspend / Resume and OSS Pause (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-suspend-resume-rnw`

- **Suspend** → `suspended`; revoke/freeze entitlements per policy; OSS deprovision/pause confirmed by events. **Resume** → `active`; **Policy allow mandatory**; re-issue entitlements; OSS reprovision ([`../PRD.md`](../PRD.md) §6.4; SEAMS **SUB-E2**). **Resume after a grace-driven suspension additionally requires the blocking payment failure resolved** (successful renewal/payment or an audited operator override) — `resume` alone never restores unpaid service ([`../PRD.md`](../PRD.md) §6.5; 2026-07-15 review fix). Resume also re-runs the overlap check (slice 03 §4.4 — entry into `active`).
- Suspension-vs-billing (pause recurring vs continue for reserved capacity) is **explicit product policy** in subscription attributes + contract clauses — never a silent assumption ([`../PRD.md`](../PRD.md) §6.4).

### 4.2 Billing-Only Pause Posture (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-collection-pause-rnw`

- `collectionPaused` is a **posture on `active`**: service + entitlements untouched; the recurring period fact for the paused window is emitted **marked `collectionPaused`** and Billing suppresses/defers the posting **per policy** — Billing owns the artifact treatment, AC 24 "not posted" holds ([`../PRD.md`](../PRD.md) §6.4; SUB-D-03, SEAMS **SUB-B2**).
- **Renewal during the window (SUB-D-12, AC 29):** renewal **evaluation and term extension continue** (the term stays deterministic for rating/Billing), but the payment pre-check, grace entry, and dunning handoff are **suspended** for renewals whose collection falls inside the window; the deferred collection runs when the window ends. A pause never converts into a payment-failure suspension by itself.
- **Pause applied while already in grace (2026-07-15 review fix):** a `pauseCollection` set on a subscription **already inside the grace ladder** (the common dispute/hardship reaction to a failed charge) **freezes the ladder** — the grace clock stops, dunning is held, and no exit to `suspended`/`cancelled` fires — for the duration of the window; on `resumeCollection` (or window end) the ladder resumes with the **remaining** grace time (elapsed time before the pause counts, time inside it does not). This is the same "collection deferred, service preserved" invariant as SUB-D-12 applied to an in-flight ladder rather than a fresh renewal; the pause-day limit (§15, Product/Billing) bounds indefinite deferral so a pause cannot be used to escape suspension forever. `GraceLadderState` records the freeze/thaw as evaluated fields for replay.
- The posture is an auditable window (start/end/limit/reason) bounded by Contract/Policy, set/cleared by the `pauseCollection`/`resumeCollection` transitions (SUB-D-08). **Open (§15):** pause-day limits + resume proration — Product/Billing.

### 4.3 Renewal Evaluation, Auto vs Manual (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-renewal-rnw`

- The renewal job evaluates `Renewal` (`autoRenew`, term windows) from Contract; **auto** extends when the payment method is valid + contract allows; **manual** requires an explicit `TransitionRequest` of type **`renew`** (SUB-D-08) with the same idempotency rules ([`../PRD.md`](../PRD.md) §6.5).
- Attempts are **keyed to prevent double term extension**: the term-extension effect is idempotent on `(subscriptionId, currentTermSequence)` — the monotonic index of the term being renewed — derived, not client-supplied, so a crashed-and-retried `RenewalJob` firing (or a duplicate manual `renew`) resolves to the **already-extended** term instead of extending twice (the same derive-the-key discipline as the scheduled-intent firing, slice 01 §3.6). Manual renewal creates a new term window with fresh snapshot refs (pricing-side segments only — the `(currency, region)` segment persists, slice 02 §4.2).
- **Late success inside grace:** the new term starts at the **old term end** (backdated — continuous coverage, no gap); the previously blocked next-term recurring fact is emitted with its **original** `(subscriptionId, billing period)` key ([`../PRD.md`](../PRD.md) §6.5 grace policy 5).

### 4.4 Grace Ladder and Policy (normative)

- [ ] `p1` - **ID**: `cpt-cf-bss-subscriptions-normative-grace-rnw`

- Default grace: **7 calendar days** from grace start (first auditable failed pre-check or aligned post-renewal billing failure); jurisdiction bounds come from Contract/regional template within Legal ([`../PRD.md`](../PRD.md) §6.5).
- Recurring during grace: the **blocked next-term recurring MUST NOT be emitted** until renewal succeeds or grace resolves to failure (paused); usage-rated charges MAY continue until `suspended` unless Contract/Policy freezes them.
- Exit is **hybrid — whichever first**: grace interval elapses, OR Payments declares no further automated retries. Move to `cancelled` only per contract-defined steps after suspend/final dunning ([`../PRD.md`](../PRD.md) §6.5). Subscription stores **evaluated fields** for audit + idempotent jobs + replay (SEAMS **SUB-C1**, **SUB-F1**).

### 4.5 Notices and Opt-Out (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-notices-rnw`

- Notice triggers at platform default **30/14/7/1 days** before term end; Contract/regional template MAY override within published bounds. **Triggers + intervals owned here; delivery = Notifications/Comms** ([`../PRD.md`](../PRD.md) §6.5; SEAMS **SUB-F2**).
- **Short terms:** intervals ≥ the term length are skipped — only offsets that fit inside the current term fire (a monthly term gets 14/7/1 by default, not a 30-day notice at term start); the effective set is an evaluated field for audit.
- A renewal **opt-out** is a scheduled non-renewal at term end (cancel at term boundary; no further attempts), idempotent — via the Foundation `IntentScheduler`; opting back in is the `unschedule` of that intent (SUB-D-08).

### 4.6 Dunning Handoff (normative)

- [ ] `p2` - **ID**: `cpt-cf-bss-subscriptions-normative-dunning-rnw`

- A post-renewal billing failure hands off to **dunning** (Billing/Payments §4.4–4.5); the same grace rules + triggers apply. Subscriptions emits the failure/grace signals + audit trail; **dunning execution + PSP webhook payloads are Billing/Payments + Design** ([`../PRD.md`](../PRD.md) §6.5; SEAMS **SUB-B5**, **SUB-F1**).

## 5. Traceability

- **PRD**: [`../PRD.md`](../PRD.md) §6.4 (`fr-suspend-resume`, `fr-suspension-billing-posture`, `fr-collection-pause`), §6.5 (`fr-renewal-evaluation`, `fr-renewal-auto-manual`, `fr-renewal-notices`, `fr-failed-renewal-ladder`, `fr-grace-policy`), §10 (renewal-monitoring UC), §7.1 (NFRs), §15 (pause mechanics open), §16 (Contracts-grace risk).
- **Seams**: **SUB-C1**, **SUB-B2**, **SUB-F1**, **SUB-B5**, **SUB-E2**, **SUB-C5**, **SUB-F2** — [`../SEAMS.md`](../SEAMS.md).
- **Decisions**: SUB-D-03 (`collectionPaused`) — [`../DECISIONS.md`](../DECISIONS.md).
- **Slices**: [`01-foundation-lifecycle.md`](./01-foundation-lifecycle.md) (gate, scheduler), [`05-entitlements.md`](./05-entitlements.md) (freeze/re-issue), [`08-events-billing.md`](./08-events-billing.md) (posture events), [`09-consumer-contracts.md`](./09-consumer-contracts.md) (Payments/Billing contracts).
