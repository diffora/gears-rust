<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Rows, Windows & Dimension (Feature) -->
<!-- Related: ../DECOMPOSITION.md, ../DESIGN.md, ../PRD.md | Owners: BSS Pricing team -->

# Feature: Rows, Windows & Dimension

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-featstatus-rows-windows-dimension-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-pricing-feature-rows-windows-dimension`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Draft a temporary money change](#draft-a-temporary-money-change)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [normalize-and-select](#normalize-and-select)
  - [model-and-floor](#model-and-floor)
  - [reserve-write-confirm](#reserve-write-confirm)
- [4. States (CDSL)](#4-states-cdsl)
  - [Rows, Windows & Dimension states](#rows-windows--dimension-states)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Type-appropriate row models](#type-appropriate-row-models)
  - [Half-open tier arithmetic](#half-open-tier-arithmetic)
  - [Per-chain window normalization](#per-chain-window-normalization)
  - [Default fallback after value tails](#default-fallback-after-value-tails)
  - [Usage successor structure guard](#usage-successor-structure-guard)
  - [Minimum fee aggregation grain](#minimum-fee-aggregation-grain)
  - [Existing-chain temporary pairs](#existing-chain-temporary-pairs)
  - [Temporary value without its own chain](#temporary-value-without-its-own-chain)
  - [Draft and pending ownership guards](#draft-and-pending-ownership-guards)
  - [Reference reservation barrier](#reference-reservation-barrier)
  - [Durable confirmation and release recovery](#durable-confirmation-and-release-recovery)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

**Delivery:** phase 2c. Every checkbox is an implementation obligation, not an assertion about the legacy code.

This feature implements [slice 03](../design/03-rows-windows-dimension.md) — `cpt-cf-bss-pricing-design-slice-03`.
[DECOMPOSITION](../DECOMPOSITION.md) records integration order; [DESIGN §3](../DESIGN.md#3-technical-architecture)
is the schema and transaction authority. Unchecked phase 3/4 work is not part of the phase 2 core gate.

### 1.2 Purpose

Implement immutable money chains, models, temporary pairs and row floors; protect every price reference with durable reservation recovery.

Requirements: `cpt-cf-bss-pricing-fr-price-row`, `cpt-cf-bss-pricing-fr-chain-windows`, `cpt-cf-bss-pricing-fr-pair-guard`, `cpt-cf-bss-pricing-fr-min-fee`, `cpt-cf-bss-pricing-fr-temporary-pair`, `cpt-cf-bss-pricing-fr-reference-protocol`.

Architecture: `cpt-cf-bss-pricing-component-rows`, `cpt-cf-bss-pricing-component-reservations-client`, `cpt-cf-bss-pricing-principle-reserve-before-write`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-constraint-two-backends`, `cpt-cf-bss-pricing-constraint-no-row-locks`, `cpt-cf-bss-pricing-seq-reserve-write-confirm`, `cpt-cf-bss-pricing-seq-temporary-pair`.

### 1.3 Actors

`cpt-cf-bss-pricing-actor-finance-manager`, `cpt-cf-bss-pricing-actor-products`. Every operation authenticates and derives tenant scope before storage, replay or cross-gear calls.
Holding multiple permissions never bypasses separation of duties.

### 1.4 References

- [PRD](../PRD.md), especially the numbered acceptance criteria referenced below.
- [DESIGN](../DESIGN.md), §3 model, API contracts, transaction sequences and DDL.
- [Slice 03](../design/03-rows-windows-dimension.md), including API, data and event obligations.
- [DECISIONS](../DECISIONS.md), D-384–D-400; spec means `docs/superpowers/specs/2026-09-24-pricebook-model-design.md` in the main checkout.
- Source: spec §2 decisions 4–8, 13–17, §2.2, §5–§8, §10, §12–§13; the phase 2 plan supplies delivery boundaries and D-399/D-400.

