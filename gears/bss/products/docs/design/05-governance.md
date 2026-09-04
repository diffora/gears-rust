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
| `cpt-cf-bss-products-actor-catalog-admin` | Approver |
| `cpt-cf-bss-products-actor-finance-reviewer` | The mandatory second lens on finance-material fields; approval-queue consumer |
| `cpt-cf-bss-products-actor-auditor` | Reads the immutable approval/audit/break-glass trails |
| `cpt-cf-bss-products-actor-platform-owner` | Break-glass **initiator and acting principal** — two distinct platform principals, or post-hoc review (`inst-bg-open`); cross-tenant read/audit-export only (v1) |

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
  approvers against pricing's structural submitter + one — pricing's count is **structural** rather than
  configured — two columns under `chk_pricing_approval_distinct_principals`, plus an approver-less
  `AutoPublishable` path — so the two gears differ by default **and** by mechanism (P-D-11)
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
| C1 | Quorum for a material change: the tenant's configured **`N` distinct approvers, each distinct from the author**, each holding CatalogAdmin or FinanceReviewer — `N` a typed-policy value, **default 2, floor 0** (P-D-11). Not configurable: finance-material fields (`taxCategory`, `glCode`, `PlanTier`) require ≥ 1 FinanceReviewer **among** the approvers (the predicate governs who, not how many) and the record states the predicate as **unsatisfiable at `N = 0` only** — where there are no approvers to hold the role, so the descriptor stays satisfiable rather than blocking on a role no principal could hold. **At every `N >= 1` the predicate binds and a tenant that has designated no FinanceReviewer simply has an unapprovable change, which is correct** (`inst-gv-finance-predicate` is the normative arm; this row previously read "when the configured `N` cannot carry it", which an implementer could read as *the available approvers cannot satisfy it* and build into a finance-review fail-open at `N = 2` — Blocking 2 of the review); self-approval refused at every `N ≥ 1`; `N` reachable only by explicit configuration, absent ⇒ default; initial `N` from tenant provisioning, later changes material under the then-current quorum (C4) | PRD `fr-materiality-gated-publish`; P-D-11 |
| C2 | Distinctness is by **principal**, never by role: one human holding both roles is one approver | pricing G2, adopted |
| C3 | An approval pins the internal revision AND stores the submitted content snapshot; **any frozen-content write — save OR lifecycle transition — bumps `internal_revision` and fires the invalidation hook**, **except any transition that consumes an approval in the same transaction — `draft→published`, which the publish door owns, and every gated edge P-D-30 put the gate phase on — which bumps once and fires no hook** (**P-D-26**, extended by **P-D-34** — the same transaction consumes the approval, and a hook firing against the record the act is consuming has no defined ordering) (M-2 fix: head-at-revision-N is therefore byte-identical to the snapshot at revision N; transition-written columns cannot drift under a pinned approval), superseding open approvals and re-queuing with the diff re-presented | PRD `fr-materiality-gated-publish` |
| C4 | Materiality is a typed, configurable policy with the §17.1 interim default enforceable at launch; **the policy's own mutation is material** (the two-person rule's foundation must not be single-person-editable — the pricing D-10 lesson, adopted) | PRD §17.1 |
| C5 | Break-glass (§6.8) is **read + audit-export only** in v1; any write under elevation is refused, full stop. **Canonical boundary (M-3):** slice 07's flag-gated correction write does **not** run under a `BreakGlassSession` — a §6.8 session never authorizes a write; 07 reuses only the elevation *ceremony shape* (the `N`-governed quorum with `quorumReduced` recorded + mandatory reason + recording — P-D-13's sixth site) as its own distinct mechanism | PRD `fr-breakglass-action-scope` |
| C6 | Deny-by-default: no grant, no access; grants are GTS-typed resource×action pairs checked at every door | PRD `fr-tenant-isolation-breakglass` |
| C7 | Audit **sealing** is a platform capability, not this gear's: pricing's G4/D-14 in-gear hash chain is deliberately **not** adopted. v1 writes the append-only trail 01 §4.4 scopes to refusals, reads under elevation, and committed acts declared to emit no broker event (**P-D-21**) — the event stream being the record for everything that succeeds over a **reserved, unwritten** sealing seam; the requirements the platform capability must satisfy are P-D-08 S1–S9, carried as a PRD §15 open owned by Architecture. Until activation, audit immutability is the trigger whitelist on both engines (01 **P-D-35** made the `REVOKE` arm Postgres-only; **P-D-46** withdrew it outright) and nothing cryptographic — completeness ships, tamper-evidence does not | P-D-08 |
| C8 | Role predicates **narrow within** the C1 base set; they never replace it, and v1 registers **no** extension point that could (P-D-10). `inst-gv-finance-predicate` is the only one and it is additive — C1 already demands CatalogAdmin-or-FinanceReviewer, the predicate demands that one of the two *be* a FinanceReviewer. A predicate that replaced the base set would be a bypass surface: register a kind whose predicate admits anyone and a material change passes on one signature. Any future replacing predicate therefore owes two guards — the numeric quorum still binds, and registering or changing a kind's predicate is itself material (as C4 already makes the materiality policy's own mutation) | P-D-10 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `MaterialityEvaluator` | Decides material / non-material for a change set: bucket-iii field touches (registered by owning slices), the PRD-enumerated ops, affected-entity count ≥ the configured trigger, or a `GovernedLiveOp` kind registered material by its owning slice (§3.1(d)) |
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

