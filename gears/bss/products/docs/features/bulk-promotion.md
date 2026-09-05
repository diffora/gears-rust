# Feature: Bulk & Promotion

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-bulk-promotion-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-bulk-promotion`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Import a batch](#import-a-batch)
  - [Export](#export)
  - [Promote between environments](#promote-between-environments)
  - [Bulk lifecycle](#bulk-lifecycle)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Batch mechanics](#batch-mechanics)
  - [Error taxonomy](#error-taxonomy)
- [4. States (CDSL)](#4-states-cdsl)
  - [BulkBatch State Machine](#bulkbatch-state-machine)
- [5. Definitions of Done](#5-definitions-of-done)
  - [The two tables and the ledger's append-only discipline](#the-two-tables-and-the-ledgers-append-only-discipline)
  - [The batch state machine, and its one consuming edge](#the-batch-state-machine-and-its-one-consuming-edge)
  - [The import door and its two key scopes](#the-import-door-and-its-two-key-scopes)
  - [The reserved idempotency lane, which ships unused](#the-reserved-idempotency-lane-which-ships-unused)
  - [The stage phase, over two row kinds with different natures](#the-stage-phase-over-two-row-kinds-with-different-natures)
  - [The `ChangeReport`, which is what the quorum signs](#the-changereport-which-is-what-the-quorum-signs)
  - [The commit phase, and the gate mode this feature gives a caller](#the-commit-phase-and-the-gate-mode-this-feature-gives-a-caller)
  - [The override ceremony that survives the lane](#the-override-ceremony-that-survives-the-lane)
  - [One batch, one catalog version — and a window this feature does not close](#one-batch-one-catalog-version--and-a-window-this-feature-does-not-close)
  - [The coalesced completion event](#the-coalesced-completion-event)
  - [Export, and what makes it deterministic](#export-and-what-makes-it-deterministic)
  - [The promotion resolver, total over identity](#the-promotion-resolver-total-over-identity)
  - [The bulk lifecycle arm, and its separate grant](#the-bulk-lifecycle-arm-and-its-separate-grant)
  - [The five codes, and the surface that reports them](#the-five-codes-and-the-surface-that-reports-them)
  - [Resume and abandon, both through ordinary doors](#resume-and-abandon-both-through-ordinary-doors)
  - [The four design-introduced names exist as named seams](#the-four-design-introduced-names-exist-as-named-seams)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Carried verbatim from `design/09` §6](#carried-verbatim-from-design09-6)
  - [Raised here rather than carried](#raised-here-rather-than-carried)
  - [Owed to other documents, recorded and deliberately not edited](#owed-to-other-documents-recorded-and-deliberately-not-edited)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

This feature owns catalog change at volume and the movement of a catalog between environments: the
import pipeline — parse, per-row validate, stage as drafts, aggregated report, batch approval,
per-row publish — plus export, promotion identity resolution, bulk lifecycle operations, batch and
row idempotency, the single coalesced completion event, and the wiring that tags catalog-version
requests with the batch's operation key so one batch lands in one catalog version.

### 1.2 Purpose

Ten thousand SKUs must be authorable in one act without the act becoming a governance bypass or a
partial-failure surface. Two rules carry the whole design.

**Bulk runs the ordinary pipeline per row, never a parallel one.** Every row goes through the same
registered validators interactive authoring uses, lands through the same doors, and raises the same
codes. Bulk introduces no second rule set and no second taxonomy.

**No hidden partial failure.** Every row's fate is a ledger row with a reason, siblings never block
each other, and a failure is row-local by construction — because each row's publish is atomic and
independent, and the batch is a composite act whose approval is spent exactly once.

### 1.3 Actors

| Actor | Role in this feature |
|-------|----------------------|
| `cpt-cf-bss-products-actor-catalog-admin` | Runs imports, exports and promotions and bulk lifecycle acts; reads the ledger |
| `cpt-cf-bss-products-actor-finance-reviewer` | **Approves batches** and reviews the `ChangeReport` before the quorum — materiality by first publishes and lifecycle transitions at any size, with the affected-entity trigger catching large non-material edits |

`design/09` §1.3 names exactly these two. `PRD.md` §3 scopes the product manager to authoring, whose
Needs list carries no approval queue and no lint report, and the finance reviewer to *"Reviews and
approves finance-material catalog changes"* with *"Pending-approval queue with diffs, pre-publish lint
report"* — the `ChangeReport` being that report. An earlier draft of this table gave the review to the
product manager and omitted the approver entirely; **the actor who performs the act this feature's
governance contract turns on is the finance reviewer**, and who runs an export is §7 row 22's.

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.9 (`cpt-cf-bss-products-fr-bulk-import-export`), §10's
  `cpt-cf-bss-products-usecase-bulk-operations` and
  `cpt-cf-bss-products-usecase-environment-promotion`, §12 AC #33a and AC #38's bulk row.
- [`../DECISIONS.md`](../DECISIONS.md) **P-D-02** (the bundle override ceremony's home), **P-D-17**
  (update-as-draft as the promotion's purpose, which amended the FR, AC #33a and the use case),
  **P-D-25** (one `DUPLICATE_CODE` for both reservations), **P-D-26** (the reserved
  `internal:bulk-row` idempotency lane), **P-D-46** (`closed_at` struck; the bulk window closes on a
  hard timer), **P-D-50** (this feature writes no operator free-text reason).
- [`../design/09-bulk-promotion.md`](../design/09-bulk-promotion.md) — the slice. Its §2 carries the
  **normative steps** of all four flows, §3.1 the batch mechanics and §3.2 the normative error
  roster. This document declares their ids and carries the actor, the scenarios, the Input/Output and
  the boundary.
- Sibling slices: [`../design/01-foundation.md`](../design/01-foundation.md) (the doors every row
  rides, the gate modes, the idempotency lanes),
  [`../design/05-governance.md`](../design/05-governance.md) (the approval ceremony, the materiality
  evaluator, the one-shot rule),
  [`../design/06-catalog-version.md`](../design/06-catalog-version.md) (the increment requests this
  feature tags, and the manifest export renders from),
  [`../design/02-taxonomy-attributes.md`](../design/02-taxonomy-attributes.md) (the governed
  live-entity envelope the staged live ops apply under),
  [`../design/12-consumer-contracts.md`](../design/12-consumer-contracts.md) (the export artifact's
  schema-version discipline).

**Requirements**: `cpt-cf-bss-products-fr-bulk-import-export`,
`cpt-cf-bss-products-usecase-bulk-operations`,
`cpt-cf-bss-products-usecase-environment-promotion`

**Principles**: `cpt-cf-bss-products-principle-registered-validators`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Components**: `cpt-cf-bss-products-component-capability-handlers`

**Sequences**: `cpt-cf-bss-products-seq-environment-promotion`

`design/09` §3.2 carried only `cpt-cf-bss-products-contract-bulk-errors`, and a `contract-` id stays
at its slice. So **`cpt-cf-bss-products-algo-bulk-errors` is minted here** and §3.2 points at it — the
eleventh time a `contract-`-only §3 section has needed it.

**The code surface this feature is written against, measured at `7d5864c09`.** As on
`features/clone.md`, part of what this feature needs was built ahead of its caller — and the two
seams in question are each less finished than they look.

**Two seams are prepared for a bulk row, and neither is prepared *for this feature alone*.**

- **`GateMode::PreAuthorized` ships as an explicit door argument with no production caller.**
  `domain::governance` declares the variant; both authoring doors record that an earlier revision,
  reading `inst-fd-gate-mode`'s second clause as forbidding the first, *"left
  [`GateMode::PreAuthorized`] a type with no call path at all"* (`api/rest/products.rs`) and *"no
  call path anywhere in the gear"* (`api/rest/skus.rs`). Both now take the mode as an argument, and
  within `products/src` it is passed only from `products_tests.rs`, `skus_tests.rs` and
  `governance_tests.rs`.
  **The caller it was added for is not this feature.** `domain::governance` says so: *"the caller it
  exists for, `04-lifecycle`'s scheduled-publish runner, does not exist at this commit either, so
  nothing is blocked by the refusal today."* A bulk row is **one of three** in-process callers the
  variant enumerates — *"a scheduled activation, a cascade leg, a bulk row"* (emphasis this
  document's) — so `inst-bk-commit`'s claim that the gate-mode instruction names a bulk row among its
  callers is verifiable, and the ownership claim is not.
- **`internal:bulk-row` is reserved in prose and in no code.** `api::rest` lists it among the lanes
  *"held for non-HTTP"* callers and the idempotency migration's doc repeats the roster, while the
  shipped phase says *"and this phase has none, so none is used here"* — and the doors give the
  reason: the three names *"are named rather than defined as constants because the first non-`HTTP`
  caller is the one that knows which of the three it is and what it writes in `client_key`."* Every
  occurrence in the crate is inside a doc comment: no constant, no enum, no constraint.

**What the crate's five "nothing is consumed" statements do and do not settle.**
`domain::governance` says in five places that **nothing is consumed** under that mode, and its gate
returns `GateVerdict::Refused` for it. This feature's commit phase says the batch approval **is**
consumed once. **On consumption the two agree**: the spend happens at the `approved → committing`
flip, and the per-row publishes that follow run `PreAuthorized`, **verifying** the named record
without spending it. A DoD that read the crate as forbidding consumption would invert the design.

**What they do not settle is whether the predicate can name a batch at all**, and that is registered
against this feature elsewhere. `features/governance.md` §7 row 27 records it: the mode verifies that
a record *"authorized **this subject** at **this pinned revision**"*, while a bulk row's revision is
its own — *"Both fail by construction, and the mode carries only an id with no plan-membership
operand."* The shipped seam is narrower still: `evaluate`'s subject is an `EntityRef` whose
`entity_kind` is `{Product, Sku}`, so a `bulk_batch` subject cannot be named. That row names *"this
feature with 04 and 09"* as owners, and §7's owed subsection carries it rather than this section
claiming the reconciliation is complete.

**Everything the bulk path is made of is absent** — zero occurrences each across `products/src`:

- the tables `products_bulk_batch` and `products_bulk_row`;
- all three routes — `bulk/imports`, `bulk/exports`, `bulk/lifecycle`;
- all five codes this feature declares, plus `STALE_LIVE_OP`, which is
  `02-taxonomy-attributes`' and which this feature's commit phase raises;
- the coalesced event `CatalogBulkOperationCompleted`;
- **`operation_key`** — the whole mechanism that makes one batch land in one catalog version.

## 2. Actor Flows (CDSL)

Each flow below is **declared here and stepped in
[`../design/09-bulk-promotion.md`](../design/09-bulk-promotion.md) §2**, whose steps are the
normative ones. What this section carries is the triggering actor, the scenarios and the boundary.

### Import a batch

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-import`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`, with
`cpt-cf-bss-products-actor-finance-reviewer` reviewing the report and approving

**Success Scenarios**:

- **The door is idempotent on the batch key.** `POST /bss-products/v1/bulk/imports` spends
  `bulk × execute`; a replayed batch returns the existing `BulkBatch` rather than starting a second.
- **Row keys are batch-scoped**, and a row reaching the publish door resolves that door's key as the
  reserved lane `internal:bulk-row` with the row's id as the client key. A row re-listed in a **new**
  batch is a **new act** — its stage validation decides its fate against the store — and only a
  retry **within** the batch no-ops against the ledger.
- **Stage runs the ordinary pipeline per row.** Product and SKU rows run the same registered
  validators as interactive authoring and land as `draft` through the Foundation doors.
- **Live-entity rows have no draft state, so they stage as pending operations.** Categories,
  attribute definitions and recognized-set members validate at stage as a **dry run** against the
  live tree and sets, and are recorded in the ledger as pending `GovernedLiveOp`s applied at commit
  under the batch approval.
- **Dependency order is categories and vocabularies, then Products, then SKUs**, and commit
  preserves it.
- **The `ChangeReport` is generated from the ledger** — counts, per-type summary, a deterministic
  sample, the pre-publish lint findings, the scope-values lint, and the itemised override-carrying
  rows named individually — and submitted to `05-governance` as **one** approval with subject kind
  `bulk_batch`.
- **The commit phase publishes per row in `PreAuthorized` mode**, each row pinned to its ledger
  revision, and the batch's catalog-version requests carry the `operation_key` so the whole batch
  lands in **one** `CatalogVersion`.
- **Completion emits exactly one `CatalogBulkOperationCompleted`** with the ledger digest, and the
  ledger stays queryable.
- **The batch is readable, and that is the reader C1 requires** (**P-D-61**):
  `GET /bss-products/v1/bulk/batches/{batchId}` spends **`bulk × read`** — its own grant, a reader
  not being an executor — and returns the batch state plus the `RowLedger` one entry per row with its
  disposition, code and reason. **One route serves both lanes**, the key being the batch id.

**Error Scenarios**:

- **A row whose in-batch dependency failed fails `BULK_DEPENDENCY_FAILED` without touching the
  store.**
- **A row edited after the report fails `STALE_REVISION` alone**, in the ledger. The batch approval
  stands, siblings publish, and the ledger names the row. **A post-report edit to a member entity
  does not supersede the batch** — which is why the state machine needs no supersession edge.
- **A live-entity operation whose pinned expected target state moved fails `STALE_LIVE_OP` alone**,
  row-locally, exactly as an edited entity row fails `STALE_REVISION`.
- **A row whose override condition appeared after the report fails
  `BULK_OVERRIDE_UNACKNOWLEDGED`** rather than publishing unacknowledged.
- **Over the configured bounds** — max rows per batch, max concurrent batches per tenant — the door
  refuses `BULK_LIMIT`.

**Boundary, and the four things this flow deliberately is not.**

**It is not a second rule set.** Row-level validation rules belong to the features that own them;
bulk runs the same pipeline per row and never a parallel one. That is the whole reason a bulk row
raises the owning feature's codes verbatim inside the ledger.

**It is not the approval ceremony.** `05-governance` owns the quorum, the materiality evaluation and
the one-shot rule. This feature submits one approval and consumes it once.

**It does not increment the catalog version.** `06-catalog-version` does. This feature tags its
requests with the `operation_key`, and **the bulk window closes on a five-minute hard maximum rather
than on any signal this feature sends** — which is why **P-D-46** struck the stored close marker.

**It writes no operator free-text reason** (**P-D-50**). `batch-abandoned` is a literal constant, the
ceremony's reason lives on the approval record and the mass-retire reason on `04-lifecycle`'s — so
`02-taxonomy-attributes`' content-PII enumeration no longer names this feature.

### Export

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-export`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin` — `design/09` §1.3's own attribution. §7 row
22 carries the three-way disagreement between that, this document's earlier reading, and `PRD.md`'s
two role entries.

**Success Scenarios**:

- `GET /bss-products/v1/bulk/exports?catalogVersionId=` spends **`catalog_version × read`** — its own
  grant, decoupled from the import pair, because an export renders a catalog-version manifest and is
  auditor-shaped rather than an authoring act.
- **It is rendered from the manifest**: entity halves from frozen versions, capture halves from the
  capture store.
- **It is byte-for-byte deterministic for a given version.**
- **The artifact header carries a schema format version**, versioned in `products-sdk` under
  `12-consumer-contracts`' compatibility discipline, and the format carries the stable codes and
  canonical names — the promotion identities — plus full content.

**Error Scenarios**:

- A `catalogVersionId` this tenant has none of is the ordinary not-found; export introduces no code
  of its own.

**Boundary.** **Export artifacts are streamed, not stored** — determinism makes storage redundant,
and that is the stated reason rather than a size argument.

### Promote between environments

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-promote`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin` at the target

**Success Scenarios**:

- **Promotion *is* import.** The target imports a source export; there is no promotion door and no
  governance bypass.
- **`PromotionResolver` classifies every row, exhaustively, in four classes**: an unknown identity
  **creates** with re-minted ids; an identity bound to matching content is a **no-op**; an identity
  bound to **different** content is an **update-as-draft** against the existing entity — *"a
  same-identity content difference is the promotion's purpose"* (**P-D-17**, which amended the
  requirement, the acceptance criterion and the use case to match); and an incompatible kind or type,
  a `retired` holder **or a dirty head** all **conflict**.
- **The reviewer's pre-approval view is the `ChangeReport`** — staged content against the target's
  current heads, the only diff producible before anything publishes.

**Error Scenarios**:

- **`PROMOTION_IDENTITY_CONFLICT`** where the identity is bound to an incompatible kind or type, or
  to a **`retired`** holder — revival is clone-only, which is what makes the resolver total.
- **`PROMOTION_DIRTY_HEAD`** where the target head carries unpublished local edits or an open
  approval. An import **never silently merges into in-flight work** and never supersedes a local
  approval.

**Boundary.** The catalog-version diff of the post-commit verification view is **not** the reviewer's
pre-approval view, and the slice records that the requirement's own sentence calling the version diff
*"the reviewer's view for approvals"* still conflicts with that. This document does not resolve it,
and **it is not a §7 row**: the slice flags it inline in `inst-pm-review` (M4), not in its §6, so no
carried row can contain it. §7's owed subsection records it against the PRD's owner with `06`.

### Bulk lifecycle

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-bulk-lifecycle`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:

