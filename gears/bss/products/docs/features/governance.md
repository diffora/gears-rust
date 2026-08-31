# Feature: Governance & Approvals

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-governance-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-governance`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Submit a change for approval](#submit-a-change-for-approval)
  - [Decide — approve or reject](#decide--approve-or-reject)
  - [Publish or apply against the gate](#publish-or-apply-against-the-gate)
  - [Read the pending queue](#read-the-pending-queue)
  - [Break-glass elevation](#break-glass-elevation)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Materiality mechanics](#materiality-mechanics)
  - [RBAC catalog](#rbac-catalog)
  - [Error taxonomy](#error-taxonomy)
- [4. States (CDSL)](#4-states-cdsl)
  - [ApprovalRecord State Machine](#approvalrecord-state-machine)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Approval record store](#approval-record-store)
  - [Decision store and the one-principal-one-decision floor](#decision-store-and-the-one-principal-one-decision-floor)
  - [Break-glass session store](#break-glass-session-store)
  - [Materiality evaluator](#materiality-evaluator)
  - [Materiality policy object](#materiality-policy-object)
  - [Stored content snapshot](#stored-content-snapshot)
  - [Stored quorum descriptor](#stored-quorum-descriptor)
  - [Finance predicate and its unsatisfiable arm](#finance-predicate-and-its-unsatisfiable-arm)
  - [Supersession on frozen-content write](#supersession-on-frozen-content-write)
  - [Quorum evaluator](#quorum-evaluator)
  - [Self-approval refusal](#self-approval-refusal)
  - [Approver scope](#approver-scope)
  - [Decision recording and rejection](#decision-recording-and-rejection)
  - [Override ceremony](#override-ceremony)
  - [The gate host](#the-gate-host)
  - [One-shot consumption](#one-shot-consumption)
  - [PreAuthorized mode](#preauthorized-mode)
  - [System-signal subjects](#system-signal-subjects)
  - [Pending queue and the inbox envelope](#pending-queue-and-the-inbox-envelope)
  - [RBAC catalog registration](#rbac-catalog-registration)
  - [Break-glass elevation](#break-glass-elevation-1)
  - [Break-glass read-only enforcement](#break-glass-read-only-enforcement)
  - [Break-glass expiry](#break-glass-expiry)
  - [PII block on every operator reason](#pii-block-on-every-operator-reason)
  - [Governance error taxonomy](#governance-error-taxonomy)
  - [Governance events](#governance-events)
  - [Audit trail for governance acts](#audit-trail-for-governance-acts)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Raised here rather than carried](#raised-here-rather-than-carried)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

This feature is the **gate** every other feature invokes and none re-implements: the materiality
evaluator, the two-person approval workflow with a **stored** pinned content snapshot, the
FinanceReviewer role predicate, the uncomposed-bundle override ceremony, deny-by-default RBAC over
the gear's GTS-typed resource-and-action catalog, and the time-boxed, read-only **break-glass**
elevation.

It plugs into `01-foundation` as the governance phase of the publish door and into
`02-taxonomy-attributes` as the approver of `GovernedLiveOp` envelopes.

**The seam it fills already exists in code.** `01-foundation` ships the host contract — the
`GovernanceGate` trait, `GateMode`, `GateVerdict`, `GateAuthorization`, `ApprovalDisposition`,
`EntityRef` and `ApprovalId` — together with `NoMaterialityPolicyGate`, the host the gear runs
**until this feature registers one**. That module states in its own words that it contains no
materiality evaluator, no `ApprovalRecord`, no record store and no grant check, and that their
absence is the design's rather than an omission. This feature is what fills them, and it **mints
no parallel vocabulary**: it implements the trait that exists.

### 1.2 Purpose

Financial-grade control with exactly one enforcement point. No feature may publish, apply a live
op, or elevate around this gate — and the gate is built so that two historical failure classes
cannot recur:

- **A re-derived "pinned" snapshot that diffs a draft against itself.** The record **stores** the
  submitted content at submission and renders the approver's diff from that stored copy, never
  from the live head.
- **One human wearing two roles counting as two approvers.** Distinctness is by **principal**,
  never by role, and the store enforces it.

**Requirements** — carried from [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.5:

- Whole: `cpt-cf-bss-products-fr-materiality-gated-publish`,
  `cpt-cf-bss-products-fr-tenant-isolation-breakglass`,
  `cpt-cf-bss-products-fr-breakglass-action-scope`
- Surfaces: `cpt-cf-bss-products-usecase-approval-publish` — the one entry that lists a use case
  in its own Requirements block, so this line **is** carried rather than claimed

**Principles**: `cpt-cf-bss-products-principle-governance-at-entity-publish`.

**Constraints**: `cpt-cf-bss-products-constraint-gts-types-not-instances` — the authz resource and
action catalog declares GTS-typed resources.

**Component**: `cpt-cf-bss-products-component-capability-handlers`.

**Sequence**: none of its own — this feature's gate phase runs **inside**
`cpt-cf-bss-products-seq-authoring-publish`.

**Not applicable or delegated**: **authentication** and the identity claims this feature reads are
the platform IdP's; **observability** and outbox delivery are `01-foundation`'s; **read
performance** is `08-read-models`'; **erasure of approver identities** is
`10-retention-erasure`'s, and this feature guarantees only that the refs are pseudonymous from
birth. **Operator-facing message wording** is the API layer's — the six codes are the contract.
**Rollout** is forward-only migration per `01-foundation`; there is no feature flag, and the
break-glass window and the materiality policy are the two runtime knobs, both configured rather
than compiled. **Authorization is this feature's own subject**, not a delegation — it is the RBAC
catalog below.

**Out of scope**, mirroring [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.5: the doors
themselves (`01-foundation`, `02-taxonomy-attributes`); scheduling, where `04-lifecycle` pins
approvals and this feature only validates them at activation through the gate; the break-glass
**correction** door, a feature-flag-gated write mechanism owned by `07-reference-signal` that
reuses only this feature's elevation **ceremony shape**; and erasure of approver identities
(`10-retention-erasure`).

### 1.3 Actors

| Actor | Role in this feature |
|-------|----------------------|
| `cpt-cf-bss-products-actor-product-manager` | Submits changes; never approves their own work |
| `cpt-cf-bss-products-actor-catalog-admin` | Approver |
| `cpt-cf-bss-products-actor-finance-reviewer` | The mandatory second lens on finance-material fields; approval-queue consumer |
| `cpt-cf-bss-products-actor-auditor` | Reads the immutable approval, audit and break-glass trails |
| `cpt-cf-bss-products-actor-platform-owner` | Break-glass initiator and acting principal — two distinct platform principals, or post-hoc review; cross-tenant read and audit-export only in v1 |

### 1.4 References

- [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.5 — the entry this feature realizes
- [`../design/05-governance.md`](../design/05-governance.md) — the design slice. **This FEATURE is
  the declaration site of the five `flow-` ids and the three `algo-` ids**, and the slice's §2 and
  §3 point here for them; there is one definition site per id. One `algo-` id moved here from the
  slice; two — `cpt-cf-bss-products-algo-rbac-catalog` and
  `cpt-cf-bss-products-algo-governance-errors` — are **minted here**, because §3.2's catalog and
  §3.3's code roster each carried only a `contract-` id, which a FEATURE may not define.
  **The slice's step lists remain the normative ones and are not copied here**: re-spelling the 19
  instruction steps it owns would fork the set's own instruction register and leave two texts
  where only one can be true. §2 and §3 carry the actor, the scenarios and the boundary.
  - **§4's state machine is a second exception, and its ids are this document's.** The template
    requires a step id per transition row, and the slice expresses the `ApprovalRecord`'s states as
    a column domain in §4 rather than as rows, so no row can be reused. The five `inst-ap-*` ids
    and `cpt-cf-bss-products-state-approval-record` are declared here and cited by no slice.
    **Where §4 and the slice differ on a rule, the slice governs.**
  - **§5 restates the slice's §4 storage shapes**, a third exception on the same terms: a
    Definition of Done must name the columns and constraints it obliges. **Where §5 and the
    slice's §4 differ on a column-level fact, the slice governs.**
  - **`contract-` ids are cited but not defined here.** A FEATURE may **define** only `flow`,
    `algo`, `state`, `dod` and `featstatus` ids, plus the `inst-` steps of a state machine it
    declares. `cpt-cf-bss-products-contract-rbac` and
    `cpt-cf-bss-products-contract-governance-errors` remain the slice's and are cited by id,
    which survives a renumber where a section number does not.
  - **Twelve `inst-*` ids this slice cites are owned elsewhere** and are referenced, never
    claimed: `inst-fd-governance-gate`, `inst-fd-approval-hook`, `inst-fd-gate-verdict`,
    `inst-fd-gate-mode-gate`, `inst-fd-save-txn` (`01-foundation`); `inst-av-pii-block`,
    `inst-av-pii-reason` (`02-taxonomy-attributes`); `inst-cl-bundle-override`
    (`03-sku-classification`); `inst-lc-undeprecate` (`04-lifecycle`); `inst-cc-clear`
    (`06-catalog-version`); `inst-bk-override` (`09-bulk-promotion`); `inst-cc-rbac`
    (`12-consumer-contracts`).
- **Dependencies**: `cpt-cf-bss-products-feature-foundation` is the only build-time dependency,
  and its `GovernanceGate` trait is the interface this feature implements. Every other feature is
  a **consumer**: `02`, `03`, `04`, `06`, `07`, `09` and `10` each register a `GovernedLiveOp`
  kind, a role predicate or an override condition with it.
- [`../PRD.md`](../PRD.md) §6.7, §6.8; §12 AC #26, #30, #31; §17.1 (the interim materiality
  default, the affected-entity trigger, and the break-glass window)
- [`../DESIGN.md`](../DESIGN.md) §1.3 layering, §2.1 principles, §2.2 constraints
- [`../DECISIONS.md`](../DECISIONS.md) — P-D-02, P-D-08, P-D-10, P-D-11, P-D-13, P-D-14, P-D-18,
  P-D-21, P-D-26, P-D-28, P-D-30, P-D-34, P-D-36, P-D-39, P-D-41, P-D-43, P-D-45, P-D-48, P-D-50
- [`./foundation.md`](./foundation.md) — the gate seam, the pipeline and the audit trail

## 2. Actor Flows (CDSL)

**Use cases**: `cpt-cf-bss-products-usecase-approval-publish`

The step lists live in [`../design/05-governance.md`](../design/05-governance.md) §2 — see §1.4.
Each flow below names its actor, what success and failure look like, and where its boundary runs.

### Submit a change for approval

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-submit`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- Submission runs the materiality evaluator over the change set and writes an `ApprovalRecord`
  whose quorum descriptor is **stored**, not derived: `required = N` for a material change,
  `min(N, 1)` for a non-material one, alongside the raw configured `N`
