<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Books & Entries (Feature) -->
<!-- Related: ../DECOMPOSITION.md, ../DESIGN.md, ../PRD.md | Owners: BSS Pricing team -->

# Feature: Books & Entries

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-featstatus-books-entries-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-pricing-feature-books-entries`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Author a book and entry](#author-a-book-and-entry)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [book-and-key](#book-and-key)
  - [dimension-registry](#dimension-registry)
  - [settings-and-export](#settings-and-export)
- [4. States (CDSL)](#4-states-cdsl)
  - [Books & Entries states](#books--entries-states)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Currency books and validity](#currency-books-and-validity)
  - [One entry per book key](#one-entry-per-book-key)
  - [Restricted entry metadata edits](#restricted-entry-metadata-edits)
  - [Tenant dimension registry](#tenant-dimension-registry)
  - [Versioned billing defaults](#versioned-billing-defaults)
  - [Read-only book export](#read-only-book-export)
  - [Creation uses the reference service](#creation-uses-the-reference-service)
- [6. Acceptance Criteria](#6-acceptance-criteria)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

**Delivery:** phase 2c. Every checkbox is an implementation obligation, not an assertion about the legacy code.

This feature implements [slice 02](../design/02-books-entries.md) — `cpt-cf-bss-pricing-design-slice-02`.
[DECOMPOSITION](../DECOMPOSITION.md) records integration order; [DESIGN §3](../DESIGN.md#3-technical-architecture)
is the schema and transaction authority. Unchecked phase 3/4 work is not part of the phase 2 core gate.

### 1.2 Purpose

Author currency books and unique SKU entries, maintain dimensions/settings and export book facts; hand entry creation/removal to the reservation service.

Requirements: `cpt-cf-bss-pricing-fr-dimension-registry`, `cpt-cf-bss-pricing-fr-price-book`, `cpt-cf-bss-pricing-fr-entry-key`, `cpt-cf-bss-pricing-fr-book-export`, `cpt-cf-bss-pricing-fr-settings`.

Architecture: `cpt-cf-bss-pricing-component-books`, `cpt-cf-bss-pricing-component-reservations-client`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-principle-reserve-before-write`, `cpt-cf-bss-pricing-constraint-one-replay-store`, `cpt-cf-bss-pricing-seq-reserve-write-confirm`.

### 1.3 Actors

`cpt-cf-bss-pricing-actor-finance-manager`, `cpt-cf-bss-pricing-actor-products`, `cpt-cf-bss-pricing-actor-auditor`. Every operation authenticates and derives tenant scope before storage, replay or cross-gear calls.
Holding multiple permissions never bypasses separation of duties.

### 1.4 References

- [PRD](../PRD.md), especially the numbered acceptance criteria referenced below.
- [DESIGN](../DESIGN.md), §3 model, API contracts, transaction sequences and DDL.
- [Slice 02](../design/02-books-entries.md), including API, data and event obligations.
- [DECISIONS](../DECISIONS.md), D-384–D-425; spec means `docs/superpowers/specs/2026-09-24-pricebook-model-design.md` in the main checkout.
- Source: spec §2 decisions 4–8, 13–17, §2.2, §5–§8, §10, §12–§13; the phase 2 plan supplies delivery boundaries and D-399/D-400.

## 2. Actor Flows (CDSL)

### Author a book and entry

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-flow-books-entries`

1. [ ] - `p1` - Finance Manager creates a uniquely coded currency book and reads its ETag. - `inst-books-entries-flow-1`
2. [ ] - `p1` - Select a published non-bundle SKU, recurring period if applicable and optional registered dimension key. - `inst-books-entries-flow-2`
3. [ ] - `p1` - Resolve replay and pass the stable entry identity to the reserve-write-confirm protocol in slice 03. - `inst-books-entries-flow-3`
4. [ ] - `p1` - After reservation, re-read SKU type/lifecycle, derive charge kind, enforce key uniqueness and persist through that protocol. - `inst-books-entries-flow-4`
5. [ ] - `p1` - Return book/entry facts; later money is drafted as prices, not embedded in the key. - `inst-books-entries-flow-5`

## 3. Processes / Business Logic (CDSL)

### book-and-key

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-books-entries-book-and-key`

1. [ ] - `p1` - Validate currency and nonempty validity interval; scope code uniqueness to tenant. - `inst-books-entries-book-and-key-1`
2. [ ] - `p1` - Derive charge_kind from the current SKU; recurring accepts month/year, usage and one_time require null period. - `inst-books-entries-book-and-key-2`
3. [ ] - `p1` - Enforce the book/SKU/kind/coalesced-period unique index and map races to a conflict. - `inst-books-entries-book-and-key-3`
4. [ ] - `p1` - PATCH name/validity or permitted entry overrides conditionally; reject currency edits and dimension changes after valued prices exist. - `inst-books-entries-book-and-key-4`

### dimension-registry

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-books-entries-dimension-registry`

1. [ ] - `p1` - Validate key syntax and distinct values (none yet, or at least two; exactly one is DIM_VALUES_FEW); a tenant with no stored registry reads the seed region with no values, and the first entry naming region stores it in that entry's transaction. - `inst-books-entries-dimension-registry-1`
2. [ ] - `p1` - Reject unknown price values or values without a declared entry dimension. - `inst-books-entries-dimension-registry-2`
3. [ ] - `p1` - Before removing a value, atomically check all tenant prices referencing it, including historical prices. - `inst-books-entries-dimension-registry-3`
4. [ ] - `p1` - Apply the direct versioned registry edit without an approval unit. - `inst-books-entries-dimension-registry-4`

### settings-and-export

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-books-entries-settings-and-export`

