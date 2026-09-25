<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Rows, Windows & Dimension (Design, Slice 3) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../DECISIONS.md | Owners: BSS Pricing team -->

# DESIGN — Rows, Windows & Dimension (Slice 3)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-design-slice-03`

<!-- toc -->

- [1. Context](#1-context)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Draft a temporary money change](#draft-a-temporary-money-change)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [normalize-and-select](#normalize-and-select)
  - [model-and-floor](#model-and-floor)
  - [reserve-write-confirm](#reserve-write-confirm)
- [4. States (CDSL)](#4-states-cdsl)
- [5. API Surface](#5-api-surface)
- [6. Data Model](#6-data-model)
- [7. Events & Alarms](#7-events--alarms)
- [8. Definitions of Done](#8-definitions-of-done)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Non-Functional Considerations](#10-non-functional-considerations)

<!-- /toc -->

## 1. Context

**Delivery:** phase 2c. Every checkbox is an implementation obligation, not an assertion about the legacy code.

Implement immutable money chains, models, temporary pairs and row floors; protect every price reference with durable reservation recovery.

Requirements: `cpt-cf-bss-pricing-fr-price-row`, `cpt-cf-bss-pricing-fr-chain-windows`, `cpt-cf-bss-pricing-fr-pair-guard`, `cpt-cf-bss-pricing-fr-min-fee`, `cpt-cf-bss-pricing-fr-temporary-pair`, `cpt-cf-bss-pricing-fr-reference-protocol`. Architecture: `cpt-cf-bss-pricing-component-rows`, `cpt-cf-bss-pricing-component-reservations-client`, `cpt-cf-bss-pricing-principle-reserve-before-write`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-constraint-two-backends`, `cpt-cf-bss-pricing-constraint-no-row-locks`, `cpt-cf-bss-pricing-seq-reserve-write-confirm`, `cpt-cf-bss-pricing-seq-temporary-pair`.
[FEATURE](../features/rows-windows-dimension.md) owns the executable flow/algorithm/DoD identifiers; this slice defines no duplicate DoDs.
Dependencies: `cpt-cf-bss-pricing-feature-books-prices`.
Source: PriceBook spec §2.2, §5–§8, §12–§13 and [DECISIONS](../DECISIONS.md) D-384–D-400.

## 2. Actor Flows (CDSL)

### Draft a temporary money change

Actors: `cpt-cf-bss-pricing-actor-finance-manager`, `cpt-cf-bss-pricing-actor-products`. Feature flow: `cpt-cf-bss-pricing-flow-rows-windows-dimension`.

1. [ ] - `p1` - Finance Manager chooses a price, dimension value, model inputs, effective_from and optional temporary_until. - `inst-rows-windows-dimension-flow-1`
2. [ ] - `p1` - Read chain history and validate model, dates, dimension membership and usage structure. - `inst-rows-windows-dimension-flow-2`
3. [ ] - `p1` - For an existing chain, copy the return money selected at the end into a linked row; for a previously unowned value create only one closed row. - `inst-rows-windows-dimension-flow-3`
4. [ ] - `p1` - Persist draft rows atomically with author attribution and return their ETags; later edits require draft/unlocked state. - `inst-rows-windows-dimension-flow-4`
5. [ ] - `p1` - Submit the complete pair or single row through slice 05; common-date shifts preserve temporary duration. - `inst-rows-windows-dimension-flow-5`

## 3. Processes / Business Logic (CDSL)

### normalize-and-select

Feature algorithm: `cpt-cf-bss-pricing-algo-rows-windows-dimension-normalize-and-select`.

1. [ ] - `p1` - Group approved rows by price_id and nullable dim_value and sort by effective_from. - `inst-rows-windows-dimension-normalize-and-select-1`
2. [ ] - `p1` - Refuse same-start/overlap conflicts; set predecessor ends to successor starts independently per chain. - `inst-rows-windows-dimension-normalize-and-select-2`
3. [ ] - `p1` - Keep default tail open; retain an explicit value-tail end so selection can fall back afterward. - `inst-rows-windows-dimension-normalize-and-select-3`
4. [ ] - `p1` - At date/value select an in-force own row then default; report uncovered when neither applies. - `inst-rows-windows-dimension-normalize-and-select-4`

### model-and-floor

Feature algorithm: `cpt-cf-bss-pricing-algo-rows-windows-dimension-model-and-floor`.

1. [ ] - `p1` - Derive permitted models from charge kind; validate nonnegative prices and coherent model parameters. - `inst-rows-windows-dimension-model-and-floor-1`
2. [ ] - `p1` - Evaluate per_unit, graduated, volume and package with decimal arithmetic and half-open tier bands; recurring/one_time allow flat or per_unit. - `inst-rows-windows-dimension-model-and-floor-2`
3. [ ] - `p1` - Preserve usage model kind, package size and SKU (unit, usage_type_ref) read as of each row's start across successors, a start before the SKU's first version reading that first version (D-402); CHAIN_MODEL_CHANGED fails submit and is rechecked at apply. - `inst-rows-windows-dimension-model-and-floor-3`
4. [ ] - `p1` - Aggregate rated amounts after included quantities by row/subscription/period across every bound value and slice; apply the prorated row floor, then promotions. - `inst-rows-windows-dimension-model-and-floor-4`

### reserve-write-confirm

Feature algorithm: `cpt-cf-bss-pricing-algo-rows-windows-dimension-reserve-write-confirm`.

1. [ ] - `p1` - Replay first; Tx A claims the key, mints price_id and inserts a create_price op in reserving before reserve. - `inst-rows-windows-dimension-reserve-write-confirm-1`
2. [ ] - `p1` - Reserve idempotently per (owner, kind, ref_id), then re-read SKU type/lifecycle; a refusal moves the op to cancelling. - `inst-rows-windows-dimension-reserve-write-confirm-2`
3. [ ] - `p1` - Tx B commits the price, reservation_id, reference_state = confirmation_pending and op written together. - `inst-rows-windows-dimension-reserve-write-confirm-3`
4. [ ] - `p1` - Confirm after commit; Tx C sets price confirmed, op done and answers the key. Retry transient failure with bounded backoff; never release on timeout. - `inst-rows-windows-dimension-reserve-write-confirm-4`
5. [ ] - `p1` - On REFERENCE_RELEASED during confirm keep the price confirmation_pending and start a rereserve_price op. Reconcile confirmed prices through states(): re-reserve when not fenced, else mark lost, audit, enqueue PriceReferenceLost and refuse new rows with PRICE_REFERENCE_LOST; re-reserve lost prices once their SKU admits a reservation. - `inst-rows-windows-dimension-reserve-write-confirm-5`
6. [ ] - `p1` - Cancellation persists op cancelling before release; deletion removes the price and inserts delete_price op releasing in one transaction. Release finishes the op; every op not done survives restart and is never dropped. - `inst-rows-windows-dimension-reserve-write-confirm-6`

## 4. States (CDSL)

Rows move draft → pending → approved through the unit engine; pending ownership prohibits edits. Rejected proposals retain their review history and cannot become approved by direct PATCH; replacement proposals use drafts. Withdrawal unlocks the proposal for authoring. Price reference_state moves confirmation_pending → confirmed, or lost on proven release. Durable ops move reserving → written → done for creation, cancelling → done for refusal, or releasing → done for deletion; a rereserve_price op recovers a released receipt when the SKU is not fenced (D-401).

State definition: `cpt-cf-bss-pricing-state-rows-windows-dimension` in the FEATURE.

## 5. API Surface

POST /bss-pricing/v1/prices/{id}/rows; PATCH/DELETE /bss-pricing/v1/rows/{id} draft only. Submit and publish-changes enter slice 05. Price create/delete use slice 02 doors but this slice owns their reference protocol. Products calls are reserve, SKU read, confirm and release through ProductsClient.

[DESIGN §3.3](../DESIGN.md#33-api-contracts) fixes canonical errors and route prefixes.
Each mounted route must appear in all four censuses with authz and precondition expectations.

## 6. Data Model

pricing_price_row holds version_no, nullable dim_value, model/price_json, min_fee, eligibility, dates, keep_for_bound, pair links, state, author and approval ownership. Approved chain indexes are per price/value/start. pricing_price holds reservation_id and reference_state; pricing_reference_op holds durable work before reserve, retry metadata and cancellation/deletion work without a price FK. closed_explicitly preserves a temporary row's end through normalization. No row-level frozen descriptors or remote reference count exists.

Tenant-scoped parent validation is required even where foreign keys use entity ids. Never substitute a
cross-gear read for transactional local ownership/version guards. Approved money and historical pins survive.

## 7. Events & Alarms

PriceRowsPublished is emitted only by successful price_rows apply, together with ApprovalUnitDecided. Proven lost receipts emit PriceReferenceLost with tenant, price, SKU and reservation identity. Pending confirmation/release depth and last failures remain observable; retry is bounded and restart-safe.

Audit and outbox inserts use the same mutation transaction; retry is lifecycle-managed and observes shutdown.

## 8. Definitions of Done

The sole definitions live in [features/rows-windows-dimension.md](../features/rows-windows-dimension.md):

- `cpt-cf-bss-pricing-dod-row-models` — Type-appropriate row models.
- `cpt-cf-bss-pricing-dod-tier-bands-half-open` — Half-open tier arithmetic.
- `cpt-cf-bss-pricing-dod-chain-windows` — Per-chain window normalization.
- `cpt-cf-bss-pricing-dod-dimension-fallback` — Default fallback after value tails.
- `cpt-cf-bss-pricing-dod-pair-guard` — Usage successor structure guard.
- `cpt-cf-bss-pricing-dod-min-fee-row-period` — Minimum fee aggregation grain.
- `cpt-cf-bss-pricing-dod-temporary-pair` — Existing-chain temporary pairs.
- `cpt-cf-bss-pricing-dod-temporary-value-fallback` — Temporary value without its own chain.
- `cpt-cf-bss-pricing-dod-row-pending-guard` — Draft and pending ownership guards.
- `cpt-cf-bss-pricing-dod-reference-protocol` — Reference reservation barrier.
- `cpt-cf-bss-pricing-dod-confirmation-retry` — Durable confirmation and release recovery.

## 9. Acceptance Criteria

1. PRD AC #4 / `cpt-cf-bss-pricing-dod-row-models`: Given usage and recurring prices, when valid models are drafted then they persist; usage flat, negative prices and approved-money PATCH fail.
2. PRD AC #4 / `cpt-cf-bss-pricing-dod-tier-bands-half-open`: Given a boundary of 1000, when quantity equals 1000 then volume selects the next band; 999 remains in the prior band.
3. PRD AC #5 / `cpt-cf-bss-pricing-dod-chain-windows`: Given default and EU rows, when EU gains a successor then default stays unchanged; duplicate approved start, overlap and past start fail.
4. PRD AC #5 / `cpt-cf-bss-pricing-dod-dimension-fallback`: Given a closed EU tail and an open default, when its end date arrives then default applies; if both are absent selection reports uncovered.
5. PRD AC #6 / `cpt-cf-bss-pricing-dod-pair-guard`: Given a package predecessor, when only its amount changes then validation passes; a size, model, dated unit or meter change returns 400 CHAIN_MODEL_CHANGED.
6. PRD AC #7 / `cpt-cf-bss-pricing-dod-min-fee-row-period`: Given two 10 charges bound to one row with floor 30 then the result is 30; two distinct floor-30 rows yield 60, not 30 or 120.
7. PRD AC #8 / `cpt-cf-bss-pricing-dod-temporary-pair`: Given an existing EU chain, when a five-day temporary change shifts by three days then both boundaries shift and the return stays EU; partial pair submission fails.
8. PRD AC #8 / `cpt-cf-bss-pricing-dod-temporary-value-fallback`: Given only a default chain, when a temporary EU override ends then EU follows the current default; no paired return row exists.
9. PRD AC #4 / `cpt-cf-bss-pricing-dod-row-pending-guard`: Given a pending row and its old ETag, when PATCH or DELETE runs then it is refused and unit content is unchanged; an unlocked current draft can be edited.
10. PRD AC #11 / `cpt-cf-bss-pricing-dod-reference-protocol`: Given Products and Pricing on real SQLite/Postgres storage, when reserve races retire then both cannot succeed; a live price blocks retire until durable removal and release.
11. PRD AC #11 / `cpt-cf-bss-pricing-dod-confirmation-retry`: Given a committed price and lost confirm response, when retry resumes then it confirms safely without release; a forced released receipt is surfaced and deletion release failure remains queued.

## 10. Non-Functional Considerations

All paths enforce tenant isolation, deny-by-default authz and append-only attribution. Mutations use conditional
versions, required POST replay and PATCH/PUT preconditions; PostgreSQL serializable chain changes and SQLite
writer serialization preserve the same invariants. A failed audit/outbox write cannot leave a committed act.
No timeout releases a live reference. Review and apply retain typed database failures for bounded retry.
Implementation gates cover both backends and route censuses; document gates cover toc, language and identifier ownership.