- The record **stores the submitted content snapshot**, and the diff approvers see is rendered
  from that copy against the last published version
- A tenant at `N = 0` publishes approver-less by policy, and the record says exactly that
- A frozen-content write on the subject after submission flips the record `superseded`, and
  re-submission is an explicit human act with the new diff re-presented

**Error Scenarios**:
- Free text in a submission reason carrying prohibited personal data — `CONTENT_PII_BLOCKED`,
  `02-taxonomy-attributes`' code raised at this door

**Boundary**: the evaluator runs **once**, at submission, against the policy in force at that
instant. A policy change between submit and publish neither re-judges nor voids a pending
approval. Re-submission is **never automatic**: an auto-resubmit would pin content nobody re-read.

### Decide — approve or reject

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-decide`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- Each decision appends one row carrying the approver principal, the verdict, the instant, and a
  mandatory reason on reject
- The quorum evaluator answers "satisfied" only when the descriptor is met by **distinct
  principals** holding the required roles
- A rejection finalizes the record and leaves the subject as it was; `ApprovalDecided` is emitted
  either way
- Where the subject carries named override conditions, each approver acknowledges the lint
  findings **by name**, and the acknowledgments are stored on the record and in audit

**Error Scenarios**:
- The author deciding on their own submission at any `N ≥ 1` — `SELF_APPROVAL_FORBIDDEN`, by
  principal and not by role
- An approver whose brand and region claims do not cover the subject's scope —
  `APPROVER_SCOPE_EXCEEDED`, audited like any scope violation
- A decision arriving on a record already superseded — `APPROVAL_SUPERSEDED`
- A reject reason carrying prohibited personal data — `CONTENT_PII_BLOCKED`

**Boundary**: scope is read with the Foundation's two boundaries — the scope columns are `NOT
NULL` and the **empty set means unrestricted** — so an unrestricted claim set covers every
subject, an unrestricted subject scope is covered only by an unrestricted claim set, and between
two non-empty sets it is ordinary subset. At `N = 0` the **author** performs the override
acknowledgment and the record carries `quorumReduced`: the ceremony's product is an informed
decision, so it survives an empty quorum instead of vanishing with it.

### Publish or apply against the gate

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-gate`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- The Foundation's gate phase runs on **any gated act**, not on publish alone, and a transition
  door consumes a satisfied record exactly as the publish door does
