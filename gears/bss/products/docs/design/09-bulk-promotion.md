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
deprecate/retire beyond the parent cascade), and the coalesced eventing that keeps a 10K-row
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

**In**: the import pipeline (parse → per-row validate → stage as drafts → aggregated report →
batch approval → per-row publish), export, promotion identity resolution, bulk lifecycle ops
(p2), batch/row idempotency, the coalesced event, the 06 `operation_key` wiring.

**Out**: row-level validation rules (each slice's registered validators — bulk runs the same
pipeline per row, never a parallel one); the approval ceremony (05); the CatalogVersion
increment itself (06 — this slice only tags its requests with the `operation_key`).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Per-row idempotency + batch idempotency, two levels; per-row success/failure reported — no hidden partial failure; never a partially-inconsistent **published** state | PRD `fr-bulk-import-export` |
| C2 | Dependent rows: two-phase (stage-all-then-commit) or dependency-ordered — an orphan is never committed (a SKU row whose Product row failed, fails: `BULK_DEPENDENCY_FAILED`) | PRD AC #33/#38 |
| C3 | Bulk import lands entities in `draft`; publication is gated on the **aggregated change report** through the 05 quorum (the affected-entity trigger makes any batch ≥ 10 material) | PRD `fr-bulk-import-export` |
| C4 | Export is deterministic for a given `catalogVersionId` (rendered from the 06 manifest, never from heads) | PRD AC #33 |
| C5 | Promotion identity: `skuCode` for SKUs; `productCode` else `(brandId, canonical internal name)` for Products — total under P-D-04; system ids re-minted by the target. **Four classifications, exhaustive** (the resolver's totality — `inst-pm-resolve` is the flow half of this row): unknown identity ⇒ **create**; identity bound to matching content ⇒ **no-op**; identity bound to **different** content ⇒ **update-as-draft** against the existing entity; identity bound to an incompatible kind/type, a `retired` holder, or a dirty head ⇒ **conflict**. AC #33a's "never a silent merge" is satisfied by the update landing in `draft` under the batch's own quorum — *silent* is what it forbids, and a staged draft in the `ChangeReport` is the opposite of silent. (Previously this row read "identity collision with different content = per-row conflict", which gave the modal promotion row the opposite disposition from the flow that implements it — item 15 of the 2026-08-26 review. That amendment stood against three unamended PRD statements for a wave; **P-D-17** now carries the classification normatively in `fr-bulk-import-export`, AC #33a and `usecase-environment-promotion`.) | PRD AC #33a, P-D-17 |
| C6 | One coalesced `CatalogBulkOperationCompleted` per batch summarizes the act (the PRD's no-storm clause); **row-level domain events (`SkuPublished`, `SkuDeprecated`, …) are still emitted** — the 08 projector depends on them, as does pricing's AC #82 adoption block **on its own arm — retirement or unpublishing** (a plain mass-deprecation has no pricing-side counterpart AC yet; slice 12's `ObligationRegister` carries the ask), and per-aggregate ordering keeps 10K of them consumable (H1 fix: the earlier blanket suppression broke both; the only thing coalesced away is per-row bulk-progress noise) | PRD `fr-bulk-import-export` |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `BulkBatch` | The unit: rows + batch key + state machine `staging → reported → approved → committing → completed/failed` |
| `RowLedger` | Per-row state: parsed/validated/staged/published/failed(reason) — the no-hidden-partial-failure surface |
| `ChangeReport` | The aggregated approval artifact: counts, per-type summary, sample rows, lint findings — what the 05 quorum signs |
| `PromotionResolver` | Maps source rows to target identities per C5 and classifies each, exhaustively and in C5's own order: create / no-op / update-as-draft / conflict (four — the count §5's promotion matrix and C5 both carry) |

### 1.8 Context & Dependencies

**Consumed**: the 01 doors per row (create/save — the same validators, same codes); the 05
gate (batch approval; the report is the approval's stored snapshot); 06 (`operation_key` on
the batch's increment requests; export reads the manifest); 08 (report rendering reuses
projection lookups). **Produced**: `CatalogBulkOperationCompleted`; the `RowLedger` surface;
export artifacts.

## 2. Actor Flows (CDSL)

### Import a batch

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-import`

1. [ ] - `p1` - `POST /bss-products/v1/bulk/imports` (`bulk × execute`) with the batch key (idempotent: a replayed batch returns the existing `BulkBatch`); rows carry per-row keys, **batch-scoped** (M1 fix: a row re-listed in a NEW batch is a new act — its stage validation decides its fate against the store, e.g. `DUPLICATE_SKU_CODE`; only a retry **within** the batch no-ops against the ledger) - `inst-bk-keys`
2. [ ] - `p1` - **Stage phase**: every Product/SKU row runs the ordinary per-row pipeline (parse → the same registered validators as interactive authoring — never a parallel rule set) and lands as `draft` via the 01 doors. **Live-entity rows — categories, attribute definitions, recognized-set members — have no draft state (H2/M5 fix)**: they validate at stage as a dry-run against the live tree/sets and are recorded in the ledger as **pending `GovernedLiveOp`s**, applied at commit under the batch approval (their promotion identities: category = `(parent path, normalized name)`, set member = `(set kind, member code)`, definition = `key`). Failures land in the `RowLedger` with their ordinary error codes; dependency order: categories/vocabularies → Products → SKUs, and a dependent row whose in-batch dependency failed fails `BULK_DEPENDENCY_FAILED` without touching the store (C2) - `inst-bk-stage`
3. [ ] - `p1` - The `ChangeReport` is generated from the ledger (counts, per-type summary, deterministic sample, the `fr-prepublish-lint` attention conditions (02/03 — L5), and a **scope-values lint** naming region/brand values unseen in the target tenant's catalog — L7) and submitted to the 05 gate as one material approval with subject kind **`bulk_batch`**, its stored snapshot the report + **the ledger's per-row pinned revisions** (H3 fix); a post-report edit to a member entity does NOT supersede the batch — that row fails its per-row pin at commit, row-locally (the batch state machine therefore needs no supersession edge — L2 resolved by the same decision) - `inst-bk-report`
4. [ ] - `p1` - **Commit phase** (on quorum): the batch approval is **consumed once, by the `approved → committing` flip** (the 05 composite-act model, extended to batch acts in 05's own enumeration — H3); rows then publish per-row through the 01 `PublishDoor`, each pinned to its **ledger revision** — an edited row fails `STALE_REVISION` alone in the ledger, dependents of a failed row fail `BULK_DEPENDENCY_FAILED` wrapping the underlying code, commit preserves the stage ordering (categories/vocabularies → Products → SKUs — L1), siblings never block, and the published state is never partially-inconsistent because each row's publish is atomic and independent; live-entity ops apply as their `GovernedLiveOp`s under the same consumed approval; the batch's 06 increment requests carry the **`operation_key`**, and ledger completion **closes the operation via the same request door** (a `close` marker on `(source, operation_key)` — M6) so the whole batch lands in ONE CatalogVersion without waiting the 5-minute hard max - `inst-bk-commit`
5. [ ] - `p1` - **Override conditions survive the lane (Blocking 6 fix, 2026-08-26 review)**: a batch is one composite act whose approval is consumed at the `approved → committing` flip and whose per-row publishes do **not** re-enter the 05 gate (`inst-gv-one-shot`), so any row carrying an override condition — today only an uncomposed `bundle` (03 `inst-cl-bundle-override`, the ceremony **P-D-02** moved from `CatalogVersion` publish to the bundle's entity publish) — would otherwise publish with no ceremony recorded anywhere. Therefore: the **stage phase** detects override-carrying rows and names them in the `ChangeReport` as a distinct, itemised section (`skuCode` per row, never a count); the batch approval is an **`OverrideCeremony`** whose acknowledgment-by-name is over **that itemised set**, stored on the `bulk_batch` record exactly as an entity-publish override is stored on its own (05 `inst-gv-override`); and a row whose override condition **appeared after the report** (composition state changed under it) fails `BULK_OVERRIDE_UNACKNOWLEDGED` alone in the ledger rather than publishing unacknowledged — the same row-local, fail-closed posture as the per-row revision pin. A batch with no such row carries no ceremony and is unchanged - `inst-bk-override`
6. [ ] - `p1` - Completion emits ONE `CatalogBulkOperationCompleted` (C6) with the ledger digest; the ledger remains queryable - `inst-bk-complete`

### Export

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-export`

1. [ ] - `p1` - `GET /bss-products/v1/bulk/exports?catalogVersionId=` (`catalog_version × read` — export renders a 06 manifest and is auditor-shaped; decoupled from the import grant, M8): rendered from the 06 manifest (entity halves from frozen versions, capture halves from the capture store) — deterministic byte-for-byte for a given version (C4); the format carries the stable codes and canonical names (the promotion identities) plus full content - `inst-bk-export`

### Promote between environments

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-promote`

1. [ ] - `p1` - Promotion IS import (never a governance bypass — the PRD's own sentence): the target imports a source export; `PromotionResolver` classifies each row by C5 identity — unknown identity ⇒ create (ids re-minted); identity bound to matching content ⇒ no-op; identity bound to **different** content ⇒ per-row **update-as-draft** against the existing entity (M3: a same-identity content difference is the promotion's purpose, so only incompatibilities conflict — **P-D-17**, which amended the FR, AC #33a and the §10 use case to match; C5 alone had been amended in the earlier wave, leaving three PRD statements saying the opposite); identity bound to an incompatible kind/type, to a **retired** holder (revival is clone-only — resolver totality, M3), to a head **carrying unpublished local edits or an open approval** (`PROMOTION_DIRTY_HEAD` — M7, symmetric with 07's rule: an import never silently merges into in-flight work or supersedes a local approval) ⇒ `PROMOTION_IDENTITY_CONFLICT`/`PROMOTION_DIRTY_HEAD` per row - `inst-pm-resolve`
2. [ ] - `p1` - The reviewer's **pre-approval** view is the `ChangeReport` (staged content vs the target's current heads — the only diff producible before anything publishes); the AC #20a catalog-version diff is the **post-commit verification** view (previous vs new target version). The PRD use case stages the version diff pre-approval, which is temporally impossible — flagged (M4); the substance it wants (what will change) is the report - `inst-pm-review`

### Bulk lifecycle (p2)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-bulk-lifecycle`

1. [ ] - `p2` - Mass deprecate / mass retire-initiate (`POST /bss-products/v1/bulk/lifecycle`, **`bulk_lifecycle × execute`** — its own grant: the gear's most destructive batch act never rides the import pair, M8) over a filter or id list: each row runs the ordinary 04 policy doors (provenance `direct`, per-row confirmation data aggregated into one report); one batch approval (the affected-entity trigger); the retire arm schedules per-row transitions — the flip guards stay per-SKU (no bulk override of the D-47 guard exists) - `inst-bl-lifecycle`

## 3. Processes / Business Logic

### 3.1 Batch mechanics

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-batch`

1. [ ] - `p1` - `products_bulk_batch` + `products_bulk_row` (the `RowLedger`): batch state machine per §1.7; rows immutable after their terminal state (append-only evidence); the ledger is the idempotency store for row keys (distinct from 01's endpoint store — row keys are batch-scoped) - `inst-bm-tables`
2. [ ] - `p1` - A batch is **resumable**: a crash mid-commit resumes from the ledger (per-row publishes idempotent by row key). **Abandon (M2)**: created-draft rows discard through the ordinary 01 door; **update-as-draft rows revert** via the ordinary save door with the last frozen version's content as payload (revision++, audit reason `batch-abandoned` — no new door, and the head returns to its published content); pending live-entity ops are simply dropped (never applied) - `inst-bm-resume`
3. [ ] - `p2` - Size bounds: configured max rows/batch and max concurrent batches per tenant (`BULK_LIMIT`); the 10K-SKU onboarding case is the sizing fixture - `inst-bm-limits`

### 3.2 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-bulk-errors`

`BULK_DEPENDENCY_FAILED` (the AC #38 row), `PROMOTION_IDENTITY_CONFLICT`,
**`PROMOTION_DIRTY_HEAD`** (raised by `inst-pm-resolve`; no other slice owns it, and slice 12
builds the SDK error enum from every slice's registered codes — item 33 of the 2026-08-26
review), **`BULK_OVERRIDE_UNACKNOWLEDGED`** (`inst-bk-override`), `BULK_LIMIT`.
Row-level failures otherwise reuse the owning slices' codes verbatim inside the ledger — bulk
introduces no parallel taxonomy.

## 4. Data / Storage (normative shape; DDL in migrations)

§3.1's two tables (tenant-scoped; ledger append-only after terminal states); export artifacts
are streamed, not stored (determinism makes storage redundant); events per C6.

## 5. Testing posture (slice-local)

- The 10K-row fixture: stage + report + commit within the sizing envelope; ONE
  `CatalogBulkOperationCompleted`; ONE CatalogVersion (the `operation_key` probe — 06's M3
  from the other side).
- Dependency probe: Product row fails ⇒ its SKU rows fail `BULK_DEPENDENCY_FAILED`, siblings
  proceed; no orphan at any point (positive + negative in one batch).
- Idempotency: batch replay returns the batch; row replay inside a retry commits nothing twice
  (ledger-keyed); crash-resume mid-commit completes without duplicates.
- Promotion matrix: create / no-op / update-as-draft / conflict — one fixture over C5's four
  classifications, incl. the codeless Product resolved by `(brandId, canonical name)`.
- Per-row pin probe (H3 semantics): a row edited after the report fails `STALE_REVISION` alone
  at commit — the batch approval stands, siblings publish, the ledger names the row.
- Bulk-lifecycle probe: mass retire schedules per-row; a referenced row's flip defers under the
  ordinary guard — the batch never force-retires.

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-fr-bulk-import-export` (whole), the promotion clause + AC #33/#33a +
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
  identity and land as per-row conflicts directing to the 07 correction door; stated in
  `inst-pm-resolve`'s classification, worth its own probe when built.
- **Export format versioning** (schema evolution of the artifact) rides slice 12's vN→vN+1
  discipline; named here so the exporter carries a format version from day one.