Declared by [`../features/governance.md`](../features/governance.md) §2 as `cpt-cf-bss-products-flow-submit`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Submission runs `MaterialityEvaluator` over the change set (head-row pending edits vs last published version; for a `GovernedLiveOp`, the op payload): **material** ⇒ quorum descriptor per C1 (`required = N`); **non-material** ⇒ `min(N, 1)` — so a tenant at `N = 0` publishes approver-less by policy and the record says exactly that, which is the point of P-D-11 and replaces the earlier "nothing publishes approver-less" interim; first publish and every lifecycle transition **to `published`/`deprecated`/`retired`** are material at their **initiating human act** — the mechanical stages of an approved scheduled act (the `effectiveAt` flip, cascade legs) re-enter the gate only in 01's `PreAuthorized(approvalId)` mode, consuming nothing further (H-2, see `inst-gv-one-shot`); a bucket-iv-only re-publish is the non-material case (a re-publish is not a transition, 01's head-row model), and `draft→discarded` is ungated beyond authz (M-1) - `inst-gv-materiality`
2. [ ] - `p1` - The `ApprovalRecord` **stores the submitted content snapshot** at submission time — the diff shown to approvers is rendered from the STORED snapshot against the last published version, never re-derived from the live head (the pricing pinned-content defect, designed out) - `inst-gv-stored-snapshot`
3. [ ] - `p1` - Finance-material field touches (`taxCategory`, `glCode`, `PlanTier` — the bucket registry marks them) set the FinanceReviewer predicate on the quorum descriptor **when `N ≥ 1`**. At **`N = 0`** the predicate has no subject — there are no approvers to hold the role — so it is **not set**; instead the descriptor records `predicateUnsatisfiable = finance_reviewer` and stays satisfiable. Without this arm the descriptor would demand a role no principal could hold and `inst-fd-governance-gate` would raise `APPROVAL_REQUIRED` forever, re-blocking exactly the tenant P-D-11 unblocked: `taxCategory` is required at publish for product/service types, so a one-person tenant's first such SKU would be unpublishable (the hole CodeRabbit found in the P-D-11 wave). `N ≥ 1` is unaffected — a lone approver on a finance-material change must be a FinanceReviewer, and a tenant that has designated none simply has an unapprovable change, which is correct - `inst-gv-finance-predicate`
4. [ ] - `p1` - A frozen-content write on the subject after submission (save or transition — C3) fires 01's `inst-fd-approval-hook`: the record flips `superseded`, and **re-submission is an explicit human act** (the submitter or any write-granted principal; never automatic — auto-resubmit would pin content nobody re-read, L-3) with the new diff re-presented — approvers never decide on stale content - `inst-gv-supersede`

### Decide (approve / reject)