- A record in state `satisfied` whose pinned revision equals the door's expected revision answers
  yes, and the verdict returns the authorizing record's id **and** whether it carried the
  uncomposed-bundle override acknowledgment
- A stage entering in `PreAuthorized` mode naming a **consumed** record that authorized this
  subject at this pinned revision answers yes and **consumes nothing further**
- A `system_signal` subject — a publish whose sole content is a system-owned flag cleared by an
  inbound governed signal — is auto-satisfied with the signal reference as the authorizing
  principal, audited like any decision
- Consumption is one-shot: the act marks the record `consumed` **in the same transaction**, and a
  failed attempt consumes nothing

**Error Scenarios**:
- A record whose decisions numerically meet `required` while the role predicate is unmet —
  `APPROVER_ROLE_REQUIRED`
- Anything else — `APPROVAL_REQUIRED`, which stays `01-foundation`'s code raised through this gate

**Boundary**: the gate **never re-evaluates materiality at publish** — the verdict was fixed at
submission. One approval never authorizes two **human** acts, but a **scheduled act is one
composite act**: a retirement approval authorizes initiation and the `effectiveAt` flip, a cascade
approval authorizes the whole plan including its per-child legs, and a bulk batch is one composite
act. The later mechanical stages re-enter through `PreAuthorized` rather than demanding a fresh
`satisfied` record. A `system_signal` publish requires a **clean head** and on a dirty head is
**deferred, never refused**.

### Read the pending queue

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-queue`

**Actor**: `cpt-cf-bss-products-actor-finance-reviewer`

*Door: `GET /bss-products/v1/approvals?state=pending`, grant `approval × read`.*

**Success Scenarios**:
- The queue returns the common inbox envelope — subject ref and kind, state, submitter, submission
  instant, and a quorum block carrying `required`, `satisfied`, `financeRequired`,
  `predicateUnsatisfiable`, `quorumReduced` and `configuredQuorum` — plus the per-kind diff payload
- `required` is the record's **effective** count, never the raw configured `N`, so a card cannot
  show "2 required" for a record that closes on one; `configuredQuorum` carries the raw value for
  surfaces that need it

**Error Scenarios**:
- A caller without `approval × read` — refused by the pre-pipeline authorization gate, whose code
  and status are **open item 3** and are declared by no document today

**Boundary**: the envelope is deliberately merge-compatible with the sibling pricing gear's queue,
so one studio inbox renders both with per-kind cards. Heterogeneous quorums render per card, which
makes parity with that gear a configuration question rather than a schema one.

### Break-glass elevation

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-breakglass`

**Actor**: `cpt-cf-bss-products-actor-platform-owner`

**Success Scenarios**:
- Elevation opens a session carrying a mandatory reason, a configured time-boxed window and a
  named target tenant, itself either two-person-approved or post-hoc-reviewed, with both paths
  recorded and the post-hoc path raising its review obligation as an **alert**
- `BreakGlassElevated` is emitted alongside a distinct alert channel
- Under elevation, every access is individually audited with the session id, the reason and the
  correlation id

**Error Scenarios**:
- Any write attempt under elevation — `BREAKGLASS_WRITE_FORBIDDEN`, with no exception in v1
- Any call past the window — `BREAKGLASS_EXPIRED`; `BreakGlassExpired` is emitted
- A reason carrying prohibited personal data — `CONTENT_PII_BLOCKED`

**Boundary**: v1 is **read and audit-export only**. The two-person floor here is **two distinct
platform principals, outside the tenant's configured `N` entirely** — the acting principal is a
platform owner and the subject is another tenant's data, so no tenant configuration has standing
over it. **No writer can produce that fixed floor today** (open item 9): the descriptor's
`required` is defined only as `N` or `min(N, 1)`. Standing cross-tenant access is not grantable at
all — the grant model has no such shape.

## 3. Processes / Business Logic (CDSL)

The step lists live in [`../design/05-governance.md`](../design/05-governance.md) §3 — see §1.4.

### Materiality mechanics

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-materiality`

**Input**: the change set, the bucket registry, the enumerated ops, the affected-entity count, the
registered `GovernedLiveOp` kinds, and the materiality policy in force at the submission instant

**Output**: material or non-material, and from it the stored quorum descriptor — `required`, the
finance predicate, `predicateUnsatisfiable`, `quorumReduced`, `configuredQuorum` and the override
conditions

Four inputs decide materiality: a **bucket-iii field touch**, where the owning feature registers
its columns' tags exactly as it registers validators and the Foundation's head-door guard and this
judgement read the same registry; the **enumerated ops**, being lifecycle transitions to
`published`, `deprecated` or `retired`, the category operations and material attribute-definition
changes; the **affected-entity count** at or above the configured trigger for batch acts; and a
**`GovernedLiveOp` kind registered material by its owning feature**.

The policy object — field set, trigger **and** the approver count `N` — is itself a
`GovernedLiveOp` subject whose own mutation is always material, on its own resource pair
`materiality_policy × write` rather than a config administrator's general grant.

**Boundary**: a registered kind's role predicate **narrows within** the base role set and never
replaces it, and v1 registers no extension point that could. A replacing predicate would be a
bypass surface — register a kind whose predicate admits anyone and a material change passes on one
signature. The metering-unit field is bucket **ii** and so is never seen as an ordinary touch.

### RBAC catalog

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-rbac-catalog`

**Input**: the caller's tenant-scoped claims from the identity provider, the door being entered,
and the GTS-typed resource-and-action pair that door names

**Output**: admission, or a deny-by-default refusal whose **code and status are open item 3** —
no document declares them, while the Foundation requires every code to carry a status and every
refusal to be audited with its reason

Grants are GTS-typed resource-by-action pairs, tenant-scoped, checked at every door, deny by
default: no grant, no access. Role bundles mirror the PRD actors. The registry never mutates
tenant topology.