- Mass deprecate and mass retire-initiate over a filter or id list, through
  `POST /bss-products/v1/bulk/lifecycle`, spending **`bulk_lifecycle × execute`** — **its own grant**,
  because the gear's most destructive batch act does not ride the import pair.
- **Each row runs the ordinary lifecycle policy doors**, provenance `direct`, per-row confirmation
  data aggregated into one report.
- **The batch is material at any size** by its lifecycle transitions, with the affected-entity
  trigger additionally catching large batches.
- One batch approval, consumed once by the same `approved → committing` flip, each row's transition
  door running in `PreAuthorized` mode naming that record.

**Error Scenarios**:

- The retire arm schedules per-row transitions and **the flip guards stay per-SKU**: no bulk override
  of the reference guard exists, so a referenced row defers under the ordinary guard and the batch
  never force-retires.

**Boundary.** This is the `p2` arm of the feature, and it is the one place where a batch's blast
radius is a lifecycle change rather than content. The grant split is the control.

## 3. Processes / Business Logic (CDSL)

Each process below is **declared here and specified in
[`../design/09-bulk-promotion.md`](../design/09-bulk-promotion.md) §3**.

### Batch mechanics

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-batch`

**Input**: the submitted rows with the batch key and per-row keys; the target tenant's current heads,
live tree and sets; the approval record once the quorum is met.

**Output**: `products_bulk_batch` and `products_bulk_row` — the `RowLedger` — carrying every row's
terminal state and reason; the `ChangeReport`; the per-row publishes and live-entity applies; the
catalog-version requests tagged with the `operation_key`; and one completion event with the ledger
digest.

**Boundary**: the ledger is **the idempotency store for row keys**, distinct from the Foundation's
endpoint store because row keys are **batch-scoped**. Rows are immutable after their terminal state —
append-only evidence.

**A batch is resumable, and abandonment uses no new door.** A crash mid-commit resumes from the
ledger, per-row publishes being idempotent by row key. On abandon: created-draft rows discard through
the ordinary discard door; **update-as-draft rows revert** through the ordinary save door with the
last frozen version's content as payload, so the head returns to its published content with a
revision bump and the literal audit reason `batch-abandoned`; pending live-entity operations are
simply dropped, never applied.

**Size bounds are configured** — max rows per batch and max concurrent batches per tenant, refused
`BULK_LIMIT` — and the ten-thousand-SKU onboarding case is the sizing fixture rather than an
illustration.

### Error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-bulk-errors`

**Input**: a row at any phase, or a batch at its door.

**Output**: one of five codes this feature declares, or — for a row-level failure of any other kind —
**the owning feature's code verbatim inside the ledger**.

**Boundary**: the five are `BULK_DEPENDENCY_FAILED`, `PROMOTION_IDENTITY_CONFLICT`,
`PROMOTION_DIRTY_HEAD`, `BULK_OVERRIDE_UNACKNOWLEDGED` and `BULK_LIMIT`. **Bulk introduces no
parallel taxonomy**, and that is the rule the roster's shortness follows from.

**Four of the five are per-row ledger outcomes, not responses of the batch door.** The import surface
answers **202**; the row carries the outcome. So the statuses the slice assigns **to those four** —
409 for the two promotion codes, 422-architectural for `BULK_DEPENDENCY_FAILED` and
`BULK_OVERRIDE_UNACKNOWLEDGED` — **apply only where a caller asks a single row's disposition**, and
there is no surface in this design that does. §7 carries which surface reports the ledger.

**`BULK_LIMIT` is the exception and the door's own refusal.** An over-bounds batch is refused at the
import door, so its status applies there rather than nowhere — and which status is the slice's open
question, its whole mapping being recorded as proposed.

**The 422s are architectural, not wire — by this gear's own choice** (`design/01` §3.3), *not*
because no path can produce one. `infra::error_mapping` is explicit: *"**This is a choice, not an
impossibility**, and the design set says so explicitly"*, the transport-override mechanism *"genuinely
exists and genuinely reaches 422"*. Under the choice each reaches the wire as a **400** carrying its
code, and no endpoint may declare a 422 for an error carrying a registry code.

**And the statuses themselves are recorded as proposed.** `design/09` §3.2 says so in as many words —
the mapping follows the sibling gear's *"checked against it code by code"*, except the 503 class this
gear added, and the note ends *"Proposed per row and open to correction; the requirement is that
every code carries one."* The **codes** are fixed; the statuses are the part still open.

## 4. States (CDSL)

### BulkBatch State Machine

- [ ] `p1` - **ID**: `cpt-cf-bss-products-state-bulk-batch`

The rows below render the state domain `design/09` §1.7 states — six transitions and a terminality
row since **P-D-69** completed the machine. The slice declares no `state-` id for it, as no slice in
this set declares one.

**States**: `staging`, `reported`, `approved`, `committing`, `completed`, `failed`, `abandoned`

**Initial State**: `staging`, on the import door's admission of the batch key.

**Transitions**:

1. [ ] - `p1` - **FROM** `staging` **TO** `reported` **WHEN** every row has reached a stage outcome
   and the `ChangeReport` is generated from the ledger and submitted to the governance gate as one
   approval with subject kind `bulk_batch`. **The executor is the gear-owned batch worker's claim
   transaction — the one that stages the last row** (**P-D-54**), which generates and submits the
   report in that same transaction, so the report exists exactly when the ledger says staging is done
   and no detection pass sits between - `inst-bb-edge-report`
2. [ ] - `p1` - **FROM** `reported` **TO** `approved` **WHEN** the quorum evaluator finds the stored
   descriptor met; the report and the ledger's per-row pinned revisions are the approval's stored
   snapshot, so **a post-report edit to a member entity does not supersede the batch** — that row
   fails its own pin at commit, row-locally - `inst-bb-edge-approve`
3. [ ] - `p1` - **FROM** `approved` **TO** `committing` **WHEN** the commit phase begins, and **this
   edge is where the batch approval is consumed — once, for the whole composite act**. Every per-row
   publish that follows runs in `PreAuthorized` mode naming that consumed record and **spends
   nothing further**, which is the property the shipped gate makes structural rather than a rule a
   door must remember - `inst-bb-edge-commit`
