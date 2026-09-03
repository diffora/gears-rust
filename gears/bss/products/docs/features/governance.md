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
- Surfaces: `cpt-cf-bss-products-usecase-approval-publish` — listed in §2.5's own Requirements
  Covered block, **one of six entries that carry a surface there**, so this line is carried rather
  than claimed

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
catalog below. **Rate limiting** on elevation and on decide is the platform gateway's:
`APPROVER_SCOPE_EXCEEDED` and `SELF_APPROVAL_FORBIDDEN` are enumerable oracles over principals and
scopes, and an unthrottled elevation attempt is an unbounded cross-tenant probe, so the delegation
is named rather than left silent. **Write-path latency** on the gate phase is
`01-foundation`'s publish-door budget, which this phase runs inside; the one unbounded thing this
feature adds is the stored content snapshot, bounded transitively by the entity's own frozen-content
cap. **Presentation** is the studio's: this feature owns the stored by-name acknowledgment set and
the envelope's field semantics, not the rendering, and accessibility is not applicable to an API and
SDK surface. **Rollback** of the host registration means reverting to the no-policy host, which
leaves every record this feature wrote intact and unconsumable — see the first raised item in §7.

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
  a **consumer**. `02`, `03`, `04`, `06`, `07`, `09` and `10` each register a `GovernedLiveOp`
  kind, a role predicate or an override condition with it; `08` and `12` consume it differently —
  the read model projects approval state and the consumer contract pins the inbox envelope.
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
`cpt-cf-bss-products-contract-governance-errors`. `APPROVAL_SUPERSEDED` sits at 409 and this
feature's other five at 403; the two borrowed codes carry their own statuses —
`APPROVAL_REQUIRED` at 403 and `CONTENT_PII_BLOCKED` at 422. **`APPROVER_ROLE_REQUIRED`'s status is open item 13**: the convention puts 409 where the
current state refuses the act and 403 where the caller may not act at all, and by its stated raise
site the caller may publish while it is the record's state that refuses.

## 4. States (CDSL)

### ApprovalRecord State Machine

- [ ] `p1` - **ID**: `cpt-cf-bss-products-state-approval-record`

The five rows below are this document's rendering of the slice's §4 column domain; see §1.4.

**States**: `pending`, `satisfied`, `consumed`, `rejected`, `superseded`

**Initial State**: `pending` for every human-submitted subject. A `system_signal` record is
inserted `pending` and flipped `satisfied` **in the same transaction**, so row 1's auto-satisfy arm
is a transition and not a second initial state.

**Transitions**:
1. [ ] - `p1` - **FROM** `pending` **TO** `satisfied` **WHEN** the quorum evaluator finds the stored descriptor met by distinct principals holding the required roles — **or, for a `system_signal` subject, at submission, the signal reference being the authorizing principal**. *Which transaction performs this write is open item 11: every other value has a named writer and this one has only an evaluator* - `inst-ap-edge-satisfy`
2. [ ] - `p1` - **FROM** `satisfied` **TO** `consumed` **WHEN** the authorized act commits, in the **same transaction** as that act; a failed attempt consumes nothing, and a `PreAuthorized` stage naming an already-consumed record consumes nothing further - `inst-ap-edge-consume`
3. [ ] - `p1` - **FROM** `pending` **TO** `rejected` **WHEN** an approver holding `approval × decide` rejects with a mandatory reason; the subject stays as it was - `inst-ap-edge-reject`
4. [ ] - `p1` - **FROM** `pending` **TO** `superseded` and **FROM** `satisfied` **TO** `superseded` **WHEN** a frozen-content write lands on the subject and fires the Foundation's approval-invalidation hook; re-submission is an explicit human act and is never automatic - `inst-ap-edge-supersede`
5. [ ] - `p1` - **No transition other than those above is admitted** — in particular there is no path out of `superseded`, so the auto-resubmit the design forbids is unreachable — `rejected`, `consumed` **and `superseded`** are all terminal, and the record is **append-only after finalization** - `inst-ap-terminal`

**The `pending → satisfied` writer is unnamed**, which is open item 11 rather than a gap this
document fills: if satisfaction is evaluated at gate time instead of written at decision time,
then the `satisfied` branch of the store's partial unique index is dead, and whether a record at
`required = 0` is **born** satisfied is unstated.

## 5. Definitions of Done

Twenty-seven, counted by `grep` on this file rather than from the plan that sized them.
**Twenty are separately testable.** Seven are not, and each names what it needs:
`dod-gate-host`, `dod-one-shot-consumption` and `dod-preauthorized-mode` are exercised through
`01-foundation`'s publish door and need that door's test harness — the existing `RecordingGate`
double is the shape, and this feature replaces it with a real host; `dod-pii-on-reasons` needs
`10-retention-erasure`'s detector, which does not exist, so it is testable only against a stub, and
a stub that refuses every string satisfies it — a clean-text control is part of the obligation;
`dod-inbox-envelope`'s merge-compatibility half is `12-consumer-contracts`'; `dod-supersede` fires
only through the Foundation's save and transition doors and needs the same harness as the three
above; and `dod-rbac-catalog`'s deny-by-default refusal cannot be asserted until open item 3
declares the denial's code, so only its declarative half — the row set, the extension of the three
shipped rows, and each pair resolving to a door or being marked unspent — is testable today.

### Approval record store

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-approval-store`

The system **MUST** create `products_approval` on both engines with the subject kind and ref, the
pinned `internal_revision`, the **stored** `content_snapshot`, the `diff_basis`, the **stored**
`quorum_descriptor`, the state, a pseudonymous submitter and timestamps. A **partial**
`UNIQUE (tenant_id, subject_kind, subject_ref) WHERE state IN ('pending','satisfied')` **MUST**
admit one open approval per subject. The row **MUST** be append-only after finalization. A
schema-oracle golden **MUST** exist on both engines with a perturbation case proving it can fail.

**The table ships and the tick does not: rows 9, 11 and 14 are live and are about this table's own
columns.** Row 11 asks which transaction writes `state = satisfied` — every other value has a named
writer and this one has only an evaluator, *"and nothing says whether a record at `required = 0` is
born satisfied"*; row 14 asks what the **entity-shaped** columns (pinned revision, content snapshot,
diff basis) hold for the subject kinds that are **not entities**, which is at least three of the
five; row 9 asks whether a break-glass two-person approval is an `ApprovalRecord` at all. The DDL
admits all five kinds and pins the shapes it can, and the three questions are about what the columns
MEAN rather than what they permit — so the table is usable and the DoD is not met.

**Implements**: `cpt-cf-bss-products-flow-submit`,
`cpt-cf-bss-products-state-approval-record`

**Constraints**: `cpt-cf-bss-products-constraint-gts-types-not-instances`

**Touches**:
- DB Table: `products_approval`
- Entities: `ApprovalRecord`

### Decision store and the one-principal-one-decision floor

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-decision-store`

The system **MUST** create `products_approval_decision` carrying the approver principal **as an
`actor_ref` — pseudonymous, never a raw identifier**, these rows being append-only, so one raw
identifier written is unreachable by erasure forever — the verdict, the reason, the override acknowledgments and the instant, with
`UNIQUE (approval_id, approver_principal)`. **That index is the physical floor under
distinctness-by-principal**: one principal, one decision, whatever roles they hold.

**The table ships and the tick does not: row 6 is live**, routing the approval retention-and-erasure
interplay to `10-retention-erasure` while this feature guarantees only that approver refs are
pseudonymous from birth. That guarantee is built — the column is an `actor_ref` and the rows are
append-only, so one raw identifier written would be unreachable by erasure forever — but the
interplay the row names is `10`'s to state.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval_decision`
- Entities: `ApprovalDecision`

### Break-glass session store

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-breakglass-store`

The system **MUST** create `products_breakglass_session` carrying the session id, the principal as
an `actor_ref` — pseudonymous, never a raw identifier — the target tenant, the reason, the window as a half-open interval, and the
approval path — a two-person reference or a post-hoc obligation **state ∈ {pending, reviewed}**
with `reviewed_by (actor_ref)` / `reviewed_at`, discharged by the **second platform principal's**
late decision (**P-D-68** — one ceremony, two timings; no new door) — plus the **`expired_emitted`**
CAS stamp the expiry event's one emitter flips (P-D-68). Elevated audit rows **MUST**
carry the session id.

**Open item 20's half is answered and the other half is not, and the table separates them.**
P-D-68 arm 3 enumerated the state set (`{pending, reviewed}`) and named the discharger (the second
platform principal's late decision — one ceremony, two timings, no new door), so this DoD's earlier
claim that neither existed no longer holds. What P-D-68
**deliberately did not** presuppose is whether that decision's record is an `ApprovalRecord`: so
`two_person_approval_ref` is a nullable reference carrying **no FK**, and a CHECK makes the two
paths exclusive rather than asserting what the reference points at. The precedent is this gear's
own — `products_bulk_batch.approval_ref` shipped without an FK for the same reason, one slice
earlier.

**Implements**: `cpt-cf-bss-products-flow-breakglass`

**Touches**:
- DB Table: `products_breakglass_session`
- Entities: `BreakGlassSession`

### Materiality evaluator

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-materiality-evaluator`