**Boundary**: the catalog is normative at
[`../design/05-governance.md`](../design/05-governance.md) §3.2 as
`cpt-cf-bss-products-contract-rbac`, and this feature does not restate its twenty-three rows.
`01-foundation` ships **three** of them in `authz.rs` — `product` and `sku` over
`read`/`write`/`publish` — and its module doc says the rest belong to the features that build
those doors. **Eleven rows carry no route at all** (open item 1), and whether the remedy is
declaring the routes or admitting the grants are unspent is not this document's call.

### Error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-governance-errors`

**Input**: a refused governance act and the rule that refused it

**Output**: one canonical code carrying its declared RFC 9457 status

This feature declares six codes: `SELF_APPROVAL_FORBIDDEN`, `APPROVER_SCOPE_EXCEEDED`,
`APPROVER_ROLE_REQUIRED`, `APPROVAL_SUPERSEDED`, `BREAKGLASS_WRITE_FORBIDDEN` and
`BREAKGLASS_EXPIRED`. Two more appear in its response map and are **declared elsewhere**:
`APPROVAL_REQUIRED` (`01-foundation`, raised through this gate) and `CONTENT_PII_BLOCKED`
(`02-taxonomy-attributes`).

**Boundary**: the roster and every status are specified by
`cpt-cf-bss-products-contract-governance-errors`. `APPROVAL_SUPERSEDED` sits at 409 and the rest
at 403. **`APPROVER_ROLE_REQUIRED`'s status is open item 13**: the convention puts 409 where the
current state refuses the act and 403 where the caller may not act at all, and by its stated raise
site the caller may publish while it is the record's state that refuses.

## 4. States (CDSL)

### ApprovalRecord State Machine

- [ ] `p1` - **ID**: `cpt-cf-bss-products-state-approval-record`

The five rows below are this document's rendering of the slice's §4 column domain; see §1.4.

**States**: `pending`, `satisfied`, `consumed`, `rejected`, `superseded`

**Initial State**: `pending`

**Transitions**:
1. [ ] - `p1` - **FROM** `pending` **TO** `satisfied` **WHEN** the quorum evaluator finds the stored descriptor met by distinct principals holding the required roles — **or, for a `system_signal` subject, at submission, the signal reference being the authorizing principal**. *Which transaction performs this write is open item 11: every other value has a named writer and this one has only an evaluator* - `inst-ap-edge-satisfy`
2. [ ] - `p1` - **FROM** `satisfied` **TO** `consumed` **WHEN** the authorized act commits, in the **same transaction** as that act; a failed attempt consumes nothing, and a `PreAuthorized` stage naming an already-consumed record consumes nothing further - `inst-ap-edge-consume`
3. [ ] - `p1` - **FROM** `pending` **TO** `rejected` **WHEN** an approver holding `approval × decide` rejects with a mandatory reason; the subject stays as it was - `inst-ap-edge-reject`
4. [ ] - `p1` - **FROM** `pending` **TO** `superseded` and **FROM** `satisfied` **TO** `superseded` **WHEN** a frozen-content write lands on the subject and fires the Foundation's approval-invalidation hook; re-submission is an explicit human act and is never automatic - `inst-ap-edge-supersede`
5. [ ] - `p1` - **No transition other than those above is admitted**, `rejected` and `consumed` are terminal, and the record is **append-only after finalization** - `inst-ap-terminal`

**The `pending → satisfied` writer is unnamed**, which is open item 11 rather than a gap this
document fills: if satisfaction is evaluated at gate time instead of written at decision time,
then the `satisfied` branch of the store's partial unique index is dead, and whether a record at
`required = 0` is **born** satisfied is unstated.

## 5. Definitions of Done

Twenty-seven, counted by `grep` on this file rather than from the plan that sized them. **Twenty-two are separately testable.** Five are not, and each names what it needs:
`dod-gate-host`, `dod-one-shot-consumption` and `dod-preauthorized-mode` are exercised through
`01-foundation`'s publish door and need that door's test harness — the existing `RecordingGate`
double is the shape, and this feature replaces it with a real host; `dod-pii-on-reasons` needs
`10-retention-erasure`'s detector, which does not exist, so it is testable only against a stub, and
a stub that refuses every string satisfies it — a clean-text control is part of the obligation;
and `dod-inbox-envelope`'s merge-compatibility half is `12-consumer-contracts`'.

### Approval record store

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-approval-store`

The system **MUST** create `products_approval` on both engines with the subject kind and ref, the
pinned `internal_revision`, the **stored** `content_snapshot`, the `diff_basis`, the **stored**
`quorum_descriptor`, the state, a pseudonymous submitter and timestamps. A **partial**
`UNIQUE (tenant_id, subject_kind, subject_ref) WHERE state IN ('pending','satisfied')` **MUST**
admit one open approval per subject. The row **MUST** be append-only after finalization. A
schema-oracle golden **MUST** exist on both engines with a perturbation case proving it can fail.

**Implements**: `cpt-cf-bss-products-flow-submit`,
`cpt-cf-bss-products-state-approval-record`

**Constraints**: `cpt-cf-bss-products-constraint-gts-types-not-instances`

**Touches**:
- DB Table: `products_approval`
- Entities: `ApprovalRecord`

### Decision store and the one-principal-one-decision floor

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-decision-store`

The system **MUST** create `products_approval_decision` carrying the approver principal **as an
`actor_ref`**, the verdict, the reason, the override acknowledgments and the instant, with
`UNIQUE (approval_id, approver_principal)`. **That index is the physical floor under
distinctness-by-principal**: one principal, one decision, whatever roles they hold.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval_decision`
- Entities: `ApprovalDecision`

### Break-glass session store

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-breakglass-store`

The system **MUST** create `products_breakglass_session` carrying the session id, the principal as
an `actor_ref`, the target tenant, the reason, the window as a half-open interval, and the
approval path — a two-person reference or a post-hoc obligation state. Elevated audit rows **MUST**
carry the session id. **The post-hoc obligation's state set is enumerated nowhere and no door
discharges it** (open item 20).

**Implements**: `cpt-cf-bss-products-flow-breakglass`

**Touches**:
- DB Table: `products_breakglass_session`
- Entities: `BreakGlassSession`