4. [ ] - `p1` - **FROM** `committing` **TO** `completed` **WHEN** every row has reached a terminal
   ledger state, whatever the mix of published, applied, no-op and failed; completion emits exactly
   one `CatalogBulkOperationCompleted` with the ledger digest. **A batch with failed rows still
   completes** — parts-succeeded is the honest end state, not an error. **The executor is the same
   worker's claim transaction at the other end — the one that lands the last row's terminal state**
   (**P-D-54**), and the event is emitted **inside that CAS**: the winner emits, a re-claim after a
   lease expiry finds the state already flipped and emits nothing, which is where "exactly one" comes
   from - `inst-bb-edge-complete`
5. [ ] - `p1` - **FROM** `reported` **TO** `abandoned` **WHEN** the batch approval is rejected or
   explicitly withdrawn (**P-D-69**) — executing `inst-bm-resume`'s abandon procedure (created drafts
   discarded, update-drafts reverted, pending live-ops dropped) and releasing the
   tenant slot - `inst-bb-edge-abandon`
6. [ ] - `p1` - **FROM** `staging` or `committing` **TO** `failed` **WHEN** the batch worker's
   attempt budget exhausts (**P-D-69** — `inst-ar-failure`'s own arm on the activation runner);
   **row failures stay row-local and never enter it** - `inst-bb-edge-fail`
7. [ ] - `p1` - **No transition other than the six above is admitted.** Rows are immutable after
   their own terminal states — the ledger is append-only evidence from then on - `inst-bb-terminal`

**Terminal states**: `completed`, `failed` and `abandoned`.

**The three absences §7 row 5 carried are answered** (**P-D-69**): the rejection edge and the abandon
state are edge 5's, `failed`'s entry is edge 6's, and the abandon procedure of §3.1 now has the state
it writes. Row 6 — what ends a batch never approved — stays open, as does row 7, edge 3's executor.

**Three of the four edges fired on a condition with no executor; two now have one and edge 3 does
not.** Edge 1 fires when every row has reached a stage outcome, edge 4 when every row has reached a
terminal state, and edge 3 *"on quorum"* — none named a door, actor or signal, and the import door
cannot be one, because it answers **202**. **P-D-54** gives edges 1 and 4 the gear-owned batch
worker's claim transaction, which closes §7 row 26. **Edge 3 stays §7 row 7's**, carried and owned
with `05`, with two live candidates: this worker, or `05`'s decide door flipping the state in the same
transaction as the quorum verdict. A builder taking edge 3 as settled by that decision would put the
commit-phase start in whatever code sits nearest — the substitution row 7 exists to prevent.

## 5. Definitions of Done

Every DoD below names what exists at `7d5864c09`. Two seams ship and are named as such; everything
else this feature is made of is absent, so the rest create.

### The two tables and the ledger's append-only discipline

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-bulk-tables`

`products_bulk_batch` and `products_bulk_row` exist, tenant-scoped, the second being the `RowLedger`
— **landed 2026-09-01 as `m20260901_000014`**, with the seven-state roster, P-D-69's `mode`, the
worker's claim columns, the row-freeze trigger and P-D-50's closed reason set, all CHECK- and
trigger-pinned on both engines.

**Rows are immutable after their terminal state**, which makes the ledger append-only evidence rather
than working state. That is what lets a crash resume from it and what makes "no hidden partial
failure" checkable after the fact.

**The ledger is the idempotency store for row keys**, and it is **not** the Foundation's endpoint
store: row keys are **batch-scoped**, so the same row id in a different batch is a different act.

**The column shape is now `design/09` §4's** (**P-D-61**), authored from the values with a stated
writer and from nothing else: the batch carries `batch_key` UNIQUE with `tenant_id`, `lane`, the seven
states (**P-D-69** added `abandoned`), `operation_key`, `approval_ref`, and the worker's
`claimed_at`/`attempt`; the row carries
`(tenant_id, batch_id, row_key)`, `entity_kind`/`entity_id`, `pinned_revision`, `disposition`, `code`,
`reason` — **a literal from a closed set, never operator text** (**P-D-50**) — `governed_live_op` and
`override_acknowledged`. **The `ChangeReport` has no table**: it is derived from the ledger at the
report edge. No counter duplicating the ledger was added.

**Implements**: `cpt-cf-bss-products-algo-batch`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_bulk_batch`, `products_bulk_row`
- Entities: `BulkBatch`, `RowLedger`

### The batch state machine, and its one consuming edge

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-batch-state-machine`

The seven states and six edges of §4 are implemented, with `completed`, `failed` and `abandoned` terminal (**P-D-69** completed the machine).

**Two of the four edges have a named executor and one does not.** Edges 1 and 4 are flipped by the
gear-owned batch worker's claim transaction (**P-D-54**); edge 3's executor is **§7 row 7**'s and
still open. A build that flips edge 1 or edge 4 from a door has the wrong actor — that door answered
**202** and is gone.

**The `approved → committing` edge is the whole feature's governance contract.** It is the single
place a batch approval is consumed, and every per-row act after it verifies without spending. A build
that consumed per row would spend one record many times; a build that consumed nowhere would publish
under no ceremony.

**The absences closed** (**P-D-69**): `reported → abandoned` on rejection or explicit withdrawal
(the abandon procedure's state, releasing the tenant slot), and `staging|committing → failed` on the
worker's attempt-budget exhaustion — `inst-ar-failure`'s own arm — with row failures never entering
it. Row 6 still owns the tenant slot a never-approved batch holds.

**The record it flips has no table in this gear.** The consuming edge writes `05-governance`'s
approval record, and **no migration for it ships** — seven exist and none is it, while
`domain::governance` states *"There is no materiality evaluator here, no `ApprovalRecord`, no record
store and no grant check"*. So this DoD's write path is `05`'s to supply.

**Ticked (P-D-149).** The seven states and six edges run in `infra::bulk_worker`: edge 1 in
`stage_next_batch` (now carrying the report and its record), edges 2 and 3 in `begin_commit` — one
transaction that evaluates the batch's record through the stored host, **consumes it once** and
moves `reported → approved → committing` — edge 4 in `complete_batch`, `reported → abandoned` on a
rejected or superseded record and on the reaper's `bulk_batch_ttl_hours` (P-D-127 row 6, `168`
interim), and `staging|committing → failed` on the worker's attempt budget (`ATTEMPT_BUDGET`, five
claims). The write path `05` owed is the shipped record store: `repo::submit_approval` at the report
edge, `repo::settle_authorization` at the flip. Probes:
`a_batch_reports_commits_under_one_consumed_record_and_completes`,
`a_rejected_record_abandons_and_the_reaper_takes_a_stale_report`.

**Implements**: `cpt-cf-bss-products-state-bulk-batch`, `cpt-cf-bss-products-algo-batch`

**Touches**:
- DB Table: `products_bulk_batch`, `products_approval` (`05-governance`'s, unshipped)
- Modules: `domain::governance`
- Entities: `BulkBatch`, `BatchWorker`

### The import door and its two key scopes

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-import-door`

`POST /bss-products/v1/bulk/imports` exists, spending `bulk × execute`, idempotent on the batch key —
a replay returns the existing batch. **The request carries a batch-level `mode ∈ {import, promote}`,
default `import`** (**P-D-69**): only `promote` engages the `PromotionResolver`'s update-as-draft,
and under `import` a bound `skuCode` with different content stays `DUPLICATE_CODE` — a silent
auto-update on collision would convert typos into overwrites.

**The route ships.** **Two key scopes, and conflating them is the defect this DoD exists to
prevent.** The **batch** key is the door's idempotency key. The **row** keys are batch-scoped and live in the ledger. A row
reaching the publish door resolves *that* door's key as the reserved lane `internal:bulk-row` with the
row's id as the client key — a third scope, the Foundation's.

**A row re-listed in a new batch is a new act.** Its stage validation decides its fate against the
store — a code collision is the ordinary `DUPLICATE_CODE`, one code covering both reservations — and
only a retry **within** the batch no-ops against the ledger.

**Implements**: `cpt-cf-bss-products-flow-import`

**Touches**:
- API: `POST /bss-products/v1/bulk/imports`
- Modules: `api::rest`

### The reserved idempotency lane, which ships unused

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-idempotency-lane`

Per-row publishes resolve the publish door's idempotency key as the reserved lane
`internal:bulk-row` with the row's id as the client key (**P-D-26**) — **the ledger row's surrogate
id** (**P-D-69**), so a row re-listed in a new batch has a new key and stays a new act with no batch
column added to the shipped primary key. **The claim's response columns store P-D-42's synthetic
`200` with the row's `{disposition, code, reason, entity_id, published_version}` as the outcome
record, and its `payload_hash` digests the row's staged payload** — one digest rule for all three
internal lanes, now stated in `design/01` §4.4 (P-D-69).

**The lane was reserved in prose and in no code**, and the doors said why: the three names *"are named
rather than defined as constants because the first non-`HTTP` caller is the one that knows which of
the three it is and what it writes in `client_key`."*

**So this DoD adds the constant, the `client_key` rule and the caller** — not merely a caller for an
existing lane; a build that minted a new lane name instead would leave the reserved one dead. The
constant is `api::rest::bulk::INTERNAL_BULK_ROW_LANE` and the `client_key` rule is the ledger row's
`row_id`, minted by the import door with the row. **The caller is the batch worker's per-row
publish**, which arrives with `dod-stage-phase`.

**And a claim is not enough: the store demands an answer.** `payload_hash` is `NOT NULL` and
`chk_products_idempotency_response_group` admits only `claimed` with both response columns null or
`answered` with both set — so an internal lane must answer with **P-D-42**'s synthetic `200` and its
own outcome record as the body. A row that published and left its key `claimed` would be re-executed
by the crash-resume path, which §6 asserts it will not be. **What a bulk row's outcome record and
payload digest are is §7 rows 24 and 25's**, a row having no request body to digest.

**Implements**: `cpt-cf-bss-products-flow-import`

**Touches**:
- DB Table: `products_idempotency`
- Modules: `api::rest`

### The stage phase, over two row kinds with different natures

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-stage-phase`

**Product and SKU rows** run the ordinary per-row pipeline — parse, then **the same registered
validators as interactive authoring** — and land as `draft` through the Foundation doors.

**Live-entity rows have no draft state, and that is why they are treated differently.** Categories,
attribute definitions and recognized-set members validate at stage as a **dry run** against the live
tree and sets, and are recorded as **pending `GovernedLiveOp`s** applied at commit under the batch
approval. Their promotion identities are stated: a category by `(parent path, normalized name)`, a
set member by `(set kind, member code)`, a definition by its key. **Their half is built when their
stores are**: `02`/`03` own the tree, the definitions and the sets, and the shipped worker refuses a
row of any kind it cannot stage rather than queueing it silently.

**Dependency order is normative**: categories and vocabularies, then Products, then SKUs — at stage
**and** at commit. A dependent row whose in-batch dependency failed fails `BULK_DEPENDENCY_FAILED`
**without touching the store**. *(With one stageable class today there is nothing to order; the
code ships and its raiser arrives with the classes that make an order observable.)*

**Never a parallel rule set.** This is the sentence the whole feature's correctness rests on: a bulk
row that skipped a validator interactive authoring runs would make bulk a governance bypass by
omission rather than by design.

**The phase has a named executor, and the ceiling moves with it.** Rows are staged by the gear-owned
batch worker under a claim, and §4's edge 1 is flipped by the claim transaction that stages the last
row (**P-D-54**). So `inst-bm-limits`' per-tenant concurrent-batch ceiling is enforced **at claim**,
not only at admission — a ceiling checked only by the door drifts as batches hang.

**Implements**: `cpt-cf-bss-products-flow-import`, `cpt-cf-bss-products-algo-batch`

**Principles**: `cpt-cf-bss-products-principle-registered-validators`

**Touches**:
- DB Table: `products_bulk_row`
- Entities: `RowLedger`, `BatchWorker`

### The `ChangeReport`, which is what the quorum signs

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-change-report`

The report is generated **from the ledger** and carries: counts, a per-type summary, a
**deterministic** sample, the pre-publish lint findings, a **scope-values lint** naming region and
brand values unseen in the target tenant's catalog, and the **itemised** override-carrying rows —
each by `skuCode`, never as a count.

It is submitted as **one** approval with subject kind `bulk_batch`, carrying the batch's
affected-entity count. **Materiality is `05-governance`'s to decide**, not this feature's: first
publishes and lifecycle transitions are material at any size, and the affected-entity trigger catches
large non-material edits.

**Its stored snapshot is the report plus the ledger's per-row pinned revisions**, and that pairing is
what makes a post-report edit fail **one row** instead of superseding the batch.

**The itemisation is load-bearing.** An override acknowledgment over a count cannot be an
acknowledgment by name, and the ceremony this feature stores on the batch's approval record is
acknowledgment-by-name over that set.

**Ticked (P-D-149).** `report_and_submit` renders the report from the ledger at edge 1 — counts,
the per-kind summary, the deterministic sample (the first five row keys), the dry-run lint per
staged row through the same functions the `validate` doors run (P-D-125), the scope-values lint
(`repo::known_scope_values`: region and brand values no head outside the batch carries), the
**itemised** override rows by `skuCode`, and every row's pinned revision — and submits it as one
`bulk_batch` approval whose `content_snapshot` is the report and whose pin is the report's
`ledgerDigest` (`SubjectPin::LedgerDigest`, P-D-127 row 23). Materiality is `05`'s: the act is
`MaterialAct::BatchAct`, `affected` saturating on any first publish. The record's id is pinned on
`products_bulk_batch.approval_ref` before the state moves.

**Implements**: `cpt-cf-bss-products-flow-import`

**Touches**:
- Entities: `ChangeReport`
- DB Table: `products_bulk_row`

### The commit phase, and the gate mode this feature gives a caller

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-commit-phase`

On quorum, rows publish per row through the Foundation's publish door **in `PreAuthorized` mode naming
the batch's consumed record**, each pinned to its ledger revision. Live-entity operations apply as
their `GovernedLiveOp`s under the same consumed approval, each re-validating its pinned expected
target state at apply.

**`GateMode::PreAuthorized` ships, and it ships with no call path.** Both authoring doors say a
slice's absence *"left [`GateMode::PreAuthorized`] a type with no call path at all"*, and the variant's
own doc enumerates what it is for: *"a scheduled activation, a cascade leg, **a bulk row**."* **This
DoD builds the caller, not the mode** — and the caller alone is not enough: the shipped
`NoMaterialityPolicyGate` refuses every call in this mode, so the commit phase also depends on
`05-governance` registering a record-backed host. Whether that host's predicate can name a batch at
all is §7 row 19's.

**And nothing is consumed under it.** The shipped gate returns a refusal for that mode, and
`domain::governance` states five times that the mode spends nothing — *"A record was **verified**,
not spent"*. The batch approval is consumed at the `approved → committing` flip and nowhere else. A
DoD that read the crate as forbidding consumption altogether would invert the design; a build that
consumed per row would spend one record many times.

**Every failure at commit is row-local**, and each has its own code: an edited row `STALE_REVISION`, a
moved live-target `STALE_LIVE_OP`, a dependent of a failed row `BULK_DEPENDENCY_FAILED` wrapping the
underlying code, a late override `BULK_OVERRIDE_UNACKNOWLEDGED`. **Siblings never block**, and the
published state is never partially inconsistent because each row's publish is atomic and independent.

**Ticked (P-D-149).** `commit_rows` walks the ledger under the batch's consumed record: each
Product or SKU row through `products::run_publish` / `skus::run_publish` in
`GateMode::PreAuthorized(approvalId)` over `StoredApprovalGate::bulk_row` — the host verifies the
consumed record and **spends nothing** — pinned to its ledger revision, claimed on the reserved
`internal:bulk-row` lane whose stored answer is the ledger outcome. Every failure is row-local and
coded in the ledger: `STALE_REVISION` for an edited head, `BULK_DEPENDENCY_FAILED:<code>` for a SKU
whose parent row failed this pass (Products walk first), `BULK_OVERRIDE_UNACKNOWLEDGED` for a late
bundle, the owning door's code verbatim otherwise; siblings never block and a batch with failed rows
reaches `completed`. Probe: `a_row_edited_after_the_report_fails_stale_revision_alone`.

**Implements**: `cpt-cf-bss-products-flow-import`, `cpt-cf-bss-products-state-bulk-batch`

**Touches**:
- Modules: `domain::governance`, `api::rest`
- DB Table: `products_bulk_row`

### The override ceremony that survives the lane

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-bulk-override-ceremony`

**The problem this DoD solves, stated first because the rule is unreadable without it.** A batch is
one composite act: its approval is consumed at the flip and its per-row publishes do **not** re-enter
the governance gate. So a row carrying an override condition — today only an uncomposed `bundle` —
would otherwise publish with **no ceremony recorded anywhere**.

Therefore: the **stage** phase detects override-carrying rows and names them in the report as a
distinct itemised section; the batch approval is an **override ceremony** whose
acknowledgment-by-name is over **that itemised set**, stored on the batch's approval record exactly as
an entity-publish override is stored on its own; and a row whose override condition **appeared after
the report** fails `BULK_OVERRIDE_UNACKNOWLEDGED` alone in the ledger.

**A batch with no such row carries no ceremony and is unchanged.** The mechanism is conditional, not
a tax on every batch.

**This DoD is the batch application of a rule `05-governance` already owns**, and the id is
deliberately distinct from it: `features/governance.md`'s `cpt-cf-bss-products-dod-override-ceremony`
states the general obligation — *"The record **MUST** be the ceremony's only home: a lane that
publishes an override subject without one is a defect, not an exemption"* — and the bulk lane is
exactly such a lane. What this DoD adds is the itemised set, the batch-scoped acknowledgment and the
late-condition refusal; it does not restate the general rule.

**Ticked (P-D-149).** The report edge itemises every uncomposed-bundle row by `skuCode`, marks it
`override_acknowledged` in the ledger and records one `BUNDLE_OVERRIDE_REQUIRED/{skuCode}` entry per
row in the record's `overrideConditions`; `05`'s decide door then demands every entry named in
`override_acknowledgments` (an unnamed condition is `VALIDATION`), so a satisfied record **is** the
acknowledgment-by-name over the itemised set. At commit the itemised row publishes with its flag
raised under the one batch ceremony; a bundle whose condition appeared after the report — not in the
set — fails `BULK_OVERRIDE_UNACKNOWLEDGED` alone before its publish is attempted. At effective quorum
zero nobody can acknowledge by name and the worker is not an author (P-D-68), so the itemised rows
fail there with the closed-set reason `no-acknowledger-at-quorum-zero`. Probe:
`the_ceremony_itemises_bundles_and_a_late_bundle_fails_alone`.

**Implements**: `cpt-cf-bss-products-flow-import`

**Touches**:
- Entities: `ChangeReport`
- DB Table: `products_bulk_row`

### One batch, one catalog version — and a window this feature does not close

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-operation-key`

The batch's catalog-version requests carry the **whole increment request** —
`(source, lane = bulk, request_key, operation_key)` — and it is the **lane**, not the key alone, that
holds the window open: `features/catalog-version.md` states the tuple as a `MUST`, and `design/06`
coalesces interactive requests within seconds while *"a **keyed bulk batch stays open** until the
**5-minute hard max** from its earliest request"*. A batch tagged with an `operation_key` on an
`interactive` lane closes after seconds and **shreds across many `CatalogVersion`s** — the exact
failure §6's flagship probe claims to assert against.

**`operation_key` occurs zero times in the crate**; the mechanism is entirely unbuilt.

**The window closes on a five-minute hard maximum, not on a signal from here.** **P-D-46** struck the
stored close marker for that reason, and the marker on `(source, operation_key)` an earlier pass had
added went with it. So this DoD wires a **tag**, and a build that added a close call would be building
a mechanism the decision removed.

**The tag rides an in-process contract, not the REST route.**
`features/catalog-version.md` publishes the increment request as **a client trait in the SDK**, a
typed contract resolved from `ClientHub` with the in-process binding as the default, and classifies
this feature as *"a registered **internal** requester whose requests carry an `operation_key`"* — the
REST door being that contract's **out-of-process** binding. So a bulk commit tags through the trait;
wiring it at the door would make an in-process commit self-call its own HTTP surface.

**Ticked (P-D-149).** The import and lifecycle doors write the batch's `operation_key` (the batch
id) at intake; when a commit pass publishes anything it enqueues **one** increment request through
the store the SDK binding itself calls — `repo::enqueue_increment_request` with
`(source = "bulk", lane = bulk, request_key = batchId, operation_key)` — idempotent on the key, so a
resumed commit re-enqueues nothing. The **lane** holds the window (`design/06`'s five-minute hard
max), the key tags it; no close call exists (P-D-46). Probe: the machine probe reads the request
back with its lane and key.

