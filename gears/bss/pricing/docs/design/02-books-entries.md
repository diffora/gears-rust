<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Books & Entries (Design, Slice 2) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ../DECISIONS.md | Owners: BSS Pricing team -->

# DESIGN — Books & Entries (Slice 2)

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-design-slice-02`

<!-- toc -->

- [1. Context](#1-context)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Author a book and entry](#author-a-book-and-entry)
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

Author currency books and unique SKU entries, maintain dimensions/settings and export book facts; hand entry creation/removal to the reservation service.

Requirements: `cpt-cf-bss-pricing-fr-dimension-registry`, `cpt-cf-bss-pricing-fr-price-book`, `cpt-cf-bss-pricing-fr-entry-key`, `cpt-cf-bss-pricing-fr-book-export`, `cpt-cf-bss-pricing-fr-settings`. Architecture: `cpt-cf-bss-pricing-component-books`, `cpt-cf-bss-pricing-component-reservations-client`, `cpt-cf-bss-pricing-principle-book-money-independent`, `cpt-cf-bss-pricing-principle-reserve-before-write`, `cpt-cf-bss-pricing-constraint-one-replay-store`, `cpt-cf-bss-pricing-seq-reserve-write-confirm`.
[FEATURE](../features/books-entries.md) owns the executable flow/algorithm/DoD identifiers; this slice defines no duplicate DoDs.
Dependencies: `cpt-cf-bss-pricing-feature-foundation`.
Source: PriceBook spec §2.2, §5–§8, §12–§13 and [DECISIONS](../DECISIONS.md) D-384–D-406.

## 2. Actor Flows (CDSL)

### Author a book and entry

Actors: `cpt-cf-bss-pricing-actor-finance-manager`, `cpt-cf-bss-pricing-actor-products`, `cpt-cf-bss-pricing-actor-auditor`. Feature flow: `cpt-cf-bss-pricing-flow-books-entries`.

1. [ ] - `p1` - Finance Manager creates a uniquely coded currency book and reads its ETag. - `inst-books-entries-flow-1`
2. [ ] - `p1` - Select a published non-bundle SKU, recurring period if applicable and optional registered dimension key. - `inst-books-entries-flow-2`
3. [ ] - `p1` - Resolve replay and pass the stable entry identity to the reserve-write-confirm protocol in slice 03. - `inst-books-entries-flow-3`
4. [ ] - `p1` - After reservation, re-read SKU type/lifecycle, derive charge kind, enforce key uniqueness and persist through that protocol. - `inst-books-entries-flow-4`
5. [ ] - `p1` - Return book/entry facts; later money is drafted as prices, not embedded in the key. - `inst-books-entries-flow-5`

## 3. Processes / Business Logic (CDSL)

### book-and-key

Feature algorithm: `cpt-cf-bss-pricing-algo-books-entries-book-and-key`.

1. [ ] - `p1` - Validate currency and nonempty validity interval; scope code uniqueness to tenant. - `inst-books-entries-book-and-key-1`
2. [ ] - `p1` - Derive charge_kind from the current SKU; recurring accepts month/year, usage and one_time require null period. - `inst-books-entries-book-and-key-2`
3. [ ] - `p1` - Enforce the book/SKU/kind/coalesced-period unique index and map races to a conflict. - `inst-books-entries-book-and-key-3`
4. [ ] - `p1` - PATCH name/validity or permitted entry overrides conditionally; reject currency edits and dimension changes after valued prices exist. - `inst-books-entries-book-and-key-4`

### dimension-registry

Feature algorithm: `cpt-cf-bss-pricing-algo-books-entries-dimension-registry`.

1. [ ] - `p1` - Validate key syntax and distinct values (none yet, or at least two; exactly one is DIM_VALUES_FEW); a tenant with no stored registry reads the seed region with no values, and the first entry naming region stores it in that entry's transaction. - `inst-books-entries-dimension-registry-1`
2. [ ] - `p1` - Reject unknown price values or values without a declared entry dimension. - `inst-books-entries-dimension-registry-2`
3. [ ] - `p1` - Before removing a value, atomically check all tenant prices referencing it, including historical prices. - `inst-books-entries-dimension-registry-3`
4. [ ] - `p1` - Apply the direct versioned registry edit without an approval unit. - `inst-books-entries-dimension-registry-4`

### settings-and-export

Feature algorithm: `cpt-cf-bss-pricing-algo-books-entries-settings-and-export`.

1. [ ] - `p1` - Authorize settings separately from book authoring and enforce If-Match on settings changes. - `inst-books-entries-settings-and-export-1`
2. [ ] - `p1` - Resolve defaults by SKU billing-timing override then tenant timing; preserve rounding and per-type template inputs for binding. - `inst-books-entries-settings-and-export-2`
3. [ ] - `p1` - For export, authorize read and select only the scoped book, entries and prices; preserve price identities and chain windows. - `inst-books-entries-settings-and-export-3`
4. [ ] - `p1` - Return JSON without modifying state, submitting units or evaluating totals. - `inst-books-entries-settings-and-export-4`

## 4. States (CDSL)

A book is valid or invalid for a queried date according to its optional interval; no approval state is added. An entry progresses through reference confirmation in slice 03. Currency and key identity are fixed; editable metadata uses versions. Registry/settings changes are direct and versioned.

State definition: `cpt-cf-bss-pricing-state-books-entries` in the FEATURE.

## 5. API Surface

Under /bss-pricing/v1: POST/GET /price-books; GET/PATCH /price-books/{id}; GET /price-books/{id}/entries; GET /price-books/{id}/export; POST /price-books/{id}/entries; GET/PATCH/DELETE /price-book-entries/{id}; GET/PUT /settings and /dimension-keys. Entry deletion refuses approved or pending prices with 409 ENTRY_PRICES_IN_USE and another author's draft with 403 NOT_DRAFT_AUTHOR (D-404); the caller's draft and all rejected prices are deleted with the entry (a rejected price's review history stays in its unit snapshot), and DELETE answers 204 once the removal commits while the release completes as durable reference work. POST requires Idempotency-Key; PATCH/PUT require If-Match, and a stale token is 409 STALE_REVISION. A new entry on a SKU that is fenced, retiring or retired, deprecated or still draft is 409 SKU_FENCED (Products' own refusal), SKU_RETIRING, SKU_DEPRECATED or SKU_DRAFT; a bundle SKU is 409 BUNDLE_SKU_NOT_PRICEABLE. A dimension_key change after a valued price, or removing a registry key an entry names, is 409 DIMENSION_KEY_IN_USE. Permissions are read, author and settings as appropriate.

[DESIGN §3.3](../DESIGN.md#33-api-contracts) fixes canonical errors and route prefixes.
Each mounted route must appear in all four censuses with authz and precondition expectations.

## 6. Data Model

pricing_price_book stores code, name, currency, validity and version. pricing_price_book_entry uses a normalized nullable period in its unique book/SKU/kind key and carries dimension_key, invoice_line_override and its reference receipt. pricing_dimension_key stores tenant/key/allowed values; pricing_settings stores defaults and invoice-line templates by SKU type. Entry authoring delegates reference work to slice 03.

Tenant-scoped parent validation is required even where foreign keys use entity ids. Never substitute a
cross-gear read for transactional local ownership/version guards. Approved money and historical pins survive.

## 7. Events & Alarms

Book/registry/settings edits do not invent new publish events. Required audit and replay persist with direct acts. Entry reference failures and lost receipts use slice 03 recovery; approved price events are defined by slices 05 and 07.

Audit and outbox inserts use the same mutation transaction; retry is lifecycle-managed and observes shutdown.

## 8. Definitions of Done

The sole definitions live in [features/books-entries.md](../features/books-entries.md):

- `cpt-cf-bss-pricing-dod-book-currency-validity` — Currency books and validity.
- `cpt-cf-bss-pricing-dod-entry-key-unique` — One entry per book key.
- `cpt-cf-bss-pricing-dod-entry-metadata` — Restricted entry metadata edits.
- `cpt-cf-bss-pricing-dod-dimension-registry` — Tenant dimension registry.
- `cpt-cf-bss-pricing-dod-settings-defaults` — Versioned billing defaults.
- `cpt-cf-bss-pricing-dod-book-export` — Read-only book export.
- `cpt-cf-bss-pricing-dod-entry-reference-handoff` — Creation uses the reference service.

## 9. Acceptance Criteria

1. PRD AC #2 / `cpt-cf-bss-pricing-dod-book-currency-validity`: Given a EUR book, when name/validity changes with its ETag then currency stays EUR; duplicate tenant code or inverted dates are refused.
2. PRD AC #3 / `cpt-cf-bss-pricing-dod-entry-key-unique`: Given the same nonrecurring SKU twice, when concurrent creates use null period then only one entry persists; a bundle has no entry.
3. PRD AC #3 / `cpt-cf-bss-pricing-dod-entry-metadata`: Given a valued price, when a dimension change is requested then it is refused DIMENSION_KEY_IN_USE; given an approved or pending price, entry deletion is refused ENTRY_PRICES_IN_USE, and NOT_DRAFT_AUTHOR (D-404) while another author's draft exists; the caller's drafts and all rejected prices are deleted with the entry; allowed metadata updates retain receipt identity.
4. PRD AC #1 / `cpt-cf-bss-pricing-dod-dimension-registry`: Given EU prices, when US is added then it is available; deleting EU or drafting UNKNOWN is refused without changing the registry.
5. PRD AC #13 / `cpt-cf-bss-pricing-dod-settings-defaults`: Given default arrears and SKU advance, when inputs bind then advance wins; stale settings update fails with no partial changes.
6. PRD AC #12 / `cpt-cf-bss-pricing-dod-book-export`: Given two tenants, when one exports its book then only its facts appear; foreign-book access is denied and price/unit counts do not change.
7. PRD AC #3 / `cpt-cf-bss-pricing-dod-entry-reference-handoff`: Given Products unavailable before reserve, when an entry is created then REGISTRY_UNAVAILABLE leaves no entry; a successful create retains its live receipt.

## 10. Non-Functional Considerations

All paths enforce tenant isolation, deny-by-default authz and append-only attribution. Mutations use conditional
versions, required POST replay and PATCH/PUT preconditions; PostgreSQL serializable chain changes and SQLite
writer serialization preserve the same invariants. A failed audit/outbox write cannot leave a committed act.
No timeout releases a live reference. Review and apply retain typed database failures for bounded retry.
Implementation gates cover both backends and route censuses; document gates cover toc, language and identifier ownership.