### Materiality evaluator

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-materiality-evaluator`

The system **MUST** decide materiality from the four declared inputs and **MUST** evaluate **once**,
at submission, against the policy in force at that instant — never at the reader's clock and never
again at publish. A bucket-iii touch **MUST** be material; a bucket-iv-only re-publish **MUST** be
non-material; the policy object's own mutation **MUST** be material regardless of direction.

**Implements**: `cpt-cf-bss-products-algo-materiality`

**Touches**:
- Entities: `MaterialityEvaluator`

### Materiality policy object

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-materiality-policy`

The system **MUST** carry the policy as field set, trigger count **and** the approver count `N`,
as a `GovernedLiveOp` subject on its **own** resource pair `materiality_policy × write` — never a
config administrator's general grant, so that the holder of a config grant cannot weaken the
threshold that governs them. `N` **MUST** default to 2 with a floor of 0, be reachable only by
explicit configuration, and take its initial value from tenant provisioning.

**Implements**: `cpt-cf-bss-products-algo-materiality`

**Touches**:
- Entities: `MaterialityEvaluator`

### Stored content snapshot

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-stored-snapshot`

The system **MUST** store the submitted content on the record at submission and **MUST** render the
approver's diff from that stored copy against the last published version, **never** re-deriving it
from the live head. **This is the flagship probe**: submit, edit the head, and the superseded
record's diff still renders the original submission against the published version. It **MUST** be
written red first — a re-derived diff shows the draft against itself, which is the exact defect
this rule exists to prevent.

**Implements**: `cpt-cf-bss-products-flow-submit`

**Touches**:
- DB Table: `products_approval`

### Stored quorum descriptor

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-quorum-descriptor`

The system **MUST** store the descriptor at submission and **MUST NOT** re-derive any of it:
`required` as the **effective** count, `configuredQuorum` as the `N` in force at submission, the
finance predicate, `predicateUnsatisfiable`, `quorumReduced` and the override conditions. Deriving
`configuredQuorum` from current policy would change a **pending** record when the tenant edits `N`.

**Implements**: `cpt-cf-bss-products-flow-submit`

**Touches**:
- DB Table: `products_approval`

### Finance predicate and its unsatisfiable arm

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-finance-predicate`

The system **MUST** set the FinanceReviewer predicate on a finance-material change **when
`N ≥ 1`**, and at **`N = 0` MUST NOT** set it — recording `predicateUnsatisfiable` instead and
leaving the descriptor satisfiable. Without that arm the descriptor demands a role no principal
could hold and the gate refuses forever, which would re-block precisely the one-person tenant the
quorum floor exists to unblock. At every `N ≥ 1` the predicate **MUST** bind, and a tenant that has
designated no FinanceReviewer simply has an unapprovable change.

**Implements**: `cpt-cf-bss-products-flow-submit`

**Touches**:
- DB Table: `products_approval`

### Supersession on frozen-content write

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-supersede`

The system **MUST** flip an open record `superseded` when the Foundation's approval-invalidation
hook fires on a frozen-content write to the subject, and **MUST** require an explicit human
re-submission with the new diff re-presented. Re-submission **MUST NOT** be automatic. **Whether
the hook's no-fire exception still reads for `draft→published` alone or widens to every transition
that consumes an approval in the same transaction is open item 23.**

**Implements**: `cpt-cf-bss-products-flow-submit`,
`cpt-cf-bss-products-state-approval-record`

**Touches**:
- DB Table: `products_approval`

### Quorum evaluator

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-quorum-evaluator`

The system **MUST** count **distinct principals** against the stored descriptor, roles included,
and **MUST** treat a recorded `predicateUnsatisfiable` as met for the evaluator while it stays
visible as unmet-by-policy in the record and the inbox envelope — the only way that predicate may
be discharged, and never how one is discharged at `N ≥ 1`. A probe **MUST** prove one human holding
both CatalogAdmin and FinanceReviewer counts **once**.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval_decision`
- Entities: `QuorumEvaluator`

### Self-approval refusal

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-self-approval`

The system **MUST** refuse the author's own decision at every `N ≥ 1` with
`SELF_APPROVAL_FORBIDDEN`, **by principal and never by role**, with a paired positive control
proving a different principal is admitted.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval_decision`

### Approver scope

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-approver-scope`

The system **MUST** refuse a decision whose approver's brand and region claims do not cover the
subject's scope with `APPROVER_SCOPE_EXCEEDED`, and **MUST** audit it like any scope violation.
Scope **MUST** be read with the Foundation's two boundaries: the columns are `NOT NULL` and the
empty set means **unrestricted**. A paired in-scope control **MUST** exist.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval_decision`

### Decision recording and rejection

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-decide`

The system **MUST** append one decision row per approver, refuse a decision on an already
superseded record with `APPROVAL_SUPERSEDED`, finalize a rejection with its mandatory reason while
leaving the subject unchanged, and emit `ApprovalDecided` on either verdict.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval_decision`

### Override ceremony

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-override-ceremony`

The system **MUST** require each approver to acknowledge the named lint findings **by name** where
the subject carries override conditions, and **MUST** store the acknowledgments on the record and
in audit — an informed override, never a blind one. At `N = 0` the **author** performs the
acknowledgment and the record carries `quorumReduced`. The record **MUST** be the ceremony's only
home: a lane that publishes an override subject without one is a defect, not an exemption.
**Where the `N = 0` acknowledgment is stored is open item 10** — the only column for
acknowledgments sits on the decision row, which demands an approver principal and a verdict the
author does not have.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval`, `products_approval_decision`

### The gate host

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-gate-host`

The system **MUST** implement `01-foundation`'s `GovernanceGate` trait and register that host in
place of `NoMaterialityPolicyGate`, which is what the gear runs until this feature exists. It
**MUST NOT** mint a parallel vocabulary: `GateMode`, `GateVerdict`, `GateAuthorization`,
`ApprovalDisposition`, `EntityRef` and `ApprovalId` already exist and are the seam. The verdict
**MUST** carry the authorizing record's id and whether that record held the uncomposed-bundle
override acknowledgment, **and nothing more** — the Foundation learns nothing about who approved,
against which rule, in how many steps or when.

**Implements**: `cpt-cf-bss-products-flow-gate`

**Touches**:
- Entities: `GovernanceGate`, `GateVerdict`

### One-shot consumption

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-one-shot-consumption`

The system **MUST** flip a satisfied record `consumed` **in the same transaction as the authorized
act**, and a failed attempt **MUST** consume nothing. A probe **MUST** drive two publishes off one
satisfied approval and prove the second fails.