**Implements**: `cpt-cf-bss-products-flow-import`

**Touches**:
- Crates: `products-sdk`
- Modules: `api::rest`

### The coalesced completion event

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-coalesced-event`

Completion emits **exactly one** `CatalogBulkOperationCompleted` with the ledger digest. It ships
nowhere.

**"Exactly one" has a mechanism, not a convention.** The event is emitted **inside edge 4's CAS**
(**P-D-54**) — the transaction that flips `committing → completed` is the one that emits — so a
re-claim after a lease expiry finds the state already flipped and emits nothing. A build that emits
in a step after the flip carries no such guarantee.

**It is additive, not a suppression.** Row-level domain events all emit; what the summary coalesces is
per-row **progress noise**. `12-consumer-contracts`' event bookkeeping lint reads the summary as an
addition to the register, and the slice records that explicitly so the lint does not read it as
events withheld.

**It is also this slice's only event, and the other instructions now say so** (**P-D-61**): the eight
state-changing instructions of `design/09` carry an inline `**no event**` marker on 01's convention,
because a row's own act is announced by the 01 and 04 doors it drives and the batch's history — the
ledger, the `ChangeReport`, 05's approval record — is audit-plane (**P-D-21**). **What lint 12 reads is
the `EventRegister` table**, authored and never harvested (**P-D-45**), so this slice owes it exactly
one row: `CatalogBulkOperationCompleted` → `inst-bk-complete`. That authoring is `design/12` §6's
standing per-slice debt and is not discharged here.

**Adding it is not a one-line change.** The event roster is enumerated at **seven** sites across four
files: the payload-type constant, the `SCHEMA_REFS` row and the typed-event `match` arm in
`infra::events`; the `catalog_event!` invocation **and `prepare_every_event_type`'s per-type
`prepare` call** in `infra::broker`; and both `THE_EIGHT` literals in the two test modules.

**The seventh is the one whose omission fails at runtime rather than at boot**, and it says so
itself: *"Prepare each of the eight event types by name, so a registration missing any one of them
fails the boot rather than a door's transaction."* An event registered in `SCHEMA_REFS` and not
prepared there fails **inside the batch's own commit transaction** — the failure that function exists
to move. A ninth event not wired to the broker at all reaches a runtime `NoTypedEvent`, which
`infra::events` documents separately.

**Ticked (P-D-149).** The mechanism shipped with `complete_batch` (the CAS that emits); the digest
operand §7 row 31 named is pinned by P-D-127 — `(row_key, disposition, code, entity_id)` per row,
sorted, through `domain::canonical` — and is the same rendering the report edge pins on the record.
The seven roster sites carry the event; `lint 12`'s `EventRegister` row stays `design/12`'s debt.

**Implements**: `cpt-cf-bss-products-flow-import`

**Touches**:
- Modules: `infra::events`, `infra::broker`

### Export, and what makes it deterministic

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-export`

