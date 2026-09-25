<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Books & Prices (Design, Slice 2) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../DECISIONS.md | Owners: BSS Pricing team -->

# DESIGN — Books & Prices (Slice 2)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-design-slice-02`

<!-- toc -->

- [1. Context](#1-context)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Author a book and price](#author-a-book-and-price)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [book-and-key](#book-and-key)
  - [dimension-registry](#dimension-registry)
  - [settings-and-export](#settings-and-export)
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

Author currency books and unique SKU prices, maintain dimensions/settings and export book facts; hand price creation/removal to the reservation service.

Requirements: `cpt-cf-bss-pricing-fr-dimension-registry`, `cpt-cf-bss-pricing-fr-price-book`, `cpt-cf-bss-pricing-fr-price-key`, `cpt-cf-bss-pricing-fr-book-export`, `cpt-cf-bss-pricing-fr-settings`. Architecture: `cpt-cf-bss-pricing-component-books`, `cpt-cf-bss-pricing-component-reservations-client`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-principle-reserve-before-write`, `cpt-cf-bss-pricing-constraint-one-replay-store`, `cpt-cf-bss-pricing-seq-reserve-write-confirm`.
[FEATURE](../features/books-prices.md) owns the executable flow/algorithm/DoD identifiers; this slice defines no duplicate DoDs.
Dependencies: `cpt-cf-bss-pricing-feature-foundation`.
Source: PriceBook spec §2.2, §5–§8, §12–§13 and [DECISIONS](../DECISIONS.md) D-384–D-400.

## 2. Actor Flows (CDSL)

### Author a book and price

Actors: `cpt-cf-bss-pricing-actor-finance-manager`, `cpt-cf-bss-pricing-actor-products`, `cpt-cf-bss-pricing-actor-auditor`. Feature flow: `cpt-cf-bss-pricing-flow-books-prices`.

1. [ ] - `p1` - Finance Manager creates a uniquely coded currency book and reads its ETag. - `inst-books-prices-flow-1`
2. [ ] - `p1` - Select a published non-bundle SKU, recurring period if applicable and optional registered dimension key. - `inst-books-prices-flow-2`
3. [ ] - `p1` - Resolve replay and pass the stable price identity to the reserve-write-confirm protocol in slice 03. - `inst-books-prices-flow-3`
4. [ ] - `p1` - After reservation, re-read SKU type/lifecycle, derive charge kind, enforce key uniqueness and persist through that protocol. - `inst-books-prices-flow-4`
5. [ ] - `p1` - Return book/price facts; later money is drafted as rows, not embedded in the key. - `inst-books-prices-flow-5`

## 3. Processes / Business Logic (CDSL)

### book-and-key

Feature algorithm: `cpt-cf-bss-pricing-algo-books-prices-book-and-key`.

1. [ ] - `p1` - Validate currency and nonempty validity interval; scope code uniqueness to tenant. - `inst-books-prices-book-and-key-1`
2. [ ] - `p1` - Derive charge_kind from the current SKU; recurring accepts month/year, usage and one_time require null period. - `inst-books-prices-book-and-key-2`
3. [ ] - `p1` - Enforce the book/SKU/kind/coalesced-period unique index and map races to a conflict. - `inst-books-prices-book-and-key-3`
4. [ ] - `p1` - PATCH name/validity or permitted price overrides conditionally; reject currency edits and dimension changes after valued rows exist. - `inst-books-prices-book-and-key-4`

### dimension-registry

Feature algorithm: `cpt-cf-bss-pricing-algo-books-prices-dimension-registry`.

1. [ ] - `p1` - Validate key syntax and sufficient distinct values; seed region for the tenant. - `inst-books-prices-dimension-registry-1`
2. [ ] - `p1` - Reject unknown row values or values without a declared price dimension. - `inst-books-prices-dimension-registry-2`
3. [ ] - `p1` - Before removing a value, atomically check all tenant rows referencing it, including historical rows. - `inst-books-prices-dimension-registry-3`
4. [ ] - `p1` - Apply the direct versioned registry edit without an approval unit. - `inst-books-prices-dimension-registry-4`

### settings-and-export

Feature algorithm: `cpt-cf-bss-pricing-algo-books-prices-settings-and-export`.

1. [ ] - `p1` - Authorize settings separately from book authoring and enforce If-Match on settings changes. - `inst-books-prices-settings-and-export-1`
2. [ ] - `p1` - Resolve defaults by SKU billing-timing override then tenant timing; preserve rounding and per-type template inputs for binding. - `inst-books-prices-settings-and-export-2`
3. [ ] - `p1` - For export, authorize read and select only the scoped book, prices and rows; preserve row identities and chain windows. - `inst-books-prices-settings-and-export-3`
4. [ ] - `p1` - Return JSON without modifying state, submitting units or evaluating totals. - `inst-books-prices-settings-and-export-4`

## 4. States (CDSL)

A book is valid or invalid for a queried date according to its optional interval; no approval state is added. A price progresses through reference confirmation in slice 03. Currency and key identity are fixed; editable metadata uses versions. Registry/settings changes are direct and versioned.

State definition: `cpt-cf-bss-pricing-state-books-prices` in the FEATURE.

## 5. API Surface

Under /bss-pricing/v1: POST/GET /price-books; GET/PATCH /price-books/{id}; GET /price-books/{id}/prices; GET /price-books/{id}/export; POST /price-books/{id}/prices; PATCH/DELETE /prices/{id}; GET/PUT /settings and /dimension-keys. Price deletion refuses approved or pending rows with 409 PRICE_ROWS_IN_USE; draft and rejected rows are deleted with the price (a rejected row's review history stays in its unit snapshot), and DELETE answers 204 once the removal commits while the release completes as durable reference work. POST requires Idempotency-Key; PATCH/PUT require If-Match. Permissions are read, author and settings as appropriate.

[DESIGN §3.3](../DESIGN.md#33-api-contracts) fixes canonical errors and route prefixes.
Each mounted route must appear in all four censuses with authz and precondition expectations.

## 6. Data Model

pricing_price_book stores code, name, currency, validity and version. pricing_price uses a normalized nullable period in its unique book/SKU/kind key and carries dimension_key, invoice_line_override and its reference receipt. pricing_dimension_key stores tenant/key/allowed values; pricing_settings stores defaults and invoice-line templates by SKU type. Price authoring delegates reference work to slice 03.

Tenant-scoped parent validation is required even where foreign keys use entity ids. Never substitute a
cross-gear read for transactional local ownership/version guards. Approved money and historical pins survive.

## 7. Events & Alarms

Book/registry/settings edits do not invent new publish events. Required audit and replay persist with direct acts. Price reference failures and lost receipts use slice 03 recovery; approved row events are defined by slices 05 and 07.

Audit and outbox inserts use the same mutation transaction; retry is lifecycle-managed and observes shutdown.

## 8. Definitions of Done

The sole definitions live in [features/books-prices.md](../features/books-prices.md):

- `cpt-cf-bss-pricing-dod-book-currency-validity` — Currency books and validity.
- `cpt-cf-bss-pricing-dod-price-key-unique` — One price per book key.
- `cpt-cf-bss-pricing-dod-price-metadata` — Restricted price metadata edits.
- `cpt-cf-bss-pricing-dod-dimension-registry` — Tenant dimension registry.
- `cpt-cf-bss-pricing-dod-settings-defaults` — Versioned billing defaults.
- `cpt-cf-bss-pricing-dod-book-export` — Read-only book export.
- `cpt-cf-bss-pricing-dod-price-reference-handoff` — Creation uses the reference service.

## 9. Acceptance Criteria

1. PRD AC #2 / `cpt-cf-bss-pricing-dod-book-currency-validity`: Given a EUR book, when name/validity changes with its ETag then currency stays EUR; duplicate tenant code or inverted dates are refused.
2. PRD AC #3 / `cpt-cf-bss-pricing-dod-price-key-unique`: Given the same nonrecurring SKU twice, when concurrent creates use null period then only one price persists; a bundle has no price.
3. PRD AC #3 / `cpt-cf-bss-pricing-dod-price-metadata`: Given a valued or approved row, when dimension change or price deletion is requested then it is refused; allowed metadata updates retain receipt identity.
4. PRD AC #1 / `cpt-cf-bss-pricing-dod-dimension-registry`: Given EU rows, when US is added then it is available; deleting EU or drafting UNKNOWN is refused without changing the registry.
5. PRD AC #13 / `cpt-cf-bss-pricing-dod-settings-defaults`: Given default arrears and SKU advance, when inputs bind then advance wins; stale settings update fails with no partial changes.
6. PRD AC #12 / `cpt-cf-bss-pricing-dod-book-export`: Given two tenants, when one exports its book then only its facts appear; foreign-book access is denied and row/unit counts do not change.
7. PRD AC #3 / `cpt-cf-bss-pricing-dod-price-reference-handoff`: Given Products unavailable before reserve, when a price is created then REGISTRY_UNAVAILABLE leaves no price; a successful create retains its live receipt.

## 10. Non-Functional Considerations

All paths enforce tenant isolation, deny-by-default authz and append-only attribution. Mutations use conditional
versions, required POST replay and PATCH/PUT preconditions; PostgreSQL serializable chain changes and SQLite
writer serialization preserve the same invariants. A failed audit/outbox write cannot leave a committed act.
No timeout releases a live reference. Review and apply retain typed database failures for bounded retry.
Implementation gates cover both backends and route censuses; document gates cover toc, language and identifier ownership.