The system **MUST** decide materiality from the four declared inputs and **MUST** evaluate **once**,
at submission, against the policy in force at that instant. **Every input fails closed**: an
unresolvable materiality policy, claim set or bucket registry **MUST** refuse the act rather than
fall back to a default — a policy resolving to absent-implies-default at floor 0 would publish a
finance-material change on one signature, and the default's own wording makes that reading
arguable — never at the reader's clock and never
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

The system **MUST** require the `approval × submit` grant, **MUST** store the submitted content on
the record at submission, and **MUST** render the approver's diff from that stored copy against the
last published version, **never** re-deriving it from the live head. **On a first publish there is
no last published version**: `diff_basis` is then NULL and the diff renders as a whole-content
addition against no basis. That arm is stated because the slice makes a first publish material, so
a first-publish record exists and must carry a basis — and filling the gap by convention would most
plausibly diff the draft against the head, which is the re-derivation this rule forbids. **This is the flagship probe**: submit, edit the head, and the superseded
record's diff still renders the original submission against the published version. It **MUST** be
written red first — a re-derived diff shows the draft against itself, which is the exact defect
this rule exists to prevent.

**The flagship probe is built, and the two probes that stood in for it each covered one half.**
`repo::governance::governance_tests::the_superseded_records_diff_renders_the_submission_not_the_edited_head`
drives the whole chain — freeze a published version, submit against it, **edit the head** through
the save door's own write, run the door's own `supersede_open_approval`, read the record back, and
render from it — with the basis content read off the frozen row and the head's post-edit name read
off the head row rather than retyped. The red-first obligation was discharged by **perturbation**,
each half separately, and the split is the measurement rather than a reading of the two names:

| Perturbation | `approval_tests` renderer probe | `repo_tests` column probe | the probe above |
|---|---|---|---|
| `render_diff` stops reading its snapshot argument | red | **green** | red |
| the store stops preserving the submitted bytes | **green** | red | red |

So the renderer probe is narrow rather than empty — it does catch a renderer that drops its
argument — but it has no head, no store and no supersession, and the `edited_head` local it names
is passed to nothing. **That assertion has been removed rather than kept**: a non-falsifiable
assertion in a probe is indistinguishable from a passing one, and the claim it reached for lives in
the store probe where a head really moves. (Two earlier statements of this were themselves wrong —
the body carried **four** assertions, not three, and the vacuous one was the fourth. The probe is
now named for what it measures, `the_diff_renders_the_snapshot_it_was_handed`.) The column probe
catches the
store, but supersedes by a second *submission* rather than by a head edit and never renders a diff.
Neither could see the defect the rule is about: a **caller** handing the live head over where the
stored snapshot belongs. The probe above renders the live head a second time and asserts the two
answers differ, so the positive assertion is not satisfiable by both.

**The tick still does not follow, and the blocker is the first clause, not the probe.** This DoD
opens *"MUST require the `approval × submit` grant"*; `authz.rs` mints
`approval × submit` and **no door spends it** — §7 row 12, and §3.2 of the slice records the pair as
having no route declared. The grant clause is unbuildable until that door lands, and wire doors are
not this slice's to declare.

**Implements**: `cpt-cf-bss-products-flow-submit`

**Touches**:
- DB Table: `products_approval`

### Stored quorum descriptor

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-quorum-descriptor`

The system **MUST** store the descriptor at submission and **MUST NOT** re-derive any of it:
`required` as the **effective** count, `configuredQuorum` as the `N` in force at submission, the
finance predicate, `predicateUnsatisfiable`, `quorumReduced` and the override conditions. Deriving
`configuredQuorum` from current policy would change a **pending** record when the tenant edits `N`.

**Five of the six ship, and the missing writer for the sixth ceremony now exists.** `describe_quorum`
stores `required`, `configuredQuorum`, the finance predicate, `predicateUnsatisfiable` and
`quorumReduced`; the **override conditions** are the sixth name and wait on `dod-override-ceremony`'s
missing operand, so the roster is five by declaration rather than by omission. Separately,
`describe_platform_quorum` is the writer §7 row 9 says does not exist — *"`required` is defined only
as `N` or `min(N, 1)` … **no writer can produce a fixed 2**"* — supplying **P-D-13**'s one
non-`N`-governed site, cross-tenant break-glass elevation, at a fixed floor of two platform
principals. It answers only the floor: whether that ceremony's record is an `ApprovalRecord`, and
which row holds it, are row 9's other halves and are untouched, because a descriptor is a value that
renders the same whichever row stores it. The probe is armed at **`N = 0`** on purpose — at `N = 2`
a fixed floor and a configured count are indistinguishable.

**What still blocks the tick**: the sixth name, plus §7 rows 15 and 39 on what `quorumReduced`
means. **And one field this build had to read and no artifact defines**: what `configuredQuorum`
carries for a ceremony no tenant configures. It carries the floor here, because `inst-gv-queue` puts
the field on the wire and a card rendering the target tenant's `N` beside a platform ceremony would
assert exactly the standing P-D-13 denies — but that is a reading, and it is registered rather than
presented as the design's.

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

**Both arms ship and are probed across `N = 0..4`; the operand does not.** `describe_quorum` sets
the predicate at every `N >= 1` and records `predicateUnsatisfiable = finance_reviewer` at `N = 0`
instead, leaving the descriptor satisfiable. **The tick is blocked on the two inputs neither the
gear nor the donor has.** *Whether a change is finance-material* cannot be computed: the
instruction names `taxCategory`, `glCode` and `PlanTier`, and none is a registered column in
`domain::bucket`'s roster — they are 03's and 03 has not registered them — so a registry lookup
would answer "not finance-material" for all three and `finance_material` is an argument.
*Whether a principal holds FinanceReviewer* is §7 row 25. Measured 2026-09-02 against the donor as
well: `gears/bss/pricing` resolves its own `FinanceReviewer` through the **grant**, which answers
for the caller in front of it and cannot answer C1's question, which is about the roles a set of
**already recorded** approvers held when they decided. So this is not the donor's shape declined;
it is an operand neither gear has.

**Implements**: `cpt-cf-bss-products-flow-submit`

**Touches**:
- DB Table: `products_approval`

### Supersession on frozen-content write

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-supersede`

The system **MUST** flip an open record `superseded` when the Foundation's approval-invalidation
hook fires on a frozen-content write to the subject, and **MUST** require an explicit human
re-submission with the new diff re-presented. Re-submission **MUST NOT** be automatic. The hook
**MUST NOT** fire on any transition that **consumes an approval in the same transaction** — the
publish edge and every gated edge alike — because a hook firing against the record the act is
consuming has no defined ordering. That is the slice's C3 as it now stands, re-measured
2026-08-31; see open item 23 for the stale §6 bullet that said otherwise.

**Built, and the third clause needed no code because the floor already carries it.**
`transition::invalidation_for` returns `ApprovalInvalidation::Skip` for exactly the consuming
edges — *"a hook firing against the record the act is consuming has no defined ordering"* — so the
doors read the floor's answer rather than re-deriving the condition, and a probe holds it closed:
a publish against a `satisfied` record leaves it `satisfied`, not `superseded`.

**The store-backed write lives in the door's transaction, not behind the domain trait, and the
reason is the trait's shape.** `ApprovalInvalidationHook::invalidate(&self, subject)` is
**synchronous and storeless** — right while there was no store to reach, and unable to carry a
transactional write now that there is one. Changing that signature is `01-foundation`'s act
(`dod-approval-hook`) and a different one from this, so the seam still runs as the pure half and
`repo::supersede_open_approval` runs beside it on the transaction the act already holds.

**The supersede carries the open-state predicate on its `UPDATE`, and that is what makes the
concurrency claim true rather than merely stated.** The first build filtered the write by id alone
— the predicate sat on the preceding read — so two concurrent frozen-content writes both saw the
open record, the winner finalized it, and the loser's write met
`'products_approval: a finalized approval is immutable'`: a **legal** act answering 500. Three
independent review lenses found it. With the predicate on the write the loser matches zero rows and
reports nothing superseded, which is the same answer as "nothing was open". A subject with no open record is a **no-op rather than an
error**, because the write is legal whether or not a ceremony was open against it, and a finalized
record is outside the predicate — on the read **and** on the write — and never reopened. Three probes: the supersede itself with a
record count proving **nothing was re-submitted**, the consuming edge, and the no-open-record and
finalized cases together.

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