`GET /bss-products/v1/bulk/exports?catalogVersionId=` exists, spending **`catalog_version × read`** —
its own grant, decoupled from the import pair because an export is auditor-shaped. **The route does
not ship.**

Rendered from the catalog-version manifest: entity halves from frozen versions, capture halves from
the capture store. **Byte-for-byte deterministic** for a given version. The header carries a schema
format version, and the format carries the promotion identities plus full content.

**Artifacts are streamed, not stored** — determinism makes storage redundant.

**What determinism actually requires is not stated here and is not this DoD's to state.** §7 row 14
carries it: a byte-identical rendering needs a fixed ordering over the manifest's members and a
canonical serialization, and the slice names neither.

**Ticked (P-D-149).** `GET /bss-products/v1/bulk/exports?catalogVersionId=`
(`bulk::export_catalog_version`) on **`bulk × read`** — P-D-127's grant, which supersedes the
`catalog_version × read` this DoD's first sentence names — renders the stored manifest: every entry's
frozen version row (`repo::entity_version_at`, never the head) with its C5 identity, every capture
from the capture store, entries sorted by `(entity_kind, entity_id)` and captures by kind, the
header carrying `format_version = 1` (`EXPORT_FORMAT_VERSION`, §7 row 3's number). Byte-identical
for a version, streamed as one response, nothing stored; an unknown version is
`CATALOG_VERSION_UNKNOWN`. Probe: `the_export_is_byte_identical_for_a_version`.

**Implements**: `cpt-cf-bss-products-flow-export`

**Touches**:
- API: `GET /bss-products/v1/bulk/exports`
- Crates: `products-sdk`

### The promotion resolver, total over identity

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-promotion-resolver`

`PromotionResolver` runs **only when the batch's `mode` is `promote`** (**P-D-69** — the operand
that separates it from a plain import, which refuses the same collision as `DUPLICATE_CODE`), and
classifies every row into **exactly four** classes, in the slice's own order:
**create** on an unknown identity, ids re-minted; **no-op** where the identity is bound to matching
content; **update-as-draft** where it is bound to different content; and **conflict** where the identity is
bound to an incompatible kind or type, to a `retired` holder, or to a head carrying unpublished local
edits or an open approval — C5's own three arms, a dirty head not being a property of the binding.

**Update-as-draft is the promotion's purpose, and the register had to say so twice.** **P-D-17**
amended the requirement, the acceptance criterion and the use case to match the resolver, three PRD
statements having said the opposite after an earlier wave amended the constraint alone.

**The identities are C5's and are stated there**: `skuCode` for SKUs; `productCode`, else
`(brandId, canonical internal name)`, for Products; and for the live-entity kinds a category by
`(parent path, normalized name)`, a set member by `(set kind, member code)`, a definition by its key.
**What the resolver compares to decide *matching content* is stated nowhere** — §7 row 21 — and
without it `no-op` and `update-as-draft` have no predicate separating them.

**Totality is what makes the classification safe.** A `retired` holder conflicts because revival is
clone-only; a dirty head conflicts because an import never silently merges into in-flight work or
supersedes a local approval.

**Ticked (P-D-149).** `resolve_promotion` runs in the stage pass when `mode = promote`: identity is
the exported id, then `skuCode` / `productCode`, then `(brandId, normalized name)` (P-D-127 row 4);
an unknown identity **creates**; a `retired` or `discarded` holder is `PROMOTION_IDENTITY_CONFLICT`
(revival is clone-only); a head with unpublished edits (a draft, or a rendering that differs from its
last version row — the correction door's own predicate) or an open publish approval is
`PROMOTION_DIRTY_HEAD`; else the staged fields the save door recognises are compared canonically to
the head's content (row 21) — all equal is `no_op`, a difference is **update-as-draft** through the
ordinary save door, the row stamped with the head, the revision the save left and the touched
fields. A bucket-ii difference reaches that door and fails `ILLEGAL_FIELD_MUTATION` naming `07`'s
correction door (§7 row 2). Probe:
`a_promote_batch_classifies_no_op_update_as_draft_and_conflict_then_reverts`.

**Implements**: `cpt-cf-bss-products-flow-promote`

**Touches**:
- Entities: `PromotionResolver`

### The bulk lifecycle arm, and its separate grant

- [x] `p2` - **ID**: `cpt-cf-bss-products-dod-bulk-lifecycle`

`POST /bss-products/v1/bulk/lifecycle` exists, spending **`bulk_lifecycle × execute`** — **its own
grant**, so the gear's most destructive batch act cannot be reached with the import pair. **The route
does not ship, and neither does the grant** — and **who mints it is settled** (**P-D-69**):
`05-governance`'s catalog DoD mints all four of this feature's grant instances, the shipped roster
being one closed set under a two-way set-equality assertion, and a closed set takes one writer.

Each row runs the ordinary lifecycle policy doors with provenance `direct`, per-row confirmation data
aggregated into one report. The batch is **material at any size** by its transitions. One approval,
consumed once by the same flip, each row's transition door in `PreAuthorized` mode.

**The per-SKU flip guards stay.** No bulk override of the reference guard exists, so a referenced row
defers under the ordinary guard and **the batch never force-retires**. This is the one place where
"bulk runs the ordinary pipeline" prevents a whole class of operator accident.

**Ticked (P-D-149).** `POST /bss-products/v1/bulk/lifecycle` (`bulk::start_lifecycle_batch`) on
**`bulk_lifecycle × execute`**, its own label and grant — the import pair does not reach it — lands a
`lifecycle`-lane batch with one row per id whose `governed_live_op` is the op; the stage pass
validates each head against the ordinary `04` guard and pins its revision; the report is material at
any size; the commit drives `run_deprecate` / `run_retire` in `PreAuthorized` mode with provenance
`direct`, the reason the closed-set literal `bulk-lifecycle` (P-D-50), the per-head guards intact —
a referenced SKU defers under its own guard and nothing force-retires. Rows read `applied`. Probes:
`a_lifecycle_batch_deprecates_through_the_ordinary_door`,
`the_lifecycle_door_lands_a_lane_batch_and_replays_its_key`.

**Implements**: `cpt-cf-bss-products-flow-bulk-lifecycle`

**Touches**:
- API: `POST /bss-products/v1/bulk/lifecycle`
- Modules: `api::rest`

### The five codes, and the surface that reports them

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-bulk-errors`

Five codes: `BULK_DEPENDENCY_FAILED`, `PROMOTION_IDENTITY_CONFLICT`,
`PROMOTION_DIRTY_HEAD`, `BULK_OVERRIDE_UNACKNOWLEDGED`, `BULK_LIMIT` — **all five ship** as
`DomainError` variants with their ladder arms, `BULK_LIMIT` raised by the import door and the other
four's raisers arriving with the resolver, the stage phase and the override ceremony.
`STALE_LIVE_OP`, which this feature's commit phase raises, is `02-taxonomy-attributes`' and ships
nowhere.

**Row-level failures otherwise reuse the owning feature's code verbatim inside the ledger.** No
parallel taxonomy — which is why this roster is five rows and not thirty.

**Four of the five are per-row ledger outcomes and the door answers 202**, so their statuses apply
only where a caller asks a single row's disposition — **and that caller ships** (**P-D-61**):
`GET /bss-products/v1/bulk/batches/{batchId}`, spending its own **`bulk × read`** grant, returns the
batch state plus the `RowLedger` one entry per row, and those statuses are what it returns per row.
One route serves both lanes. It was minted rather than the statuses declared dormant because C1 and
`PRD.md` both carry *"report per-row success/failure"* as a **MUST**.

**The statuses are proposed, and this DoD carries that rather than pinning them.** `design/09` §3.2
ends *"Proposed per row and open to correction; the requirement is that every code carries one."* The
codes are fixed; §6's criteria say which of the two they assert.

**Implements**: `cpt-cf-bss-products-algo-bulk-errors`

**Touches**:
- Modules: `domain::error`, `domain::validation`, `infra::error_mapping`

### Resume and abandon, both through ordinary doors

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-resume-abandon`

**Resume**: a crash mid-commit resumes from the ledger, per-row publishes being idempotent by row
key — and a **published** row's re-execution is stopped one layer earlier still: its
`internal:bulk-row` claim stores the ledger outcome as its response record (**P-D-69**), so the
resume replays the stored answer.

**Abandon now has its state** (**P-D-69**): `reported → abandoned`, entered on the batch approval's
rejection or explicit withdrawal, terminal, releasing the tenant slot — the procedure below is its
executor.

**Abandon uses no new door**, and each row kind has its own path: created drafts **discard** through
the ordinary discard door; **update-as-draft rows revert** through the ordinary save door with the
last frozen version's content as the payload, so the head returns to its published content with a
revision bump; pending live-entity operations are **dropped**, never applied.

**The audit reason is a literal constant** — `batch-abandoned` — and this feature writes **no**
operator free-text reason at all (**P-D-50**). That is why `02-taxonomy-attributes`' content-PII
enumeration no longer names it, and a build that added a free-text field would put this feature back
inside a hook it was measured out of.

**Abandon has no state to land in.** §4's machine has no abandon state, so what a batch's state
becomes after abandonment is §7 row 5's, not this DoD's.

**Ticked (P-D-149).** Resume: a re-claimed commit skips every disposed row and replays a claimed
one from the `internal:bulk-row` lane's stored answer — the ledger is the record — and a second
sweep over a completed batch publishes nothing twice. Abandon: created drafts discard, lifecycle rows
drop their pending op untouched, and an **update-as-draft row reverts through the ordinary save
door** to the fields its marker names at their last frozen values, the head returning to its
published content with a revision bump. Probe: the promotion probe's revert, the machine probe's
second sweep.

**Implements**: `cpt-cf-bss-products-algo-batch`

**Touches**:
- DB Table: `products_bulk_batch`, `products_bulk_row`
- Modules: `api::rest`

### The four design-introduced names exist as named seams

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-bulk-seams`

`design/09` §1.7 introduces four names and each is addressable:

- **`BulkBatch`** — the unit: rows, batch key and the state machine §4 declares. *Addressable:
  `infra::storage::entity::bulk_batch` and the import door's own module.*
- **`RowLedger`** — per-row state, and **the no-hidden-partial-failure surface**. *Addressable:
  `infra::storage::entity::bulk_row`, read out by `GET /bulk/batches/{id}`.*
- **`ChangeReport`** — the aggregated approval artifact the quorum signs. *Arrives with its own
  DoD.*
- **`PromotionResolver`** — the four-way classifier of §5's resolver DoD. *Arrives with its own
  DoD.*

**DECOMPOSITION §2.9's entity list carried three of the four** — `BulkBatch`, `RowLedger`,
`ChangeReport` — and this pass swept `PromotionResolver` in beside them, on the rule §2.7 follows.
Whether that field is a listing convention or an ontological claim is registered as
`features/clone.md` §7 row 24 and assigned to the design-set owner; this document cites that row and
raises no second one.

**Implements**: `cpt-cf-bss-products-algo-batch`, `cpt-cf-bss-products-flow-promote`

**Touches**:
- Entities: `BulkBatch`, `RowLedger`, `ChangeReport`, `PromotionResolver`, `BatchWorker`

## 6. Acceptance Criteria

**The sizing flagship**

- [ ] The ten-thousand-row fixture stages, reports and commits within the sizing envelope, emits
      **exactly one** `CatalogBulkOperationCompleted`, and lands in **exactly one** `CatalogVersion` —
      the `operation_key` probe, asserted from this side and from `06-catalog-version`'s.
      **Blocked on §7 row 9**: `operation_key` has a stated writer and no declared column.

**No hidden partial failure**

- [ ] A Product row fails, its SKU rows fail `BULK_DEPENDENCY_FAILED`, **and its siblings proceed** —
      positive and negative in one batch, with **no orphan at any point**.
- [x] A row edited after the report fails `STALE_REVISION` **alone** at commit: the batch approval
      stands, siblings publish, and the ledger names the row.
- [ ] A live-entity operation whose target moved fails `STALE_LIVE_OP` **alone**, the same
      row-local posture.
- [x] A batch with failed rows reaches **`completed`**, not `failed` — parts-succeeded is an end
      state and not an error.

**The governance contract**

- [x] The batch approval is consumed **exactly once**, at the `approved → committing` flip. A probe
      counts consumptions across a batch of many rows and asserts **one**.
- [x] Every per-row publish runs in `PreAuthorized` mode and **spends nothing**: the record's state
      after commit is the one the flip left.
- [ ] A per-row publish attempted **without** a consumed record naming that subject is refused —
      asserted against `05-governance`'s record-backed host, **not** against the shipped
      `NoMaterialityPolicyGate`, whose refusal under this mode is **unconditional**: it ignores the
      subject and the revision and names the missing record store. A probe run against today's host
      passes for the wrong reason and stops testing anything the moment a real host is registered.
- [x] A row whose override condition appeared **after** the report fails
      `BULK_OVERRIDE_UNACKNOWLEDGED` alone, while the acknowledged itemised set publishes under the
      one batch ceremony.
- [x] The report's override section names rows **by `skuCode`**, never as a count — the case an
      acknowledgment-by-name cannot be built from.

**Idempotency, across all three key scopes**

- [x] A replayed **batch** key returns the existing batch and starts no second one.
- [ ] A **row** retried within the batch commits nothing twice, keyed on the ledger.
- [ ] A row re-listed in a **new** batch is a new act, and a code collision is the ordinary
      `DUPLICATE_CODE`.
- [ ] Per-row publishes resolve the publish door's key on the reserved **`internal:bulk-row`** lane —
      the lane that ships unused, asserted by name so a build cannot mint a fourth.
- [x] A crash mid-commit resumes and completes **without duplicates**.

**Promotion**

- [x] One fixture over the resolver's **four** classifications — create, no-op, update-as-draft,
      conflict — including the codeless Product resolved by `(brandId, canonical internal name)`,
      which is `design/09` C5's fallback where `productCode` is absent.
- [ ] A `retired` holder conflicts, and the refusal names clone as the revival path.
- [x] A target head carrying unpublished local edits or an open approval fails
      `PROMOTION_DIRTY_HEAD` — **both** cases, since either alone passes a build that checks only the
      other.

**Export**

- [x] Two exports of the same `catalogVersionId` are **byte-identical**. **Blocked on §7 row 14**:
      no document states the ordering and serialization that makes this so.
- [x] The artifact header carries a schema format version.

**Bulk lifecycle**

- [ ] Mass retire schedules per-row transitions, and a referenced row's flip **defers under the
      ordinary guard** — the batch never force-retires.
- [ ] The lifecycle door refuses a caller holding only the import pair.

**The error roster, one control per code**

- [ ] Each of the **five** declared codes has a failing case **and** a passing control — five pairs.
      `STALE_LIVE_OP` is `02-taxonomy-attributes`' and is asserted here as a raise, not a declaration.
- [ ] Each assertion names the **code**, not the status: `design/09` §3.2 records the statuses as
      *"Proposed per row and open to correction"*, and a probe pinning a proposed number would fail
      on a correction rather than on a defect.

**Bounds**

- [ ] A batch over the configured row maximum is refused `BULK_LIMIT`, and so is a tenant over the
      concurrent-batch maximum — two operands, one code.

## 7. Known unknowns

**The arithmetic of this section.** Thirty-one rows: **eighteen carried verbatim** from
[`../design/09-bulk-promotion.md`](../design/09-bulk-promotion.md) §6 — the slice's full count, not a
selection — and **thirteen raised here**: eleven by the three-lens review of this document, one by
**P-D-86**'s staged-payload answer, and one by the build of the terminal edges. The carried count is
built from an independent primitive: the number of `- ` line starts in that section, which is
eighteen and agrees with both the split and the transcribed rows. A final subsection carries defects
owed to other documents; those are not rows.

**Thirty rows now: the twenty-nine below plus row 30, raised by the group 13a build when it reached
the wall.** **Thirteen of the thirty block no DoD**: row 1, plus rows 8, 9, 18 and 26 (resolved on
**2026-08-31**: **P-D-54** answered row 26, freeing `cpt-cf-bss-products-dod-stage-phase` and
promoting row 18 to sole blocker of `dod-coalesced-event`; **P-D-61** answered rows 8, 9 and 18,
freeing `dod-bulk-errors`, `dod-bulk-tables` and `dod-coalesced-event`), and rows 5, 15, 19, 20, 24,
25 and 27, resolved on **2026-09-01** by **P-D-69**, freeing `dod-resume-abandon`, `dod-import-door`,
`dod-promotion-resolver`, `dod-idempotency-lane` and `dod-bulk-lifecycle`. All are kept in place
rather than struck. `dod-batch-state-machine` remains blocked by rows 6 and 7 — the never-approved
batch's tenant slot and the commit-phase trigger, neither answered here — and
`dod-stage-phase`, freed by P-D-54 and re-blocked by row 30, is **freed again by P-D-86**, which
gives the phase the payload column its executor had nothing to execute over.

**Carried, not answered**, and registered against **its owner's** register. **Three departures from
verbatim, declared so the claim is checkable.** First, the slice's inline `Owner:` sentence and any
provenance marker become this document's `**Owner**:` field — and rows 1, 2 and 3 state no owner in
any form, so they are carried unassigned. Second, every bare `§N` inside a carried row is
**`design/09`'s numbering, not this document's**. Third, each row gains a `**Blocks**:` field, which
`design/09` §6 does not have.

**One question is deliberately NOT raised here because another document owns the rule it turns on**:
whether DECOMPOSITION's entity field is a listing convention or an ontological claim —
`features/clone.md` §7 row 24, the design-set owner's.

### Carried verbatim from `design/09` §6

1. **C6 after the H1 fix is barely a deviation**: domain events all emit; only per-row
    bulk-progress noise is folded into the coalesced summary — recorded so slice 12's completeness
    check reads the summary event as additive, not as suppression.
    **Blocks**: no DoD — it records why the coalesced event reads as additive to slice 12's lint,
    not as suppression.
    **Owner**: `12-consumer-contracts` — a note for its lint, not a question (assigned by P-D-127, 2026-09-03).

2. ~~**Update-as-draft promotion onto a live entity** stages bucket-iii/iv changes on the target's
    head.~~ **Answered (P-D-149, 2026-09-05): built as stated** — the resolver passes every
    differing recognised field to the ordinary save door, so a bucket-ii difference onto a published
    identity fails there with `ILLEGAL_FIELD_MUTATION` naming `07`'s correction door; the probe is
    the resolver's. *The item's text stood as:* bucket-i/ii differences (type, meter) cannot be
    promoted onto an existing identity and land as per-row conflicts directing to the 07 correction
    door; these rows classify as update-as-draft and fail at the save door with 01's
    `ILLEGAL_FIELD_MUTATION`, whose reason names 07's correction door; worth its own probe when built.
    **Blocks**: no DoD — **resolved by P-D-149** *(was: `cpt-cf-bss-products-dod-promotion-resolver`.)*
    **Owner**: this feature — a probe owed when the resolver is built (assigned by P-D-127, 2026-09-03).

3. ~~**Export format versioning** (schema evolution of the artifact) rides slice 12's vN→vN+1
    discipline.~~ **Answered (P-D-149, 2026-09-05): the artifact carries `format_version = 1`**
    (`bulk::EXPORT_FORMAT_VERSION`) in its header from day one; the vN→vN+1 discipline stays `12`'s.
    *The item's text stood as:* named here so the exporter carries a format version from day one.
    **Blocks**: no DoD — **resolved by P-D-149** *(was: `cpt-cf-bss-products-dod-export`.)*
    **Owner**: this feature with `12-consumer-contracts` (assigned by P-D-127, 2026-09-03).

4. ~~**A renamed Product carrying no `productCode` is promoted as a create, not an update.**~~ **Answered (P-D-127, 2026-09-03): identity is `productId`, then `productCode`, then `normalized(name)`** — an export carries ids, so a round-trip row resolves by id whatever was renamed; a hand-authored row without either is a create when unmatched. *The item's text stood as:*
    `normalized(name)` is both bucket-iii in 01 §4.1 (a published Product is renameable) and this
    slice's C5 fallback promotion identity where `productCode` is absent — and `product_code` is
    nullable, so a rename between promotions makes the target resolve *unknown identity* and
    create a second Product, which C5's four-way classification exists to prevent.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-promotion-resolver`.)*
    **Owner**: this slice. *(Filed from 01 §6 by the slice-01 eighth lens pass — the pointer
    claimed it was registered here and it was not.)*

5. ~~**The batch state machine has no rejection edge and no abandon state.**~~
    **Answered in the slice (owner call, 2026-09-01 — P-D-69 arm 1): `reported → abandoned` on the
    approval's rejection or explicit withdrawal — the abandon procedure's own state, terminal,
    releasing the tenant slot — and `failed` entered by `staging|committing → failed` on the worker's
    attempt-budget exhaustion**, `inst-ar-failure`'s arm, row failures staying row-local. Rows 6 and 7
    stay open.
    Original text: `reported` has one
    stated exit while the approval it waits on can be `rejected`; nothing states what enters
    `failed` (every row failure is explicitly row-local, "siblings never block"); and
    `inst-bm-resume`'s abandon path has no state to write. A rejected batch sits in `reported`
    forever holding its staged drafts and a slot against the concurrency ceiling.
    **Blocks**: no DoD — **resolved by P-D-69**; `cpt-cf-bss-products-dod-resume-abandon` is freed, `dod-batch-state-machine` staying blocked by rows 6, 7 and 19.
    **Owner**: was this slice, with 05 if abandonment releases the approval. *(Raised by the slice-09
    first lens pass.)*; **closed**.

6. ~~**What ends a batch that is never approved?**~~ **Answered (P-D-127, 2026-09-03): a reaper tick in `gear.rs`'s loop flips it to `abandoned` (P-D-69's state) after `bulk_batch_ttl_hours`, 168 interim**, superseding the record. *The item's text stood as:* No timeout, expiry or reaper is stated, and the
    approval it waits on has no deadline either — so an abandoned-but-unabandoned batch consumes a
    tenant slot permanently.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-batch-state-machine`.)*
    **Owner**: this slice with 05. *(Raised by the slice-09 first lens pass.)*

7. ~~**What door, actor or signal starts the commit phase?**~~ **Answered (P-D-127, 2026-09-03): the bulk worker** — `batch_tick` observes the record `satisfied`, and its claim transaction writes `committing` and consumes the record; the one-shot is enforced there. *The item's text stood as:* "On quorum" names no executor: this
    slice declares three routes and none is a commit, it names no runner, and 05's decide door is
    itself unowned (05 §6). The answer fixes which transaction writes `committing` and therefore
    where the one-shot consumption is enforced.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-commit-phase`,)*
    `cpt-cf-bss-products-dod-batch-state-machine`.
    **Owner**: this slice with 05. *(Raised by the slice-09 first lens pass.)*

