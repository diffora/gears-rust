<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Books & Prices (Feature) -->
<!-- Related: ../DECOMPOSITION.md, ../DESIGN.md, ../PRD.md | Owners: BSS Pricing team -->

# Feature: Books & Prices

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-featstatus-books-prices-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-pricing-feature-books-prices`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Author a book and price](#author-a-book-and-price)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [book-and-key](#book-and-key)
  - [dimension-registry](#dimension-registry)
  - [settings-and-export](#settings-and-export)
- [4. States (CDSL)](#4-states-cdsl)
  - [Books & Prices states](#books--prices-states)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Currency books and validity](#currency-books-and-validity)
  - [One price per book key](#one-price-per-book-key)
  - [Restricted price metadata edits](#restricted-price-metadata-edits)
  - [Tenant dimension registry](#tenant-dimension-registry)
  - [Versioned billing defaults](#versioned-billing-defaults)
  - [Read-only book export](#read-only-book-export)
  - [Creation uses the reference service](#creation-uses-the-reference-service)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

**Delivery:** phase 2c. Every checkbox is an implementation obligation, not an assertion about the legacy code.

This feature implements [slice 02](../design/02-books-prices.md) — `cpt-cf-bss-pricing-design-slice-02`.
[DECOMPOSITION](../DECOMPOSITION.md) records integration order; [DESIGN §3](../DESIGN.md#3-technical-architecture)
is the schema and transaction authority. Unchecked phase 3/4 work is not part of the phase 2 core gate.

### 1.2 Purpose

Author currency books and unique SKU prices, maintain dimensions/settings and export book facts; hand price creation/removal to the reservation service.

Requirements: `cpt-cf-bss-pricing-fr-dimension-registry`, `cpt-cf-bss-pricing-fr-price-book`, `cpt-cf-bss-pricing-fr-price-key`, `cpt-cf-bss-pricing-fr-book-export`, `cpt-cf-bss-pricing-fr-settings`.

Architecture: `cpt-cf-bss-pricing-component-books`, `cpt-cf-bss-pricing-component-reservations-client`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-principle-reserve-before-write`, `cpt-cf-bss-pricing-constraint-one-replay-store`, `cpt-cf-bss-pricing-seq-reserve-write-confirm`.

### 1.3 Actors

`cpt-cf-bss-pricing-actor-finance-manager`, `cpt-cf-bss-pricing-actor-products`, `cpt-cf-bss-pricing-actor-auditor`. Every operation authenticates and derives tenant scope before storage, replay or cross-gear calls.
Holding multiple permissions never bypasses separation of duties.

### 1.4 References

- [PRD](../PRD.md), especially the numbered acceptance criteria referenced below.
- [DESIGN](../DESIGN.md), §3 model, API contracts, transaction sequences and DDL.
- [Slice 02](../design/02-books-prices.md), including API, data and event obligations.
- [DECISIONS](../DECISIONS.md), D-384–D-400; spec means `docs/superpowers/specs/2026-09-24-pricebook-model-design.md` in the main checkout.
- Source: spec §2 decisions 4–8, 13–17, §2.2, §5–§8, §10, §12–§13; the phase 2 plan supplies delivery boundaries and D-399/D-400.

## 2. Actor Flows (CDSL)

### Author a book and price

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-flow-books-prices`

1. [ ] - `p1` - Finance Manager creates a uniquely coded currency book and reads its ETag. - `inst-books-prices-flow-1`
2. [ ] - `p1` - Select a published non-bundle SKU, recurring period if applicable and optional registered dimension key. - `inst-books-prices-flow-2`
3. [ ] - `p1` - Resolve replay and pass the stable price identity to the reserve-write-confirm protocol in slice 03. - `inst-books-prices-flow-3`
4. [ ] - `p1` - After reservation, re-read SKU type/lifecycle, derive charge kind, enforce key uniqueness and persist through that protocol. - `inst-books-prices-flow-4`
5. [ ] - `p1` - Return book/price facts; later money is drafted as rows, not embedded in the key. - `inst-books-prices-flow-5`

## 3. Processes / Business Logic (CDSL)

### book-and-key

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-books-prices-book-and-key`

1. [ ] - `p1` - Validate currency and nonempty validity interval; scope code uniqueness to tenant. - `inst-books-prices-book-and-key-1`
2. [ ] - `p1` - Derive charge_kind from the current SKU; recurring accepts month/year, usage and one_time require null period. - `inst-books-prices-book-and-key-2`
3. [ ] - `p1` - Enforce the book/SKU/kind/coalesced-period unique index and map races to a conflict. - `inst-books-prices-book-and-key-3`
4. [ ] - `p1` - PATCH name/validity or permitted price overrides conditionally; reject currency edits and dimension changes after valued rows exist. - `inst-books-prices-book-and-key-4`

### dimension-registry

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-books-prices-dimension-registry`

1. [ ] - `p1` - Validate key syntax and sufficient distinct values; seed region for the tenant. - `inst-books-prices-dimension-registry-1`
2. [ ] - `p1` - Reject unknown row values or values without a declared price dimension. - `inst-books-prices-dimension-registry-2`
3. [ ] - `p1` - Before removing a value, atomically check all tenant rows referencing it, including historical rows. - `inst-books-prices-dimension-registry-3`
4. [ ] - `p1` - Apply the direct versioned registry edit without an approval unit. - `inst-books-prices-dimension-registry-4`

### settings-and-export

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-books-prices-settings-and-export`

