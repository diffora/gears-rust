<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./02-taxonomy-attributes.md | Owners: BSS Product Catalog team -->

# DESIGN — Governance & Access Control (Slice 5)

<!-- toc -->

- [1. Context](#1-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
  - [1.5 Scope](#15-scope)
  - [1.6 Constraints & Assumptions](#16-constraints--assumptions)
  - [1.7 Naming & Design-Introduced Names](#17-naming--design-introduced-names)
  - [1.8 Context & Dependencies](#18-context--dependencies)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Submit a change for approval](#submit-a-change-for-approval)
  - [Decide (approve / reject)](#decide-approve--reject)
  - [Publish / apply against the gate](#publish--apply-against-the-gate)
  - [Read the pending queue (the studio inbox contract)](#read-the-pending-queue-the-studio-inbox-contract)
  - [Break-glass elevation (read/audit-export only)](#break-glass-elevation-readaudit-export-only)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 Materiality mechanics](#31-materiality-mechanics)
  - [3.2 RBAC catalog](#32-rbac-catalog)
  - [3.3 Error taxonomy (slice-owned codes)](#33-error-taxonomy-slice-owned-codes)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice is the **gate** every other slice invokes and none re-implements: the materiality
evaluator, the two-person approval workflow with a **stored** pinned content snapshot, the
FinanceReviewer role predicate, the P-D-02 override ceremony, RBAC deny-by-default over the
gear's GTS-typed resource/action catalog, and the time-boxed, read-only **break-glass**
elevation. It plugs into the Foundation as the governance phase of the `PublishDoor` (01
`inst-fd-governance-gate`) and as the approver of `GovernedLiveOp` envelopes (02 §3.1).

### 1.2 Purpose

Financial-grade control with exactly one enforcement point: no slice can publish, apply a live
op, or elevate around this gate, and the gate itself is built so the two historical failure
classes cannot recur — a re-derived "pinned" snapshot that diffs a draft against itself (the
pricing lesson: **store the snapshot, never re-derive it**), and a single human wearing two
roles counting as two approvers (**principals, not roles**).

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-product-manager` | Submits changes; never approves own work |
| `cpt-cf-bss-products-actor-catalog-admin` | Approver; break-glass initiator (with second approver or post-hoc review) |
| `cpt-cf-bss-products-actor-finance-reviewer` | The mandatory second lens on finance-material fields; approval-queue consumer |
| `cpt-cf-bss-products-actor-auditor` | Reads the immutable approval/audit/break-glass trails |
| `cpt-cf-bss-products-actor-platform-owner` | Break-glass subject: cross-tenant read/audit-export only (v1) |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.7 (`fr-materiality-gated-publish`), §6.8
  (`fr-tenant-isolation-breakglass`, `fr-breakglass-action-scope`); AC #26, #30, #31; §17.1
  (interim materiality default; affected-entity trigger ≥ 10)
- [`../DECISIONS.md`](../DECISIONS.md) P-D-02 (override at entity publish), P-D-10 (no
  gear-side Legal role — C8's narrowing clause), P-D-11 (`N` is a policy value, default 2 floor
  0 — C1), P-D-13 (the shorthand's enumerated reach: `inst-bg-open`'s fixed platform floor,
  `quorumReduced` on the reducible ceremonies)
- Pricing `design/05-governance.md` — the pattern donor (G2 principal distinctness, approver
  scope, policy-mutation-is-material); divergence: this gear's quorum **defaults** to `N = 2`
  approvers against pricing's structural submitter + one — one configuration value wide, not a
  different mechanism (P-D-11)
- [`./01-foundation.md`](./01-foundation.md) `inst-fd-governance-gate`,
  `inst-fd-approval-hook`; [`./02-taxonomy-attributes.md`](./02-taxonomy-attributes.md)
  `GovernedLiveOp`

### 1.5 Scope

**In**:
- materiality evaluation (field-set driven off the bucket registry + enumerated ops + affected-entity count)
- the approval workflow (submit → quorum → publish/apply) over both entity publishes and `GovernedLiveOp`s
- stored pinned snapshots + diff rendering
- the override ceremony
- approver constraints (distinctness, roles, scope)
- the pending-approvals read surface (the studio inbox contract)
- RBAC catalog
- break-glass elevation + its audit.

**Out**:
- the doors themselves (01/02)
- scheduling (04 pins approvals, this slice only validates them at activation through the gate)
- the slice-07 break-glass **correction** door — a distinct, feature-flag-gated WRITE mechanism that merely reuses this slice's elevation ceremony
- erasure of approver identities (10).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Quorum for a material change: the tenant's configured **`N` distinct approvers, each distinct from the author**, each holding CatalogAdmin or FinanceReviewer — `N` a typed-policy value, **default 2, floor 0** (P-D-11). Not configurable: finance-material fields (`taxCategory`, `glCode`, `PlanTier`) require ≥ 1 FinanceReviewer **among** the approvers (the predicate governs who, not how many) and the record states the predicate as **unsatisfiable at `N = 0` only** — where there are no approvers to hold the role, so the descriptor stays satisfiable rather than blocking on a role no principal could hold. **At every `N >= 1` the predicate binds and a tenant that has designated no FinanceReviewer simply has an unapprovable change, which is correct** (`inst-gv-finance-predicate` is the normative arm; this row previously read "when the configured `N` cannot carry it", which an implementer could read as *the available approvers cannot satisfy it* and build into a finance-review fail-open at `N = 2` — Blocking 2 of the 2026-08-26 review); self-approval refused at every `N ≥ 1`; `N` reachable only by explicit configuration, absent ⇒ default; initial `N` from tenant provisioning, later changes material under the then-current quorum (C4) | PRD `fr-materiality-gated-publish`; P-D-11 |
| C2 | Distinctness is by **principal**, never by role: one human holding both roles is one approver | pricing G2, adopted |
| C3 | An approval pins the internal revision AND stores the submitted content snapshot; **any frozen-content write — save OR lifecycle transition — bumps `internal_revision` and fires the invalidation hook**, **except `draft→published`, which the publish door owns: it bumps once and fires no hook** (**P-D-26** — the same transaction consumes the approval, and a hook firing against the record the act is consuming has no defined ordering) (M-2 fix: head-at-revision-N is therefore byte-identical to the snapshot at revision N; transition-written columns cannot drift under a pinned approval), superseding open approvals and re-queuing with the diff re-presented | PRD `fr-materiality-gated-publish` |
| C4 | Materiality is a typed, configurable policy with the §17.1 interim default enforceable at launch; **the policy's own mutation is material** (the two-person rule's foundation must not be single-person-editable — the pricing D-10 lesson, adopted) | PRD §17.1 |
| C5 | Break-glass (§6.8) is **read + audit-export only** in v1; any write under elevation is refused, full stop. **Canonical boundary (M-3):** slice 07's flag-gated correction write does **not** run under a `BreakGlassSession` — a §6.8 session never authorizes a write; 07 reuses only the elevation *ceremony shape* (two-person + mandatory reason + recording) as its own distinct mechanism | PRD `fr-breakglass-action-scope` |
| C6 | Deny-by-default: no grant, no access; grants are GTS-typed resource×action pairs checked at every door | PRD `fr-tenant-isolation-breakglass` |
| C7 | Audit **sealing** is a platform capability, not this gear's: pricing's G4/D-14 in-gear hash chain is deliberately **not** adopted. v1 writes the complete append-only trail (01 §4.4) over a **reserved, unwritten** sealing seam; the requirements the platform capability must satisfy are P-D-08 S1–S9, carried as a PRD §15 open owned by Architecture. Until activation, audit immutability is the trigger whitelist — plus `REVOKE` on Postgres, which SQLite has no equivalent for (01 **P-D-35**) — and nothing cryptographic — completeness ships, tamper-evidence does not | P-D-08 |
| C8 | Role predicates **narrow within** the C1 base set; they never replace it, and v1 registers **no** extension point that could (P-D-10, 2026-08-26). `inst-gv-finance-predicate` is the only one and it is additive — C1 already demands CatalogAdmin-or-FinanceReviewer, the predicate demands that one of the two *be* a FinanceReviewer. A predicate that replaced the base set would be a bypass surface: register a kind whose predicate admits anyone and a material change passes on one signature. Any future replacing predicate therefore owes two guards — the numeric quorum still binds, and registering or changing a kind's predicate is itself material (as C4 already makes the materiality policy's own mutation) | P-D-10 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `MaterialityEvaluator` | Decides material / non-material for a change set: bucket-iii field touches (registered by owning slices), the PRD-enumerated ops, or affected-entity count ≥ the configured trigger |
| `ApprovalRecord` | The stored unit: subject ref + pinned revision + **stored content snapshot** + rendered diff basis + quorum descriptor + state. The descriptor also carries **`quorumReduced`** when the effective count is below the retained-name default of 2 (P-D-13) — the count's counterpart to `predicateUnsatisfiable`, so a one-person act is never read off an audit trail that says "two-person". The descriptor carries `required` = the **effective** count — `N` for a material change, `min(N, 1)` for a non-material one (`inst-gv-materiality`), which is also what `inst-gv-queue` exposes — and, when a mandatory predicate cannot be carried at that count (finance-material at `N = 0`), an explicit **`predicateUnsatisfiable`** marker — the control's absence is a stored fact, not something a later reader infers from a config value (the P-D-08 `seal_state` instinct, same reason) |
| `QuorumEvaluator` | Counts distinct approving principals against the descriptor (role predicates included) |
| `OverrideCeremony` | The P-D-02 variant: approvers explicitly acknowledge named lint findings; the acknowledgment is part of the record. At `N = 0` the **author** performs it (P-D-13) — informedness, not head-count, is what the ceremony buys, so it is never skipped for want of an approver |
| `BreakGlassSession` | The time-boxed elevation record every elevated read hangs off |

### 1.8 Context & Dependencies

**Consumed**: IdP claims (principals, roles, brand/region scope); the bucket registry (01/03);
lint findings (03 bundle, 02 attention conditions); config store (materiality policy, trigger
count, break-glass window). **Produced**: the gate verdicts 01/02/04 consume; `ApprovalDecided`,
`BreakGlassElevated`/`BreakGlassExpired` events; the pending-approvals envelope the studio
inbox merges with pricing's.

## 2. Actor Flows (CDSL)

### Submit a change for approval

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-submit`

1. [ ] - `p1` - Submission runs `MaterialityEvaluator` over the change set (head-row pending edits vs last published version; for a `GovernedLiveOp`, the op payload): **material** ⇒ quorum descriptor per C1 (`required = N`); **non-material** ⇒ `min(N, 1)` — so a tenant at `N = 0` publishes approver-less by policy and the record says exactly that, which is the point of P-D-11 and replaces the earlier "nothing publishes approver-less" interim; first publish and every lifecycle transition **to `published`/`deprecated`/`retired`** are material at their **initiating human act** — the mechanical stages of an approved scheduled act (the `effectiveAt` flip, cascade legs) do not re-enter the gate (H-2, see `inst-gv-one-shot`); a bucket-iv-only re-publish is the non-material case (a re-publish is not a transition, 01's head-row model), and `draft→discarded` is ungated beyond authz (M-1) - `inst-gv-materiality`
2. [ ] - `p1` - The `ApprovalRecord` **stores the submitted content snapshot** at submission time — the diff shown to approvers is rendered from the STORED snapshot against the last published version, never re-derived from the live head (the pricing pinned-content defect, designed out) - `inst-gv-stored-snapshot`
3. [ ] - `p1` - Finance-material field touches (`taxCategory`, `glCode`, `PlanTier` — the bucket registry marks them) set the FinanceReviewer predicate on the quorum descriptor **when `N ≥ 1`**. At **`N = 0`** the predicate has no subject — there are no approvers to hold the role — so it is **not set**; instead the descriptor records `predicateUnsatisfiable = finance_reviewer` and stays satisfiable. Without this arm the descriptor would demand a role no principal could hold and `inst-fd-governance-gate` would raise `APPROVAL_REQUIRED` forever, re-blocking exactly the tenant P-D-11 unblocked: `taxCategory` is required at publish for product/service types, so a one-person tenant's first such SKU would be unpublishable (the hole CodeRabbit found in the P-D-11 wave, 2026-08-26). `N ≥ 1` is unaffected — a lone approver on a finance-material change must be a FinanceReviewer, and a tenant that has designated none simply has an unapprovable change, which is correct - `inst-gv-finance-predicate`
4. [ ] - `p1` - A frozen-content write on the subject after submission (save or transition — C3) fires 01's `inst-fd-approval-hook`: the record flips `superseded`, and **re-submission is an explicit human act** (the submitter or any write-granted principal; never automatic — auto-resubmit would pin content nobody re-read, L-3) with the new diff re-presented — approvers never decide on stale content - `inst-gv-supersede`

### Decide (approve / reject)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-decide`

1. [ ] - `p1` - `approval × decide` grant required; the author's own decision is refused `SELF_APPROVAL_FORBIDDEN` — by principal, not role (C2) - `inst-gv-self`
2. [ ] - `p1` - The approver's brand/region claims MUST cover the subject's scope; an out-of-scope decision is refused `APPROVER_SCOPE_EXCEEDED` and audited like any scope violation (the pricing `inst-ap-scope` analogue) - `inst-gv-scope`
3. [ ] - `p1` - Each decision appends a `products_approval_decision` row (approver principal, verdict, mandatory reason on reject, instant); `QuorumEvaluator` answers "satisfied" only when the descriptor is met by **distinct principals** with the required roles — a predicate recorded `predicateUnsatisfiable` (no subject at the configured `N`) counts as met **for the evaluator** while remaining visible as unmet-by-policy in the record and the inbox envelope; that is the only way it may be discharged, and it is never how a predicate is discharged at `N ≥ 1` - `inst-gv-quorum`
4. [ ] - `p1` - A rejection finalizes the record `rejected` with the reason; the subject stays as it was (AC #26 reading per 01: a first-publish draft stays draft, a published head keeps its pending edits unpublished); `ApprovalDecided` emitted either way - `inst-gv-reject`
5. [ ] - `p1` - **`OverrideCeremony`** (P-D-02): when the subject carries named override conditions (an uncomposed `bundle` at its entity publish — 03 `inst-cl-bundle-override`), each approver explicitly acknowledges the lint findings by name; the acknowledgments are stored on the record and in audit — an informed override, never a blind one. **At `N = 0` the author performs the acknowledgment** and the record carries `quorumReduced` (P-D-13): the ceremony's product is an informed decision, so it survives an empty quorum instead of vanishing with it. The record is also the ceremony's only home — a lane that publishes an override subject without one is a design defect, not an exemption (see 09 `inst-bk-override`) - `inst-gv-override`

### Publish / apply against the gate

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-gate`

1. [ ] - `p1` - **The Foundation's gate phase runs on any gated act, not on publish alone** (**P-D-30**) — which is what hosts this slice's gate on 04's un-deprecation edge: a transition door consumes the `satisfied` record exactly as the publish door does, and `inst-gv-one-shot`'s same-transaction flip binds identically there. Separately, **authorization is a pre-pipeline gate rather than a phase** (same call), run before 01's pipeline opens so a denied caller neither consumes an idempotency key nor writes a claim row; the denial's code and status are this slice's to declare, and remain open below. The Foundation gate (01 `inst-fd-governance-gate`) asks this slice: a satisfied, non-superseded `ApprovalRecord` whose pinned revision equals the door's expected revision ⇒ yes; anything else ⇒ `APPROVAL_REQUIRED` (or `STALE_REVISION` upstream). The gate never re-evaluates materiality at publish — the verdict was fixed at submission (a policy change between submit and publish neither re-judges nor voids a pending approval; the pricing evaluated-once rule, adopted) - `inst-gv-gate`
2. [ ] - `p1` - `GovernedLiveOp` apply asks the same question with the op envelope as subject; the expected-target-state check (`STALE_LIVE_OP`) is 02's, the quorum answer is this slice's - `inst-gv-liveop-gate`
3. [ ] - `p1` - **System-signal subjects**: a publish whose sole content is a system-owned flag cleared by an inbound governed signal (today: 06's `compositionPending` clearing) uses subject kind `system_signal` (**P-D-14**) — the `ApprovalRecord` is auto-satisfied with the **signal reference as the authorizing principal**, audited like any decision; no human approver, and no exemption from this gate. The head must be **clean**: a `system_signal` publish carries the flag and nothing else, and is not carried out rather than carrying unpublished bucket-iii/iv edits out under a record with no human approver. *Whether that means a refusal or a deferral is an open owner question registered with P-D-14: this row states the guarantee, slice 06 `inst-cc-clear` says "deferred, never refused", and P-D-14 said refused.* `N` has no standing over it — the principal is not a tenant principal — so `N = 0` neither weakens nor strengthens it. Consuming a satisfied approval is one-shot: the publish/apply marks it `consumed` **in the same transaction as the authorized act** — a failed attempt consumes nothing (M-5: the `ActivationRunner`'s replay after a crash rides the Foundation idempotency store keyed by transition id, so a post-commit crash replays the stored outcome instead of re-consuming). One approval never authorizes two **human acts** — but a **scheduled act is one composite act** (H-2 fix, the P-D-02 extension): the retirement approval authorizes initiation *and* the `effectiveAt` flip, a cascade approval authorizes the whole `CascadePlan` including its per-child legs, and a **bulk batch is one composite act** (09: consumed by the `approved → committing` flip, per-row publishes pinned to the ledger); the approval is consumed at the initiation transaction, and the later mechanical stages execute the already-approved act without re-entering this gate. **Mechanically that is 01's `PreAuthorized(approvalId)` door mode** (Blocking 9 fix, 2026-08-26 review): the stage names the consumed record, the gate verifies it authorized this subject at this pinned revision instead of demanding a `satisfied` one, and consumes nothing further. Stating only "without re-entering this gate" left slice 04's "drives the ordinary Foundation publish door" reading a `consumed` record and failing every scheduled publish terminally. The runner still re-validates fail-closed — staleness supersedes, it never re-approves - `inst-gv-one-shot`

### Read the pending queue (the studio inbox contract)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-queue`

1. [ ] - `p1` - `GET /bss-products/v1/approvals?state=pending` returns the common inbox envelope — `{subjectRef, subjectKind, state, submitter, submittedAt, quorum: {required, satisfied, financeRequired, predicateUnsatisfiable, quorumReduced, configuredQuorum}}` (`required` is **the `ApprovalRecord`'s effective count** — `N` for a material change, `min(N, 1)` for a non-material one per `inst-gv-materiality` — never the raw configured `N`, so a card cannot show "2 required" for a record that closes on one; `configuredQuorum` carries the raw `N` when a surface needs it. Heterogeneous quorums therefore render per card and parity with pricing's queue is a configuration rather than a schema question — P-D-11) + the per-kind diff payload — deliberately merge-compatible with pricing's queue so the studio renders one inbox with per-kind cards (the PRD-era UI requirement recorded at design time) - `inst-gv-queue`

### Break-glass elevation (read/audit-export only)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-breakglass`

1. [ ] - `p1` - Elevation opens a `BreakGlassSession`: mandatory reason, time-boxed window (configured), scope named (which tenant); itself **two-person-approved or post-hoc-reviewed** — and this "two-person" is a **fixed floor of two distinct platform principals, outside the tenant's configured `N` entirely** (P-D-13: the acting principal is a platform owner and the subject is another tenant's data, so no tenant configuration has standing over it; the post-hoc-review arm is the escape the floor needs, so the floor blocks nobody) — (both paths recorded; the post-hoc path raises the review obligation as an alert, not a silent log line); `BreakGlassElevated` emitted + a distinct alert channel - `inst-bg-open`
2. [ ] - `p1` - Under elevation: cross-tenant **read and audit-export only**; every access is individually audited with the session id, reason, and correlation id; any write attempt is refused `BREAKGLASS_WRITE_FORBIDDEN` — no exception in v1 (C5) - `inst-bg-readonly`
3. [ ] - `p1` - Expiry is hard: past the window every elevated call fails `BREAKGLASS_EXPIRED`; `BreakGlassExpired` emitted; standing cross-tenant access is not grantable in the catalog at all — the grant model has no such shape - `inst-bg-expiry`

## 3. Processes / Business Logic

### 3.1 Materiality mechanics

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-materiality`

1. [ ] - `p1` - Inputs: (a) the **`BucketRegistry`** — a **Foundation** artifact named in 01 §1.7 beside `RegisteredValidator` (**P-D-28**: a slice registers its columns' bucket tags exactly as it registers validators, code not config; the Foundation's head-door guard and this slice's materiality judgement read the same registry) — bucket-iii fields registered by their slices (03: `PlanTier`, `taxCategory`, `glCode`, `sellable`; 01: frame) make any touch material; the FR-enumerated **metering-unit** field is bucket ii — it reaches publish only through the slice-07 correction door (itself `N`-governed with the reduction recorded — P-D-13), so the evaluator never sees it as an ordinary touch (L-1); (b) the PRD-enumerated ops — lifecycle transitions **to `published`/`deprecated`/`retired`** (the FR's exact enumeration — `draft→discarded` is outside it and stays ungated beyond its own authz, M-1), category create/rename/re-parent/retire/delete, material attribute-definition changes; (c) the affected-entity count ≥ the configured trigger (interim 10) for batch acts (09); (d) **`GovernedLiveOp` kinds registered material by their owning slice** (H-1 fix): 02's taxonomy ops (= the enumeration), 03's recognized-set add/deprecate/remove and `PlanTier` taxonomy ops, 04's `ScheduledTransition` **cancel** ops (the governed retirement abort — `inst-lc-undeprecate`; without this line the evaluator judged it non-material and `inst-gv-materiality` would set `required = min(N, 1)`, one approver at the default, for the only act that unwinds a cascade), 06's freeze-participant membership ops, 07's reference-producer registration ops, 10's PII-allow-list ops. **A registered kind's approver role predicate NARROWS within the C1 base set and never replaces it; v1 registers no extension point that could** (C8, P-D-10 2026-08-26 — this clause previously granted a replacing predicate, with 10's Legal-designated role as its only intended user, flagged as a design reading of AC #35's "Legal sign-off"; the product call retired both, since AC #35 asks for a *recorded* sign-off and not a role) — the PRD/slice-03 phrase "elevated approval" **means exactly this material quorum**, with the FinanceReviewer predicate on the Finance-owned code sets and not on the Product+Rating-owned unit set - `inst-mt-inputs`
2. [ ] - `p1` - The policy object — **field set + trigger + the approver count `N`** (item 36 of the 2026-08-26 review: `N` was omitted, though C1 and P-D-11 both require every later change to it to be material under the then-current quorum, which only holds if it is part of the governed object) — is a `GovernedLiveOp` subject whose **own mutation is always material** (C4), on its **own** resource pair `materiality_policy × write`, never a config-admin's general grant: pricing builds a separate resource precisely so the holder of a config grant cannot weaken the threshold that governs it - `inst-mt-policy-material`
3. [ ] - `p2` - Evaluated once at submission against the policy in force at the submission instant (never the reader's clock — the pricing D-194 lesson, adopted) - `inst-mt-once`

### 3.2 RBAC catalog

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-rbac`

GTS-typed resources × actions, deny-by-default: `product × read|write|publish`,
`sku × read|write|publish`, `category × read|write`, `attribute_definition × write`,
`recognized_set × write`, `plan_tier × write`, `approval × submit|read|decide`,
`scheduled_transition × write|cancel|read` (04's doors + the 08 dashboard projection — M4; the
governed cancel is a `GovernedLiveOp` subject kind on `ApprovalRecord`), `catalog_version × read|publish|request|ack|release|force_complete` (the `release` action is **P-D-18**'s door) (06's doors: S2S
request/ack/release via service-identity claims, operator publish/force-complete),
`freeze_participant × write` (06), **`metadata × write`** (02's metadata-map door — added
2026-08-26: the door existed with no pair, and P-D-06 makes the map mutable in place on a
**published** entity with no version bump, so inheriting `sku × write` would let anyone who can
author drafts mutate content a `CatalogVersion` captures), **`materiality_policy × write`** (the C4/P-D-11 object —
field set + trigger + `N`; separate from every config grant so the threshold's own holder cannot
weaken it, item 36 of the 2026-08-26 review), `compliance × export` (10's DSAR surface — never
folded into `audit × export`),
`reference_signal × post` + `reference_producer × write` + `sku × correct` (07's doors),
`erasure × execute` + `pii_allowlist × write` (10's doors),
`bulk × execute` (09 import) + `bulk_lifecycle × execute` (09's mass-retire lane — its own grant), `breakglass × elevate`, `audit × read|export` (M-4 fix). Role bundles mirror the
PRD actors (ProductManager, CatalogAdmin, FinanceReviewer, Auditor, PlatformOwner); grants are
tenant-scoped claims from the IdP — the registry never mutates tenant topology. Every door
names its pair; slice 12's coverage check asserts no door is unnamed.

### 3.3 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-governance-errors`

`SELF_APPROVAL_FORBIDDEN`, `APPROVER_SCOPE_EXCEEDED`, `APPROVER_ROLE_REQUIRED` (raised by the
**gate** when the descriptor is numerically met but the role predicate is not — L-2),
`APPROVAL_SUPERSEDED` (raised at **decide** on a superseded record — L-2),
`BREAKGLASS_WRITE_FORBIDDEN`, `BREAKGLASS_EXPIRED`. `APPROVAL_REQUIRED` stays 01's (raised
through the gate).

**Problem responses (RFC 9457):** `SELF_APPROVAL_FORBIDDEN`, `BREAKGLASS_WRITE_FORBIDDEN`, `BREAKGLASS_EXPIRED`, `APPROVAL_REQUIRED`, `APPROVER_SCOPE_EXCEEDED`, `APPROVER_ROLE_REQUIRED` (403); `APPROVAL_SUPERSEDED` (409).

*Statuses added 2026-08-26, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, 2026-08-02, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"* — the citation was right the first time;
a 2026-08-26 pass re-pointed it at D-186 and was wrong to, D-186 being a later amendment scoped to
one config route) and where an earlier pass here wrongly wrote
412 and called that pricing's convention — **403** where the caller may not perform the act at
all, **404** only where a path segment names a resource this tenant has none of. **503** where retry
is the remedy is this gear's own addition — pricing's set carries no 503 at all, so that one
class is not "checked against it". **The 422s here are architectural, not wire** — see 01 §3.3, which quotes the sibling
plan-price gear's rule (the `MUST NOT` being this gear's own choice, 01 §3.3): no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for an error **carrying a registry code** in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.
  Codes listed here for the response map but **declared elsewhere**: `APPROVAL_REQUIRED` (slice 01) — the status is repeated, not a second declaration, so the one-declaration rule stands.*

## 4. Data / Storage (normative shape; DDL in migrations)

- **`products_approval`** — `approval_id` (PK) · `tenant_id` · subject `(kind, ref)` · pinned
  `internal_revision` · **`content_snapshot`** (stored at submission — never re-derived) ·
  `diff_basis` (the published version id diffed against) · `quorum_descriptor` (**stored at submission, never re-derived** — 2026-08-26: `predicateUnsatisfiable`
  and `configuredQuorum` were required by §3.1 and §4's evaluator and named in neither shape, and
  deriving `configuredQuorum` from current policy would change a **pending** record when the
  tenant edits `N`) — `configuredQuorum` (the `N` in force at submission), required count,
  finance predicate, override conditions, **`quorum_reduced`** — P-D-13) · `state ∈ {pending, satisfied, consumed, rejected,
  superseded}` · `submitter` (pseudonymous) · timestamps. Partial `UNIQUE (tenant_id,
  subject_kind, subject_ref) WHERE state IN ('pending','satisfied')` — one open approval per
  subject; a new submission explicitly supersedes the open one (L-4). Append-only after
  finalization.
- **`products_approval_decision`** — `(approval_id, approver_principal)` UNIQUE · verdict ·
  reason · override acknowledgments · instant. The UNIQUE is C2's physical floor: one principal,
  one decision.
- **`products_breakglass_session`** — session id · principal (**as `actor_ref`** — pseudonymous like every actor-bearing store, M5 of the slice-10 review) · target tenant · reason ·
  window `[from, until)` · approval path (`two_person` ref | `post_hoc` obligation state) ·
  timestamps. Elevated audit rows carry the session id.
- **Events**: `ApprovalDecided` (both verdicts), `BreakGlassElevated`, `BreakGlassExpired` —
  broker-native; submissions/supersessions are audit-plane (explicit "no broker event": the
  queue is a pull surface, and every submission already rides the entity's own audit row).

## 5. Testing posture (slice-local)

- **The stored-snapshot probe is the flagship**: submit → edit the head → the superseded
  record's diff still renders the ORIGINAL submission against the published version (a
  re-derived diff would show the draft against itself — the exact pricing defect, RED first).
- One-human-two-roles probe: a principal holding CatalogAdmin + FinanceReviewer counts once
  (C2), and the finance predicate is satisfiable by that one principal only as one of the two.
- Self-approval refusal + positive control; out-of-scope approver refusal + in-scope control.
- One-shot consumption: two publishes off one satisfied approval — second fails.
- Break-glass: write attempt under elevation refused; access after expiry refused; every
  elevated read leaves an audit row with the session id (count asserted, not sampled).
- Materiality: bucket-iii touch ⇒ material; bucket-iv-only re-publish ⇒ single approver;
  policy-object mutation ⇒ material regardless of direction.

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-usecase-approval-publish` (§10 use case, claimed by id here 2026-08-26 — all seven were in lint 1's universe and none was claimed); `cpt-cf-bss-products-fr-materiality-gated-publish`, `cpt-cf-bss-products-fr-tenant-isolation-breakglass`,
`cpt-cf-bss-products-fr-breakglass-action-scope`; AC #26, #30, #31; §17.1 (interim default, enforced); P-D-02
(ceremony); consumed by 01 (`inst-fd-governance-gate`), 02 (`GovernedLiveOp`), 03 (override,
finance fields), 04 (un-deprecation, retirement confirmation, scheduled-approval pinning).

**Risks & open items**:
- **Is `BREAKGLASS_WRITE_FORBIDDEN` raised at the pre-pipeline authorization gate or inside a
  pipeline phase?** **Closed by P-D-36 (2026-08-28)**: the phase unit is withdrawn, so where the
  code is *raised* no longer carries a taxonomy consequence and no carve-out depends on it. What
  remains is ordinary and already settled — this slice declares `BREAKGLASS_WRITE_FORBIDDEN`, as it
  declares `APPROVER_ROLE_REQUIRED` and `APPROVAL_SUPERSEDED`. *(Raised by the slice-01 third lens
  wave, 2026-08-27.)*
- **Does the discard door get its own grant, or inherit `product|sku × write`?** 01 §2 declares
  `POST /bss-products/v1/{products|skus}/{id}/discard` under **`… × discard`**, and this slice's
  RBAC catalog carries only `product × read|write|publish` and `sku × read|write|publish` — so
  12 `inst-cc-rbac` ("every REST/S2S door named in any slice appears in 05's RBAC catalog") has
  no pair to match on the very door P-D-31 added to make that lint green. `inst-gv-materiality`
  already settles that `draft→discarded` is ungated beyond authz, so this is the grant model
  only: minting `discard` lets a tenant withhold it, folding it into `write` does not. Owner:
  this slice (`cpt-cf-bss-products-contract-rbac`). *(Raised by the slice-01 sixth-pass review,
  2026-08-27.)*
- **What code does an authorization denial carry, and what status?** Every door in 01 §2 opens by
  authorizing deny-by-default, 01 §1.5 puts RBAC grants in this slice, and no slice declares a
  denial code — while 01 §3.3 requires every code to carry a status and 01 §4.4 requires every
  refusal to be audited with its reason. So the first step of every registry door terminates in a
  refusal with no code for a consumer to match on. Owner: the governance owner with the taxonomy
  owner. *(Raised by the slice-01 fifth-pass review, 2026-08-27.)*
- **Quorum strictness — RESOLVED 2026-08-26 (P-D-11)** (was: flagged). The count became a
  typed-policy value with default 2 and **floor 0**, after the measurement that settled it: the
  old fixed `≥ 2` left a two-person tenant unable to publish any material change and a
  one-person tenant unable to publish **anything** (first publish and every lifecycle
  transition are material, a non-material change still needed one approver, and C2 forbids
  self-approval), while the sibling plan-price gear ships `submitter + 1` in its schema
  (`pricing_approval` carries `submitter_principal`/`approver_principal` as two columns under
  `chk_pricing_approval_distinct_principals`) **and** an approver-less path
  (`PublishAuthorization::AutoPublishable`) for below-threshold non-first publishes. What did
  **not** become configurable: the FinanceReviewer predicate, the self-approval refusal at
  `N ≥ 1`, the explicit-configuration requirement, and the provisioning-time origin of the
  initial value.
- **The studio inbox envelope** is design-introduced (deliberately merge-compatible with
  pricing's queue); pricing's queue shape should be cross-checked when slice 12 pins the SDK —
  a field-name drift here costs a UI adapter later.
- **Post-hoc break-glass review** needs an owner and an SLA for the review obligation alert —
  operational, not structural; noted for the ops runbook.
- Approval retention/erasure interplay (approver principals are pseudonymous refs) is slice
  10's; this slice only guarantees the refs are pseudonymous from birth.
- **Does the authoring head read need an action of its own in the RBAC catalog?** 01 §2's `GET` is
  an authoring read, and 01 §4.3 says that read "is not a consumer read", while this slice's
  catalog lists only `read|write|publish` per kind. Owner: this slice. *(Filed from 01 §6 by the slice-01 eighth lens pass, 2026-08-28 — the pointer claimed it was registered here and it was not.)*
- **C3's no-hook exception is still worded `draft→published` only.** C3 reads "except
  `draft→published`, which the publish door owns"; **P-D-34** widened the exception to any
  transition consuming an approval in the same transaction. As written, the invalidation hook fires
  on `deprecated→published` — the gated edge P-D-30 put the gate phase on — against 01 §2, which
  says it must not. Owed: the restatement. *(Filed from 01 §6 by the slice-01 eighth lens pass, 2026-08-28 — the pointer claimed it was registered here and it was not.)*
- **What does `Gate` mode require of a gated transition?** 01 `inst-fd-gate-mode-gate` is worded for
  a publish and pins "the door's expected revision", while the transition doors are this slice's and
  04's and pin nothing stated in 01. Owner: this slice with 04. *(Filed from 01 §6 by the slice-01 eighth lens pass, 2026-08-28 — the pointer claimed it was registered here and it was not.)*