8. ~~**What surface reports the `RowLedger`?**~~
    **Answered in the slice (owner call, 2026-08-31 — P-D-61): one read route.**
    `GET /bss-products/v1/bulk/batches/{batchId}` (`design/09` §2 `inst-bk-read`) returns the batch
    state plus the `RowLedger` one entry per row, under its own **`bulk × read`** grant — a reader is
    not an executor, and the finance reviewer who signs a batch must read without gaining the right to
    start one. One route for both lanes, the key being the batch id. It was minted rather than the
    statuses declared dormant because C1 and `PRD.md` both carry *"report per-row success/failure"* as
    a **MUST**; the price is 05's roster and a third route census.
    Original text: Four per-row codes carry HTTP statuses that apply
    "where a caller asks a single row's disposition", and no route, verb or grant exists for that
    ask — the RBAC catalog mints only the two execute pairs, 08 projects no bulk read model, and
    §4 adds no read surface. C1's "per-row success/failure reported" has no reader.
    **Blocks**: no DoD — **resolved by P-D-61**; `cpt-cf-bss-products-dod-bulk-errors` is freed.
    **Owner**: was this slice with the contract owner; **closed**.

9. ~~**§4 declares two table names and no columns.**~~
    **Answered in the slice (owner call, 2026-08-31 — P-D-61): `design/09` §4 now carries the
    normative shape**, authored from exactly the values this row enumerated plus P-D-54's six states
    and the worker's claim and lease — and from nothing else: no counter duplicating the ledger, no
    table for the derived `ChangeReport`, and `reason` a literal from a closed set rather than
    operator text (**P-D-50**).
    Original text: The heading promises a normative shape and
    every sibling supplies one. Values with a stated writer and no column: the per-row pinned
    revision, the batch and row keys, the row disposition and reason, the pending `GovernedLiveOp`
    payload, the itemised override set, and the `operation_key`.
    **Blocks**: no DoD — **resolved by P-D-61**; `cpt-cf-bss-products-dod-bulk-tables` is freed.
    **Owner**: was this slice's storage owner with 05's; **closed**.