**Built as `evaluate_quorum`, with the named probe and its companion.** One human appearing twice
under two roles counts once, and a second distinct principal closes the record — the probe the DoD
names, plus the positive control without which "counts once" is satisfied by an evaluator that
counts nobody. Its companion is `design/05` §5's second bullet: the dual-role human supplies the
finance lens but **not** the second body, asserted in both directions. A recorded
`predicateUnsatisfiable` counts as met, and the probe is armed at `N = 0` **and** swept across
`N = 1..4` to show the discharge `inst-gv-quorum` forbids above zero is unreachable rather than
merely unused, the marker never being recorded there. A count met with the lens missing is its own
answer (`RolePredicateUnmet`), not a short count — L-2's distinction, without which a caller told
"not enough approvers" adds a third CatalogAdmin and fails again.

**Three blockers on the tick, and one deliberate non-answer.** §7 row 25 — no surface carries a
role, so `roles` is a per-decision operand carrying *what was true when the decision was made*,
which is what a gate-time evaluation needs and what row 25 says the decision row does not store.
§7 rows 11 and 31 — this function answers the **arithmetic** and takes no position on which
transaction writes `state = satisfied`, nor on whether a record at `required = 0` is born
satisfied. **The non-answer is §7 row 16**: C1 scopes its base role set to material changes and the
descriptor carries no `Materiality`, so the binding set is a **call operand** — `BaseRoleSet::C1`'s
pair, or `::AnyDecider` for row 16's other reading. It was a `&[ApproverRole]` whose **empty** value
meant "anyone counts", and the 2026-09-02 review measured why that was wrong in two directions: the
empty slice is the only value a caller can supply today (§7 row 25 again), so a *material* change
closed on two principals holding neither C1 role; and the permissive reading was defended by citing
row 16, which is scoped to **non-material** changes only. There is deliberately no narrowing
variant — C8 says predicates *"narrow within the C1 base set and never replace it"* and that v1
registers no extension point that could, and a caller passing `[CatalogAdmin]` alone dropped a
FinanceReviewer-only approver together with the lens `inst-gv-finance-predicate` needs.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval_decision`
- Entities: `QuorumEvaluator`

### Self-approval refusal

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-self-approval`

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

**Built as `approver_covers_subject`, and the mapping is the whole rule.** It does not
re-implement containment — `domain::containment::contains` already carries P-D-39's three clauses
— it names which side is which: **parent = the approver's claims, child = the subject's scope**.
That is forced by the instruction's own wording, since *"an unrestricted subject scope is covered
only by an unrestricted claim set"* is clause 2, the asymmetric one. Transposed, a
region-restricted approver would cover a tenant-wide subject, which is the scope rule deleted
rather than applied — so clause 2 is probed as the case that flips, with the paired in-scope
control and a brand-dimension case without which a rule checking region twice would pass every
other assertion.

**Two blockers on the tick.** `APPROVER_SCOPE_EXCEEDED` is declared at 403 by §3.3 and **is not in
`domain::error` at this commit** — two of the slice's six codes ship and four do not — so the
refusal is carried as a value naming both scopes and the dimension that failed, and its
registration is `dod-governance-errors`'. And the DoD's *"MUST audit it like any scope violation"*
has a writer, `repo::write_refusal_audit` — which, corrected on re-measurement 2026-09-02, **does
have a production caller**: `api/rest.rs`'s `audit_refusal_of_action_and_report`, reached from 30
door sites. What is missing for *this* DoD is a decide door whose refusals ride it (§7 row 12), not
the lane itself. The approver's own claims are §7 row 25 again.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval_decision`

### Decision recording and rejection

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-decide`

The system **MUST** require the `approval × decide` grant and refuse a caller without it
**before any row is appended**, **MUST** append one decision row per approver, refuse a decision on
an already superseded record with `APPROVAL_SUPERSEDED`, finalize a rejection with its mandatory reason while
leaving the subject unchanged, and emit `ApprovalDecided` on either verdict.

**The finalization had no writer and now does.** `record_decision` appended the row, refused an
unreasoned rejection, a second verdict from one principal, an author's own and a decision on a
closed record — and then left the record `pending`, so a rejection changed nothing. `finalize_rejected`
is §4 row 3's edge (`inst-ap-edge-reject`), flipping `pending -> rejected` and stamping
`finalized_at` in the **same transaction as the decision row**; `chk_products_approval_finalized`
pins that pair on both dialects, so a flip writing one without the other is refused by the engine.
The probe asserts the head is untouched as well as the record finalized — a finalizer that also
moved the subject would satisfy the first half alone — with the paired control that an **approval**
finalizes nothing, without which a flip on either verdict would close every record on its first
signature.

**The `UPDATE` carries the open-state predicate, which is `supersede_open_approval`'s lesson applied
rather than rediscovered.** That function's first build filtered by id alone with the predicate on
the preceding read, so two concurrent writes both saw the open record and the loser met the
append-only trigger — a **legal act answering 500**, found by three review lenses. Here zero rows
matched is a **refusal** rather than that function's no-op, because a decision row has already been
appended in this transaction and must roll back with it.

**A rejection on a `satisfied` record is refused rather than appended-and-not-finalized.** §4 row 5
closes the machine and admits no `satisfied -> rejected` edge; the alternative leaves a recorded
rejection against a record the gate would still authorize. The paired control is that an approval in
the same state is admitted, which makes the refusal about the edge and not the state. No writer
produces `satisfied` at this commit (§7 row 11), so the probe writes it by hand — the shortcut
`repo_tests` already takes for a state whose door is not this slice's.

**Two clauses remain unbuildable, and neither is arithmetic.** The `approval x decide` grant must be
refused *before any row is appended*, and there is no decide door to refuse at (§7 row 12); and
`ApprovalDecided` **does not exist** in `infra/events.rs`, so "emit on either verdict" is a
`dod-governance-events` patch. §7 row 17 (AC #26's third bullet) is also live against this DoD.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval_decision`

### Override ceremony

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-override-ceremony`

The system **MUST** require each approver to acknowledge the named lint findings **by name** where
the subject carries override conditions, and **MUST** store the acknowledgments on the record and
in audit — an informed override, never a blind one. At `N = 0` the **author** performs the
acknowledgment **in nullable `author_override_ack` / `author_override_ack_at` on
`products_approval`, written by the submit door only at effective quorum zero** (**P-D-68** — a
synthetic decision row would break C2's one-principal-one-decision UNIQUE), and the record carries
`quorumReduced`. The record **MUST** be the ceremony's only
home: a lane that publishes an override subject without one is a defect, not an exemption.
**What is still owed is the operand, not the storage**: the acknowledgment is "by name" over the
subject's own override conditions, and no artifact says where a subject's lint findings are read
from — `domain::validation`'s report carries no override-condition set, so nothing today can tell
an approver which findings they must name.

**Re-measured 2026-09-02 and the claim holds.** `ValidationReport` carries `Violation`s and an audit
code, and `OverrideCondition`, `LintFinding` and `AttentionCondition` return **zero** hits across
`domain/`. So the storage is complete on both sides — `override_acknowledgments` on the decision row
for `N >= 1`, `author_override_ack`/`author_override_ack_at` on the record at effective quorum zero,
routed by `ack_placement` and refused by `submit_approval` at any other count — and the *operand* is
what does not exist. An acknowledgment "by name" over a set nothing produces cannot be probed, so
this DoD's remaining half is a blocked build rather than an unwritten one. Its third clause — *"the
record MUST be the ceremony's only home: a lane that publishes an override subject without one is a
defect"* — is an assertion about the publish lanes, which are `01`'s and `09`'s doors.

**Implements**: `cpt-cf-bss-products-flow-decide`

**Touches**:
- DB Table: `products_approval`, `products_approval_decision`
- Entities: `OverrideCeremony`

### The gate host

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-gate-host`

The system **MUST** implement `01-foundation`'s `GovernanceGate` trait and register that host in
place of `NoMaterialityPolicyGate`, which is what the gear runs until this feature exists. It
**MUST NOT** mint a **parallel** vocabulary: `GateMode`, `GateVerdict`, `GateAuthorization`,
`ApprovalDisposition`, `EntityRef` and `ApprovalId` already exist and are the seam. **Extending
that seam is a different act and is expected** — the module records three extensions this feature
must decide rather than avoid: how candidate records reach a synchronous host (open item 28), how a
subject that is not a `Product` or `SKU` is expressed (item 29), and how a second gate code is
returned (item 30). Each is a signature change to `01-foundation`'s own type, taken jointly with
that feature, **not** a second vocabulary declared here. The verdict
**MUST** carry the authorizing record's id and whether that record held the uncomposed-bundle
override acknowledgment, **and nothing more** — the Foundation learns nothing about who approved,
against which rule, in how many steps or when.

