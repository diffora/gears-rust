---
description: "Draft Billing requirements for complete invoice targets, financial corrections, advances and subscription trial coverage."
---

# PRD: Billing

**Status: draft.** This document covers the Billing responsibilities needed by the Subscriptions and Rating integration. Concrete financial-provider mappings, tax treatment, grouping and allocation decisions remain open. It does not claim a complete Billing product design or an implemented provider.

<!-- toc -->

- [1. Overview](#1-overview)
  - [1.1 Purpose](#11-purpose)
  - [1.2 Background / Problem Statement](#12-background--problem-statement)
  - [1.3 Goals (Business Outcomes)](#13-goals-business-outcomes)
  - [1.4 Glossary](#14-glossary)
- [2. Actors](#2-actors)
  - [2.1 Human Actors](#21-human-actors)
  - [2.2 System Actors](#22-system-actors)
- [3. Operational Concept & Environment](#3-operational-concept--environment)
- [4. Scope](#4-scope)
  - [4.1 In scope](#41-in-scope)
  - [4.2 Out of Scope](#42-out-of-scope)
- [5. Functional Requirements](#5-functional-requirements)
  - [5.1 Complete financial targets](#51-complete-financial-targets)
  - [5.2 Full replacements](#52-full-replacements)
  - [5.3 Grouping and rounding](#53-grouping-and-rounding)
  - [5.4 Draft and posted corrections](#54-draft-and-posted-corrections)
  - [5.5 Financial execution recovery](#55-financial-execution-recovery)
  - [5.6 Advances and accepted coverage](#56-advances-and-accepted-coverage)
  - [5.7 Promotional trial coverage](#57-promotional-trial-coverage)
  - [5.8 Service boundaries and preparation cost](#58-service-boundaries-and-preparation-cost)
  - [5.9 Recoverable source evidence](#59-recoverable-source-evidence)
- [6. Non-Functional Requirements](#6-non-functional-requirements)
  - [6.1 Tenant isolation](#61-tenant-isolation)
  - [6.2 Evidence retention](#62-evidence-retention)
- [7. Public Library Interfaces](#7-public-library-interfaces)
- [8. Use Cases](#8-use-cases)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Dependencies](#10-dependencies)
- [11. Assumptions](#11-assumptions)
- [12. Risks](#12-risks)
- [13. Open Questions](#13-open-questions)
- [14. Traceability](#14-traceability)

<!-- /toc -->

## 1. Overview

### 1.1 Purpose

Billing turns complete commercial and rated evidence into invoice obligations and recoverable financial outcomes. It owns invoice grouping, final rounding, advance and benefit allocation, draft replacement and corrections to posted invoices.

The same source may be delivered repeatedly, corrected after posting or recovered without events. Billing must converge on the correct financial target without collecting or posting twice. A lost response must remain distinguishable from an unapplied operation.

### 1.2 Background / Problem Statement

Commercial and rated evidence can arrive late, out of order or after a financial operation has already completed. Billing needs its own complete target and execution history to distinguish a source correction from another charge.

### 1.3 Goals (Business Outcomes)

- Account for every commercial obligation using the exact accepted source versions.
- Preserve immutable posted history while applying supported corrections.
- Recover uncertain financial effects before deriving successor effects.
- Keep tariff value, customer payable, advances and promotional coverage distinct.

### 1.4 Glossary

| Term | Meaning |
| --- | --- |
| Commercial seal | Subscriptions' exact fact membership for one commercial group |
| Rated result | Rating's complete, versioned valuation of one fact |
| Financial target | Billing's immutable selection of source evidence and desired financial outcome |
| Execution | A financial action with a stable identity and recoverable outcome |
| Trial grant | One bounded promotional benefit with accepted scope and original expiry |

## 2. Actors

### 2.1 Human Actors

#### Authorized operator

**ID**: `cpt-cf-bss-billing-actor-operator`

Inspects pending targets and performs explicitly authorized recovery.


### 2.2 System Actors

#### Subscriptions

**ID**: `cpt-cf-bss-billing-actor-subscriptions`

Supplies facts, seals, service boundaries and authoritative activation resolution.

#### Rating

**ID**: `cpt-cf-bss-billing-actor-rating`

Supplies complete exact valuations, manifests and replacement history.

#### Ledger / Payments / selected financial provider

**ID**: `cpt-cf-bss-billing-actor-finance`

Executes authorized financial work and exposes authoritative outcomes.

#### Entitlements / selected access adapter

**ID**: `cpt-cf-bss-billing-actor-entitlements`

Consumes committed trial-coverage evidence for initial readiness.


## 3. Operational Concept & Environment

Use the repository [ToolKit architecture](../../../../docs/toolkit_unified_system/README.md). This draft requires recoverable financial providers and retained source evidence; concrete deployment and operating bounds remain to be selected.

## 4. Scope

### 4.1 In scope

Complete invoice targets, full rated replacements, line grouping and final rounding, advance allocation, draft/posted correction, financial outcome recovery, promotional trial coverage, service-cutoff accounting and scoped evidence reads.

### 4.2 Out of Scope

Tariff authoring or calculation, subscription lifecycle decisions, raw usage collection, resource execution, tax and FX policy, payment-rail execution, dunning policy, a general promotions engine, migration, invoice presentation and customer notification workflows. These require their respective owners and separate design work.

## 5. Functional Requirements

### 5.1 Complete financial targets

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-fr-complete-targets`

Billing MUST accept a financial target only from an exact commercial seal and compatible complete rated results for every required member. Usage results require their complete expected manifests and independent source proofs. Missing, unauthorized, conflicting or unsupported evidence MUST keep the target pending.

### 5.2 Full replacements

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-fr-full-replacements`

Billing MUST replace the affected fact's contribution set using immutable result versions and lineage. Replays have no new effect, stale versions cannot replace a newer head, and equal-version digest conflicts are explicit. Empty replacements, active zero-valued lines and pending work MUST remain distinct. New full amounts MUST NOT be added to their predecessors.

### 5.3 Grouping and rounding

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-fr-invoice-rounding`

Billing MUST publish a versioned invoice-line equivalence contract, aggregate compatible exact contributions, and round each final line once HALF_EVEN at its stored currency scale. Financial receivers MUST NOT round again. Grouping and complete aggregate provenance MUST survive replay and correction. Unsupported cross-line minimum allocation MUST block that profile.

### 5.4 Draft and posted corrections

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-fr-posted-corrections`

Billing MUST protect draft replacement against concurrent posting. Posted financial documents remain immutable. Corrections MUST derive from the new rounded full target and actual prior financial history, using authorized credit/debit operations. Neither a detached period-state read nor a Rating acknowledgement proves that a draft is still mutable.

### 5.5 Financial execution recovery

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-fr-financial-recovery`

Billing MUST retain a stable identity and authoritative recoverable outcome for every financial operation. An unknown predecessor MUST be recovered or proven unapplied and fenced before successor effects rely on it. Status MUST distinguish received, staged, target accepted, execution pending and effect applied/replayed.

### 5.6 Advances and accepted coverage

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-fr-advance-allocation`

Billing MUST apply an eligible existing advance once to its accepted obligation without changing the gross rated tariff or collecting again. When selected as the financial-admission owner, Billing MUST satisfy the Subscriptions financial-admission contract: durable allocated payment or accepted future postpay coverage, explicit disposition fences and recovery. A promotional grant alone does not establish payment or postpay authority.

### 5.7 Promotional trial coverage

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-fr-trial-coverage`

Billing MUST commit one operation-bound reserved grant before credit-trial readiness, make it effective at Subscriptions' actual activation time, and record that result without new approval or access delay. Unknown activation retains protection; release requires authoritative fenced non-activation. Scope, amount, currency and original expiry are immutable.

Billing MUST allocate and correct full gross results once under the same grant. Eligible unused credit survives early paid conversion under its original restrictions and expiry. Corrections after expiry cannot restore spend permission, and corrections cannot reopen a closed trial. Eligible trial expenses served before the paid boundary, including positive corrections beyond the bounded grant, are platform-borne beyond available bonus coverage. Other liability cases remain explicit policy dependencies.

### 5.8 Service boundaries and preparation cost

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-fr-service-boundaries`

Billing MUST preserve authoritative served-time and confirmed cutoff evidence. A delayed physical stop cannot extend ordinary customer tariff past a confirmed cutoff. Incorrect charges are corrected once, while valid prior obligations and explicitly accepted retention fees remain. Authorized preparation before activation, including a never-activated attempt, is platform-borne and consumes no trial bonus.

### 5.9 Recoverable source evidence

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-fr-recoverable-evidence`

Billing MUST recover complete targets and old-period corrections independently of event delivery. Stable snapshots, ordered catch-up and exact historical references MUST preserve removed and zero obligations. Snapshot expiry and retained-history loss require distinct outcomes. Incomplete pages or missing dependencies cannot advance an applied checkpoint past unresolved work.

## 6. Non-Functional Requirements

### 6.1 Tenant isolation

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-nfr-tenant-isolation`

Every command, read, recovery page and referenced object MUST be authorized for the current tenant scope. Identifiers and cursors convey no authority. Changed or expanded scope requires a new authorized bootstrap; errors cannot reveal foreign history.

### 6.2 Evidence retention

- [ ] `p1` - **ID**: `cpt-cf-bss-billing-nfr-evidence-retention`

Financially referenced terms, facts, rated results, allocations and execution lineage MUST remain recoverable for the supported correction and recovery period. Operational deletion cannot remove unresolved financial history. Exact legal retention, workload, outage and restore bounds MUST be set and demonstrated before production enablement.

## 7. Public Library Interfaces

Versioned SDK capabilities cover target intake/read, exact financial execution lookup, allocated-advance evidence, trial coverage prepare/read/resolve and scoped recovery. Exact method names, error codecs and permission actions remain design work. These interfaces expose financial outcomes without granting authority to change subscription state.

## 8. Use Cases

- Receive a complete seal and rated set, then form one invoice target.
- Recover a missing result or old-period correction through pull reads.
- Correct a draft that may have posted while the correction was prepared.
- Recover a lost posting reply before executing a newer target.
- Prepare a credit-trial grant, resolve activation and correct historical benefit allocation.

## 9. Acceptance Criteria

The following integration cases are specifications, not executed tests. The selected providers must demonstrate them together with the applicable trial and financial-admission cases.

| Case | Intervention | Required outcome |
| --- | --- | --- |
| RB-C01 | A fact/result version replays, arrives stale or conflicts on digest | Replay has no new effect; stale cannot replace newer head; same-version conflict is explicit. |
| RB-C02 | Full result changes EUR 30 to EUR 25 | Billing targets EUR 25 once, replacing draft or correcting posted history; never bills EUR 55 or expects an independent Rating delta. |
| RB-C03 | Complete empty replacement, active zero valuation or missing fact/evidence | Empty replacement removes only the fact's old contributions under normal guards with lineage retained; active zero valuation retains classified/provenanced zero lines and proofs. Missing evidence is structured pending without monetary delivery. None implies completed refund; a zero total alone cannot erase lines. |
| RB-C04 | One of 744 required October hourly children or scope/usage proof is missing | Parent/group incomplete; a proven empty hour is explicitly zero with closure evidence. |
| RB-C05 | Seal 7 names A/v4/d4; result A/v5/d5 arrives before seal 8; separately, a new complete result corrects unchanged A/v4/d4 | A/v5/d5 waits for a successor seal naming that exact reference; same-version/different-digest conflicts. A calculation-only correction of A/v4/d4 can produce a new financial target under seal 7 without a new seal. A newer observed commercial head cannot be overwritten; incomplete results and unknown financial predecessor outcomes retain their guards. |
| RB-C06 | Ten units under A gain two under B mid-period | Full timeline retains A and B lot terms; no reprice of original quantity or duplicate twelve-unit obligation; unsupported pooled usage allocation refuses. |
| RB-C07 | One-time action retries with changed transport key; a full/partial advance exists | Same qualifying occurrence and original tariff; Billing applies applicable advance once, never captures again merely because it received valuation. |
| RB-C08 | Draft posts between replacement preparation and write | Revision fence rejects stale draft mutation; recover posted basis and use one financial correction path. |
| RB-C09 | v2 correction reply is lost before v3 arrives | Recover/fence v2 before deriving v3 effects; EUR 30→25→27 converges to EUR 27 once. |
| RB-C10 | All events are lost, then an old-period correction is committed | Snapshot plus ordered catch-up discovers the complete membership and correction; no duplicate financial effect. |
| RB-C11 | Consumer crashes before checkpoint, cursor expires or history is lost | Replay exact immutable work; distinguish rebootstrap from repair-required loss, never skip a gap. |
| RB-C12 | Trial valuation is corrected after expiry/paid conversion | Preserve gross/time/terms and same-grant allocation; no expired spend, revived service or invented trial debt. |
| RB-C13 | Runtime stop is delayed beyond confirmed cutoff | Preserve truthful runtime; no ordinary customer tail charge, idempotent correction if already posted. |
| RB-C14 | Frozen binding or boundary evidence is unavailable | Pending/unpriceable, no current-price fallback, zero default or guessed time split. |
| RB-C15 | Invoice/Ledger write commits but response is lost | Recover the same execution/business key and effect; target acceptance is not falsely reported as financial completion. |
| RB-C16 | Authorization changes or incomplete snapshot pages arrive | No leaked history, mixed-scope completeness or checkpoint beyond missing references; new scope bootstraps independently. |

Additional financial-boundary cases:

| Case | Intervention | Required outcome |
| --- | --- | --- |
| RT-C11 Multi-fact invoice evidence | Two facts each contribute EUR 0.005 to the same invoice-line key; Ledger has one snapshot field | One exact EUR 0.01 aggregate and one rounding, complete aggregate provenance; no separately rounded EUR 0 items |
| RT-C13 Result versus financial gap | Full result v3 arrives before v2; v2's credit execution is unknown | Rating lineage can be recovered without applying v2 as an intermediate target; Billing must recover/fence v2 execution before v3 effects |

## 10. Dependencies

Subscriptions supplies immutable facts, exact seals, service boundaries and activation resolution. Rating supplies complete full results, manifests and historical recovery. Pricing supplies retained accepted terms. Ledger, Payments and selected financial/access providers must publish compatible execution and recovery contracts. Trial coverage also depends on the agreed Entitlements access profile.

## 11. Assumptions

Accepted commercial evidence remains immutable and recoverable. A missing provider capability keeps the affected obligation visible and pending rather than selecting a fallback tariff or losing the obligation.

## 12. Risks

The main risks are incomplete usage evidence presented as a final invoice, duplicate corrections after response loss, rounding changes caused by grouping, and unsupported grant or boundary allocation. Versioned contracts and the acceptance cases must address them before enablement.

## 13. Open Questions

Resolve invoice equivalence and cross-line minimum allocation, concrete financial-provider mappings, trial accounting/access limits, tax/FX participation and required retention/workload bounds. [Design open decisions](DESIGN.md#4-open-decisions) track these items. General Billing product scope needs a separate review before this draft can be treated as a complete product specification.

## 14. Traceability

[DESIGN.md](DESIGN.md) maps every requirement to its architecture response. The source contracts are [Subscriptions commercial evidence](../../subscriptions/docs/DESIGN.md#financial-consumer-boundary), [trial coverage](../../subscriptions/docs/features/06-trials.md), [financial admission](../../subscriptions/docs/DESIGN.md#3311-financial-admission-integration-contract) and the [shared money ADR](../../docs/ADR/0001-cpt-cf-bss-adr-exact-decimal-boundaries.md).