10. ~~**Does a `bulk_batch` approval authorize a `GovernedLiveOp` apply?**~~ **Answered (P-D-127, 2026-09-03): yes** — the batch's record is the subject for every row, live-entity ops included, under P-D-105's predicate extended to `products_bulk_batch.approval_ref` with its own writer-count guard; `05`'s composite-act enumeration gains the batch. *The item's text stood as:* This slice says the
    live-entity ops apply "under the same consumed approval", while 05's live-op gate asks its
    question with the op **envelope** as subject, and 05's composite-act enumeration names per-row
    publishes and nothing else. 05's approval carries one subject and a partial unique over it.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-commit-phase`.)*
    **Owner**: the governance owner with 05 and 02 — extend the composite-act enumeration, or give
    each live-entity row its own record. *(Raised by the slice-09 first lens pass.)*

11. ~~**Does the per-row pin fit 05's store?**~~ **Answered (P-D-127, 2026-09-03): the record's scalar pin for a batch is the ledger digest** (P-D-125's per-kind pin); the per-row pins live in `content_snapshot`, which holds the report. *The item's text stood as:* This slice states the approval's stored snapshot as
    the report **plus the ledger's per-row pinned revisions**, and 05 declares one scalar
    `internal_revision` per record — registering the mismatch itself as "What do the entity-shaped
    columns hold for the non-entity subject kinds?".
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-change-report`.)*
    **Owner**: 05's owner with 12. *(Raised by the slice-09 first lens pass.)*

12. ~~**Where does the `ChangeReport`'s diff read its two operands?**~~ **Answered (P-D-127, 2026-09-03): staged content from this slice's ledger, the target's heads through `01`'s repository read**, rendered through `domain::canonical`; `08` is not the input. *The item's text stood as:* The pre-approval view is
    "staged content vs the target's current heads", this slice books 08 as the input, and 08's C6
    projects entity content **only** from frozen version rows, never from heads — and serves no
    draft. Neither operand has a stated producer.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-change-report`.)*
    **Owner**: this slice with 08. *(Raised by the slice-09 first lens pass.)*

13. ~~**Which slice builds the lint producer?**~~ **Answered (P-D-125, 2026-09-03): a dry-run of `01`'s publish pipeline**, per row. *The item's text stood as:* The `ChangeReport` is a PRD MUST that must carry
    lint findings; 06 §6 records that no instruction, store, RBAC pair, error code or probe in
    that slice delivers the report and names this slice as co-owner of the gap.
    **Blocks**: no DoD — **resolved by P-D-125** *(was: `cpt-cf-bss-products-dod-change-report`.)*
    **Owner**: the design-set owner with 06 and this slice. *(Two lenses raised it
    independently.)*

14. ~~**What makes the export byte-deterministic?**~~ **Answered (P-D-127, 2026-09-03): P-D-29's rule applied to the artifact** — `domain::canonical`, every collection sorted by its identifier, `06`'s entries by `(entity_kind, entity_id)`. *The item's text stood as:* C4 promises byte-for-byte determinism for a
    given version; 06 §6 records that the manifest it renders from has no named sort key for its
    row collections, and `inst-bk-export` states no canonical serialization for the artifact
    itself.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-export`.)*
    **Owner**: P-D-29's owner with 06 and this slice. *(Raised by the slice-09 first lens pass.)*

15. ~~**Does every import run the `PromotionResolver`, and what selects promotion mode?**~~
    **Answered in the slice (owner call, 2026-09-01 — P-D-69 arm 2): a batch-level
    `mode ∈ {import, promote}`, default `import`.** Only `promote` engages the resolver; under
    `import` the collision stays `DUPLICATE_CODE`. Per-row mixing declined — a mixed batch is two
    batches, and a silent auto-update on collision converts typos into overwrites.
    Original text: The same
    door, the same stage phase and the identical case — a `skuCode` already bound with different
    content — gets `DUPLICATE_CODE` under `inst-bk-keys` and update-as-draft under
    `inst-pm-resolve`. No request field, header or route segment distinguishes an import from a
    promotion.
    **Blocks**: no DoD — **resolved by P-D-69**; `cpt-cf-bss-products-dod-promotion-resolver` and `dod-import-door` carry the operand.
    **Owner**: was this slice with the contract owner. *(Raised by the slice-09 first lens pass.)*; **closed**.

16. ~~**What is "update-as-draft" for a row kind with no draft state?**~~ **Answered (P-D-127, 2026-09-03): `update-as-live-op`** — the row becomes a `GovernedLiveOp` envelope applied at commit under the batch record; C5 gains the arm. *The item's text stood as:* C5 calls its four
    classifications exhaustive and this pass added the live-entity promotion identities to it, so
    a promoted category whose content differs falls into a classification it cannot occupy.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-promotion-resolver`.)*
    **Owner**: this slice with 02. *(Raised by the slice-09 first lens pass.)*

17. ~~**Is the scope-values lint blocking or advisory, and what does "unseen" mean?**~~ **Answered (P-D-127, 2026-09-03): advisory** — the finding rides the `ChangeReport` and the override ceremony acknowledges it; *"unseen"* is a token no published entity of the tenant carries. *The item's text stood as:* It appears
    once, carries no code, no threshold and no probe, and nothing says whether it stops the
    report, the approval or nothing.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-change-report`.)*
    **Owner**: this slice. *(Raised by the slice-09 first lens pass.)*

18. ~~**No instruction in this slice names its event or records "no event".**~~
    **Answered in the slice (owner call, 2026-08-31 — P-D-61): the marker is on the eight
    state-changing instructions**, and this row's premise was half wrong. **Lint 12 reads only the
    `EventRegister` table** — authored, never harvested (**P-D-45**) — so it lints the register rather
    than the instructions; what 01 supplies is the inline `**no event**` convention. Measured: 13
    instructions, one naming an event. The eight carrying the marker are `inst-bk-keys`,
    `inst-bk-stage`, `inst-bk-report`, `inst-bk-commit`, `inst-bk-override`, `inst-pm-resolve`,
    `inst-bl-lifecycle` and `inst-bm-resume`; `inst-bk-export` is a read and `inst-bm-tables`,
    `inst-bm-limits` and `inst-pm-review` declare rather than write. **The row's count of six is short
    by two**, which is a judgement about what counts as state-changing, so all eight are enumerated
    rather than totalled. Original text: 01 states the rule
    over every slice and 12 lints it; only `inst-bk-complete` names one, while six further
    state-changing instructions declare nothing.
    **Blocks**: no DoD — **resolved by P-D-61**; `cpt-cf-bss-products-dod-coalesced-event` is freed.
    **Owner**: was this slice; **closed**, the one owed `EventRegister` row staying `design/12` §6's.


### Raised here rather than carried

19. ~~**Can `PreAuthorized`'s predicate name a batch at all?**~~
    **Answered (owner call, 2026-09-01 — P-D-69 arm 3): expressible since P-D-67 widened the gate's
    subject to `(subject_kind, subject_ref)`, and the revision operand is not the gate's.** P-D-54
    edge 2 pins the approval's snapshot as the report plus the ledger's per-row revisions, so each
    per-row publish checks **its own ledger pin** row-locally, and `PreAuthorized` verifies only that
    the named record was consumed for this subject. Governance §7 row 27's measured text stands —
    the mode carries no membership operand — because membership is the ledger's.
    Original text: `features/governance.md` §7 row 27
    records that it cannot: the mode verifies a record *"authorized **this subject** at **this pinned
    revision**"*, while *"a bulk row's revision is its own"*, and *"the mode carries only an id with
    no plan-membership operand"*. The shipped seam is narrower — `evaluate`'s subject is an
    `EntityRef` whose `entity_kind` is `{Product, Sku}`, so a `bulk_batch` subject cannot be
    expressed, and one scalar revision crosses it. Weakening the predicate to *"names a consumed
    record"* is refused there as turning a terminal record into an unbounded bearer token.
    **Blocks**: no DoD — **resolved by P-D-69**; `dod-batch-state-machine` stays blocked by rows 6 and 7 only.
    **Owner**: was `05-governance`'s owner with `04-lifecycle`'s and this feature — the owner row 27
    itself names. *(Raised independently by all three lenses.)*; **closed**.

20. ~~**What does an `internal:bulk-row` claim row write into its response columns?**~~
    **Answered (owner call, 2026-09-01 — P-D-69 arm 4): the outcome record is the ledger row's
    disposition** — the claim stores P-D-42's synthetic `200` with
    `{disposition, code, reason, entity_id, published_version}` as the body, the same record the
    P-D-61 read door returns per row, so crash-resume replays the stored outcome instead of
    re-executing a published row.
    Original text: The store's rule
    is **P-D-42**'s — an internal lane *"stores a synthetic `200` and its own outcome record as the
    body"* — and `chk_products_idempotency_response_group` admits no third shape. **What a bulk row's
    outcome record is, is stated nowhere**, and `design/09` §4 declares no column for it. Without it
    a published row leaves its key `claimed` and the crash-resume path re-executes it.
    **Blocks**: no DoD — **resolved by P-D-69**; `cpt-cf-bss-products-dod-idempotency-lane` and `dod-resume-abandon` carry the record.
    **Owner**: was `01-foundation`'s storage owner with this feature; **closed**.

21. ~~**What does the resolver compare to decide *"matching content"*?**~~ **Answered (P-D-127, 2026-09-03): canonical equality of the bucket-iii/iv fields** after the save door's normalization, through `domain::canonical`; capture halves are not compared; equal ⇒ `no-op`. *The item's text stood as:* C5 says only *"identity bound
    to matching content ⇒ **no-op**"*. Which buckets participate, whether capture halves count, and
    what canonicalization runs are unstated — so `no-op` and `update-as-draft` have no predicate
    separating them, and two of the resolver's four classes are indistinguishable in a build.
    Carried row 12 asks where the report's diff reads its operands, not how they are compared.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-promotion-resolver`.)*
    **Owner**: this feature with `02-taxonomy-attributes` and `08-read-models`.