**The host is built and is not registered, and the second half is blocked by more than a file
boundary.** `domain::approval::StoredApprovalGate` implements the trait over candidate records the
door loaded, minting no vocabulary: it returns `01`'s `GateVerdict`, `GateAuthorization` and
`ApprovalDisposition` and declares none of its own. Item 28's choice is taken and taken the only way
available — *"an operand the door already loaded inside its transaction"* rather than *"an async
widening of this signature"*, because the widening is a change to `01`'s own trait in
`domain/governance.rs`, which this slice may not make. Holding what it was given also keeps the host
a pure function, so every rule is probed without a database and `evaluate`'s `Err` arm — reserved
for a host that could not **reach** an answer — is genuinely unreachable.

**Row 26 is right and its enumeration is wrong; both are measured.** The row reads *"Six production
call sites pass `GateMode::Gate`: publish, discard and save on both entities"*. At `HEAD` on
2026-09-02 there are **seven** `.evaluate(` call sites, and **five** pass the `GateMode::Gate`
literal — `products::run_deprecate`, `run_discard`, `run_save`; `skus::run_discard`, `run_save`.
The other two are `products::run_publish` and `skus::run_publish`, which pass their `mode`
**argument** (P-D-30's), set to `Gate` by the routed handler. So the row names publish among the
literal-passers, where it does not belong, and misses `run_deprecate`, a lifecycle transition it
does not mention at all. The substance is **worse** than stated, not better: all four Product sites
build a byte-identical triple — `GateSubject::entity_publish(EntityRef { .. })`,
`InternalRevision::new(inputs.expected)`, `Gate` — so nothing in `(subject, revision, mode)`
separates a publish from a save, a discard or a deprecate, and the mode does not either.

**What the build does about it, and what is owed to `01`.** The act operand rides the host's
**construction** — `StoredApprovalGate::governed(candidates)` or `::ungoverned()` — so a caller
cannot build a host without saying which kind of act it holds it for, and row 26's two wrong
answers stop being reachable by default. A paired probe drives the identical triple through both
constructions and asserts two different correct answers. But **the doors must then choose at each of
the seven sites**, and the doors are `01`'s. Named for the lead: either `GovernanceGate::evaluate`
gains a fourth argument carrying that distinction, or each door constructs the host it needs. The
first puts the operand where a host can be sure of it; the second needs no seam change and is what
the built type is shaped for. **Until one is taken, wiring any store-backed host is a choice between
refusing every save in the gear and preserving the no-policy deviation on the publish path.**

**Implements**: `cpt-cf-bss-products-flow-gate`

**Touches**:
- Entities: `GovernanceGate`, `GateVerdict`

### One-shot consumption

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-one-shot-consumption`

The system **MUST** flip a satisfied record `consumed` **in the same transaction as the authorized
act**, and a failed attempt **MUST** consume nothing. A probe **MUST** drive two publishes off one
satisfied approval and prove the second fails.

**The flip had no writer and now does; the transaction half is the door's and is not claimed.**
`repo::consume_approval` carries `state = 'satisfied'` on the **`UPDATE`**, not on a preceding read,
so two acts racing off one record produce exactly one `Consumption::Spent` whatever order they ran
in — the DoD's own probe, at the store. Zero rows matched answers
`Consumption::AlreadySpentOrClosed` rather than an error, because the caller is what knows the
meaning: a second publish must refuse while a `PreAuthorized` stage that raced a peer has the answer
it wanted, and reporting a driver failure would send both to a 500. Its companion probe drives a
**`pending`** record, without which a predicate that had drifted to "any state" would pass the
first probe unchanged. *"In the same transaction as the authorized act"* is **not** measurable
here and is not claimed: this function opens no transaction, and the module says so.

**P-D-105's arm consumes nothing, and the type is what holds that.** A mechanical stage answers
`ApprovalDisposition::Verified`, whose `approval_to_consume()` is `None` by construction, so
`inst-gv-one-shot`'s *"consuming nothing further"* is a property of the verdict rather than a rule
each stage must remember. Asserted on the scheduled-flip arm as well as the ordinary one.

**Still no tick**, and the blocker is unchanged: every door in the gear registers
`NoMaterialityPolicyGate` (measured at `052666dba`), so no authorized act's transaction reaches
`consume_approval`, and the DoD's probe — *"two publishes off one satisfied approval"* — has no
publish to drive.

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

**The mode's arm is built and `bulk_worker`'s outright refusal is retired by it.** The host answers
a `PreAuthorized` stage naming a **`consumed`** record that pinned this subject at this revision,
with `ApprovalDisposition::Verified` — so `approval_to_consume()` answers `None` and "nothing is
consumed under `PreAuthorized`" is a property of the type rather than a rule a door must remember —
while `approval_ref()` still answers the id, because a mechanical stage records which approval
stands behind the frozen version even though it spends nothing. The id is matched as well as the
shape: a subject accumulates any number of `consumed` records (the partial UNIQUE bounds only the
open one), so a stage naming a *different* consumed record at the same revision is refused, which is
the weakening §7 row 27 calls turning a terminal record into an unbounded bearer token. A paired
probe asserts the two modes read **disjoint** states in both directions.

**P-D-105 unblocked the scheduled and cascade half, and B7 built it.**
`GatedAct::ScheduledFlip { row_approval_ref }` carries the decision's operand at the host's
construction, and the arm checks **both** of P-D-105's conjuncts: the named record is `consumed`,
**and** it is the record the flipped row pins in its own `approval_ref`. Subject and revision
equality are dropped here and nowhere else — the governed arm still imposes both, and a probe
asserts the same candidate that passes the flip is refused by it. The second conjunct holds by
construction at the only production call site, since the runner sources the mode's id from that
same column; it is expressed anyway, because it is the clause that separates this predicate from
the bearer token, and an in-process caller that built the mode from anything else is refused. All
three clauses were confirmed by perturbation.

`repo::gate_candidate_by_id` is the operand's reader: `gate_candidates` filters by subject, and a
cascade leg's record names the **parent**, so a query on the leg's own subject finds nothing. The
two readers share one row-to-candidate conversion so they cannot drift.

**The tick does not follow, and the reason is now narrow.** This DoD's third clause names three
composite acts — *"a scheduled act, a cascade and a bulk batch"*. P-D-105's predicate and its
writer-count guard are scoped to `insert_scheduled_transition`, so the **bulk** population is
untouched: `products_bulk_batch.approval_ref` has the same shape, but its writers are not the
counted three, so the decision's safety argument does not transfer for free. Extending the arm to
it would be authoring, not implementing. The clause *"MUST NOT be reachable from any wire surface"*
is unchanged and holds structurally: `GateMode` is reachable from no request DTO, header reader or
query extractor, and the two routed handlers pass the `Gate` literal.

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

**Nothing was built for this, deliberately.** The auto-satisfy edge is the one arm §4 row 1 gives a
named writer — *"or, for a `system_signal` subject, at submission"*, in the same transaction — so
unlike the human arm it is not blocked by row 11. It is blocked by row 14: *"the auto-satisfied
`system_signal`'s 'signal reference as the authorizing principal' **has no column**, the decision
key being `(approval_id, approver_principal)`"*, and `approver_principal` is a `Uuid` while a signal
reference is textual. A writer that flipped the record `satisfied` while silently dropping its
authorizer would produce a **directly consumable record with no recorded authority** — the exact
shape that makes a gap untraceable, and worse than an unbuilt DoD. The other two clauses have no
operand either: *"the head MUST be clean"* needs the head, and *"on a dirty head is deferred, never
refused"* needs a defer mechanism no artifact declares.

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
it. **Nine rows carry no route** (open item 1 — eleven when it was raised; **P-D-61** and
**P-D-67** doored two) and the `discard` grant question is unresolved in
the code comment as well as the design (open item 2); this DoD obliges the catalog, not the
routes.

**The catalog ships and the tick does not: seven live §7 rows name this DoD** — 1, 2, 3, 7, 12, 18
and 24. Row 12 is the sharpest of them, asking which door carries submit, decide and
break-glass elevation, and this DoD's own scope sentence (*"obliges the catalog, not the routes"*) does not
dispose of it — a catalog whose grants no door spends is exactly what rows 1 and 12 are about. An
earlier pass ticked this on the strength of that scope sentence alone, having read this §7 — **a
table** — as empty; it is 23 rows.

**Built as an extension, and the withholding is asserted too.** `authz.rs` carries **sixteen**
labels — measured 2026-09-02 — including `01`'s `product`/`sku`, `06`'s `catalog_version`, `09`'s
`bulk`, `07`'s `reference_signal`/`reference_producer`, and this slice's four:
`approval × submit|read|decide`, `materiality_policy × write`, `breakglass × elevate`,
`audit × read|export`. The eleven rows owned
by `02`, `03`, `04` and `10` are **absent by assertion**, not by omission: a test names each one
with its owing slice and fails if it appears, because a grant declared with no owning door is a
grant nobody can review — §6's own reason for counting them. Four of governance's own seven pairs
are themselves routeless, and that is the DoD's stated scope: declared, they are countable.

*This paragraph read "fourteen" until 2026-09-02, and that was **drift rather than a birth
defect**: `labels::ALL` held exactly fourteen at `6b191a157`, the commit that wrote the sentence,
and `recognized_set` and `plan_tier` landed after it. The distinction matters because the two need
different fixes — a birth defect means the census was wrong, drift means only the number is.*

**The catalog's census in this gear is not the three sites a reader may expect.** Measured at
`HEAD`: `rest_authz` has **zero** occurrences anywhere in this gear, and `Route` has **zero** in
`src/gts/`. Both belong to the **pricing** donor, which does carry
`pricing/pricing/tests/rest_authz.rs`. What products has instead is a four-site roster, and the two
tests that hold it together are what a partial edit would leave stale:
`authz::labels::ALL`; `authz::resource_types`; `gts::permissions`' twenty-seven permission
instances; and the pair of assertions
`authz_tests::labels_all_carries_every_declared_label_in_order` and
`gts::permissions::tests::catalog_resource_types_match_authz_labels_all`, which fail if any of the
first three drifts from the others.

**Implements**: `cpt-cf-bss-products-algo-rbac-catalog`

**Constraints**: `cpt-cf-bss-products-constraint-gts-types-not-instances`

**Touches**:
- Entities: `cpt-cf-bss-products-contract-rbac` — the catalog is the design set's contract, not a
  type this document mints

### Break-glass elevation

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-breakglass-open`

The system **MUST** require the `breakglass × elevate` grant, and **MUST** open a session with a
mandatory reason, a configured window and a named target tenant, under either a two-person platform approval or a recorded post-hoc review, and **MUST**
emit `BreakGlassElevated` alongside a distinct alert channel. **A failed alert emission MUST NOT
leave a silent session**: either the elevation is refused, or it opens carrying a recorded
undelivered-alert obligation — an unreviewed cross-tenant read is what the requirement exists to
prevent. **The fixed floor of two platform
principals has no writer** (open item 9), and **the window's interim value and its no-renewal rule
live only in the PRD's interim-policy table** (open item 22).

**The session's writer is built, and the floor's descriptor writer landed with B2.**
`repo::open_breakglass_session` writes the reason, the half-open window, the target tenant and
exactly **one** approval path. The exclusivity is unrepresentable rather than guarded: `ApprovalPath`
has no value that sets both columns or neither, which is what `chk_products_breakglass_path` enforces
at the engine on both dialects — and the module deliberately restates **none** of the four `CHECK`s
in Rust, because a second guard drifts from the schema. The probe that an empty reason is refused is
therefore a probe that the schema *is* the guard. `describe_platform_quorum` (B2) supplies the fixed
floor of two platform principals that row 9 said no writer could produce; row 9's other halves —
whether that approval's record is an `ApprovalRecord`, and which row holds it — are untouched.

**`discharge_posthoc_review`** closes P-D-68 arm 3's obligation with the second platform principal's
late decision: no new door, no new grant, the `pending` predicate on the `UPDATE` so two reviewers
racing produce one discharge, and nothing to discharge on a session that took the two-person path.

**Three clauses have no operand.** *"MUST emit `BreakGlassElevated` alongside a distinct alert
channel"* — neither the event type nor an alert channel exists in the gear. *"A failed alert
emission MUST NOT leave a silent session"* — the remedy it names is a **recorded undelivered-alert
obligation**, and `products_breakglass_session` has no column for one, so this cannot be built
without a migration (which is not this slice's) or an invented stub. And the window's value is left
to the caller rather than defaulted: its interim 4 hours and the no-renewal rule live only in the
PRD's §17.1 table (row 22) and `inst-bg-open` states neither.

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

**Nothing was built for the refusal, and row 18 is why.** The refusal needs an enforcement point
that reads a live session, and no rule says where in the pre-pipeline gate that operand is read or
what it changes about `authz::access_scope`'s answer or `SecureORM`'s tenant scoping. Building one
would author row 18 from a call site. `BREAKGLASS_WRITE_FORBIDDEN` is also absent from
`domain::error` (`dod-governance-errors`' patch), so the refusal has no code even once it has a
place to be raised.

**The audit half has its writer and no caller.** `repo::write_elevated_read_audit` takes the session
id and is probed at the row; the DoD's *"the count asserted, not sampled"* needs a lane that makes
several elevated reads, and no door opens one. What this group can say is the scoping boundary it
does exercise: the session is scoped by **`target_tenant`**, so a caller in another tenant's scope
gets the same answer as one naming a session that does not exist — which is a probe, not a rule
about how a live session widens anything.

**Implements**: `cpt-cf-bss-products-flow-breakglass`

**Touches**:
- DB Table: `products_breakglass_session`

### Break-glass expiry

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-breakglass-expiry`

The system **MUST** refuse every elevated call past the window with `BREAKGLASS_EXPIRED` and emit
`BreakGlassExpired` **exactly once — by the first post-expiry act, via a CAS flip of the session's
`expired_emitted` stamp in the same transaction as that refusal** (**P-D-68**: the winner emits, a
replay emits nothing; an untouched session emits no event, its expiry a stored fact observable as a
gauge with the alerting rule on top). **Expiry gates admission, not completion** — a read admitted
inside the window finishes. Standing cross-tenant access **MUST NOT** be grantable — the grant model
has no such shape. **Item 19 was the producer question and P-D-68 closed it**: the only producer
previously named was a refused call, so an uncalled session never emits it and a session called ten times after expiry
emits ten.

**The CAS is built and the count is asserted over ten calls, not two.** `repo::admit_elevated_call`
carries `expired_emitted = false` on the **`UPDATE`**, so ten post-expiry calls produce exactly one
`emit_expired: true` — item 19's own number, answered. Two calls could not tell a CAS from a
read-then-write that happened not to race, which is why the probe runs ten. **Expiry gates admission,
not completion**: the judgement is a function of the window and the instant and touches nothing the
call goes on to do.

**The window's boundaries are swept rather than sampled.** `valid_from` itself is inside,
`valid_until` itself is **outside** — the interval is `[from, until)`, and a closed one admits one
call too many — and a call *before* `valid_from` is its own answer rather than an expiry, since
folding it in would emit `BreakGlassExpired` for a session that has not begun. The
inside-the-window admission is the positive control the acceptance criteria require, without which
an inverted comparison passes every other criterion.

**Two clauses remain.** `BREAKGLASS_EXPIRED` is not in `domain::error` and `BreakGlassExpired` is not
in `infra/events.rs`, so the refusal's code and the emission itself are both `dod-governance-errors`'
and `dod-governance-events`' patches — this function answers **who** emits and the emission has
nothing to emit yet. And *"in the same transaction as that refusal"* is the caller's: this function
opens none, and a committed flip beside a rolled-back refusal is the exactly-once guarantee
inverted.

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

**Nothing was built, and both halves of the blocker are re-measured at `HEAD` (2026-09-02).** No PII
detector, hook or stub exists anywhere in the crate — `CONTENT_PII_BLOCKED` appears only in
`infra/error_mapping.rs`'s prose and is **not a declared code** — and the detector is
`02-taxonomy-attributes`'/`10-retention-erasure`'s to ship. Writing a local stub would satisfy the
obligation's letter while proving nothing, which is the trap this DoD already names.

**And §7 row 35's premise is confirmed rather than merely carried.** The DoD obliges the hook on
*"the submission reason, the rejection reason and the break-glass session reason"*, and
`products_approval`'s column roster is fourteen wide with **no `reason` among them** — only
`products_approval_decision.reason` and `products_breakglass_session.reason` exist. So this DoD names
three reasons where two are storable. Which way that resolves — the submission carries no operator
text, or the approval row owes a column — is row 35's owner call and is **not** taken here.

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

**Two of the six ship and three more are patched but unapplied; the sixth has no raising rule.**
`SELF_APPROVAL_FORBIDDEN` (403) and `APPROVAL_SUPERSEDED` (409) are declared and mapped at `HEAD`.
Strand B's patch adds `APPROVER_SCOPE_EXCEEDED`, `APPROVER_ROLE_REQUIRED` and `BREAKGLASS_EXPIRED`
to `domain/error.rs` and `infra/error_mapping.rs` — both files are append-only shared planes this
strand may not edit, so the patch is handed over rather than applied.

**`BREAKGLASS_WRITE_FORBIDDEN` is deliberately left out.** This DoD says the codes are declared *"as
constants **on their raising rules**"*, which is P-D-36's own principle, and **no rule raises this
one**: §7 row 18 blocks the enforcement point, so nothing in the gear can produce it. Declaring it
would ship a 403 with no producer — the exact finding the three-lens review of this group already
made once, one wave earlier. The code lands with the rule, and the rule waits on row 18.

**`APPROVER_ROLE_REQUIRED` is declared at §3.3's stated 403 and its status stays §7 row 13.** One
mapping line changes if the owner rules 409. Its **raise path** is untouched and remains §7 row 30:
`GateVerdict::into_authorization` maps every gate refusal to `APPROVAL_REQUIRED` by design, so a
second gate code needs the verdict widened — a change to `01`'s type, not this slice's.

**Implements**: `cpt-cf-bss-products-algo-governance-errors`

**Touches**:
- Entities: `CanonicalError`

### Governance events

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-governance-events`

The system **MUST** emit `ApprovalDecided` on both verdicts, `BreakGlassElevated` and
`BreakGlassExpired` through the Foundation's outbox in the mutating transaction —
`BreakGlassExpired`'s mutating transaction being the **first post-expiry refusal's**, whose CAS on
`expired_emitted` makes the emission exactly-once (**P-D-68**). Submissions and
supersessions **MUST** emit no broker event — the queue is a pull surface and every submission
already rides the entity's own audit row — and that absence **MUST** be recorded as an explicit
no-event declaration.

**Implements**: `cpt-cf-bss-products-flow-submit`,
`cpt-cf-bss-products-flow-decide`, `cpt-cf-bss-products-flow-breakglass`

**None of the three event types exists, and the patch reaches a fourth file.** `ApprovalDecided`,
`BreakGlassElevated` and `BreakGlassExpired` are absent from `infra/events.rs` and
`infra/broker.rs` alike. A new broker event needs **five** insertion points across **two** forbidden
files: the payload-type token, the `SCHEMA_REFS` entry and a dispatch path in `infra/events.rs`, and
the `TypedEvent` declaration plus its `producer.prepare::<T>()` registration in
`infra/broker.rs`. The `SCHEMA_REFS` entry is the one an exhaustive `match` cannot enforce — a type
added without it compiles clean and fails at enqueue.

**What the patch stops short of, and why.** The three events fit neither existing enqueue path:
`enqueue` builds a `CatalogEventCore` from an entity's core and `enqueue_set_event` wants a
`set_kind`/`member_code`. A third path is needed, and **its body fixes what a governance event
carries on the wire**, which `12-consumer-contracts` pins. The shape is sketched and named for that
owner rather than settled here. The **no-event declaration** this DoD also requires — submissions
and supersessions emit nothing, *asserted as set equality* — belongs beside the other slices' inline
notes in `design/05` (a forbidden document) and in a door's test (which does not exist); both are
named, neither invented.

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

**All three writers ship, and one of them already has a production caller.** Corrected on
re-measurement 2026-09-02: an earlier revision of this paragraph said none did.
`repo::write_refusal_audit` is called from `api/rest.rs`'s
`audit_refusal_of_action_and_report`, which 30 door sites reach — so the refusal-audit lane is
**live**, and what this DoD still owes is that *this feature's* refusals ride it, which needs the
decide, submit and elevate doors (§7 row 12). `write_eventless_act_audit` and
`write_elevated_read_audit` genuinely have no production caller. None of the three needs a
shared-plane change, so this DoD needs **no patch** — it needs doors. The elevated-read writer takes the session id and
is probed at the row; the DoD's *"the count asserted, not sampled"* needs a lane that makes several
elevated reads, and none opens one. C7's sealing seam stays deliberately unwritten (§7 row 21*).

**Implements**: `cpt-cf-bss-products-flow-decide`,
`cpt-cf-bss-products-flow-breakglass`

**Touches**:
- DB Table: `products_audit_log` — `01-foundation`'s table, written here

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
- [ ] A break-glass session **opens**: the reason, the window and the target tenant are each
      mandatory, and a request missing any one is refused
- [ ] Opening a session takes either the two-person platform approval path or records the post-hoc
      obligation, and which path was taken is readable afterwards
- [ ] Opening a session emits `BreakGlassElevated` **and** delivers on the distinct alert channel,
      the delivery asserted rather than the call inspected
- [ ] A read **inside** the window is admitted — the positive control on `BREAKGLASS_EXPIRED`,
      without which an inverted clock comparison passes every other criterion
- [ ] A caller without `approval × decide` is refused before any decision row is appended; a caller
      holding it is admitted
- [ ] A caller without `breakglass × elevate` cannot open a session
- [ ] No raw principal identifier is persisted in any of the three tables
- [ ] `N` absent from configuration resolves to 2, an explicitly configured 0 is honoured, and a
      tenant's initial `N` comes from provisioning
- [ ] A config-grant holder who does not hold `materiality_policy × write` cannot change the field
      set, the trigger or `N`
- [ ] Two decisions by one principal on one record are refused by the store's unique index, with
      the application check bypassed
- [ ] The catalog declares deny-by-default, extends the three rows `01-foundation` ships rather
      than replacing them, and every declared pair either resolves to a door or is marked unspent
- [ ] A first publish renders its diff from the stored snapshot against a NULL basis
- [ ] Submissions and supersessions emit no broker event, asserted as set equality over the
      emitted events rather than by inspection
- [ ] Every refusal this feature raises leaves an audit row carrying its reason
- [ ] An unresolvable materiality policy refuses the act rather than publishing under a default
- [ ] No `#[ignore]`d test exists without a CI tier that runs it

## 7. Known unknowns

[`../design/05-governance.md`](../design/05-governance.md) §6 carries **23 open items**, and each is
carried below with the DoD it blocks and its owner. The table has **39 rows**, and the arithmetic is
stated rather than left to a reader: §6's twenty-three are rows **1–20, 22, 23 and 24**; row **21\***
comes from the slice's constraint C7 and not from §6; rows **25–32**, marked `**`, were raised by
the 2026-08-31 review of this document; and rows **33–39**, marked `***`, were raised on 2026-09-02 — three by the build of
the evaluator and the approval store, each by an operand the code could not find, and four by the
three-lens review of that build.

The first version of this section also claimed twenty-three and was **right by coincidence**: it
dropped §6's grant-minting item and substituted the sealing question, which lives in the slice's
constraint C7 and not in §6 at all. Both are corrected below: the grant-minting item is **row 24**
— the one standing behind rows 1, 2 and 12 — and the sealing row is marked `21*`.

**None of these is answered here.** This feature is the one enforcement point, so a question left
in it is a question about every other feature's gate.

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
| 10 | ~~**Where is the `N = 0` override acknowledgment stored?**~~ **Answered (owner call, 2026-09-01 — P-D-68 arm 1): on `products_approval`, in nullable `author_override_ack` / `author_override_ack_at`, written by the submit door only when the effective quorum is zero** — a synthetic decision row would break C2's one-principal-one-decision UNIQUE, so the fact gets a column (the P-D-50 convention); decision rows keep theirs for `N ≥ 1`. *Original text:* The author performs it and the acknowledgments are said to live "on the record and in audit", but the only column for them sits on the decision row, which demands an approver principal and a verdict the author does not have, and the approval row has no acknowledgment column | no DoD — **resolved by P-D-68**; `dod-override-ceremony` and `dod-approval-store` carry the columns | was this feature's storage owner; **closed** |
| 11 | **Which transaction writes `state = satisfied`?** Every other value has a named writer; this one has only an evaluator, and nothing says whether a record at `required = 0` is born satisfied. If satisfaction is evaluated at gate time instead, the `satisfied` branch of the partial unique index is dead | `dod-quorum-evaluator`, `dod-approval-store`, `cpt-cf-bss-products-state-approval-record` | this feature |
| 12 | **What door carries submit, decide and break-glass elevation?** The catalog mints `approval × submit\|decide` and `breakglass × elevate`, and the only route this feature declares is the inbox `GET`. The Foundation closes its own set at five wire doors | `dod-decide`, `dod-breakglass-open`, `dod-rbac-catalog` | this feature with the contract owner |
| 13 | **Is `APPROVER_ROLE_REQUIRED` 403 or 409?** The convention puts 409 where the current state refuses the act and 403 where the caller may not act at all; by its stated raise site the caller may publish and it is the record's state that refuses — which is where the sibling `APPROVAL_SUPERSEDED` sits at 409 | `dod-governance-errors` | the governance owner with the taxonomy owner |
| 14 | **What do the entity-shaped columns hold for the non-entity subject kinds?** The pinned revision, content snapshot and diff basis are fixed on every record, while **at least three** of the five kinds are not entities — a `GovernedLiveOp` envelope, a `system_signal`, a `bulk_batch`. A live op has no internal revision, no published version to diff against **and no scope for the approver-scope rule to cover**; and the auto-satisfied `system_signal`'s "signal reference as the authorizing principal" **has no column**, the decision key being `(approval_id, approver_principal)` | `dod-approval-store`, `dod-system-signal`, `dod-approver-scope` | this feature with 12, which pins the envelope's subject kind |
| 15 | **Does `quorumReduced` fire on every non-material change at the default `N = 2`?** A bucket-iv-only re-publish has an effective count of 1, so the marker would ride the majority of records, while the decision that introduced it frames it as a marker for the *reducible ceremonies*. Nothing distinguishes reduced-by-configuration from reduced-by-non-materiality | `dod-quorum-descriptor` | this feature with the audit consumer |
| 16 | **Does the base role set bind the single approver of a non-material change?** The constraint scopes its CatalogAdmin-or-FinanceReviewer floor to material changes, and a non-material one gets `min(N, 1)` with no base role set on the descriptor. Nothing says whether any holder of `approval × decide` may close one | `dod-quorum-evaluator` | this feature |
| 17 | **Does AC #26's third bullet still bind?** It carries both a superseded two-person count and a `draft` return the head-row model cannot honour; the decision that rewrote the first two bullets names neither | `dod-decide` | the PRD owner with the governance owner |
| 18 | **What does an elevation change about the authorization decision?** Deny-by-default runs over tenant-scoped claims and all repository access goes through tenant-scoped ORM queries; **no rule anywhere says how a live session widens either**, nor where in the pre-pipeline gate that operand is read | `dod-breakglass-readonly`, `dod-rbac-catalog` | this feature with the `ToolKit` owner |
| 19 | ~~**Who produces `BreakGlassExpired`, and what happens to an act in flight at expiry?**~~ **Answered (owner call, 2026-09-01 — P-D-68 arm 2): the first post-expiry act emits it, exactly once, via a CAS flip of the session's `expired_emitted` stamp in the same transaction as its refusal** — the winner emits, a replay emits nothing (P-D-54's mechanism); an untouched session emits no event, its expiry being a stored fact observable as a gauge with the alerting rule on top (P-D-59's). **Expiry gates admission, not completion** — a read admitted inside the window finishes. *Original text:* The only producer named is a refused call, so an uncalled session never emits it and a session called ten times after expiry emits ten; no sweeper is named, and nothing says whether a read begun inside the window may finish | no DoD — **resolved by P-D-68**; `dod-breakglass-expiry` and `dod-governance-events` carry the mechanism | was this feature with the ops owner; **closed** |
| ~~20~~ | ~~**What is the post-hoc obligation's state set, and who discharges it?**~~ **Answered (owner call, 2026-09-01 — P-D-68 arm 3): the state set is `{pending, reviewed}` and the discharger is the second platform principal** — rule 1's *two-person-approved or post-hoc-reviewed* is one ceremony with two timings, so the review is the second principal's decision arriving late, writing `reviewed_by`/`reviewed_at`; no new door or grant. Whether its record is an `ApprovalRecord` stays its own open item, not presupposed. *Original text:* The state is stored and the review obligation is raised as an alert; no door, event or flow writes a discharge, and no values are enumerated | no DoD — **resolved by P-D-68**; `dod-breakglass-store` and `dod-breakglass-open` carry the set and the discharger | was this feature; **closed** |
| 21* | **Is a sealing capability owed, and on what terms?** *(carried from the slice's constraint C7, not from §6)* Audit sealing is a platform capability this gear deliberately does not build; the requirements it must satisfy are carried as a PRD open owned by Architecture, and until it activates the gear ships completeness without tamper-evidence | `dod-governance-audit` | Architecture |
| 22 | **The break-glass window's two normative facts live only in the PRD's interim-policy table** — a 4-hour interim **and** "no renewal without a new session". The elevation instruction states neither, and renewal is neither forbidden nor admitted | `dod-breakglass-open` | the governance owner with the §17.1 owner |
| ~~23~~ | ~~**Is the approval hook's no-fire exception still worded for `draft→published` alone?**~~ **Closed on re-measurement 2026-08-31**: the slice's C3 already reads "except any transition that consumes an approval in the same transaction — `draft→published` … and every gated edge P-D-30 put the gate phase on", and cites P-D-34 three times. The §6 bullet is stale against its own constraint and is **owed a strike in the slice**; `dod-supersede` obliges C3's widened exception | no DoD — **resolved**: the owed strike in `design/05` §6 was filed by **P-D-68** on 2026-09-01, so `dod-supersede` is freed | was this feature; **closed** |

| 24 | **Which slice mints a grant pair when the owning slice names none?** The roster carries `scheduled_transition × write\|cancel\|read` and `product\|sku × discard` for doors that name no pair, while §3.2 asserts "Every door names its pair" and `12-consumer-contracts`' lint runs door-to-catalog only — so a catalog entry with no door is invisible to it in both directions. **This is the item the first draft dropped** | `dod-rbac-catalog` | the governance owner with 04, 08 and 12 |
| 25** | **A principal's *role* is not on any surface the gear has.** `SecurityContext` exposes `subject_id`, `subject_type`, `subject_tenant_id`, `token_scopes` and `bearer_token` — no roles, no brand claim, no region claim — and `roles`/`role_` returned **zero** hits across the gear's source when this row was written (a
measurement now stale in both directions: it was already 6 files of prose at `d6cce574b`, and
`domain::approval` has since added the first real identifiers — `ApproverRole` and
`CastDecision::roles` — as the operand this row says nothing supplies. The row's substance is
untouched: no *surface* carries a role, and the decision row stores none). Authorization is permission-based: `authz.rs` asks the policy point `(resource, action)` for the **current** caller and returns a query filter. There is no way to ask whether principal X holds role R, still less to ask it of a **past** approver at gate time. The cheapest place to hold the answer is the decision row, which today stores neither the roles nor the scope claims that were true when the decision was made | `dod-quorum-evaluator`, `dod-finance-predicate`, `dod-approver-scope`, `dod-decision-store` | this feature with the platform-identity owner |
| 26** | **The gate is invoked on `save` and `discard`, and the trait gives the host no act operand.** *(The count and the enumeration were wrong and are corrected here; the substance is worse, not better. Measured 2026-09-02: **seven** `.evaluate(` call sites, of which **five** pass the `GateMode::Gate` literal — `products::run_deprecate`, `run_discard`, `run_save`; `skus::run_discard`, `run_save` — while `products::run_publish` and `skus::run_publish` pass their `mode` **argument**. So publish was named among the literal-passers where it does not belong, and `run_deprecate`, a lifecycle transition, was missed entirely. All four Product sites build a **byte-identical** triple, so nothing in `(subject, revision, mode)` separates the four acts.)* The Foundation puts the phase at every mutating door and has it pass where the act is ungated. `evaluate(subject, expected_revision, mode)` carries no act. A store-backed host that refuses when no record exists **refuses every save and every discard in the gear**; a host that authorizes when none exists preserves the no-policy deviation **on the publish path**, which is a path to `published` that consumes no record | `dod-gate-host` | this feature with 01 |
| ~~27~~ | **Answered by P-D-105 (2026-09-02)**, jointly with `04-lifecycle` §7 row 22 — the two were one question, each naming the other. The predicate keeps its `consumed` requirement and, **for a scheduled flip only**, exchanges "authorized this subject at this revision" for "is named in the flipped row's own `approval_ref`"; a wire caller still reaches the mode from no route. This is not the weakening this row forbade: the forbidden form admits a caller that names a record, whereas here the operand is a stored column on a row whose every writer runs the gate first — three call sites, counted and guarded against a fourth. **O-B-04 is not settled by this.** That the record cannot enumerate what it authorized is the same poverty seen from the other side, and P-D-105 states it as an accepted cost rather than resolving it | ~~`dod-preauthorized-mode`, `dod-one-shot-consumption`~~ | **struck** |
| 28** | **The trait is deliberately synchronous, and the code handed this feature the choice.** `governance.rs` states it: a store-backed host needs its candidate records "either as an operand the door already loaded inside its transaction, or through an async widening of this signature. **That choice is slice 05's** … because guessing it wrong costs a signature change either way." `dod-gate-host`'s "MUST NOT mint a parallel vocabulary" read literally forbids that widening. **This is the one item that cannot be deferred past the first line of code** | `dod-gate-host`, `dod-one-shot-consumption` | this feature with 01 |
| 29** | ~~**Four of the five subject kinds cannot cross the seam.**~~ **Answered (owner call, 2026-08-31 — P-D-67 arm 4): the gate's subject widens to the approval store's own pair, `(subject_kind, subject_ref)`**, with `EntityRef` remaining the constructor for the entity kinds — the store already fixed the vocabulary, so the seam expressing less than the store records was the defect. `features/catalog-version.md` §7 row 26 is the same seam from the other side, answered by the same arm; item 14's storage half is untouched. *Original text:* The seam's subject type is an entity reference whose kind enum is exactly `Product | Sku`. A `governed_live_op`, a `system_signal`, an `sku_correction` and a `bulk_batch` have no representation to hand `evaluate`, while `dod-system-signal` obliges the gate to admit one. Item 14 sees the storage half of this and misses the seam half | no DoD — **resolved by P-D-67**; `dod-gate-host`, `dod-system-signal` and `dod-approval-store` carry the widened seam | was this feature with 01; **closed** |
| 30** | **`APPROVER_ROLE_REQUIRED` has no raise path.** The gate's only refusal channel maps every refusal to `APPROVAL_REQUIRED` through a single method that exists, in its own words, so "a door that matched on the verdict itself could choose another code". The other channel is contractually reserved for infrastructure failure. Raising a second gate code needs the verdict widened with a code — again against `dod-gate-host`'s no-parallel-vocabulary clause. **Item 13 debates this code's status while its raise path does not exist** | `dod-governance-errors`, `dod-gate-host` | this feature with 01 |
| 31** | **A tenant at `N = 0` never reaches `satisfied`.** §4's only human arm fires when the descriptor is "met by distinct principals", and at `N = 0` no decision is ever recorded, so nothing meets anything; the only auto-satisfy arm is `system_signal`. The record stays `pending` and the gate, which answers yes only to a `satisfied` record, refuses forever — **re-blocking exactly the one-person tenant the quorum floor exists to unblock**. Item 11's alternative answer is no cheaper: evaluating satisfaction at gate time makes the consume flip run `pending → consumed`, an edge §4 row 5 forbids | `cpt-cf-bss-products-state-approval-record`, `dod-quorum-evaluator`, `dod-finance-predicate` | this feature |
| 32** | **Two build obligations the code books to this feature and no DoD carries.** The entity-version migration says `approval_ref` "is nullable today, and the tightening is owed to slice 05 … to be applied **by editing this file in place**", together with whatever referential constraint this feature's own record table earns. And `authz.rs` records that registering its label type-schemas "is still owed" — **confirmed at `HEAD` 2026-09-02**: `authz_label_type_schemas()` exists and its only caller is `authz_tests.rs`, so no production path registers them. **The row's third clause is stale and is struck**: it read *"a hardcoded three-action array that fails on the first `submit`, `decide` or `elevate` the catalog mints"*, and there is **no such array in production code** — the actions are individual `&str` constants, `SUBMIT`, `DECIDE`, `ELEVATE` and `EXPORT` all exist, and the only three-element array is a local in one test (`authz_tests.rs`) asserting three specific names. The catalog has already minted all four actions and the suite is green, so whatever the clause described was overtaken by the extension it warned about | `dod-approval-store`, `dod-rbac-catalog` | this feature with 01 |
| 33*** | **The materiality policy object has no store and no door.** `inst-mt-policy-material` makes it a `GovernedLiveOp` subject on its own pair `materiality_policy × write`, and `authz.rs` mints the pair — but `DESIGN.md` §3.5 gives this slice exactly `products_approval`, `products_approval_decision` and `products_breakglass_session`, and §3.2 of the design records `materiality_policy × write` as having **no route declared**. So the shipped `MaterialityPolicy` is a value with a default and a floor that nothing can persist or mutate, and the evaluator refuses every act until one is supplied. A fourth table or a `ProductsConfig` home is the choice, and both change what "in force at the submission instant" reads | `cpt-cf-bss-products-dod-materiality-policy` | this feature with the schema owner |
| 34*** | **An unresolvable materiality input has no declared code.** `dod-materiality-evaluator` requires the act refused rather than defaulted, and the shipped refusal is a domain value (`MaterialityUnresolved`) because there is nothing to render: §3.3 names no code for it, and the gear's 503 set is **closed at three by name** — `AUDIT_UNAVAILABLE`, 08's `READ_MODEL_OVERLOADED`, 03's `USAGE_TYPE_UNAVAILABLE` (`design/01` §3.3, 12 `inst-cc-errors`) — so minting a fourth would make a closed roster consistent and wrong. The refusal reaches no wire until the submit door lands, which is when the code becomes load-bearing | `cpt-cf-bss-products-dod-governance-errors` | this feature with 12 |
| 35*** | **`dod-pii-on-reasons` names a submission reason this feature does not store.** It obliges the hook on "the submission reason, the rejection reason and the break-glass session reason", and §4 gives `products_approval` **no reason column** — only `products_approval_decision.reason` and `products_breakglass_session.reason` exist. Either the submission carries no operator text (and the DoD names two reasons, not three) or the approval row owes a column | `cpt-cf-bss-products-dod-pii-on-reasons` | this feature with its storage owner |
| 36*** | **Is the claim set an input of the materiality verdict at all?** `dod-materiality-evaluator` names it in the fail-closed clause — *"an unresolvable materiality policy, claim set or bucket registry **MUST** refuse the act"* — while `inst-mt-inputs` enumerates exactly four inputs and the claim set is none of them, and §2 puts the claims check in a **pre-pipeline authorization gate** with its own fail-closed channel. The shipped evaluator therefore requires it and decides nothing with it, which satisfies the clause's letter and nothing else. Either the DoD drops it, or the verdict becomes claims-sensitive and the design gains a fifth input | `cpt-cf-bss-products-dod-materiality-evaluator` | this feature with the platform-identity owner |
| 37*** | **Two refusals the ceremony raises have no declared code.** §3.3's roster is closed at six, and neither a **second verdict from one principal** (C2's UNIQUE, read back) nor a **decision on a record that closes on no approver** (P-D-68 arm 1) is among them. Both ship on the repository's internal channel with a stated reason rather than an invented code, the same treatment row 34 records for the unresolvable input — so both answer a 500 where a classified refusal belongs | `cpt-cf-bss-products-dod-governance-errors` | this feature with 12 |
| 38*** | **Which `subject_kind` does a materiality-policy mutation record carry?** `inst-mt-policy-material` makes the policy a `GovernedLiveOp` subject on its own pair, and §4's `CHECK` admits five kinds; row 33 records that the object has no store and no door. So `MaterialAct::PolicyMutation` has no settled subject, and nothing stops a caller pairing it with `entity_publish`. **The row's second clause was false when written and is struck**: it read *"the store cross-checks the act against the subject kind for every other pair and cannot for this one"*, and `submit_approval` cross-checks **no** pair — measured 2026-09-02, it uses `subject.kind` once, to write the column, and `act` once, to take the verdict. So the exposure is the whole `MaterialAct`/`SubjectKind` product and not one cell of it, which makes the question larger rather than answered; whether the remedy is a store-side cross-check, a typed pairing, or nothing until row 33 gives the policy a subject at all is the owner's call | `cpt-cf-bss-products-dod-materiality-policy` | this feature |
| 39*** | **Does `quorumReduced` mark an effective count below the retained-name default of 2, or a ceremony reduced by configuration?** The shipped descriptor implements the first — `required < 2` — so a non-material change at `N = 5` reads reduced. Row 15 holds the question open and frames the marker as one for the *reducible ceremonies*; nothing distinguishes reduced-by-configuration from reduced-by-non-materiality, and the descriptor carries no `Materiality` to tell them apart | `cpt-cf-bss-products-dod-quorum-descriptor` | this feature with P-D-13's owner |

*Rows marked `**` were **raised by the 2026-08-31 review of this document**, not carried from the
slice. Eight of the nine come from reading the crate rather than the design set, and the three
marked BLOCKING by that review — 25, 26 and 27 — each describe a path that either refuses a legal
act or admits an illegal one.*

### Raised here rather than carried

*Both bullets below name the obligation they block, as the table's rows do.*

- **The `NoMaterialityPolicyGate` handover has no stated moment.** *Blocks `dod-gate-host` and the
  criterion asserting the gear no longer runs the no-policy host.* `01-foundation` ships that host
  as what the gear runs "until slice 05 registers a host", and nothing says whether registration is
  a startup wiring change, a migration, or a configuration flip — nor what a deployment does with
  approval records created while the no-policy host was live. *Owner: this feature with 01.*
- **`RecordingGate` is the double this feature retires.** *Blocks `dod-gate-host`, and the approval
  probes `03-sku-classification`'s DoDs owe.* It exists today in `01-foundation`'s
  publish-door tests and is the shape `03-sku-classification`'s DoDs need for their own approval
  probes. When a real host lands, whether that double stays for the Foundation's own isolation or
  is replaced by the host is unstated, and three features' test plans depend on the answer.
  *Owner: this feature with 01 and 03.*