## 2. Actor Flows (CDSL)

### Draft a temporary money change

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-flow-rows-windows-dimension`

1. [ ] - `p1` - Finance Manager chooses a price, dimension value, model inputs, effective_from and optional temporary_until. - `inst-rows-windows-dimension-flow-1`
2. [ ] - `p1` - Read chain history and validate model, dates, dimension membership and usage structure. - `inst-rows-windows-dimension-flow-2`
3. [ ] - `p1` - For an existing chain, copy the return money selected at the end into a linked row; for a previously unowned value create only one closed row. - `inst-rows-windows-dimension-flow-3`
4. [ ] - `p1` - Persist draft rows atomically with author attribution and return their ETags; later edits require draft/unlocked state. - `inst-rows-windows-dimension-flow-4`
5. [ ] - `p1` - Submit the complete pair or single row through slice 05; common-date shifts preserve temporary duration. - `inst-rows-windows-dimension-flow-5`

## 3. Processes / Business Logic (CDSL)

### normalize-and-select

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-rows-windows-dimension-normalize-and-select`

1. [ ] - `p1` - Group approved rows by price_id and nullable dim_value and sort by effective_from. - `inst-rows-windows-dimension-normalize-and-select-1`
2. [ ] - `p1` - Refuse same-start/overlap conflicts; set predecessor ends to successor starts independently per chain. - `inst-rows-windows-dimension-normalize-and-select-2`
3. [ ] - `p1` - Keep default tail open; retain an explicit value-tail end so selection can fall back afterward. - `inst-rows-windows-dimension-normalize-and-select-3`
4. [ ] - `p1` - At date/value select an in-force own row then default; report uncovered when neither applies. - `inst-rows-windows-dimension-normalize-and-select-4`

### model-and-floor

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-rows-windows-dimension-model-and-floor`

1. [ ] - `p1` - Derive permitted models from charge kind; validate nonnegative prices and coherent model parameters. - `inst-rows-windows-dimension-model-and-floor-1`
2. [ ] - `p1` - Evaluate per_unit, graduated, volume and package with decimal arithmetic and half-open tier bands; recurring/one_time allow flat or per_unit. - `inst-rows-windows-dimension-model-and-floor-2`
3. [ ] - `p1` - Preserve usage model kind, package size and SKU (unit, usage_type_ref) read as of each row's start across successors (D-402); CHAIN_MODEL_CHANGED fails submit and is rechecked at apply. - `inst-rows-windows-dimension-model-and-floor-3`
4. [ ] - `p1` - Aggregate rated amounts after included quantities by row/subscription/period across every bound value and slice; apply the prorated row floor, then promotions. - `inst-rows-windows-dimension-model-and-floor-4`

### reserve-write-confirm

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-rows-windows-dimension-reserve-write-confirm`

1. [ ] - `p1` - Replay first; Tx A claims the key, mints price_id and inserts a create_price op in reserving before reserve. - `inst-rows-windows-dimension-reserve-write-confirm-1`
2. [ ] - `p1` - Reserve idempotently per (owner, kind, ref_id), then re-read SKU type/lifecycle; a refusal moves the op to cancelling. - `inst-rows-windows-dimension-reserve-write-confirm-2`
3. [ ] - `p1` - Tx B commits the price, reservation_id, reference_state = confirmation_pending and op written together. - `inst-rows-windows-dimension-reserve-write-confirm-3`
4. [ ] - `p1` - Confirm after commit; Tx C sets price confirmed, op done and answers the key. Retry transient failure with bounded backoff; never release on timeout. - `inst-rows-windows-dimension-reserve-write-confirm-4`
5. [ ] - `p1` - On REFERENCE_RELEASED during confirm mark the price lost, audit and enqueue PriceReferenceLost. Reconcile confirmed prices through states(): re-reserve when not fenced, else mark lost and refuse new rows with PRICE_REFERENCE_LOST. - `inst-rows-windows-dimension-reserve-write-confirm-5`
6. [ ] - `p1` - Cancellation persists op cancelling before release; deletion removes the price and inserts delete_price op releasing in one transaction. Release finishes the op; every op not done survives restart and is never dropped. - `inst-rows-windows-dimension-reserve-write-confirm-6`

