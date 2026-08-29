<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./05-governance.md, ./06-catalog-version.md | Owners: BSS Product Catalog team -->

# DESIGN — Bulk Operations & Environment Promotion (Slice 9)

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
  - [Import a batch](#import-a-batch)
  - [Export](#export)
  - [Promote between environments](#promote-between-environments)
  - [Bulk lifecycle (p2)](#bulk-lifecycle-p2)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 Batch mechanics](#31-batch-mechanics)
  - [3.2 Error taxonomy (slice-owned codes)](#32-error-taxonomy-slice-owned-codes)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice owns catalog change **at scale**: bulk import (batch + per-row idempotency,
two-phase dependent rows, per-row reporting, draft landing, batch-gated publish against an
aggregated change report), deterministic **export**, **environment promotion** (AC #33a: the
export/import loop with stable-code identity), the p2 **bulk lifecycle tooling** (mass
deprecate/retire beyond the parent cascade), and the coalesced eventing that folds per-row bulk-progress noise into one summary event, row-level domain events still emitting (C6) — not a 10K-row
batch from becoming a 10K-event storm.

### 1.2 Purpose

Onboarding and migration at ≥ 10K SKUs/tenant cannot be row-by-row, and promotion between
environments must be **the same governed door**, not a side channel: everything lands in
`draft`, everything publishes through the 05 gate, and the report the approvers see is
aggregated but honest (counts, per-type summary, sample, lint findings).

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-catalog-admin` | Runs imports/exports/promotions and bulk lifecycle acts |
| `cpt-cf-bss-products-actor-finance-reviewer` | Approves batches (material via first publishes/lifecycle transitions at any size; the affected-entity trigger additionally catches large non-material edits — L6) |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.9 (`fr-bulk-import-export` incl. the promotion clause), §5.1
  (bulk rows), AC #33, #33a, the promotion use case; §17.1 (affected-entity trigger ≥ 10)
- [`../DECISIONS.md`](../DECISIONS.md) P-D-04 (the `(brandId, canonical name)` promotion
  fallback identity); [`./05-governance.md`](./05-governance.md) `inst-mt-inputs` (c);
  [`./06-catalog-version.md`](./06-catalog-version.md) `operation_key` (the bulk lane)

### 1.5 Scope

**In**:
- the import pipeline (parse → per-row validate → stage as drafts → aggregated report → batch approval → per-row publish)
- export
- promotion identity resolution
- bulk lifecycle ops (p2)
- batch/row idempotency
- the coalesced event
- the 06 `operation_key` wiring.

**Out**:
- row-level validation rules (each slice's registered validators — bulk runs the same pipeline per row, never a parallel one)
- the approval ceremony (05)
- the CatalogVersion increment itself (06 — this slice tags its requests with the `operation_key`; the bulk window closes on D-47's five-minute hard max, not on any close operation this slice issues — **P-D-46** struck `closed_at` for that reason).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Per-row idempotency + batch idempotency, two levels; per-row success/failure reported — no hidden partial failure; never a partially-inconsistent **published** state | PRD `fr-bulk-import-export` |
| C2 | Dependent rows: two-phase (stage-all-then-commit) or dependency-ordered — an orphan is never committed (a SKU row whose Product row failed, fails: `BULK_DEPENDENCY_FAILED`) | PRD AC #33/#38 |
| C3 | Bulk import lands **Product/SKU** entities in `draft`; live-entity rows have no draft state and stage as pending `GovernedLiveOp`s (`inst-bk-stage`); publication is gated on the **aggregated change report** through the 05 quorum (the affected-entity trigger makes any batch ≥ 10 material) | PRD `fr-bulk-import-export` |
| C4 | Export is deterministic for a given `catalogVersionId` (rendered from the 06 manifest, never from heads) | PRD AC #33 |
| C5 | Promotion identity: `skuCode` for SKUs; `productCode` else `(brandId, canonical internal name)` for Products — total under P-D-04; category = `(parent path, normalized name)`, set member = `(set kind, member code)`, attribute definition = `key` (`inst-bk-stage`); system ids re-minted by the target. **Four classifications, exhaustive** (the resolver's totality — `inst-pm-resolve` is the flow half of this row): unknown identity ⇒ **create**; identity bound to matching content ⇒ **no-op**; identity bound to **different** content ⇒ **update-as-draft** against the existing entity; identity bound to an incompatible kind/type, a `retired` holder, or a dirty head ⇒ **conflict**. AC #33a's "never a silent merge" is satisfied by the update landing in `draft` under the batch's own quorum — *silent* is what it forbids, and a staged draft in the `ChangeReport` is the opposite of silent. (Previously this row read "identity collision with different content = per-row conflict", which gave the modal promotion row the opposite disposition from the flow that implements it — item 15 of the review. That amendment stood against three unamended PRD statements for a wave; **P-D-17** now carries the classification normatively in `fr-bulk-import-export`, AC #33a and `usecase-environment-promotion`.) | PRD AC #33a, P-D-17 |
| C6 | One coalesced `CatalogBulkOperationCompleted` per batch summarizes the act (the PRD's no-storm clause); **row-level domain events (`SkuPublished`, `SkuDeprecated`, …) are still emitted** — the 08 projector depends on them, as does pricing's AC #82 adoption block **on its own arm — retirement or unpublishing** (a plain mass-deprecation has no pricing-side counterpart AC yet; slice 12's `ObligationRegister` carries the ask), and per-aggregate ordering keeps 10K of them consumable (H1 fix: the earlier blanket suppression broke both; the only thing coalesced away is per-row bulk-progress noise) | PRD `fr-bulk-import-export` |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `BulkBatch` | The unit: rows + batch key + state machine `staging → reported → approved → committing → completed/failed` |
| `RowLedger` | Per-row state: parsed/validated/staged/published/applied (a live-entity op)/no-op/failed(reason) — the no-hidden-partial-failure surface |
| `ChangeReport` | The aggregated approval artifact: counts, per-type summary, sample rows, lint findings, the itemised override-carrying rows (`skuCode` per row — `inst-bk-override`) — what the 05 quorum signs |
| `PromotionResolver` | Maps source rows to target identities per C5 and classifies each, exhaustively and in C5's own order: create / no-op / update-as-draft / conflict (four — the count §5's promotion matrix and C5 both carry) |

### 1.8 Context & Dependencies

**Consumed**: the 01 doors per row (create/save — the same validators, same codes); the 05
gate (batch approval; the report is the approval's stored snapshot); 06 (`operation_key` on
the batch's increment requests; export reads the manifest); 08 (report rendering reuses
projection lookups); 02 (the `GovernedLiveOp` envelope; `fr-prepublish-lint` conditions); 03
(`inst-cl-bundle-override`); 04 (the policy doors, bulk-lifecycle arm). **Produced**: `CatalogBulkOperationCompleted`; the `RowLedger` surface;
export artifacts.

## 2. Actor Flows (CDSL)

### Import a batch

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-import`

1. [ ] - `p1` - `POST /bss-products/v1/bulk/imports` (`bulk × execute`) with the batch key (idempotent: a replayed batch returns the existing `BulkBatch`); rows carry per-row keys, **batch-scoped** (a row reaching 01's publish door resolves that door's key as the reserved lane **`internal:bulk-row`** with the row's id as `client_key` — **P-D-26**; distinct from this slice's own ledger keys) (M1 fix: a row re-listed in a NEW batch is a new act — its stage validation decides its fate against the store, e.g. `DUPLICATE_CODE` — **P-D-25**, one code for both reservations; only a retry **within** the batch no-ops against the ledger) - `inst-bk-keys`
2. [ ] - `p1` - **Stage phase**: every Product/SKU row runs the ordinary per-row pipeline (parse → the same registered validators as interactive authoring — never a parallel rule set) and lands as `draft` via the 01 doors. **Live-entity rows — categories, attribute definitions, recognized-set members — have no draft state (H2/M5 fix)**: they validate at stage as a dry-run against the live tree/sets and are recorded in the ledger as **pending `GovernedLiveOp`s**, applied at commit under the batch approval (their promotion identities: category = `(parent path, normalized name)`, set member = `(set kind, member code)`, definition = `key`). Failures land in the `RowLedger` with their ordinary error codes; dependency order: categories/vocabularies → Products → SKUs, and a dependent row whose in-batch dependency failed fails `BULK_DEPENDENCY_FAILED` without touching the store (C2) - `inst-bk-stage`
3. [ ] - `p1` - The `ChangeReport` is generated from the ledger (counts, per-type summary, deterministic sample, the `fr-prepublish-lint` attention conditions (the report is 06's door; its conditions arise in 02/03 — L5; §6), and a **scope-values lint** naming region/brand values unseen in the target tenant's catalog — L7) and submitted to the 05 gate as one approval with subject kind **`bulk_batch`**, carrying the batch's affected-entity count; its materiality is 05's `MaterialityEvaluator`'s to decide (C3: first publishes and lifecycle transitions are material at any size; the affected-entity trigger catches large non-material edits), its stored snapshot the report + **the ledger's per-row pinned revisions** (H3 fix); a post-report edit to a member entity does NOT supersede the batch — that row fails its per-row pin at commit, row-locally (the batch state machine therefore needs no supersession edge — L2 resolved by the same decision) - `inst-bk-report`
4. [ ] - `p1` - **Commit phase** (on quorum): the batch approval is **consumed once, by the `approved → committing` flip** (the 05 composite-act model, extended to batch acts in 05's own enumeration — H3); rows then publish per-row through the 01 `PublishDoor` **in `PreAuthorized(approvalId)` mode naming the batch's consumed record** (01 `inst-fd-gate-mode-preauthorized`, which names a bulk row as one of its callers), each pinned to its **ledger revision** — an edited row fails `STALE_REVISION` alone in the ledger, dependents of a failed row fail `BULK_DEPENDENCY_FAILED` wrapping the underlying code, commit preserves the stage ordering (categories/vocabularies → Products → SKUs — L1), siblings never block, and the published state is never partially-inconsistent because each row's publish is atomic and independent; live-entity ops apply as their `GovernedLiveOp`s under the same consumed approval, each re-validating its pinned expected target state at apply and failing `STALE_LIVE_OP` alone in the ledger (02 `inst-gl-envelope`), row-locally, exactly as an edited entity row fails `STALE_REVISION`; the batch's 06 increment requests carry the **`operation_key`**, and ledger completion **closes the operation via the same request door** (a `close` marker on `(source, operation_key)` — M6) so the whole batch lands in ONE CatalogVersion without waiting the 5-minute hard max - `inst-bk-commit`
5. [ ] - `p1` - **Override conditions survive the lane (Blocking 6 fix)**: a batch is one composite act whose approval is consumed at the `approved → committing` flip and whose per-row publishes do **not** re-enter the 05 gate (`inst-gv-one-shot`), so any row carrying an override condition — today only an uncomposed `bundle` (03 `inst-cl-bundle-override`, the ceremony **P-D-02** moved from `CatalogVersion` publish to the bundle's entity publish) — would otherwise publish with no ceremony recorded anywhere. Therefore: the **stage phase** detects override-carrying rows and names them in the `ChangeReport` as a distinct, itemised section (`skuCode` per row, never a count); the batch approval is an **`OverrideCeremony`** whose acknowledgment-by-name is over **that itemised set**, stored on the batch's `ApprovalRecord` (subject kind `bulk_batch`) exactly as an entity-publish override is stored on its own (05 `inst-gv-override`); and a row whose override condition **appeared after the report** (composition state changed under it) fails `BULK_OVERRIDE_UNACKNOWLEDGED` alone in the ledger rather than publishing unacknowledged — the same row-local, fail-closed posture as the per-row revision pin. A batch with no such row carries no ceremony and is unchanged - `inst-bk-override`
6. [ ] - `p1` - Completion emits ONE `CatalogBulkOperationCompleted` (C6) with the ledger digest; the ledger remains queryable - `inst-bk-complete`

### Export

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-export`

1. [ ] - `p1` - `GET /bss-products/v1/bulk/exports?catalogVersionId=` (`catalog_version × read` — export renders a 06 manifest and is auditor-shaped; decoupled from the import grant, M8): rendered from the 06 manifest (entity halves from frozen versions, capture halves from the capture store) — deterministic byte-for-byte for a given version (C4); the artifact header carries a schema format version (versioned in `products-sdk` under 12 `inst-rc-compat`), and the format carries the stable codes and canonical names (the promotion identities) plus full content - `inst-bk-export`

### Promote between environments

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-promote`

1. [ ] - `p1` - Promotion IS import (never a governance bypass — the PRD's own sentence): the target imports a source export; `PromotionResolver` classifies each row by C5 identity — unknown identity ⇒ create (ids re-minted); identity bound to matching content ⇒ no-op; identity bound to **different** content ⇒ per-row **update-as-draft** against the existing entity (M3: a same-identity content difference is the promotion's purpose, so only incompatibilities conflict — **P-D-17**, which amended the FR, AC #33a and the §10 use case to match; C5 alone had been amended in the earlier wave, leaving three PRD statements saying the opposite); identity bound to an incompatible kind/type, to a **retired** holder (revival is clone-only — resolver totality, M3), to a head **carrying unpublished local edits or an open approval** (`PROMOTION_DIRTY_HEAD` — M7, symmetric with 07's rule: an import never silently merges into in-flight work or supersedes a local approval) ⇒ `PROMOTION_IDENTITY_CONFLICT`/`PROMOTION_DIRTY_HEAD` per row - `inst-pm-resolve`
2. [ ] - `p1` - The reviewer's **pre-approval** view is the `ChangeReport` (staged content vs the target's current heads — the only diff producible before anything publishes); the AC #20a catalog-version diff is the **post-commit verification** view (previous vs new target version). `fr-catalog-version-diff` calls the version diff "the reviewer's view for approvals", which is the sentence that still conflicts — flagged (M4); the §10 use case already agrees with this slice; the substance it wants (what will change) is the report - `inst-pm-review`

### Bulk lifecycle (p2)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-bulk-lifecycle`

1. [ ] - `p2` - Mass deprecate / mass retire-initiate (`POST /bss-products/v1/bulk/lifecycle`, **`bulk_lifecycle × execute`** — its own grant: the gear's most destructive batch act never rides the import pair, M8) over a filter or id list: each row runs the ordinary 04 policy doors (provenance `direct`, per-row confirmation data aggregated into one report), the batch material by its lifecycle transitions at **any** size (05 `inst-gv-materiality`), the affected-entity trigger additionally catching large batches; one batch approval, consumed once by the same `approved → committing` flip, each row's 04 transition door running in 01's `PreAuthorized(approvalId)` mode naming that record (05 `inst-gv-one-shot`); the retire arm schedules per-row transitions — the flip guards stay per-SKU (no bulk override of the D-47 guard exists) - `inst-bl-lifecycle`

## 3. Processes / Business Logic

### 3.1 Batch mechanics

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-batch`

1. [ ] - `p1` - `products_bulk_batch` + `products_bulk_row` (the `RowLedger`): batch state machine per §1.7; rows immutable after their terminal state (append-only evidence); the ledger is the idempotency store for row keys (distinct from 01's endpoint store — row keys are batch-scoped) - `inst-bm-tables`
2. [ ] - `p1` - A batch is **resumable**: a crash mid-commit resumes from the ledger (per-row publishes idempotent by row key). **Abandon (M2)**: created-draft rows discard through the ordinary 01 door; **update-as-draft rows revert** via the ordinary save door with the last frozen version's content as payload (revision++, audit reason `batch-abandoned` — no new door, and the head returns to its published content); pending live-entity ops are simply dropped (never applied). **This slice writes no operator free-text `reason`** (**P-D-50**): `batch-abandoned` is a literal constant, the batch ceremony's reason lives on 05's `ApprovalRecord` and the mass-retire reason on 04's `inst-rt-initiate`, so 02's `inst-av-pii-reason` no longer enumerates this slice - `inst-bm-resume`
3. [ ] - `p1` - Size bounds: configured max rows/batch and max concurrent batches per tenant (`BULK_LIMIT`); the 10K-SKU onboarding case is the sizing fixture - `inst-bm-limits`

### 3.2 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-bulk-errors`

`BULK_DEPENDENCY_FAILED` (the AC #38 row — a **per-row ledger outcome**, not a response of the batch door: the surface answers 202 and the row carries this, so the status below applies only where a caller asks a single row's disposition), `PROMOTION_IDENTITY_CONFLICT`,
**`PROMOTION_DIRTY_HEAD`** (raised by `inst-pm-resolve`; no other slice owns it, and slice 12
builds the SDK error enum from every slice's registered codes — item 33 of the review), **`BULK_OVERRIDE_UNACKNOWLEDGED`** (`inst-bk-override`), `BULK_LIMIT`.
Row-level failures otherwise reuse the owning slices' codes verbatim inside the ledger — bulk
introduces no parallel taxonomy.

**Problem responses (RFC 9457):** `PROMOTION_IDENTITY_CONFLICT`, `PROMOTION_DIRTY_HEAD` (409 — per-row ledger outcomes like `BULK_DEPENDENCY_FAILED`, the status applying only where a caller asks a single row's disposition); `BULK_DEPENDENCY_FAILED`, `BULK_OVERRIDE_UNACKNOWLEDGED`, `BULK_LIMIT` (422 architectural — each reaches the wire as 400; see the note below).

*Statuses added, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"* — the citation was right the first time;
a pass re-pointed it at D-186 and was wrong to, D-186 being a later amendment scoped to
one config route) and where an earlier pass here wrongly wrote
412 and called that pricing's convention — **403** where the caller may not perform the act at
all, **404** only where a path segment names a resource this tenant has none of. **503** where retry
is the remedy is this gear's own addition — pricing's set carries no 503 at all, so that one
class is not "checked against it". **The 422s here are architectural, not wire** — see 01 §3.3, which quotes the sibling
plan-price gear's rule (the `MUST NOT` being this gear's own choice, 01 §3.3): no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for an error **carrying a registry code** in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.*

## 4. Data / Storage (normative shape; DDL in migrations)

§3.1's two tables (tenant-scoped; ledger append-only after terminal states); export artifacts
are streamed, not stored (determinism makes storage redundant); events per C6.

## 5. Testing posture (slice-local)

- The 10K-row fixture: stage + report + commit within the sizing envelope; ONE
  `CatalogBulkOperationCompleted`; ONE CatalogVersion (the `operation_key` probe — 06's M3
  from the other side).
- Dependency probe: Product row fails ⇒ its SKU rows fail `BULK_DEPENDENCY_FAILED`, siblings
  proceed; no orphan at any point (positive + negative in one batch).
- Override probe: a row whose override condition appeared after the report fails
  `BULK_OVERRIDE_UNACKNOWLEDGED` alone at commit; the acknowledged itemised set publishes under the
  one batch ceremony (`inst-bk-override`).
- Determinism probe: two exports of the same `catalogVersionId` are byte-identical (C4).
- Idempotency: batch replay returns the batch; row replay inside a retry commits nothing twice
  (ledger-keyed); crash-resume mid-commit completes without duplicates.
- Promotion matrix: create / no-op / update-as-draft / conflict — one fixture over C5's four
  classifications, incl. the codeless Product resolved by `(brandId, canonical name)`.
- Per-row pin probe (H3 semantics): a row edited after the report fails `STALE_REVISION` alone
  at commit — the batch approval stands, siblings publish, the ledger names the row.
- Bulk-lifecycle probe: mass retire schedules per-row; a referenced row's flip defers under the
  ordinary guard — the batch never force-retires.

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-usecase-bulk-operations`, `cpt-cf-bss-products-usecase-environment-promotion` (§10 use cases, claimed by id here — all seven were in lint 1's universe and none was claimed); `cpt-cf-bss-products-fr-bulk-import-export` (whole), the promotion clause + AC #33/#33a +
the promotion use case + `usecase-bulk-operations` (L8); §5.1 "Bulk export & bulk lifecycle
tooling" (p2 half — priorities reconciled: export determinism and promotion are MUSTs of the
p1 FR, so their flows build at p1 while faceted bulk-lifecycle tooling stays p2, L3); AC #38
row "a bulk row whose in-batch dependency failed"; the §17.1 affected-entity trigger (consumed
via 05).

**Risks & open items**:
- **C6 after the H1 fix is barely a deviation**: domain events all emit; only per-row
  bulk-progress noise is folded into the coalesced summary — recorded so slice 12's
  completeness check reads the summary event as additive, not as suppression.
- **Update-as-draft promotion onto a live entity** stages bucket-iii/iv changes on the
  target's head — bucket-i/ii differences (type, meter) cannot be promoted onto an existing
  identity and land as per-row conflicts directing to the 07 correction door; these rows classify as update-as-draft and fail at the save door with 01's
  `ILLEGAL_FIELD_MUTATION`, whose reason names 07's correction door; worth its own probe when built.
- **Export format versioning** (schema evolution of the artifact) rides slice 12's vN→vN+1
  discipline; named here so the exporter carries a format version from day one.
- **A renamed Product carrying no `productCode` is promoted as a create, not an update.**
  `normalized(name)` is both bucket-iii in 01 §4.1 (a published Product is renameable) and this
  slice's C5 fallback promotion identity where `productCode` is absent — and `product_code` is
  nullable, so a rename between promotions makes the target resolve *unknown identity* and create a
  second Product, which C5's four-way classification exists to prevent. Owner: this slice. *(Filed from 01 §6 by the slice-01 eighth lens pass — the pointer claimed it was registered here and it was not.)*
- **The batch state machine has no rejection edge and no abandon state.** `reported` has one stated
  exit while the approval it waits on can be `rejected`; nothing states what enters `failed` (every
  row failure is explicitly row-local, "siblings never block"); and `inst-bm-resume`'s abandon path
  has no state to write. A rejected batch sits in `reported` forever holding its staged drafts and a
  slot against the concurrency ceiling. Owner: this slice, with 05 if abandonment releases the
  approval. *(Raised by the slice-09 first lens pass.)*
- **What ends a batch that is never approved?** No timeout, expiry or reaper is stated, and the
  approval it waits on has no deadline either — so an abandoned-but-unabandoned batch consumes a
  tenant slot permanently. Owner: this slice with 05. *(Raised by the slice-09 first lens pass.)*
- **What door, actor or signal starts the commit phase?** "On quorum" names no executor: this slice
  declares three routes and none is a commit, it names no runner, and 05's decide door is itself
  unowned (05 §6). The answer fixes which transaction writes `committing` and therefore where the
  one-shot consumption is enforced. Owner: this slice with 05. *(Raised by the slice-09 first lens pass.)*
- **What surface reports the `RowLedger`?** Four per-row codes carry HTTP statuses that apply "where
  a caller asks a single row's disposition", and no route, verb or grant exists for that ask — the
  RBAC catalog mints only the two execute pairs, 08 projects no bulk read model, and §4 adds no read
  surface. C1's "per-row success/failure reported" has no reader. Owner: this slice with the contract
  owner. *(Raised by the slice-09 first lens pass.)*
- **§4 declares two table names and no columns.** The heading promises a normative shape and every
  sibling supplies one. Values with a stated writer and no column: the per-row pinned revision, the
  batch and row keys, the row disposition and reason, the pending `GovernedLiveOp` payload, the
  itemised override set, and the `operation_key`. Owner: this slice's storage owner with 05's.
  *(Raised by the slice-09 first lens pass.)*
- **Does a `bulk_batch` approval authorize a `GovernedLiveOp` apply?** This slice says the live-entity
  ops apply "under the same consumed approval", while 05's live-op gate asks its question with the op
  **envelope** as subject, and 05's composite-act enumeration names per-row publishes and nothing
  else. 05's approval carries one subject and a partial unique over it. Owner: the governance owner
  with 05 and 02 — extend the composite-act enumeration, or give each live-entity row its own record.
  *(Raised by the slice-09 first lens pass.)*
- **Does the per-row pin fit 05's store?** This slice states the approval's stored snapshot as the
  report **plus the ledger's per-row pinned revisions**, and 05 declares one scalar
  `internal_revision` per record — registering the mismatch itself as "What do the entity-shaped
  columns hold for the non-entity subject kinds?". Owner: 05's owner with 12. *(Raised by the slice-09 first lens pass.)*
- **Where does the `ChangeReport`'s diff read its two operands?** The pre-approval view is "staged
  content vs the target's current heads", this slice books 08 as the input, and 08's C6 projects
  entity content **only** from frozen version rows, never from heads — and serves no draft. Neither
  operand has a stated producer. Owner: this slice with 08. *(Raised by the slice-09 first lens pass.)*
- **Which slice builds the lint producer?** The `ChangeReport` is a PRD MUST that must carry lint
  findings; 06 §6 records that no instruction, store, RBAC pair, error code or probe in that slice
  delivers the report and names this slice as co-owner of the gap. Owner: the design-set owner with
  06 and this slice. *(Two lenses raised it independently.)*
- **What makes the export byte-deterministic?** C4 promises byte-for-byte determinism for a given
  version; 06 §6 records that the manifest it renders from has no named sort key for its row
  collections, and `inst-bk-export` states no canonical serialization for the artifact itself. Owner:
  P-D-29's owner with 06 and this slice. *(Raised by the slice-09 first lens pass.)*
- **Does every import run the `PromotionResolver`, and what selects promotion mode?** The same door,
  the same stage phase and the identical case — a `skuCode` already bound with different content —
  gets `DUPLICATE_CODE` under `inst-bk-keys` and update-as-draft under `inst-pm-resolve`. No request
  field, header or route segment distinguishes an import from a promotion. Owner: this slice with the
  contract owner. *(Raised by the slice-09 first lens pass.)*
- **What is "update-as-draft" for a row kind with no draft state?** C5 calls its four classifications
  exhaustive and this pass added the live-entity promotion identities to it, so a promoted category
  whose content differs falls into a classification it cannot occupy. Owner: this slice with 02.
  *(Raised by the slice-09 first lens pass.)*
- **Is the scope-values lint blocking or advisory, and what does "unseen" mean?** It appears once,
  carries no code, no threshold and no probe, and nothing says whether it stops the report, the
  approval or nothing. Owner: this slice. *(Raised by the slice-09 first lens pass.)*
- **No instruction in this slice names its event or records "no event".** 01 states the rule over
  every slice and 12 lints it; only `inst-bk-complete` names one, while six further state-changing
  instructions declare nothing. Owner: this slice. *(Raised by the slice-09 first lens pass.)*