**Implements**: `cpt-cf-bss-products-flow-gate`,
`cpt-cf-bss-products-state-approval-record`

**Touches**:
- DB Table: `products_approval`

### PreAuthorized mode

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-preauthorized-mode`

The system **MUST** answer yes to a stage entering in `PreAuthorized` mode that names a
**consumed** record which authorized this subject at this pinned revision, and **MUST** consume
nothing further. That mode **MUST NOT** be reachable from any wire surface — no request field, no
header, no query parameter — so its reuse is bounded by in-process callers rather than by a grant.
A scheduled act, a cascade and a bulk batch are each **one composite act**, and their mechanical
stages re-enter here rather than demanding a fresh record.

**Implements**: `cpt-cf-bss-products-flow-gate`

**Touches**:
- Entities: `GateMode`

### System-signal subjects

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-system-signal`

The system **MUST** admit a `system_signal` subject kind whose record is auto-satisfied with the
signal reference as the authorizing principal, audited like any decision, with no human approver
and no exemption from the gate. The head **MUST** be clean: such a publish carries the flag and
nothing else, and on a dirty head is **deferred, never refused**. The configured `N` **MUST** have
no standing over it, the principal not being a tenant principal.

**Implements**: `cpt-cf-bss-products-flow-gate`

**Touches**:
- DB Table: `products_approval`

### Pending queue and the inbox envelope

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-inbox-envelope`

The system **MUST** serve `GET /bss-products/v1/approvals?state=pending` under `approval × read`,
returning the common envelope with the quorum block carrying `required` as the **effective** count
and `configuredQuorum` as the raw `N`. **A card MUST NOT be able to show "2 required" for a record
that closes on one.** The envelope **MUST** stay merge-compatible with the sibling gear's queue —
that half is `12-consumer-contracts`' to assert.

**Implements**: `cpt-cf-bss-products-flow-queue`

**Touches**:
- API: `GET /bss-products/v1/approvals?state=pending`
- DB Table: `products_approval`

### RBAC catalog registration

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-rbac-catalog`

The system **MUST** declare the GTS-typed resource-and-action catalog deny-by-default and check the
pair at every door. `01-foundation`'s `authz.rs` already ships three rows and states that the rest
belong to the features that build those doors, so this feature **MUST** extend rather than replace
it. **Eleven rows carry no route** (open item 1) and the `discard` grant question is unresolved in
the code comment as well as the design (open item 2); this DoD obliges the catalog, not the
routes.

**Implements**: `cpt-cf-bss-products-algo-rbac-catalog`

**Constraints**: `cpt-cf-bss-products-constraint-gts-types-not-instances`

**Touches**:
- Entities: `RbacCatalog`

### Break-glass elevation

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-breakglass-open`

The system **MUST** open a session with a mandatory reason, a configured window and a named target
tenant, under either a two-person platform approval or a recorded post-hoc review, and **MUST**
emit `BreakGlassElevated` alongside a distinct alert channel. **The fixed floor of two platform
principals has no writer** (open item 9), and **the window's interim value and its no-renewal rule
live only in the PRD's interim-policy table** (open item 22).

**Implements**: `cpt-cf-bss-products-flow-breakglass`

**Touches**:
- DB Table: `products_breakglass_session`

### Break-glass read-only enforcement

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-breakglass-readonly`

The system **MUST** refuse every write attempt under elevation with `BREAKGLASS_WRITE_FORBIDDEN`,
with no exception in v1, and **MUST** individually audit every elevated access with the session id,
the reason and the correlation id — **the count asserted, not sampled**. **What an elevation
changes about the authorization decision and the repository's tenant scoping is open item 18**: no
rule states how a live session widens either.

**Implements**: `cpt-cf-bss-products-flow-breakglass`

**Touches**:
- DB Table: `products_breakglass_session`

### Break-glass expiry

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-breakglass-expiry`

The system **MUST** refuse every elevated call past the window with `BREAKGLASS_EXPIRED` and emit
`BreakGlassExpired`. Standing cross-tenant access **MUST NOT** be grantable — the grant model has
no such shape. **Who produces the expiry event is open item 19**: the only producer named is a
refused call, so an uncalled session never emits it and a session called ten times after expiry
emits ten.

**Implements**: `cpt-cf-bss-products-flow-breakglass`

**Touches**:
- DB Table: `products_breakglass_session`

### PII block on every operator reason

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-pii-on-reasons`

The system **MUST** run `02-taxonomy-attributes`' write-block hook on every operator free-text
reason this feature stores — the submission reason, the rejection reason and the break-glass
session reason — refusing `CONTENT_PII_BLOCKED` before the row is written. These records are never
edited and erasure is a map-only tombstone, so personal data typed into one is unreachable by
erasure forever; failing closed at the door is the only reach erasure has over them. The detector
does not exist, so a **clean-text positive control is part of this obligation** — a stub that
refuses every string would otherwise satisfy it.

**Implements**: `cpt-cf-bss-products-flow-submit`,
`cpt-cf-bss-products-flow-decide`, `cpt-cf-bss-products-flow-breakglass`

**Touches**:
- DB Table: `products_approval_decision`, `products_breakglass_session`

### Governance error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-governance-errors`

The system **MUST** declare its six codes as constants on their raising rules and register them
into the Foundation's taxonomy, each carrying its declared RFC 9457 status.
`APPROVER_ROLE_REQUIRED` **MUST** be raised by the **gate** when the descriptor is numerically met
and the role predicate is not; `APPROVAL_SUPERSEDED` **MUST** be raised at **decide**.
`APPROVAL_REQUIRED` stays `01-foundation`'s.

**Implements**: `cpt-cf-bss-products-algo-governance-errors`

**Touches**:
- Entities: `CanonicalError`

### Governance events

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-governance-events`

The system **MUST** emit `ApprovalDecided` on both verdicts, `BreakGlassElevated` and
`BreakGlassExpired` through the Foundation's outbox in the mutating transaction. Submissions and
supersessions **MUST** emit no broker event — the queue is a pull surface and every submission
already rides the entity's own audit row — and that absence **MUST** be recorded as an explicit
no-event declaration.

**Implements**: `cpt-cf-bss-products-flow-submit`,
`cpt-cf-bss-products-flow-decide`, `cpt-cf-bss-products-flow-breakglass`

**Contract**: `cpt-cf-bss-products-contract-registry-events`

**Touches**:
- Entities: `ApprovalDecided`, `BreakGlassElevated`, `BreakGlassExpired`

### Audit trail for governance acts

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-governance-audit`