22. **Who runs an export?** `design/09` §1.3 gives it to the catalog admin; `PRD.md` §3 gives the
    catalog admin *"runs bulk import/export"* and the auditor *"exports for compliance"*; this
    document's own flow named the auditor before this pass corrected it to the slice's attribution.
    Nothing can settle it from the RBAC catalog, whose columns are resource × action, door and slice
    — **there is no actor column** — so the grant is mapped to a route and never to a role.
    **Blocks**: `cpt-cf-bss-products-dod-export`.
    **Owner**: *(P-D-127, 2026-09-03: the grant half is answered — export spends `bulk × read`, import `bulk × execute`, both in `gts/permissions.rs`; the role half stays the PRD owner's.)* the PRD owner with `05-governance`'s catalog owner.

23. ~~**Can `content_snapshot` carry N per-row pins?**~~ **Answered (P-D-127, 2026-09-03): yes** — `content_snapshot` holds the report, and the report carries the per-row pins; the scalar pin is the ledger digest. *The item's text stood as:* This feature's approval snapshot is the report
    **plus the ledger's per-row pinned revisions**, and `design/05` declares one `content_snapshot`
    column beside one pinned `internal_revision`. Whether one JSON column may hold N pins, and
    whether the pins are part of what the quorum signs, is unstated — and there is no shipped
    approval table to measure against. Carried row 11 registers the `internal_revision` half only.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-change-report`,)*
    `cpt-cf-bss-products-dod-commit-phase`.
    **Owner**: `05-governance`'s owner, as the second half of carried row 11.

24. ~~**What does a bulk row's `payload_hash` digest, a row having no request body?**~~
    **Answered (owner call, 2026-09-01 — P-D-69 arm 5): one rule for all three lanes** — an internal
    lane's `payload_hash` digests the canonical serialization of the act's own input record (the bulk
    row's staged payload, the `ScheduledTransition` row, the cascade leg), now stated in `design/01`
    §4.4, keeping a replayed key with different content detectable.
    Original text: The column is
    `NOT NULL` and the shipped door sources it from a digest over the parsed request body. All three
    internal lanes have the same gap, and `features/lifecycle.md` leaves its sibling lane's
    `client_key` open for the same reason without reaching the digest. One rule should serve all
    three.
    **Blocks**: no DoD — **resolved by P-D-69**; `cpt-cf-bss-products-dod-idempotency-lane` carries the rule.
    **Owner**: was `01-foundation`'s storage owner with this feature and `04-lifecycle`; **closed**.

25. ~~**Is the lane's `client_key` the ledger row's surrogate id or the caller's batch-scoped row
    key?**~~
    **Answered (owner call, 2026-09-01 — P-D-69 arm 6): the ledger row's surrogate id** — P-D-26's
    *"its own id"* at its natural referent. A row re-listed in a new batch has a new ledger row and
    therefore a new key: the new-act rule holds with no batch column added to the shipped primary
    key.
    Original text: The shipped primary key is `(tenant_id, endpoint, client_key)` with **no batch column**,
    so under the second reading a row re-listed in a new batch replays the first batch's answer —
    contradicting this feature's own rule that such a row is a new act. **P-D-26** says only *"its
    own id in `client_key`"*.
    **Blocks**: no DoD — **resolved by P-D-69**; `cpt-cf-bss-products-dod-idempotency-lane` and `dod-import-door` carry the referent.
    **Owner**: was this feature's storage owner with `01-foundation`'s, when §4's row columns land; **closed**.

26. ~~**What executes edges 1 and 4?**~~
    **Answered (owner call, 2026-08-31 — P-D-54): the gear-owned batch worker's claim transaction, at
    both ends.** Edge 1 is flipped by the claim that stages the last row, edge 4 by the claim that
    lands the last row's terminal state, and `CatalogBulkOperationCompleted` is emitted **inside edge
    4's CAS**, which is where *"exactly one"* comes from. Crash recovery is the claim's **lease** — the
    actor `design/09`'s `inst-bm-resume` has needed since it promised a batch resumes from the ledger.
    The normative text names no framework; the register carries the platform measurement, and the
    donor's inline posture is named as not transferring, because that door answers before the work
    starts. Original text: Both fire on a condition over every row — a stage outcome, a
    terminal ledger state — and neither names a door, actor or signal; the import door cannot be one,
    having answered **202**. Carried row 7 registers edge 3's executor and not these two.
    **Blocks**: no DoD — **resolved by P-D-54**; `cpt-cf-bss-products-dod-stage-phase` is freed,
    and `cpt-cf-bss-products-dod-batch-state-machine` and `cpt-cf-bss-products-dod-coalesced-event`
    carry the answer while staying blocked by their other rows.
    **Owner**: was this feature; **closed**.

27. ~~**Which of the grants must be minted, and by whom?**~~
    **Answered (owner call, 2026-09-01 — P-D-69 arm 7): `05-governance`'s catalog DoD mints all four
    instances.** The shipped roster is one closed set under a two-way set-equality assertion, and a
    closed set takes one writer — the lesson this corpus paid for at four sites. This feature's doors
    consume the grants.
    Original text: This feature spends
    `bulk × execute`, `catalog_version × read`, `bulk_lifecycle × execute` and — since **P-D-61** —
    **`bulk × read`**, so the count is **four, not three**. All four are in
    `design/05`'s RBAC catalog; **none is in the shipped permission roster**, which holds exactly six
    ids, all `product_*` and `sku_*`, under a two-way set-equality assertion. Whether this feature
    mints the instances or `05-governance`'s catalog DoD does is stated nowhere.
    **Blocks**: no DoD — **resolved by P-D-69**; `dod-import-door`, `dod-export` and `dod-bulk-lifecycle` are unblocked by it.
    **Owner**: was `05-governance`'s owner with this feature; **closed**.

28. ~~**What makes the `ChangeReport`'s sample deterministic?**~~ **Answered (P-D-127, 2026-09-03): the first N rows by `row_key` ascending**, N on the P-D-107 idiom, 20 interim. *The item's text stood as:* The report carries *"a deterministic
    sample"* and no document states a size, a selection rule or an ordering. A sample with no rule
    cannot be reproduced between the report the quorum signs and the commit that follows it.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-change-report`.)*
    **Owner**: this feature, alongside carried row 17's scope-values-lint question.

29. ~~**Where is the batch's itemised override acknowledgment stored?**~~ **Answered by the crate (P-D-127, 2026-09-03): the decision row's `override_acknowledgments`** (text, JSON), beside each ledger row's `override_acknowledged`; the approval row needs no column. *The item's text stood as:* This feature stores it on the
    batch's approval record *"exactly as an entity-publish override is stored on its own"*, and
    `features/governance.md` records that **there is no such column**: the only acknowledgment column
    sits on the decision row, which demands an approver principal and a verdict, and *"the approval
    row has no acknowledgment column"*. Its own open item 10 is the same gap from the other side.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-bulk-override-ceremony`.)*
    **Owner**: `05-governance`'s owner.

30. ~~**Where does a Product or SKU row's staged payload live between the import door and the
    worker?**~~
    **Answered (owner call, 2026-09-01 — P-D-86): a `staged_payload` column on the ledger
    row**, nullable, carrying the canonical serialization of the row's imported content — written
    by the import door, read by the worker, appended by an in-place edit of
    `m20260901_000014` and shape-CHECKed against the row class. The synchronous-door alternative
    is measurably wrong: it would put a whole batch's validation inside one HTTP call, against
    the 202, the sizing fixture and the executor `dod-stage-phase` and P-D-54 both name.
    `governed_live_op` keeps its stated meaning; folding the two would rewrite a gloss.
    Original text: `design/09` §4's ledger columns are `entity_kind`, `entity_id`, `pinned_revision`,
    `disposition`, `code`, `reason`, `governed_live_op`, `override_acknowledged` and `terminal_at` —
    and `governed_live_op` is scoped by its own gloss to *"the pending payload a **live-entity** row
    stages"*. So a Product or SKU row has **no column carrying the content it was imported with**,
    while two settled statements presuppose one: `cpt-cf-bss-products-dod-stage-phase` has those
    rows *"run the ordinary per-row pipeline — **parse**, then the same registered validators as
    interactive authoring"*, and **P-D-69** arm 5 (row 24 above) fixes the lane's `payload_hash` as
    a digest over *"the bulk row's **staged payload**"*. Measured at `6b8c31c23`: the import door
    ships and records the ledger; it accepts no content per row, because there is nowhere to put it.
    Two homes are visible and this document authors neither — a payload column on the ledger row,
    symmetric with `governed_live_op` and appended by the same in-place edit the chain uses; or a
    door that stages synchronously, which contradicts the worker `dod-stage-phase` and **P-D-54**
    both name as the phase's executor. *(Raised by the group 13a build, which reached the wall.)*
    **Blocks**: no DoD — **resolved by P-D-86**; `cpt-cf-bss-products-dod-stage-phase` is freed again.
    **Owner**: was this feature with `01-foundation`'s storage owner; **closed**.

31. ~~**What does "the ledger digest" cover?**~~ **Answered (P-D-127, 2026-09-03): the executor's set is pinned** — `(row_key, disposition, code, entity_id)` per row, sorted by `row_key`, through `domain::canonical`, payload and timestamps excluded; `design/09` owes the sentence. *The item's text stood as:* `inst-bk-complete` and `dod-coalesced-event` both
    require the completion summary to carry it, and **neither the design nor any decision defines a
    computation** — no field set, no ordering, no rendering rule. The shipped executor renders the
    ledger's own terminal facts — `(row_key, disposition, code, entity_id)` per row, sorted by
    `row_key` — through `domain::canonical`, the gear's single rendering rule, and takes its
    `content_digest`; it excludes the staged payload (a `no_op` row never applied it) and the
    timestamps (which differ between a run and its replay). **That covered set is the executor's
    choice, not a document's**, which is why `dod-coalesced-event` carries a bare marker rather
    than a tick. A consumer verifying the digest needs the set pinned somewhere it can read.
    **Blocks**: no DoD — **resolved by P-D-127** *(was: `cpt-cf-bss-products-dod-coalesced-event`)*
    **Owner**: this feature with `12-consumer-contracts` — the verifying side is the consumer's.
### Owed to other documents, recorded and deliberately not edited

- **`features/governance.md` §7 row 27 names this feature as a co-owner of a by-construction
  failure.** *"`PreAuthorized`'s predicate cannot admit the composite acts this feature declares…
  Both fail by construction"*, owner *"this feature with 04 and 09"*. Recorded as row 19 above; the
  register entry is `features/governance.md`'s.
- **`design/09` §2's `inst-pm-review` flags a live PRD conflict inline rather than in its §6**, so no
  carried row can hold it: `fr-catalog-version-diff` calls the version diff *"the reviewer's view for
  approvals"* while this feature's pre-approval view is the `ChangeReport`. Owner: the PRD owner with
  `06-catalog-version`.
- **Two sibling FEATUREs name an actor their slice's §1.3 does not.** Measured across all twelve:
  `features/catalog-version.md` adds `billing` and `features/reference-signal.md` adds `contracts`;
  the other ten match their slices exactly. Both are additions rather than drops and may be justified
  by those features' own flows — recorded as a measured divergence, not a repair. *(Found by
  censusing the class after this document's own actor table dropped its approver.)*