Declared by [`../features/governance.md`](../features/governance.md) §2 as `cpt-cf-bss-products-flow-decide`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `approval × decide` grant required; at every `N ≥ 1` (C1) the author's own decision is refused `SELF_APPROVAL_FORBIDDEN` — by principal, not role (C2) - `inst-gv-self`
2. [ ] - `p1` - The approver's brand/region claims MUST cover the subject's scope, read with 01 **P-D-39**'s two
  boundaries (the scope columns are `NOT NULL` and the **empty set means unrestricted**): an
  unrestricted claim set covers every subject, an unrestricted subject scope is covered only by an
  unrestricted claim set, and between two non-empty sets it is ordinary subset; an out-of-scope decision is refused `APPROVER_SCOPE_EXCEEDED` and audited like any scope violation (the analogue of pricing's approver-scope rule; the donor id struck per **P-D-43**) - `inst-gv-scope`
3. [ ] - `p1` - Each decision appends a `products_approval_decision` row (approver principal, verdict, mandatory reason on reject, instant); a decision arriving on a record already `superseded` is refused **`APPROVAL_SUPERSEDED`** (§3.3); `QuorumEvaluator` answers "satisfied" only when the descriptor is met by **distinct principals** with the required roles — a predicate recorded `predicateUnsatisfiable` (no subject at the configured `N`) counts as met **for the evaluator** while remaining visible as unmet-by-policy in the record and the inbox envelope; that is the only way it may be discharged, and it is never how a predicate is discharged at `N ≥ 1` - `inst-gv-quorum`
4. [ ] - `p1` - A rejection finalizes the record `rejected` with the reason; the subject stays as it was (01 §6 files AC #26's literal "returns the entity to `draft`" as an open question against the head-row model — `PRD` §15, unanswered; this slice's reading is that a first-publish draft stays draft and a published head keeps its pending edits unpublished); `ApprovalDecided` emitted either way; the reason passes 02's `inst-av-pii-block` before the row is written, a hit failing `CONTENT_PII_BLOCKED` (**P-D-50**; 02 `inst-av-pii-reason` enumerates this door) - `inst-gv-reject`
5. [ ] - `p1` - **`OverrideCeremony`** (P-D-02): when the subject carries named override conditions (an uncomposed `bundle` at its entity publish — 03 `inst-cl-bundle-override`), each approver explicitly acknowledges the lint findings by name; the acknowledgments are stored on the record and in audit — an informed override, never a blind one. **At `N = 0` the author performs the acknowledgment** and the record carries `quorumReduced` (P-D-13): the ceremony's product is an informed decision, so it survives an empty quorum instead of vanishing with it. The record is also the ceremony's only home — a lane that publishes an override subject without one is a design defect, not an exemption (see 09 `inst-bk-override`) - `inst-gv-override`

### Publish / apply against the gate

Declared by [`../features/governance.md`](../features/governance.md) §2 as `cpt-cf-bss-products-flow-gate`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - **The Foundation's gate phase runs on any gated act, not on publish alone** (**P-D-30**) — which is what hosts this slice's gate on 04's un-deprecation edge: a transition door consumes the `satisfied` record exactly as the publish door does, and `inst-gv-one-shot`'s same-transaction flip binds identically there. Separately, **authorization is a pre-pipeline gate rather than a phase** (same call), run before 01's pipeline opens so a denied caller neither consumes an idempotency key nor writes a claim row; the denial's code and status are this slice's to declare, and remain open below. The Foundation gate (01 `inst-fd-governance-gate`) asks this slice: an `ApprovalRecord` in state `satisfied` (`superseded` being a value of the same column, §4) whose pinned revision equals the door's expected revision ⇒ yes; a stage entering in 01's `PreAuthorized(approvalId)` mode naming a **`consumed`** record that
  authorized this subject at this pinned revision ⇒ yes, consuming nothing further (`inst-gv-one-shot`);
  a record whose decisions numerically meet `required` while the role predicate is unmet ⇒
  **`APPROVER_ROLE_REQUIRED`** (§3.3); anything else ⇒ `APPROVAL_REQUIRED` (or `STALE_REVISION`
  upstream). On **yes** the verdict returns the authorizing record's id **and whether it carried the
  uncomposed-bundle override acknowledgment** — 01 `inst-fd-gate-verdict`'s `composition_pending`
  operand. The gate never re-evaluates materiality at publish — the verdict was fixed at submission (a policy change between submit and publish neither re-judges nor voids a pending approval; the pricing evaluated-once rule, adopted) - `inst-gv-gate`
2. [ ] - `p1` - `GovernedLiveOp` apply asks the same question with the op envelope as subject; the expected-target-state check (`STALE_LIVE_OP`) is 02's, the quorum answer is this slice's - `inst-gv-liveop-gate`
3. [ ] - `p1` - **System-signal subjects**: a publish whose sole content is a system-owned flag cleared by an inbound governed signal (today: 06's `compositionPending` clearing) uses subject kind `system_signal` (**P-D-14**) — the `ApprovalRecord` is auto-satisfied with the **signal reference as the authorizing principal**, audited like any decision; no human approver, and no exemption from this gate. The head must be **clean**: a `system_signal` publish carries the flag and nothing else, and on a dirty head is **deferred, never refused** — held until the head is clean rather than carrying unpublished bucket-iii/iv edits out under a record with no human approver (P-D-14 as confirmed by **P-D-48**; slice 06 `inst-cc-clear` states the mechanics). `N` has no standing over it — the principal is not a tenant principal — so `N = 0` neither weakens nor strengthens it. Consuming a satisfied approval is one-shot: the publish/apply marks it `consumed` **in the same transaction as the authorized act** — a failed attempt consumes nothing (M-5: the `ActivationRunner`'s replay after a crash rides the Foundation idempotency store keyed by transition id, so a post-commit crash replays the stored outcome instead of re-consuming). One approval never authorizes two **human acts** — but a **scheduled act is one composite act** (H-2 fix, the P-D-02 extension): the retirement approval authorizes initiation *and* the `effectiveAt` flip, a cascade approval authorizes the whole `CascadePlan` including its per-child legs, and a **bulk batch is one composite act** (09: consumed by the `approved → committing` flip, per-row publishes pinned to the ledger); the approval is consumed at the initiation transaction, and the later mechanical stages execute the already-approved act without re-entering this gate. **Mechanically that is 01's `PreAuthorized(approvalId)` door mode** (Blocking 9 fix): the stage names the consumed record, the gate verifies it authorized this subject at this pinned revision instead of demanding a `satisfied` one, and consumes nothing further. Stating only "without re-entering this gate" left slice 04's "drives the ordinary Foundation publish door" reading a `consumed` record and failing every scheduled publish terminally. The runner still re-validates fail-closed — staleness supersedes, it never re-approves - `inst-gv-one-shot`

### Read the pending queue (the studio inbox contract)

Declared by [`../features/governance.md`](../features/governance.md) §2 as `cpt-cf-bss-products-flow-queue`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `GET /bss-products/v1/approvals?state=pending` (`approval × read`) → **200** returns the common inbox envelope — `{subjectRef, subjectKind, state, submitter, submittedAt, quorum: {required, satisfied, financeRequired, predicateUnsatisfiable, quorumReduced, configuredQuorum}}` (`required` is **the `ApprovalRecord`'s effective count** — `N` for a material change, `min(N, 1)` for a non-material one per `inst-gv-materiality` — never the raw configured `N`, so a card cannot show "2 required" for a record that closes on one; `configuredQuorum` carries the raw `N` when a surface needs it. Heterogeneous quorums therefore render per card and parity with pricing's queue is a configuration rather than a schema question — P-D-11) + the per-kind diff payload — deliberately merge-compatible with pricing's queue so the studio renders one inbox with per-kind cards (the PRD-era UI requirement recorded at design time) - `inst-gv-queue`

### Break-glass elevation (read/audit-export only)

Declared by [`../features/governance.md`](../features/governance.md) §2 as `cpt-cf-bss-products-flow-breakglass`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Elevation opens a `BreakGlassSession`: mandatory reason, time-boxed window (configured), scope named (which tenant); itself **two-person-approved or post-hoc-reviewed** — and this "two-person" is a **fixed floor of two distinct platform principals, outside the tenant's configured `N` entirely** (P-D-13: the acting principal is a platform owner and the subject is another tenant's data, so no tenant configuration has standing over it; the post-hoc-review arm is the escape the floor needs, so the floor blocks nobody) — (both paths recorded; the post-hoc path raises the review obligation as an alert, not a silent log line); `BreakGlassElevated` emitted + a distinct alert channel; the reason passes 02's `inst-av-pii-block` before the row is written, a hit failing `CONTENT_PII_BLOCKED` (**P-D-50**; 02 `inst-av-pii-reason` enumerates this door) **The two approvers live on the session row — `approver_a`, `approver_b`, distinct platform principals — and an elevated call substitutes a read-only `AccessScope::for_tenant(target)` in the pre-pipeline gate, every write refused; post-hoc review within `breakglass_review_sla_hours`, 24 interim (P-D-133, 2026-09-04).** - `inst-bg-open`
2. [ ] - `p1` - Under elevation: cross-tenant **read and audit-export only**; every access is individually audited with the session id, reason, and correlation id; any write attempt is refused `BREAKGLASS_WRITE_FORBIDDEN` — no exception in v1 (C5) - `inst-bg-readonly`
3. [ ] - `p1` - Expiry is hard: past the window every elevated call fails `BREAKGLASS_EXPIRED`; `BreakGlassExpired` is emitted **exactly once, by the first post-expiry act, via a CAS flip of the session's `expired_emitted` stamp in the same transaction as its refusal — the winner emits, a replay emits nothing, and a session never touched after expiry emits no event at all, its expiry being a stored fact observable as a gauge with the alerting rule on top** (**P-D-68**, on P-D-54's and P-D-59's mechanisms); **expiry gates admission, not completion** — an elevated read admitted inside the window finishes (P-D-68); standing cross-tenant access is not grantable in the catalog at all — the grant model has no such shape - `inst-bg-expiry` **No renewal (P-D-132, 2026-09-03):** a session is never extended; a second window is a second session and a second two-person ceremony; the window is `breakglass_window_hours`, 4 interim.

## 3. Processes / Business Logic

### 3.1 Materiality mechanics

Declared by [`../features/governance.md`](../features/governance.md) §3 as `cpt-cf-bss-products-algo-materiality`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Inputs: (a) the **`BucketRegistry`** — a **Foundation** artifact named in 01 §1.7 beside `RegisteredValidator` (**P-D-28**: a slice registers its columns' bucket tags exactly as it registers validators, code not config; the Foundation's head-door guard and this slice's materiality judgement read the same registry) — bucket-iii fields registered by their slices (03: `PlanTier`, `taxCategory`, `glCode`, `sellable`; 01: frame) make any touch material; the FR-enumerated **metering-unit** field is bucket ii — it reaches publish through the save door while `published_version = 0` (01 `inst-fd-save-txn`, **P-D-41**), and after first publish only through the slice-07 correction door (itself `N`-governed with the reduction recorded — P-D-13), so the evaluator never sees it as an ordinary touch (L-1); (b) the PRD-enumerated ops — lifecycle transitions **to `published`/`deprecated`/`retired`** (the FR's exact enumeration — `draft→discarded` is outside it and stays ungated beyond its own authz, M-1), category create/rename/re-parent/retire/delete, material attribute-definition changes; (c) the affected-entity count ≥ the configured trigger (interim 10) for batch acts (09); (d) **`GovernedLiveOp` kinds registered material by their owning slice** (H-1 fix): 02's taxonomy ops (= the enumeration), 03's recognized-set add/deprecate/remove and `PlanTier` taxonomy ops, 04's `ScheduledTransition` **cancel** ops (the governed retirement abort — `inst-lc-undeprecate`; without this line the evaluator judged it non-material and `inst-gv-materiality` would set `required = min(N, 1)`, one approver at the default, for the only act that unwinds a cascade), 06's freeze-participant membership ops, 07's reference-producer registration ops, 10's PII-allow-list ops. **A registered kind's approver role predicate NARROWS within the C1 base set and never replaces it; v1 registers no extension point that could** (C8, P-D-10 — this clause previously granted a replacing predicate, with 10's Legal-designated role as its only intended user, flagged as a design reading of AC #35's "Legal sign-off"; the product call retired both, since AC #35 asks for a *recorded* sign-off and not a role) — the PRD/slice-03 phrase "elevated approval" **means exactly this material quorum**, with the FinanceReviewer predicate on the Finance-owned code sets and not on the Product+Rating-owned unit set - `inst-mt-inputs`
2. [ ] - `p1` - The policy object — **field set + trigger + the approver count `N`** (item 36 of the review: `N` was omitted, though C1 and P-D-11 both require every later change to it to be material under the then-current quorum, which only holds if it is part of the governed object) — is a `GovernedLiveOp` subject whose **own mutation is always material** (C4), on its **own** resource pair `materiality_policy × write`, never a config-admin's general grant: pricing builds a separate resource precisely so the holder of a config grant cannot weaken the threshold that governs it - `inst-mt-policy-material`
3. [ ] - `p2` - Evaluated once at submission against the policy in force at the submission instant (never the reader's clock — the pricing D-194 lesson, adopted) - `inst-mt-once`

### 3.2 RBAC catalog

Declared by [`../features/governance.md`](../features/governance.md) §3 as `cpt-cf-bss-products-algo-rbac-catalog`.
The catalog table below is this slice's and is the normative one; the FEATURE carries the obligation and the boundary.

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-rbac`

GTS-typed resources × actions, deny-by-default. **The `Doors` column is what lint 3 reads**
(**P-D-45**), for the reason P-D-43 gave lint 9's `Operand`: the pairs and the doors were both
prose, and the join between them existed in no artifact. **A cell is per action** (**P-D-50**):
where a row holds several actions and a declared route spends only one, the cell names the route
and the action it spends, and says which of the row's actions still have none — a bare route in a
multi-action row would otherwise read as if the whole row were doored.

| Resource × action | Door(s) | Slice |
|---|---|---|
| `product × read`, `sku × read` | `GET /bss-products/v1/{products\|skus}/{id}`, `GET /bss-products/v1/{products\|skus}/{id}/versions` | 01, 08 |
| `product × write`, `sku × write` | `POST /bss-products/v1/products`, `POST /bss-products/v1/skus`, `PATCH /bss-products/v1/{products\|skus}/{id}`, `POST /bss-products/v1/{products\|skus}/{id}/clone`, `POST /bss-products/v1/{products\|skus}/{id}/discard` | 01, 11 |
| `product × publish`, `sku × publish` | `POST /bss-products/v1/{products\|skus}/{id}/publish` | 01 |
| `metadata × write` | `PATCH /bss-products/v1/{products\|skus}/{id}/metadata` | 02 |
| `compliance × export` | `GET /bss-products/v1/compliance/identity-export`, and since `dd464c108` `GET /bss-products/v1/compliance/pii-allowlist` — the allow-list review **is** the compliance surface (P-D-117 item 12); both require a justification and write an access row (P-D-133) | 10 |
| `erasure × execute` | `POST /bss-products/v1/erasure-requests` | 10 |
| `bulk × execute` | `POST /bss-products/v1/bulk/imports` | 09 |
| `bulk × read` | `GET /bss-products/v1/bulk/batches/{batchId}` (**P-D-61** — the `RowLedger` reader C1 requires; one route for both lanes, and a reader is not an executor) | 09 |
| `bulk_lifecycle × execute` | `POST /bss-products/v1/bulk/lifecycle` | 09 |
| `catalog_version × request` | `POST /bss-products/v1/catalog-version-requests` | 06 |
| `catalog_version × ack`, `× release` | `POST /bss-products/v1/catalog-versions/{catalogVersionId}/acks` and `…/releases` (**P-D-67** — S2S under the participant's identity; `release` is **P-D-18**'s door) | 06 |
| `catalog_version × read`, `× force_complete` | `GET /bss-products/v1/bulk/exports?catalogVersionId=` (09's export door) spends **`× read`** (**P-D-50**); **`× force_complete`** is `POST /bss-products/v1/catalog-versions/{catalogVersionId}/force-completions` (**P-D-67** — the operator ceremony's door) | 06 |
| ~~`catalog_version × publish`~~ | **struck (P-D-125, 2026-09-03)** — no door consumes it by design: the operator lane is the request door, and the code roster carries four `catalog_version` grants without it | 06 |
| `category × read\|write` | `GET /bss-products/v1/browse…` (08's browse door, which names `category × read` explicitly) spends **`× read`** (**P-D-50**); **`× write`** is doored by **P-D-106** — `POST /bss-products/v1/categories`, `POST /bss-products/v1/categories/{categoryId}/operations` (the taxonomy ops) and `PATCH /bss-products/v1/categories/{categoryId}/attribute-values` (the live-value door) | 02 |
| `attribute_definition × write` | **P-D-106**: `POST /bss-products/v1/attribute-definitions` and `POST /bss-products/v1/attribute-definitions/{key}/operations` | 02 |
| `recognized_set × write`, `plan_tier × write` | `POST /bss-products/v1/recognized-sets/{setKind}/members` and `POST /bss-products/v1/recognized-sets/{setKind}/members/{memberCode}/transitions` (**P-D-90** — one route family, the grant chosen by `setKind`; both spelled in full because an elided span is invisible to a route census) | 03 |
| `approval × submit\|read\|decide` | `GET /bss-products/v1/approvals?state=pending` (this slice's own pending-queue door) spends **`× read`** (**P-D-50**); `POST /bss-products/v1/approvals` spends **`× submit`** and `POST /bss-products/v1/approvals/{approvalId}/decisions` spends **`× decide`** (**P-D-120**, strand B, `c1b86fcbb`) | 05 |
| `materiality_policy × write` | `PUT /bss-products/v1/materiality-policy` (**P-D-112** — strand B's first link, `8cc41aa73`; deliberately no read route, `× read` being the authoring read per P-D-134 row 7) | 05 |
| `breakglass × elevate` | `POST /bss-products/v1/breakglass-sessions` (**P-D-120**, strand B, `c1b86fcbb`; the pre-pipeline elevation gate reads the session id from a header — P-D-133) | 05 |
| `scheduled_transition × write\|cancel\|read` | **P-D-134**: `GET /bss-products/v1/scheduled-transitions` (`× read`) and `POST /bss-products/v1/scheduled-transitions/{id}/operations` with `op: cancel` (`× cancel`) — 04's doors, strand C's build, **built** (`5da022f6f`); `× write` is not minted, the retire doors writing the rows under `sku × write` / `product × write` (P-D-135) | 04 |
| `freeze_participant × write` | `POST /bss-products/v1/freeze-participants` (**P-D-67** — the governed set write) | 06 |
| `reference_signal × post`, `reference_producer × write`, `sku × correct` | **no route declared** — 07's watermark, producer and correction doors, named in prose | 07 |
| `pii_allowlist × write` | `POST /bss-products/v1/pii-allowlist-entries` and `POST /bss-products/v1/pii-allowlist-entries/{entryId}/operations` (strand D, `dd464c108` — a `GovernedLiveOp` under `inst-mt-inputs` (d); P-D-136) | 10 |
| `audit × read\|export` | **no route declared** (M-4 fix) | 05 |

**What the column measures, and what it does not** (**P-D-45**): the set declares **fourteen**
routes as `` `METHOD /bss-products/v1/…` `` code spans — one machine-readable form — while doors
elsewhere are named in prose ("the fresh-zero door", "the watermark door", "the correction door").
Lint 3's population is therefore **the declared routes**, and every one of the fourteen appears
above. The **eight** grants that carry a route are the measurable half; the rest name no route and
are marked so rather than left blank, because a blank cell reads as an oversight. **That most
grants have no declared route is registered in §6 as its own gap** — it is not a lint-3 failure
under the stated direction (door ⇒ grant), and inventing routes to fill the column would have been
exactly the normative content a review may not author.

Why individual pairs exist: `metadata × write` is 02's map door — the door existed with no pair, and
P-D-06 makes the map mutable in place on a **published** entity with no version bump, so inheriting
`sku × write` would let anyone who can author drafts mutate content a `CatalogVersion` captures.
`materiality_policy × write` is the C4/P-D-11 object (field set + trigger + `N`), separate from
every config grant so the threshold's own holder cannot weaken it. `compliance × export` is 10's
DSAR surface, never folded into `audit × export`. `bulk_lifecycle × execute` is 09's mass-retire
lane and carries its own grant. The governed cancel is a `GovernedLiveOp` subject kind on
`ApprovalRecord`.

Role bundles mirror the PRD actors (ProductManager, CatalogAdmin, FinanceReviewer, Auditor,
PlatformOwner); grants are tenant-scoped claims from the IdP — the registry never mutates tenant
topology. Every door
names its pair; slice 12's coverage check asserts no door is unnamed.

### 3.3 Error taxonomy (slice-owned codes)

Declared by [`../features/governance.md`](../features/governance.md) §3 as `cpt-cf-bss-products-algo-governance-errors`.
The code roster below is this slice's and is the normative one; the FEATURE carries the obligation and the boundary.

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-governance-errors`

`SELF_APPROVAL_FORBIDDEN`, `APPROVER_SCOPE_EXCEEDED`, `APPROVER_ROLE_REQUIRED` (raised by the
**gate** when the descriptor is numerically met but the role predicate is not — L-2),
`APPROVAL_SUPERSEDED` (raised at **decide** on a superseded record — L-2),
`BREAKGLASS_WRITE_FORBIDDEN`, `BREAKGLASS_EXPIRED` — both this slice's own, and no phase
carve-out is owed for either (**P-D-36** withdrew the phase unit: a code belongs to the rule that
raises it, and the rule to a slice). `APPROVAL_REQUIRED` stays 01's (raised through the gate).

**Problem responses (RFC 9457):** `SELF_APPROVAL_FORBIDDEN`, `BREAKGLASS_WRITE_FORBIDDEN`, `BREAKGLASS_EXPIRED`, `APPROVAL_REQUIRED`, `APPROVER_SCOPE_EXCEEDED`, `APPROVER_ROLE_REQUIRED` (403); `APPROVAL_SUPERSEDED` (409); `CONTENT_PII_BLOCKED` (422 architectural, declared by 02 — **P-D-50**).

*Statuses added, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"*) — **403** where the caller may not
perform the act at all, **404** only where a path segment names a resource this tenant has none
of. **503** where retry is the remedy is this gear's own addition — pricing's set carries no 503
at all, so that one
class is not "checked against it". Proposed per
row and open to correction; the requirement is that every code carries one.
  Codes listed here for the response map but **declared elsewhere**: `APPROVAL_REQUIRED` (slice 01) and `CONTENT_PII_BLOCKED` (slice 02, **P-D-50**) — the status is repeated, not a second declaration, so the one-declaration rule stands.*

## 4. Data / Storage (normative shape; DDL in migrations)

- **`products_approval`** — `approval_id` (PK) · `tenant_id` · subject `(kind, ref)` — `kind ∈ {entity_publish, governed_live_op, system_signal (P-D-14), sku_correction (07), bulk_batch (09)}` · pinned
  `internal_revision` · **`content_snapshot`** (stored at submission — never re-derived) ·
  `diff_basis` (the published version id diffed against) · `quorum_descriptor` (**stored at submission, never re-derived** — (`predicateUnsatisfiable`
  and `configuredQuorum` were required by §2's `inst-gv-finance-predicate`, `inst-gv-quorum` and `inst-gv-queue`, and named in neither shape, and
  deriving `configuredQuorum` from current policy would change a **pending** record when the
  tenant edits `N`) — `configuredQuorum` (the `N` in force at submission), required count,
  finance predicate, **`predicateUnsatisfiable`**, override conditions, **`quorumReduced`** — P-D-13) · `state ∈ {pending, satisfied, consumed, rejected,
  superseded}` · `submitter` (pseudonymous) · timestamps. Partial `UNIQUE (tenant_id,
  subject_kind, subject_ref) WHERE state IN ('pending','satisfied')` — one open approval per
  subject; a new submission explicitly supersedes the open one (L-4). Append-only after
  finalization.
- **`products_approval_decision`** — `(approval_id, approver_principal)` UNIQUE — the principal **as `actor_ref`**, pseudonymous — · verdict ·
  reason · override acknowledgments · instant. The UNIQUE is C2's physical floor: one principal,
  one decision. **At `N = 0` the acknowledgment has no decision row to ride, so `products_approval`
  carries nullable `author_override_ack` and `author_override_ack_at`** (**P-D-68**), written by the
  submit door only when the effective quorum is zero — a fact gets a column rather than
  parameterizing someone else's row (the P-D-50 convention).
- **`products_breakglass_session`** — session id · principal (**as `actor_ref`** — pseudonymous like every actor-bearing store, M5 of the slice-10 review) · target tenant · reason ·
  window `[from, until)` · approval path (`two_person` ref | `post_hoc` obligation
  state ∈ {pending, reviewed} with `reviewed_by (actor_ref)` / `reviewed_at` — **P-D-68**) ·
  **`expired_emitted`** (the CAS stamp `BreakGlassExpired`'s one emitter flips — P-D-68) ·
  timestamps. Elevated audit rows carry the session id.
- **`products_materiality_policy`** (**P-D-112** — the fourth table, one row per tenant) — `tenant_id` (PK) · `field_set` (the
  bucket-iii columns the tenant marks material) · `affected_entity_trigger` (≥ 0) · `approver_count` (≥ 0, the `N` of P-D-11) ·
  `updated_by` (the pseudonymous `actor_ref` the policy door resolves — named outside P-D-45's `*_actor_ref` convention,
  so lint 7 does not see it — one of eight such names, §7 row 42, P-D-144; declared here by P-D-143) · `updated_at`. Written only by `PUT /bss-products/v1/materiality-policy`,
  whose own mutation is material (C4); read once per gated act to build the `MaterialityEvaluator`.
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
- Materiality: bucket-iii touch ⇒ material; bucket-iv-only re-publish ⇒ `min(N, 1)` approvers;
  policy-object mutation ⇒ material regardless of direction.

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-usecase-approval-publish` (§10 use case, claimed by id here — all seven were in lint 1's universe and none was claimed); `cpt-cf-bss-products-fr-materiality-gated-publish`, `cpt-cf-bss-products-fr-tenant-isolation-breakglass`,
`cpt-cf-bss-products-fr-breakglass-action-scope`; AC #26, #30, #31; §17.1 (interim default, enforced); P-D-02
(ceremony); consumed by 01 (`inst-fd-governance-gate`), 02 (`GovernedLiveOp`), 03 (override,
finance fields), 04 (un-deprecation, retirement confirmation, scheduled-approval pinning), 06 (force-completion, participant-set membership, the `system_signal` composition clear), 07 (`sku_correction`, reference-producer registration), 09 (`bulk_batch`), 10 (`pii_allowlist × write`).

**Risks & open items**:
- ~~**Eleven of the twenty-three grant rows carry no route in the `Doors` column.**~~ **Answered (P-D-134, 2026-09-04): one of twenty-three today** — `04`'s scheduled-transition doors, routed and C's build. *The item's text stood as:* P-D-45's `Doors` column made this countable for the first time, and **P-D-50** then took the three contradictions the propagation audit found — `approval × read`, `category × read` and `catalog_version × read` each had a route declared elsewhere in the set while their own cell read "no route declared". *Re-measured 2026-08-31 after **P-D-61** (the `bulk × read` row) and **P-D-67** (routes for
  `catalog_version × ack`/`× release`, `× force_complete` and `freeze_participant × write`):* the
  roster is **twenty-four** rows, **sixteen** name a route, and the `Doors` column declares
  **twenty-four** routes (**P-D-90** doored `recognized_set`/`plan_tier`, whose row had none).
  *Re-measured 2026-09-03 after **P-D-106**, which doored 02's three: the rows still without one
  are **five**, `category × write` and `attribute_definition` having left the list along with the
  live-value door's route.* The earlier reading of **eight** was: `category × write` (its row doored
  on `× read` alone), `attribute_definition`,
  `approval × submit|decide`, `materiality_policy`, `breakglass`, 07's three (one row),
  `pii_allowlist` and `audit × read` — the `recognized_set`+`plan_tier` row left this list with
  **P-D-90** — plus the two known absences spelled differently
  (`scheduled_transition`; `catalog_version × publish`, which no door consumed, is struck by P-D-125). An authorization
  surface nobody can enumerate is one nobody can review. Whether the fix is declaring the routes or admitting the grants are unspent is not a review's call. Owner: this slice with each door's owner. *(Raised by the P-D-45 round; re-measured by the P-D-43…49 propagation audit; the three contradictions closed by **P-D-50**.)*
- ~~**Does the discard door get its own grant, or inherit `product|sku × write`?**~~ **Answered (P-D-134, 2026-09-04): its own action `× discard`.** *The item's text stood as:* 01 §2 declares
  `POST /bss-products/v1/{products|skus}/{id}/discard` under **`… × discard`**, and this slice's
  RBAC catalog carries only `product × read|write|publish` and `sku × read|write|publish` — so
  12 `inst-cc-rbac`, which since **P-D-45** requires every declared route to appear in the `Doors`
  column above, has no pair to match on the very door P-D-31 added to make that lint green. `inst-gv-materiality`
  already settles that `draft→discarded` is ungated beyond authz, so this is the grant model
  only: minting `discard` lets a tenant withhold it, folding it into `write` does not. Owner:
  this slice (`cpt-cf-bss-products-contract-rbac`). *(Raised by the slice-01 sixth-pass review.)*
- ~~**What code does an authorization denial carry, and what status?**~~ **Struck in `features/governance.md` §7 row 3 (P-D-119/P-D-120, 2026-09-03):** **What code does an authorization denial carry, and what status?** Every door opens by authorizing deny-by-default and no slice declares a denial code, while the Foundation requires every code to carry a status and every refusal to be audited with its reason. *The item's text stood as:* Every door in 01 §2 opens by
  authorizing deny-by-default, 01 §1.5 puts RBAC grants in this slice, and no slice declares a
  denial code — while 01 §3.3 requires every code to carry a status and 01 §4.4 requires every
  refusal to be audited with its reason. So the first step of every registry door terminates in a
  refusal with no code for a consumer to match on. Owner: the governance owner with the taxonomy
  owner. *(Raised by the slice-01 fifth-pass review.)*
- **The studio inbox envelope** is design-introduced (deliberately merge-compatible with
  pricing's queue); pricing's queue shape should be cross-checked when slice 12 pins the SDK —
  a field-name drift here costs a UI adapter later.
- ~~**Post-hoc break-glass review**~~ **Answered (P-D-133, 2026-09-04): P-D-68's second platform principal, within `breakglass_review_sla_hours` (24 interim).** *The item's text stood as:* needs an owner and an SLA for the review obligation alert —
  operational, not structural; noted for the ops runbook.
- Approval retention/erasure interplay (approver principals are pseudonymous refs) is slice
  10's; this slice only guarantees the refs are pseudonymous from birth.
- ~~**Does the authoring head read need an action of its own in the RBAC catalog?**~~ **Answered (P-D-134, 2026-09-04): no — `× read` is the authoring read.** *The item's text stood as:* 01 §2's `GET` is
  an authoring read, and 01 §4.3 says that read "is not a consumer read", while this slice's
  catalog lists only `read|write|publish` per kind. Owner: this slice. *(Filed from 01 §6 by the slice-01 eighth lens pass — the pointer claimed it was registered here and it was not.)*
- ~~**What does `Gate` mode require of a gated transition?**~~ **Answered (P-D-105, 2026-09-02): for a scheduled flip, that the record is `consumed` and the flipped row's own `approval_ref` names it** — subject/revision equality is dropped there and kept everywhere else; the operand is a stored column no caller can write. *The item's text stood as:* 01 `inst-fd-gate-mode-gate` is worded for
  a publish and pins "the door's expected revision", while the transition doors are this slice's and
  04's and pin nothing stated in 01. Owner: this slice with 04. *(Filed from 01 §6 by the slice-01 eighth lens pass — the pointer claimed it was registered here and it was not.)*
- ~~**Is a break-glass two-person approval an `ApprovalRecord`, and what holds its fixed floor?**~~ **Answered (P-D-133, 2026-09-04): no — the two platform approvers live on the session row**, P-D-111's authority completed. *The item's text stood as:*
  `inst-bg-open` requires "two distinct platform principals, outside the tenant's configured `N`
  entirely", while §1.7 defines `required` only as `N` or `min(N, 1)` — no writer can produce a
  fixed 2 — §4's row is `tenant_id`-scoped, and `inst-gv-scope` would refuse a platform approver on
  another tenant's subject. §4 stores `approval path (two_person ref | post_hoc obligation state)`,
  a ref to an unnamed thing. Owner: this slice with the platform-identity owner — the store, the
  descriptor writer, and whether the tenant-scoped approval rules apply at all.
  *(Two lenses raised it independently.)*
- ~~**Where is the `N = 0` override acknowledgment stored?**~~
  **Answered (owner call, 2026-09-01 — P-D-68): on `products_approval`, in nullable
  `author_override_ack` / `author_override_ack_at`, written by the submit door only when the
  effective quorum is zero.** A synthetic decision row would break C2's one-principal-one-decision
  UNIQUE; a fact gets a column instead (the P-D-50 convention). Original text: `inst-gv-override` has the author perform
  it and says acknowledgments are "stored on the record and in audit"; the only column for them is
  on `products_approval_decision`, whose row demands an approver principal and a verdict the author
  does not have, and `products_approval` has no acknowledgment column. Owner: this slice's storage
  owner. *(Two lenses raised it independently.)*
- ~~**Which transaction writes `state = satisfied`?**~~ **Struck in `features/governance.md` §7 row 11 (P-D-119/P-D-120, 2026-09-03):** . Two writers, each the transaction in which the fact becomes true. *The item's text stood as:* Every other value has a named writer — submit
  flips `superseded`, a rejection finalizes `rejected`, the publish/apply marks `consumed`. This one
  has only an evaluator, and nothing says whether a record at `required = 0` is born satisfied. If
  satisfaction is evaluated at gate time instead, the `satisfied` branch of §4's partial unique is
  dead. Owner: this slice. *(Raised by the slice-05 first lens pass.)*
- ~~**What door carries submit, decide and break-glass elevation?**~~ **Answered (P-D-120, 2026-09-03): `POST /approvals`, `POST /approvals/{id}/decisions`, `POST /breakglass-sessions`.** *The item's text stood as:* The catalog mints
  `approval × submit|read|decide` and `breakglass × elevate`, and the only route this slice declares
  is the inbox `GET`. 01 §1.5 closes its own set at five wire doors. `DESIGN.md` books approvals
  here and says endpoint tables live per slice. Owner: this slice with the contract owner — routes,
  verbs, grants and statuses. *(Raised by the slice-05 first lens pass.)*
- ~~**What do the entity-shaped columns hold for the non-entity subject kinds?**~~ **Struck in `features/governance.md` §7 row 14 (P-D-119/P-D-120, 2026-09-03):** **What do the entity-shaped columns hold for the non-entity subject kinds?** The pinned revision, content snapshot and diff basis are fixed on every record, while **at least three** of the five kinds are not entities — a `GovernedLiveOp` envelope, a `system_si *The item's text stood as:* §4 fixes pinned
  `internal_revision`, `content_snapshot` and `diff_basis` on every record, while at least three
  subject kinds are not entities — a `GovernedLiveOp` envelope, a `system_signal`, a `bulk_batch`.
  A live op has no internal revision, no published version to diff against, and no scope for
  `inst-gv-scope` to cover; and the auto-satisfied `system_signal` record's "signal reference as the
  authorizing principal" has no column, the decision key being `(approval_id, approver_principal)`.
  Owner: this slice with 12, which pins the envelope's `subjectKind`. *(Raised by the slice-05 first lens pass.)*
- ~~**Which slice mints a grant pair when the owning slice names none?**~~ **Answered (P-D-134, 2026-09-04): the door's owner** — `04` for the scheduled-transition doors. *The item's text stood as:* The roster carries
  `scheduled_transition × write|cancel|read` and `product|sku × discard` for doors that name no
  pair, while §3.2 asserts "Every door names its pair" and 12's lint only runs door→catalog, so a
  catalog entry with no door is invisible to it in both directions. Owner: the governance owner with
  04, 08 and 12 — is the catalog the mint, or must a slice name its pair first?
  *(Two lenses raised it independently.)*
- ~~**Is `APPROVER_ROLE_REQUIRED` 403 or 409?**~~ **Answered (P-D-119, 2026-09-03): 403, at the decide door** — the caller may not take the act whatever the record's state; `DECISION_ALREADY_RECORDED` is the 409 beside it. *The item's text stood as:* The gate now raises it (this pass), but §3.3's own
  convention puts **409** where the current state refuses the act and **403** where the caller may
  not act at all — and by its stated raise site the caller may publish and it is the record's state
  that refuses, which is where the sibling `APPROVAL_SUPERSEDED` sits at 409. Owner: the governance
  owner with the taxonomy owner. *(Raised by the slice-05 first lens pass.)*
- ~~**Does `quorumReduced` fire on every non-material change at the default `N = 2`?**~~ **Answered (P-D-120, 2026-09-03): it marks an effective count below the default of two, for any cause** — so yes at the default; a bucket-iv-only re-publish rides it. *The item's text stood as:* §1.7 sets the
  marker "when the effective count is below the retained-name default of 2", and a bucket-iv-only
  re-publish at `N = 2` has an effective count of 1 — so the marker would ride the majority of
  records. P-D-13 frames it as a marker for the *reducible ceremonies*. Nothing distinguishes
  "reduced by configuration" from "reduced by non-materiality". Owner: this slice with the audit
  consumer. *(Raised by the slice-05 first lens pass.)*
- **Does C1's base role set bind the single approver of a non-material change?** C1 scopes its
  CatalogAdmin/FinanceReviewer floor to material changes; a non-material change gets `min(N, 1)` and
  the descriptor carries no base role set. Nothing says whether any holder of `approval × decide`
  may close one. Owner: this slice. *(Raised by the slice-05 first lens pass.)*
- ~~**What does an elevation change about the authorization decision?**~~ **Answered (P-D-133, 2026-09-04): the pre-pipeline gate substitutes a read-only `AccessScope::for_tenant(target)` from the session; writes refused.** *The item's text stood as:* C6 is deny-by-default over
  tenant-scoped IdP claims and 01 C4 puts all repository access through SecureORM tenant scoping;
  no rule anywhere says how a live `BreakGlassSession` widens either the grant check or the query
  scoping. Owner: this slice with the ToolKit/SecureORM owner — the operand a door reads, and where
  in the pre-pipeline gate it is read. *(Raised by the slice-05 first lens pass.)*
- ~~**Who produces `BreakGlassExpired`, and what happens to an act in flight at expiry?**~~
  **Answered (owner call, 2026-09-01 — P-D-68): the first post-expiry act emits it, exactly once, via
  a CAS flip of the session's `expired_emitted` stamp in the same transaction as its refusal** — the
  winner emits, a replay emits nothing (P-D-54's mechanism); an untouched session emits no event, its
  expiry being a stored fact observable as a gauge with the alerting rule on top (P-D-59's), which is
  also what the review alert keys on. **Expiry gates admission, not completion**: a read admitted
  inside the window finishes. Original text: The only
  producer named is a refused call, so a session nobody calls again never emits it and a session
  called ten times after expiry emits ten; no sweeper is named in any slice, and nothing says
  whether a read begun inside the window may finish. Owner: was this slice with the ops owner;
  **closed**. *(Raised by the slice-05 first lens pass.)*
- ~~**What is the `post_hoc` obligation's state set, and who discharges it?**~~
  **Answered (owner call, 2026-09-01 — P-D-68): the set is `{pending, reviewed}`, and the discharger
  is the second platform principal** — rule 1's *"two-person-approved or post-hoc-reviewed"* is one
  ceremony with two timings, so the review is the second principal's decision arriving after the
  fact, writing `reviewed_by (actor_ref)` / `reviewed_at` and flipping the state; **no new door or
  grant is minted**. Whether that decision's record is an `ApprovalRecord` stays its own item above,
  deliberately not presupposed. Original text: §4 stores the state and
  `inst-bg-open` raises the review obligation as an alert; no door, event or flow writes a
  discharge, and no values are enumerated — §6 books only an owner and an SLA. Owner: was this
  slice; **closed**.
  *(Raised by the slice-05 first lens pass.)*
- ~~**The break-glass window's two normative facts live only in `PRD` §17.1.**~~ **Answered (P-D-132, 2026-09-03): `breakglass_window_hours` = 4 interim in `ProductsConfig`; no renewal, stated in the elevation instruction.** *The item's text stood as:* That row carries a
  4-hour interim **and** "no renewal without a new session", crediting this slice's own review with
  raising it; `inst-bg-open` states neither, and renewal is neither forbidden nor admitted here.
  Owner: the governance owner with the §17.1 owner. *(Raised by the slice-05 first lens pass.)*
- ~~**Does AC #26's third bullet still bind after P-D-11 rewrote the first two?**~~ **Answered (P-D-132, 2026-09-03): no — rewritten**: a rejection leaves the head where it is, the reason on the decision row, the quorum the configured `N`. *The item's text stood as:* It carries both a
  pre-P-D-11 count ("v1 uses a single two-person step") and the `draft` return the head-row model
  cannot honour; P-D-11's propagation names two bullets of AC #26 and not this one. Owner: the PRD
  owner with the governance owner, in the register. *(Raised by the slice-05 first lens pass.)*
- ~~**Is C3's no-hook exception still worded for `draft→published` alone?**~~
  **Closed on re-measurement (2026-09-01, filed by P-D-68): stale against its own constraint.** C3
  already reads *"except any transition that consumes an approval in the same transaction"* and this
  slice cites **P-D-34** three times, so both of this item's premises were false at HEAD —
  `features/governance.md` §7 row 23 measured it and owed this strike. Original text: C3 fires 01's
  `inst-fd-approval-hook` on any frozen-content write, while the exception it carries is written
  for a `draft→published` publish; **P-D-34** widened the unit to any transition that consumes an
  approval in the same transaction, and this slice cites P-D-34 nowhere. Either the exception
  widens with it or the narrower wording is deliberate and says so. Owner: was this slice; **closed**.
  *(Filed from 01 §6 by the P-D-43…49 propagation audit — the eighth pass's own repair note
  claimed this was filed and it was not.)*