The system **MUST** write the Foundation's append-only audit trail for the acts this feature owns
that emit no broker event: every refusal, every read under elevation, and every committed act
declared to emit none. Audit **sealing is a platform capability and is deliberately not built
here** — v1 ships completeness over a reserved, unwritten sealing seam, and tamper-evidence does
not ship. Until that capability activates, immutability is the trigger whitelist on both engines
and nothing cryptographic.

**Implements**: `cpt-cf-bss-products-flow-decide`,
`cpt-cf-bss-products-flow-breakglass`

**Touches**:
- DB Table: `products_audit_log`

## 6. Acceptance Criteria

- [ ] Submitting a material change writes a record whose stored descriptor carries `required = N`;
      a non-material one carries `min(N, 1)`; both carry `configuredQuorum` as the raw `N`
- [ ] Editing the head after submission leaves the superseded record's diff rendering the
      **original** submission against the published version, and the probe is red before the fix
- [ ] Editing the tenant's `N` does not change a pending record's `configuredQuorum`
- [ ] A finance-material change at `N ≥ 1` carries the FinanceReviewer predicate; at `N = 0` it
      carries `predicateUnsatisfiable` instead and stays satisfiable
- [ ] A one-person tenant publishes their first `product` SKU with a required `taxCategory`
- [ ] One human holding CatalogAdmin and FinanceReviewer counts once, and satisfies the finance
      predicate only as one of the two
- [ ] The author's own decision is refused `SELF_APPROVAL_FORBIDDEN` at `N ≥ 1`; a different
      principal's decision is admitted
- [ ] An out-of-scope approver is refused `APPROVER_SCOPE_EXCEEDED` and audited; an in-scope
      approver is admitted
- [ ] An unrestricted claim set covers every subject; an unrestricted subject scope is covered
      only by an unrestricted claim set
- [ ] A decision on a superseded record is refused `APPROVAL_SUPERSEDED`; a decision on a pending
      one is admitted
- [ ] A record numerically met with the role predicate unmet is refused `APPROVER_ROLE_REQUIRED`
      at the gate; with the predicate met it publishes
- [ ] A rejection finalizes the record with its reason, leaves the subject unchanged, and emits
      `ApprovalDecided`; an approval emits it too
- [ ] Two publishes off one satisfied approval produce one success; the second fails
- [ ] A failed publish attempt consumes nothing, and the record is still `satisfied` afterwards
- [ ] A `PreAuthorized` stage naming a consumed record that authorized this subject at this pinned
      revision succeeds and consumes nothing further
- [ ] No wire surface can select `PreAuthorized` — asserted against the request shape, not by
      inspection
- [ ] A `system_signal` publish on a clean head is auto-satisfied and audited; on a dirty head it
      is **deferred**, not refused
- [ ] An override subject requires each approver to acknowledge the named findings; at `N = 0` the
      author acknowledges and the record carries `quorumReduced`
- [ ] Publishing an override subject with no acknowledgment on the record fails on every lane,
      bulk included
- [ ] The pending queue returns the envelope with `required` as the effective count, and a
      non-material record at `N = 2` renders "1 required", not "2"
- [ ] A caller without `approval × read` is refused — and the code and status it meets are open
      item 3, so this criterion cannot be fully written until that is answered
- [ ] A write under elevation is refused `BREAKGLASS_WRITE_FORBIDDEN`; a read is admitted
- [ ] Every elevated read leaves an audit row carrying the session id, with the row **count**
      asserted rather than a sample inspected
- [ ] A call past the window is refused `BREAKGLASS_EXPIRED` and emits `BreakGlassExpired`
- [ ] Clean operator free text is **admitted** at all three reason doors, and prohibited content is
      refused `CONTENT_PII_BLOCKED` — the positive control on the stub
- [ ] A bucket-iii touch is material; a bucket-iv-only re-publish is `min(N, 1)`; a policy-object
      mutation is material in either direction
- [ ] Materiality is evaluated once: a policy change between submit and publish neither re-judges
      nor voids the pending approval
- [ ] A second submission on a subject with one open approval supersedes it rather than creating a
      second, proven against the partial unique index
- [ ] Each of the six codes is raised by exactly one rule and carries its declared status
- [ ] A schema-oracle golden exists for all three tables on both engines with a perturbation case
      proving it can fail
- [ ] Registering this feature's host replaces `NoMaterialityPolicyGate`, and a test asserts the
      gear no longer runs the no-policy host
- [ ] No `#[ignore]`d test exists without a CI tier that runs it

## 7. Known unknowns

[`../design/05-governance.md`](../design/05-governance.md) §6 carries **23 open items**, and all
twenty-three are carried here with the DoD each blocks and its owner. This is the largest open set
in the gear, and its size is the point: this feature is the one enforcement point, so a question
left in it is a question about every other feature's gate.

**None of these is answered here.**