## 4. States (CDSL)

### Rows, Windows & Dimension states

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-state-rows-windows-dimension`

Rows move draft → pending → approved through the unit engine; pending ownership prohibits edits. Rejected proposals retain their review history and cannot become approved by direct PATCH; replacement proposals use drafts. Withdrawal unlocks the proposal for authoring. Price reference_state moves confirmation_pending → confirmed, or lost on proven release. Durable ops move reserving → written → done for creation, cancelling → done for refusal, or releasing → done for deletion; a rereserve_price op recovers a released receipt when the SKU is not fenced (D-401).

## 5. Definitions of Done

Every DoD below is required for this feature's delivery phase. Constraints: `cpt-cf-bss-pricing-constraint-two-backends`, `cpt-cf-bss-pricing-constraint-no-row-locks`.

### Type-appropriate row models

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-row-models`

Draft validation permits exactly the model families specified by charge kind. Approved money remains immutable while controlled chain metadata can normalize on successors (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-price-row`; PRD AC #4.

### Half-open tier arithmetic

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-tier-bands-half-open`

Tier selection uses [from, to), correcting the prototype volume edge. Surviving flat, per-unit, package and tier-boundary goldens execute against the money function (spec §10, D-387).

Requirement: `cpt-cf-bss-pricing-fr-price-row`; PRD AC #4.

### Per-chain window normalization

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-chain-windows`

Approval recomputes predecessor ends within one value chain under transactional revalidation. Same-start uniqueness and domain overlap/past-start validation protect both databases (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-chain-windows`; PRD AC #5.

### Default fallback after value tails

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-dimension-fallback`

Selection prefers the in-force value chain then default. A value chain may end explicitly; missing default is legal until coverage is required (spec §5, §14).

Requirement: `cpt-cf-bss-pricing-fr-chain-windows`; PRD AC #5.

### Usage successor structure guard

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-pair-guard`

Successors preserve usage model, package size and SKU unit. Submit and apply both enforce CHAIN_MODEL_CHANGED, and supersession-continuity goldens reach that validator (spec §2 decision 16).

Requirement: `cpt-cf-bss-pricing-fr-pair-guard`; PRD AC #6.

### Minimum fee aggregation grain

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-min-fee-row-period`

Minimum fee is one floor per row/subscription/period after included quantities and before promotions. Shared values and slices are aggregated, and covered-period fraction prorates the floor (spec §2 decision 13).

Requirement: `cpt-cf-bss-pricing-fr-min-fee`; PRD AC #7.

### Existing-chain temporary pairs

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-temporary-pair`

A temporary row and return are created, edited and submitted atomically. Return money comes from versionAt at the end, inherits dim_value and shifts with preserved duration (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-temporary-pair`; PRD AC #8.

### Temporary value without its own chain

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-temporary-value-fallback`

A previously unowned value receives one row ending at temporary_until. No return row copies the default, so later default changes remain visible after the temporary interval (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-temporary-pair`; PRD AC #8.

### Draft and pending ownership guards

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-row-pending-guard`

Draft PATCH/DELETE requires current version and no pending unit. Historical approved rows cannot be deleted; pending ownership is acquired conditionally by approval submission (spec §5–§6).

Requirement: `cpt-cf-bss-pricing-fr-price-row`; PRD AC #4.

### Reference reservation barrier

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-reference-protocol`

Price writes first persist a durable op, reserve, re-read SKU, locally commit receipt/work and confirm; cancellation/removal precedes release. A remote count or timeout-based release cannot substitute for the protocol (spec §13, D-398).

Requirement: `cpt-cf-bss-pricing-fr-reference-protocol`; PRD AC #11.

### Durable confirmation and release recovery

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-confirmation-retry`

Restart resumes every pricing_reference_op not done with bounded backoff, including reserving before any price exists; an unknown commit is reconciled before release. Confirmed prices are checked through states(): a released receipt is re-reserved when the SKU is not fenced, otherwise reference_state becomes lost, new rows fail PRICE_REFERENCE_LOST and PriceReferenceLost is emitted (D-401).

Requirement: `cpt-cf-bss-pricing-fr-reference-protocol`; PRD AC #11.

## 6. Acceptance Criteria

| DoD | PRD criterion | Given / When / Then |
| --- | --- | --- |
| `cpt-cf-bss-pricing-dod-row-models` | AC #4; `cpt-cf-bss-pricing-fr-price-row` | Given usage and recurring prices, when valid models are drafted then they persist; usage flat, negative prices and approved-money PATCH fail. |
| `cpt-cf-bss-pricing-dod-tier-bands-half-open` | AC #4; `cpt-cf-bss-pricing-fr-price-row` | Given a boundary of 1000, when quantity equals 1000 then volume selects the next band; 999 remains in the prior band. |
| `cpt-cf-bss-pricing-dod-chain-windows` | AC #5; `cpt-cf-bss-pricing-fr-chain-windows` | Given default and EU rows, when EU gains a successor then default stays unchanged; duplicate approved start, overlap and past start fail. |
| `cpt-cf-bss-pricing-dod-dimension-fallback` | AC #5; `cpt-cf-bss-pricing-fr-chain-windows` | Given a closed EU tail and an open default, when its end date arrives then default applies; if both are absent selection reports uncovered. |
| `cpt-cf-bss-pricing-dod-pair-guard` | AC #6; `cpt-cf-bss-pricing-fr-pair-guard` | Given a package predecessor, when only its amount changes then validation passes; a size, model, dated unit or meter change returns 400 CHAIN_MODEL_CHANGED. |
| `cpt-cf-bss-pricing-dod-min-fee-row-period` | AC #7; `cpt-cf-bss-pricing-fr-min-fee` | Given two 10 charges bound to one row with floor 30 then the result is 30; two distinct floor-30 rows yield 60, not 30 or 120. |
| `cpt-cf-bss-pricing-dod-temporary-pair` | AC #8; `cpt-cf-bss-pricing-fr-temporary-pair` | Given an existing EU chain, when a five-day temporary change shifts by three days then both boundaries shift and the return stays EU; partial pair submission fails. |
| `cpt-cf-bss-pricing-dod-temporary-value-fallback` | AC #8; `cpt-cf-bss-pricing-fr-temporary-pair` | Given only a default chain, when a temporary EU override ends then EU follows the current default; no paired return row exists. |
| `cpt-cf-bss-pricing-dod-row-pending-guard` | AC #4; `cpt-cf-bss-pricing-fr-price-row` | Given a pending row and its old ETag, when PATCH or DELETE runs then it is refused and unit content is unchanged; an unlocked current draft can be edited. |
| `cpt-cf-bss-pricing-dod-reference-protocol` | AC #11; `cpt-cf-bss-pricing-fr-reference-protocol` | Given Products and Pricing on real SQLite/Postgres storage, when reserve races retire then both cannot succeed; a live price blocks retire until durable removal and release. |
| `cpt-cf-bss-pricing-dod-confirmation-retry` | AC #11; `cpt-cf-bss-pricing-fr-reference-protocol` | Given a committed price and lost confirm response, when retry resumes then it confirms safely without release; a forced released receipt is surfaced and deletion release failure remains queued. |

Verification uses domain tests, scoped repository tests on both backends and REST positive/denial/precondition probes as applicable. Phase 2 checks must not mark later-phase behavior implemented. Golden consumer contracts belong to phase 4.