1. [ ] - `p1` - Authorize settings separately from book authoring and enforce If-Match on settings changes. - `inst-books-prices-settings-and-export-1`
2. [ ] - `p1` - Resolve defaults by SKU billing-timing override then tenant timing; preserve rounding and per-type template inputs for binding. - `inst-books-prices-settings-and-export-2`
3. [ ] - `p1` - For export, authorize read and select only the scoped book, prices and rows; preserve row identities and chain windows. - `inst-books-prices-settings-and-export-3`
4. [ ] - `p1` - Return JSON without modifying state, submitting units or evaluating totals. - `inst-books-prices-settings-and-export-4`

## 4. States (CDSL)

### Books & Prices states

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-state-books-prices`

A book is valid or invalid for a queried date according to its optional interval; no approval state is added. A price progresses through reference confirmation in slice 03. Currency and key identity are fixed; editable metadata uses versions. Registry/settings changes are direct and versioned.

## 5. Definitions of Done

Every DoD below is required for this feature's delivery phase. Constraints: `cpt-cf-bss-pricing-constraint-one-replay-store`.

### Currency books and validity

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-book-currency-validity`

Books have tenant-unique codes, one immutable currency and optional nonempty validity. Versioned metadata edits preserve money identity (spec §5, D-384).

Requirement: `cpt-cf-bss-pricing-fr-price-book`; PRD AC #2.

### One price per book key

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-price-key-unique`

The database enforces SKU × charge kind × normalized period uniqueness inside a book. Charge kind follows the re-read SKU; a bundle or invalid period is rejected (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-price-key`; PRD AC #3.

### Restricted price metadata edits

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-price-metadata`

invoice_line_override remains editable under If-Match. dimension_key changes only before any valued row exists; approved rows also prevent price deletion (spec §7.2, phase 2c.6).

Requirement: `cpt-cf-bss-pricing-fr-price-key`; PRD AC #3.

### Tenant dimension registry

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-dimension-registry`

Keys and values are tenant-owned and validated with the four DIM errors. A referenced value cannot be removed even when its row is historical, and registry edits require no approval (spec §5, §14).

Requirement: `cpt-cf-bss-pricing-fr-dimension-registry`; PRD AC #1.

### Versioned billing defaults

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-settings-defaults`

Tenant timing, rounding, GL, tax and per-type invoice templates are stored and exposed. SKU timing overrides tenant timing at binding; settings permissions and optimistic versioning apply (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-settings`; PRD AC #13.

### Read-only book export

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-book-export`

Export returns the book and all its scoped prices/rows as JSON with ids, windows, dimensions and model inputs. It creates no approval or mutation (spec §2 decision 16).

Requirement: `cpt-cf-bss-pricing-fr-book-export`; PRD AC #12.

### Creation uses the reference service

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-price-reference-handoff`

Every price create and delete invokes slice 03 reference protocol rather than writing directly. Validation failures and duplicate-key races leave no unprotected object (spec §13).

Requirement: `cpt-cf-bss-pricing-fr-price-key`; PRD AC #3.

## 6. Acceptance Criteria

| DoD | PRD criterion | Given / When / Then |
| --- | --- | --- |
| `cpt-cf-bss-pricing-dod-book-currency-validity` | AC #2; `cpt-cf-bss-pricing-fr-price-book` | Given a EUR book, when name/validity changes with its ETag then currency stays EUR; duplicate tenant code or inverted dates are refused. |
| `cpt-cf-bss-pricing-dod-price-key-unique` | AC #3; `cpt-cf-bss-pricing-fr-price-key` | Given the same nonrecurring SKU twice, when concurrent creates use null period then only one price persists; a bundle has no price. |
| `cpt-cf-bss-pricing-dod-price-metadata` | AC #3; `cpt-cf-bss-pricing-fr-price-key` | Given a valued or approved row, when dimension change or price deletion is requested then it is refused; allowed metadata updates retain receipt identity. |
| `cpt-cf-bss-pricing-dod-dimension-registry` | AC #1; `cpt-cf-bss-pricing-fr-dimension-registry` | Given EU rows, when US is added then it is available; deleting EU or drafting UNKNOWN is refused without changing the registry. |
| `cpt-cf-bss-pricing-dod-settings-defaults` | AC #13; `cpt-cf-bss-pricing-fr-settings` | Given default arrears and SKU advance, when inputs bind then advance wins; stale settings update fails with no partial changes. |
| `cpt-cf-bss-pricing-dod-book-export` | AC #12; `cpt-cf-bss-pricing-fr-book-export` | Given two tenants, when one exports its book then only its facts appear; foreign-book access is denied and row/unit counts do not change. |
| `cpt-cf-bss-pricing-dod-price-reference-handoff` | AC #3; `cpt-cf-bss-pricing-fr-price-key` | Given Products unavailable before reserve, when a price is created then REGISTRY_UNAVAILABLE leaves no price; a successful create retains its live receipt. |

Verification uses domain tests, scoped repository tests on both backends and REST positive/denial/precondition probes as applicable. Phase 2 checks must not mark later-phase behavior implemented. Golden consumer contracts belong to phase 4.