1. [ ] - `p1` - Authorize settings separately from book authoring and enforce If-Match on settings changes. - `inst-books-entries-settings-and-export-1`
2. [ ] - `p1` - Resolve defaults by SKU billing-timing override then tenant timing; preserve rounding and per-type template inputs for binding. - `inst-books-entries-settings-and-export-2`
3. [ ] - `p1` - For export, authorize read and select only the scoped book, entries and prices; preserve price identities and chain windows. - `inst-books-entries-settings-and-export-3`
4. [ ] - `p1` - Return JSON without modifying state, submitting units or evaluating totals. - `inst-books-entries-settings-and-export-4`

## 4. States (CDSL)

### Books & Entries states

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-state-books-entries`

A book is valid or invalid for a queried date according to its optional interval; no approval state is added. An entry progresses through reference confirmation in slice 03. Currency and key identity are fixed; editable metadata uses versions. Registry/settings changes are direct and versioned.

## 5. Definitions of Done

Every DoD below is required for this feature's delivery phase. Constraints: `cpt-cf-bss-pricing-constraint-one-replay-store`.

### Currency books and validity

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-book-currency-validity`

Books have tenant-unique codes, one immutable currency and optional nonempty validity. Versioned metadata edits preserve money identity (spec §5, D-384).

Requirement: `cpt-cf-bss-pricing-fr-price-book`; PRD AC #2.

### One entry per book key

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-entry-key-unique`

The database enforces SKU × charge kind × normalized period uniqueness inside a book. Charge kind follows the re-read SKU; a bundle or invalid period is rejected (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-entry-key`; PRD AC #3.

### Restricted entry metadata edits

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-entry-metadata`

invoice_line_override remains editable under If-Match. dimension_key changes only before any valued price exists; approved or pending prices prevent entry deletion with ENTRY_PRICES_IN_USE and another author's draft with 403 NOT_DRAFT_AUTHOR (D-404), while the caller's draft and rejected prices are deleted with the entry (spec §7.2, phase 2c.6).

Requirement: `cpt-cf-bss-pricing-fr-entry-key`; PRD AC #3.

### Tenant dimension registry

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-dimension-registry`

Keys and values are tenant-owned and validated with the four DIM errors. The registry starts seeded with region, declared with no values; the first entry naming it stores the seed. A referenced value cannot be removed even when its price is historical, and registry edits require no approval (spec §5, §14).

Requirement: `cpt-cf-bss-pricing-fr-dimension-registry`; PRD AC #1.

### Versioned billing defaults

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-settings-defaults`

Tenant timing, rounding, GL, tax and per-type invoice templates are stored and exposed. SKU timing overrides tenant timing at binding; settings permissions and optimistic versioning apply (spec §5).

Requirement: `cpt-cf-bss-pricing-fr-settings`; PRD AC #13.

### Read-only book export

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-book-export`

Export returns the book and all its scoped entries/prices as JSON with ids, windows, dimensions and model inputs. It creates no approval or mutation (spec §2 decision 16).

Requirement: `cpt-cf-bss-pricing-fr-book-export`; PRD AC #12.

### Creation uses the reference service

- [x] `p1` - **ID**: `cpt-cf-bss-pricing-dod-entry-reference-handoff`

Every entry create and delete invokes slice 03 reference protocol rather than writing directly. Validation failures and duplicate-key races leave no unprotected object (spec §13).

Requirement: `cpt-cf-bss-pricing-fr-entry-key`; PRD AC #3.

## 6. Acceptance Criteria

| DoD | PRD criterion | Given / When / Then |
| --- | --- | --- |
| `cpt-cf-bss-pricing-dod-book-currency-validity` | AC #2; `cpt-cf-bss-pricing-fr-price-book` | Given a EUR book, when name/validity changes with its ETag then currency stays EUR; duplicate tenant code or inverted dates are refused. |
| `cpt-cf-bss-pricing-dod-entry-key-unique` | AC #3; `cpt-cf-bss-pricing-fr-entry-key` | Given the same nonrecurring SKU twice, when concurrent creates use null period then only one entry persists; a bundle has no entry. |
| `cpt-cf-bss-pricing-dod-entry-metadata` | AC #3; `cpt-cf-bss-pricing-fr-entry-key` | Given a valued price, when a dimension change is requested then it is refused DIMENSION_KEY_IN_USE; given an approved or pending price, entry deletion is refused ENTRY_PRICES_IN_USE, and NOT_DRAFT_AUTHOR (D-404) while another author's draft exists; the caller's drafts and all rejected prices are deleted with the entry; allowed metadata updates retain receipt identity. |
| `cpt-cf-bss-pricing-dod-dimension-registry` | AC #1; `cpt-cf-bss-pricing-fr-dimension-registry` | Given EU prices, when US is added then it is available; deleting EU or drafting UNKNOWN is refused without changing the registry. |
| `cpt-cf-bss-pricing-dod-settings-defaults` | AC #13; `cpt-cf-bss-pricing-fr-settings` | Given default arrears and SKU advance, when inputs bind then advance wins; stale settings update fails with no partial changes. |
| `cpt-cf-bss-pricing-dod-book-export` | AC #12; `cpt-cf-bss-pricing-fr-book-export` | Given two tenants, when one exports its book then only its facts appear; foreign-book access is denied and price/unit counts do not change. |
| `cpt-cf-bss-pricing-dod-entry-reference-handoff` | AC #3; `cpt-cf-bss-pricing-fr-entry-key` | Given Products unavailable before reserve, when an entry is created then REGISTRY_UNAVAILABLE leaves no entry; a successful create retains its live receipt. |

Verification uses domain tests, scoped repository tests on both backends and REST positive/denial/precondition probes as applicable. Phase 2 checks must not mark later-phase behavior implemented. Golden consumer contracts belong to phase 4.