| # | The question | Blocks | Owner |
|---|---|---|---|
| 1 | **Eleven of the twenty-three grant rows carry no route.** Nine of the eleven are unmeasured, and an authorization surface nobody can enumerate is one nobody can review. Whether the fix is declaring the routes or admitting the grants are unspent is not a review's call | `dod-rbac-catalog` | this feature with each door's owner |
| 2 | **Does the discard door get its own grant, or inherit `product\|sku × write`?** `01-foundation` §2 declares the route under `× discard` and the catalog carries only `read\|write\|publish`. `authz.rs` took `write` and recorded the contradiction in its own module doc. Minting `discard` lets a tenant withhold it; folding it into `write` does not | `dod-rbac-catalog` | this feature |
| 3 | **What code does an authorization denial carry, and what status?** Every door opens by authorizing deny-by-default and no slice declares a denial code, while the Foundation requires every code to carry a status and every refusal to be audited with its reason. **So the first step of every registry door terminates in a refusal with no code for a consumer to match on** | `dod-rbac-catalog`, `dod-inbox-envelope` | the governance owner with the taxonomy owner |
| 4 | **The studio inbox envelope is design-introduced.** The sibling gear's queue shape should be cross-checked when `12-consumer-contracts` pins the SDK; a field-name drift here costs a UI adapter later | `dod-inbox-envelope` | this feature with 12 |
| 5 | **Post-hoc break-glass review needs an owner and an SLA** for its obligation alert | `dod-breakglass-open` | the ops owner |
| 6 | **Approval retention and erasure interplay** is `10-retention-erasure`'s; this feature guarantees only that approver refs are pseudonymous from birth | `dod-decision-store` | 10 |
| 7 | **Does the authoring head read need an action of its own?** The Foundation's `GET` is an authoring read and its own §4.3 says that read is not a consumer read, while the catalog lists only `read\|write\|publish` per kind | `dod-rbac-catalog` | this feature |
| 8 | **What does `Gate` mode require of a gated transition?** The Foundation's mode instruction is worded for a publish and pins "the door's expected revision", while the transition doors are this feature's and `04-lifecycle`'s and pin nothing stated there | `dod-gate-host`, `dod-preauthorized-mode` | this feature with 04 |
| 9 | **Is a break-glass two-person approval an `ApprovalRecord`, and what holds its fixed floor?** The elevation demands two distinct platform principals outside the tenant's `N`, while `required` is defined only as `N` or `min(N, 1)` — **no writer can produce a fixed 2** — the store's row is tenant-scoped, and the approver-scope rule would refuse a platform approver on another tenant's subject. The stored approval path is a reference to an unnamed thing | `dod-breakglass-open`, `dod-approval-store`, `dod-quorum-descriptor` | this feature with the platform-identity owner |
| 10 | **Where is the `N = 0` override acknowledgment stored?** The author performs it and the acknowledgments are said to live "on the record and in audit", but the only column for them sits on the decision row, which demands an approver principal and a verdict the author does not have, and the approval row has no acknowledgment column | `dod-override-ceremony`, `dod-approval-store` | this feature's storage owner |
| 11 | **Which transaction writes `state = satisfied`?** Every other value has a named writer; this one has only an evaluator, and nothing says whether a record at `required = 0` is born satisfied. If satisfaction is evaluated at gate time instead, the `satisfied` branch of the partial unique index is dead | `dod-quorum-evaluator`, `dod-approval-store`, `cpt-cf-bss-products-state-approval-record` | this feature |
| 12 | **What door carries submit, decide and break-glass elevation?** The catalog mints `approval × submit\|decide` and `breakglass × elevate`, and the only route this feature declares is the inbox `GET`. The Foundation closes its own set at five wire doors | `dod-decide`, `dod-breakglass-open`, `dod-rbac-catalog` | this feature with the contract owner |
| 13 | **Is `APPROVER_ROLE_REQUIRED` 403 or 409?** The convention puts 409 where the current state refuses the act and 403 where the caller may not act at all; by its stated raise site the caller may publish and it is the record's state that refuses — which is where the sibling `APPROVAL_SUPERSEDED` sits at 409 | `dod-governance-errors` | the governance owner with the taxonomy owner |
| 14 | **What do the entity-shaped columns hold for the non-entity subject kinds?** The pinned revision and diff basis are entity-shaped, and four of the five subject kinds are not entities | `dod-approval-store` | this feature |
| 15 | **Does `quorumReduced` fire on every non-material change at the default `N = 2`?** A bucket-iv-only re-publish has an effective count of 1, so the marker would ride the majority of records, while the decision that introduced it frames it as a marker for the *reducible ceremonies*. Nothing distinguishes reduced-by-configuration from reduced-by-non-materiality | `dod-quorum-descriptor` | this feature with the audit consumer |
| 16 | **Does the base role set bind the single approver of a non-material change?** The constraint scopes its CatalogAdmin-or-FinanceReviewer floor to material changes, and a non-material one gets `min(N, 1)` with no base role set on the descriptor. Nothing says whether any holder of `approval × decide` may close one | `dod-quorum-evaluator` | this feature |
| 17 | **Does AC #26's third bullet still bind?** It carries both a superseded two-person count and a `draft` return the head-row model cannot honour; the decision that rewrote the first two bullets names neither | `dod-decide` | the PRD owner with the governance owner |
| 18 | **What does an elevation change about the authorization decision?** Deny-by-default runs over tenant-scoped claims and all repository access goes through tenant-scoped ORM queries; **no rule anywhere says how a live session widens either**, nor where in the pre-pipeline gate that operand is read | `dod-breakglass-readonly`, `dod-rbac-catalog` | this feature with the `ToolKit` owner |
| 19 | **Who produces `BreakGlassExpired`, and what happens to an act in flight at expiry?** The only producer named is a refused call, so an uncalled session never emits it and a session called ten times after expiry emits ten; no sweeper is named, and nothing says whether a read begun inside the window may finish | `dod-breakglass-expiry`, `dod-governance-events` | this feature with the ops owner |
| 20 | **What is the post-hoc obligation's state set, and who discharges it?** The state is stored and the review obligation is raised as an alert; no door, event or flow writes a discharge, and no values are enumerated | `dod-breakglass-store`, `dod-breakglass-open` | this feature |
| 21 | **Is a sealing capability owed, and on what terms?** Audit sealing is a platform capability this gear deliberately does not build; the requirements it must satisfy are carried as a PRD open owned by Architecture, and until it activates the gear ships completeness without tamper-evidence | `dod-governance-audit` | Architecture |
| 22 | **The break-glass window's two normative facts live only in the PRD's interim-policy table** — a 4-hour interim **and** "no renewal without a new session". The elevation instruction states neither, and renewal is neither forbidden nor admitted | `dod-breakglass-open` | the governance owner with the §17.1 owner |
| 23 | **Is the approval hook's no-fire exception still worded for `draft→published` alone?** A later decision widened the unit to any transition that consumes an approval in the same transaction, and this slice cites that decision nowhere. Either the exception widens with it, or the narrower wording is deliberate and says so | `dod-supersede` | this feature |

### Raised here rather than carried

- **The `NoMaterialityPolicyGate` handover has no stated moment.** `01-foundation` ships that host
  as what the gear runs "until slice 05 registers a host", and nothing says whether registration is
  a startup wiring change, a migration, or a configuration flip — nor what a deployment does with
  approval records created while the no-policy host was live. *Owner: this feature with 01.*
- **`RecordingGate` is the double this feature retires.** It exists today in `01-foundation`'s
  publish-door tests and is the shape `03-sku-classification`'s DoDs need for their own approval
  probes. When a real host lands, whether that double stays for the Foundation's own isolation or
  is replaced by the host is unstated, and three features' test plans depend on the answer.
  *Owner: this feature with 01 and 03.*
