<!-- CONFLUENCE_TITLE: [BSS]: Pricing — PriceBook Design -->
<!-- Related: ./PRD.md, ./DECISIONS.md, ./design/ | Owners: BSS Pricing team -->

# DESIGN — Pricing: PriceBook

- [ ] `p1` - **DESIGN implementation status**

<!-- toc -->

- [1. Architecture Overview](#1-architecture-overview)
  - [1.1 Architectural Vision](#11-architectural-vision)
  - [1.2 Architecture Drivers](#12-architecture-drivers)
  - [1.3 Architecture Layers](#13-architecture-layers)
- [2. Principles & Constraints](#2-principles--constraints)
  - [2.1 Design Principles](#21-design-principles)
  - [2.2 Constraints](#22-constraints)
- [3. Technical Architecture](#3-technical-architecture)
  - [3.1 Domain Model](#31-domain-model)
  - [3.2 Component Model](#32-component-model)
  - [3.3 API Contracts](#33-api-contracts)
  - [3.4 Internal Dependencies](#34-internal-dependencies)
  - [3.5 External Dependencies](#35-external-dependencies)
  - [3.6 Interactions & Sequences](#36-interactions--sequences)
  - [3.7 Database schemas & tables](#37-database-schemas--tables)
- [4. Additional context](#4-additional-context)
- [5. Traceability](#5-traceability)

<!-- /toc -->

## 1. Architecture Overview

### 1.1 Architectural Vision

Products owns the SKU registry and reference barrier; Pricing owns per-currency books and immutable approved
money chains. A shared approval engine governs proposed business content through gear-local transactions.
Plans arrive in phase 3; the owner defers promotions (D-409), migration requests and retirement (D-410) and the
sold-as bundle and grants (D-411). Consumer resolution arrives in phase 4, and quote is not built (D-415).
This is the target design, not a claim that the legacy pricing implementation has already been replaced.

Authority is the PriceBook spec §2, §2.2 and §13, then [DECISIONS](DECISIONS.md), then code, then prose.
The plan's D-399 deviation removes the phase 2 SkuChanged listener; current SKU facts come from ProductsClient.

### 1.2 Architecture Drivers

| Requirement | Driver | Satisfied by |
| --- | --- | --- |
| `cpt-cf-bss-pricing-fr-dimension-registry` | A tenant registry stores dimension keys and their allowed values, seeded with region: with nothing stored, GET /dimension-keys reads region with no values, and the first entry naming it stores the seed in its own transaction. A key has no values yet or at least two (DIM_VALUES_FEW). | Books & Entries, phase 2; §3 and slice 02. |
| `cpt-cf-bss-pricing-fr-price-book` | A book has a tenant-unique code, name, immutable currency and optional valid_from/valid_until dates. | Books & Entries, phase 2; §3 and slice 02. |
| `cpt-cf-bss-pricing-fr-entry-key` | Inside a book there is one entry per (sku_id, charge_kind, period), with null period normalized for uniqueness. | Books & Entries, phase 2; §3 and slice 02. |
| `cpt-cf-bss-pricing-fr-price` | Draft prices carry model, price_json, dates, optional dim_value and min_fee, eligibility all or new, note and author. | Prices, Windows & Dimension, phase 2; §3 and slice 03. |
| `cpt-cf-bss-pricing-fr-chain-windows` | Windows are half-open and close independently for each (price_book_entry_id, dim_value), including the null default chain. | Prices, Windows & Dimension, phase 2; §3 and slice 03. |
| `cpt-cf-bss-pricing-fr-pair-guard` | On a usage chain, a successor preserves model kind, package size and SKU metering as of each price's start (D-402). | Prices, Windows & Dimension, phase 2; §3 and slice 03. |
| `cpt-cf-bss-pricing-fr-min-fee` | The floor belongs to a price per subscription per billing period, aggregating every value and slice rated by that price; pricing stores and validates min_fee, and Rating applies the floor (D-415). | Prices, Windows & Dimension, phase 2; §3 and slice 03. |
| `cpt-cf-bss-pricing-fr-temporary-pair` | A temporary change on an existing chain creates two prices in one approval unit. | Prices, Windows & Dimension, phase 2; §3 and slice 03. |
| `cpt-cf-bss-pricing-fr-publish-changes` | Publish changes lists all draft prices of one book with full money, window, chain, predecessor and impact information, all pre-selected. | Approvals, phase 2; §3 and slice 05. |
| `cpt-cf-bss-pricing-fr-approval-units` | Use bss-approval for prices now and plan_revision in phase 3; the promotion (D-409) and migration (D-410) kinds are deferred. | Approvals, phase 2; §3 and slice 05. |
| `cpt-cf-bss-pricing-fr-reference-protocol` | Before reserve, claim the key and persist a create op; reserve with Products, re-read the SKU, commit the entry with reference_state = confirmation_pending and op written, then confirm and atomically finish the op and answer the key (D-401). | Prices, Windows & Dimension, phase 2; §3 and slice 03. |
| `cpt-cf-bss-pricing-fr-book-export` | Provide one read-only JSON export of a tenant-scoped book with its entries and prices. | Books & Entries, phase 2; §3 and slice 02. |
| `cpt-cf-bss-pricing-fr-settings` | Tenant settings provide default billing timing, rounding, GL code, tax category and invoice-line templates by SKU type. | Books & Entries, phase 2; §3 and slice 02. |
| `cpt-cf-bss-pricing-fr-events` | Persist PricesPublished and ApprovalUnitDecided with state and audit in the toolkit outbox, using broker TypedEvent envelopes; include PriceBookEntryReferenceLost for a failed reference confirmation that proves release. | Read Contract & Events, phase 2; §3 and slice 07. |
| `cpt-cf-bss-pricing-fr-plans` | A plan has immutable published revisions; each revision binds one book and contains paid, optional or included items, availability, minimal Grants and optional sold-as bundle SKU. Grants and the sold-as bundle SKU are deferred (D-411). | Plans, phase 3; §3 and slice 04. |
| `cpt-cf-bss-pricing-fr-promotions` | A dated percentage promotion targets plans and recurring or recurring-plus-usage charges. | Promotions & Migrations; deferred by the owner (D-409); §3 and slice 06. |
| `cpt-cf-bss-pricing-fr-migrations` | An approved migration_request records target plan/revision, subscription ids, next_renewal or explicit date, and the period-aware preview, then emits SubscriptionMigrationRequested. | Promotions & Migrations; deferred by the owner (D-410); §3 and slice 06. |
| `cpt-cf-bss-pricing-fr-resolve` | GET /pricing/v1/resolve accepts plan_revision_id, date and optional pins and returns each item's full default/value chain matrix and active promotion (id, version) (deferred with promotions, D-409), without totals. | Read Contract & Events, phase 4; §3 and slice 07. |
| `cpt-cf-bss-pricing-fr-price-read` | GET /pricing/v1/prices/{id} serves a pinned price forever, including closed, superseded and keep_for_bound prices. | Read Contract & Events, phase 4; §3 and slice 07. |
| `cpt-cf-bss-pricing-fr-quote` | GET /pricing/v1/quote is the Studio preview with quantities and optional-item choices, returning totals. | Not built (D-415); §3 and slice 07. |
| `cpt-cf-bss-pricing-nfr-authz` | Every door authenticates and enforces deny-by-default pricing:read, author, submit, approve or settings through PolicyEnforcer. | Foundation, phase 2; §3 and slice 01. |
| `cpt-cf-bss-pricing-nfr-audit` | Append tenant, actor, subject, correlation and before/after facts with each governed act. | Foundation, phase 2; §3 and slice 01. |
| `cpt-cf-bss-pricing-nfr-tenant-isolation` | Every repository uses SecureORM and PDP-derived AccessScope; child prices are reached through scoped parents. | Foundation, phase 2; §3 and slice 01. |
| `cpt-cf-bss-pricing-nfr-two-backends` | The fresh migration chain, constraints and transactional rules work on SQLite and Postgres. | Foundation, phase 2; §3 and slice 01. |
| `cpt-cf-bss-pricing-nfr-idempotency-concurrency` | All pricing POSTs require Idempotency-Key; PATCH/PUT require If-Match. | Foundation, phase 2; §3 and slice 01. |

| Architectural decision | Effect |
| --- | --- |
| `cpt-cf-bss-pricing-adr-book-per-currency` | Books own currency and entry identity; plans select a book. |
| `cpt-cf-bss-pricing-adr-price-chains-per-dimension-value` | Windows normalize per value with default fallback. |
| `cpt-cf-bss-pricing-adr-one-approval-unit-shape` | One engine, independent subjects, generation-aware review. |
| `cpt-cf-bss-pricing-adr-reference-reservation` | Synchronous reserve, local commit and durable confirm/release recovery. |

### 1.3 Architecture Layers

REST doors → pure domain rules and ApprovalSubjects → SecureORM repositories and shared approval Store.
Ports isolate ProductsClient, the broker and clock. Infrastructure wires configuration, authz, scoped transactions,
toolkit outbox dispatch and durable reference retry. Pure book/entry/price/money rules port the prototype with
half-open band boundaries corrected by D-387. There is no domain filesystem dependency or SKU cache.

## 2. Principles & Constraints

### 2.1 Design Principles

**ID**: `cpt-cf-bss-pricing-principle-book-money-independent`

Book money and revision structure are independent facts. A rejected revision does not undo approved prices.
Bindings preserve the money price and dated SKU descriptors needed to replay an invoice.

**ID**: `cpt-cf-bss-pricing-principle-reserve-before-write`

Reserve before a durable reference; release only after durable cancellation or removal. A timeout is not proof
of rollback. Products keeps an unresolved reservation live and refuses retirement while it exists. A copied plan
item is the one exception: it is written unreserved and attaches after the write, because the source revision's
live reference already protects the SKU (D-413).

**ID**: `cpt-cf-bss-pricing-principle-business-content-fingerprint`

Hash proposed item business content and common date, excluding locks, versions and operational metadata.
Drift refreshes the unit, making old decisions stale; reviewers must see and vote on the new generation.

### 2.2 Constraints

**ID**: `cpt-cf-bss-pricing-constraint-two-backends`

SQLite and Postgres share invariants and scoped repository behavior. The migration chain starts at 000001;
stand data is not migrated. Both schema goldens and real writer races are implementation gates.

**ID**: `cpt-cf-bss-pricing-constraint-no-row-locks`

SecureORM has no FOR UPDATE. Conditional versions and pending ownership serialize subjects; Postgres uses
serializable transactions for chain changes. Retry serialization once, with SQLite lock-upgrade errors using the
same bounded transaction retry path. Clone attempt inputs and re-read guards inside each attempt.

**ID**: `cpt-cf-bss-pricing-constraint-one-replay-store`

Require Idempotency-Key for POST and If-Match for PATCH/PUT. A 24-hour scoped replay store owns claims and
responses; no approval-unit key exists. A retry cannot allocate an unrelated entry/ref_id before consulting replay.

## 3. Technical Architecture

### 3.1 Domain Model

Glossary (the owner's rename, spec §2.3):

- **PriceBookEntry** — a book's line (SKU × charge kind × period); its Prices are dated amounts per dimension-value chain.
  It carries the dimension key, the invoice-line override and the Products reservation.
- **Price** — one dated amount in the chain of one dimension value: model, money (`price_json`), min_fee,
  eligibility, window and state.
- **Chain** — the prices of one entry and one dimension value; a concept, not an entity.

Names before the rename, kept here on purpose so older records stay readable:

| Earlier name | Name now |
| --- | --- |
| `price`, table `pricing_price`, `/price-books/{id}/prices`, `/prices/{id}` | `price_book_entry`, table `pricing_price_book_entry`, `/price-books/{id}/entries`, `/price-book-entries/{id}` |
| `price_row`, table `pricing_price_row`, `/prices/{id}/rows`, `/rows/{id}`, `GET /pricing/v1/price-rows/{id}` | `price`, table `pricing_price`, `/price-book-entries/{id}/prices`, `/prices/{id}`, `GET /pricing/v1/prices/{id}` |
| `price_id` (on prices and reference ops), `row_id`, `row_ids` | `price_book_entry_id`, `price_id`, `price_ids` |
| approval kind `price_rows`; events `PriceRowsPublished`, `PriceReferenceLost`; reference kind `price` | `prices`; `PricesPublished`, `PriceBookEntryReferenceLost`; `price_book_entry` |
| op kinds `create_price`, `delete_price`, `rereserve_price` | `create`, `delete`, `rereserve` on `(ref_kind, ref_id)`, plus `attach` for a copied item (D-412, D-413) |
| line codes `PRICE_KEY_TAKEN`, `PRICE_REFERENCE_LOST`, `PRICE_PERIOD_INVALID`, `PRICE_ROWS_IN_USE`, `PRICE_NOT_FOUND`, `PRICE_CONFIRMATION_PENDING`, `PRICE_WRITE_REFUSED` | `ENTRY_KEY_TAKEN`, `ENTRY_REFERENCE_LOST`, `ENTRY_PERIOD_INVALID`, `ENTRY_PRICES_IN_USE`, `ENTRY_NOT_FOUND`, `ENTRY_CONFIRMATION_PENDING`, `ENTRY_WRITE_REFUSED` |
| amount codes `ROW_NOT_DRAFT`, `ROW_NOT_IN_BOOK`, `ROW_LOCKED_PENDING`, `ROW_VERSION_TAKEN`, `ROW_NOT_PENDING`, `ROW_NOT_FOUND`, `NO_DRAFT_ROWS`, `TEMPORARY_ROW_FIXED`; phase 3 `ROW_SKU_DEPRECATED` | `PRICE_NOT_DRAFT`, `PRICE_NOT_IN_BOOK`, `PRICE_LOCKED_PENDING`, `PRICE_VERSION_TAKEN`, `PRICE_NOT_PENDING`, `PRICE_NOT_FOUND`, `NO_DRAFT_PRICES`, `TEMPORARY_PRICE_FIXED`; phase 3 `ITEM_SKU_DEPRECATED` |

The shared approval engine still names its own lock conflict `ROW_LOCKED_PENDING` (Products answers it for SKUs);
the pricing doors answer `PRICE_LOCKED_PENDING` for a price and `ROW_LOCKED_PENDING` for a plan revision (phase 3).

PriceBook contains PriceBookEntry; PriceBookEntry contains Price chains keyed by nullable dim_value. SKU type determines
charge_kind; period is recurring-only. A price carries a pricing model and inputs, min_fee, eligibility, dates,
state and approval attribution. It contains no frozen descriptors. Plan, PlanRevision and PlanItem are
phase 3 types; Promotion (D-409) and MigrationRequest (D-410) are deferred by the owner; phase 4 assembles Resolution
outputs (Quote is not built, D-415). A plan projects its published revision number; a revision binds one book and
its items (D-407).

```mermaid
classDiagram
  PriceBook "1" --> "many" PriceBookEntry
  PriceBookEntry "1" --> "many" Price
  DimensionKey "0..1" <-- "many" PriceBookEntry
  ApprovalUnit "1" --> "many" ApprovalItem
  ApprovalUnit "1" --> "many" ApprovalDecision
  ApprovalItem --> Price
  PriceBookEntry --> ReferenceReceipt
  Plan "1" --> "many" PlanRevision
  PlanRevision --> PriceBook
  PlanRevision "1" --> "many" PlanItem
  PlanItem --> PriceBookEntry
```

Approved prices are immutable money facts. Conditional normalization may close predecessor windows and set
keep_for_bound when the successor is new; it cannot rewrite money or remove historical pins. Value chains can
end explicitly. Tier arithmetic uses decimal/money types without binary float rounding and [from, to) bands.
Every decimal of a price (amount, rate, package size and package price, tier up_to and rate) travels as a JSON string;
a JSON number is refused 400 AMOUNT_INVALID, because reading it would round it through f64.
Min-fee accounting groups by price/subscription/period across values and slices before promotions; Rating applies it
(D-415).

### 3.2 Component Model

#### Books

**ID**: `cpt-cf-bss-pricing-component-books`

Phase 2: dimension registry, settings, book validity, entry identity and read-only export.

#### Prices

**ID**: `cpt-cf-bss-pricing-component-prices`

Phase 2: price models, chain normalization, pair guard, minimum fees and temporary pairs.

#### Approvals

**ID**: `cpt-cf-bss-pricing-component-approvals`

Phase 2 prices subject and unit doors; phase 3 plan_revision subject (the promotion and migration subjects are deferred, D-409, D-410). Shared Store and engine, generation refresh, SoD and atomic terminal acts.

#### Reservations client

**ID**: `cpt-cf-bss-pricing-component-reservations-client`

Phase 2: ProductsClient reserve/re-read/write/confirm and durable cancellation/release. Persist reference loss. Phase 3 extends the protocol to plan_item; sold_as is deferred (D-411).

#### Events

**ID**: `cpt-cf-bss-pricing-component-events`

Phase 2 core TypedEvent payloads and toolkit outbox dispatcher; phase 3 adds the plan events PlanRevisionPublished and PlanReferenceLost (promotion events deferred, D-409; migration-request and retirement events deferred, D-410).

#### Plans

**ID**: `cpt-cf-bss-pricing-component-plans`

Phase 3: revision composition, sale-date checks, blocked_by and clone; retirement prerequisites are deferred (D-410).

#### Promotions and migrations

**ID**: `cpt-cf-bss-pricing-component-promotions`

Deferred by the owner: approved migration requests without executing subscription moves (D-410) and nonoverlapping versioned promotions (D-409).

#### Read contract

**ID**: `cpt-cf-bss-pricing-component-read-contract`

Phase 4: resolve matrix, renewal walk, period bindings and durable pin reads; the Studio quote is not built (D-415); versioned Products reads at binding time.

### 3.3 API Contracts

The mounted authoring base is `/bss-pricing/v1`. Every mutation passes authenticated PolicyEnforcer scope,
headers + Bytes, preconditions::parse_body and correlation::establish. OperationBuilder registers matching
OpenAPI success/error schemas. POST requires Idempotency-Key; PATCH/PUT require If-Match. Queue reads return
stored snapshots and live impact: a prices unit's GET /approval-units item and GET /approval-units/{id}, and the GET
publish-changes listing, carry the same impact object, {prices, entries, plans, subscriptions}: from phase 3, plans lists
every plan revision, in any state, whose items name an entry of the unit or listing, as { plan_id, code, revision_id,
rev_no, state }, and subscriptions reads "unavailable until the Subscriptions integration" (it read "unavailable until
phase 3" in phase 2). A plan_revision unit's impact is {subscriptions} alone, with the same text: publishing a revision
moves no existing pin (D-394). Never label unavailable impact as a measured zero. The stored prices snapshot also carries each
entry SKU's current descriptors, beside the fingerprinted after, never in it (D-408); their read is best-effort, and a
registry that cannot answer or refuses the caller records "descriptors": "unavailable" and refuses nothing (D-416).
Products read (D-416): the reads a rule needs are made as the caller, so the plan_revision submitter and its final
approver, readers of GET /plan-revisions/{id}/checks, plan item authors, price-book entry authors (the period rule and
the create re-read), and the submitter and final approver of a prices unit on a usage chain (the dated metering read, D-402) need products read; an approve-only reviewer votes on every other
unit and rejects any unit.

| Area | Phase | Operations below the authoring base |
| --- | --- | --- |
| Books | 2 | POST/GET /price-books; GET/PATCH /price-books/{id}; GET /price-books/{id}/entries; GET /price-books/{id}/export |
| Entries | 2 | POST /price-books/{id}/entries with sku_id, period?, dimension_key?; GET /price-book-entries/{id} reads one entry with its ETag (price_book_entry read); PATCH /price-book-entries/{id} for invoice_line_override and permitted dimension_key changes; DELETE /price-book-entries/{id} answers 204 once removed, deleting its draft and rejected prices with it; approved or pending prices refuse 409 ENTRY_PRICES_IN_USE, and another author's draft 403 NOT_DRAFT_AUTHOR (D-404); from phase 3 an entry a plan item names, in a revision of any state, refuses 409 ENTRY_IN_USE, judged in the delete's transaction (D-408) |
| Prices | 2 | POST /price-book-entries/{id}/prices; PATCH/DELETE /prices/{id} draft only, by its author (D-404); POST /prices/{id}/submit; POST /price-books/{id}/publish-changes with price_ids? and common_effective_date? |
| Approval units | 2 | GET /approval-units?state&kind&ref_id; GET /approval-units/{id}; POST /approval-units/{id}/approve or /reject with generation, /withdraw by submitter. Every unit door dispatches on the unit's stored kind (phase 3): its subject, the domain event its apply writes and the impact its card shows; a stored kind pricing does not record is a corrupt row (500), never judged as `prices` |
| Policy/settings | 2 | GET/PUT /approval-policy, /settings, /dimension-keys; PUT /approval-policy sets the default (`*`) or one kind's quorum, `prices` or `plan_revision` (phase 3); any other kind is 400 POLICY_KIND_INVALID |
| Reference work | 2 | GET /reference-ops?state&limit&cursor lists the tenant's durable reference ops in op-id order (config settings permission); limit 1 to 1000, default 100; the next page starts after next_cursor |
| Plans | 3 | POST /plans with code, name, book_id (201: the plan and its draft rev 1 on that book); GET /plans; GET/PATCH /plans/{id} (name); POST /plans/{id}/revisions copies the published revision (book, availability, items) into a new draft under D-413, refused while a draft or pending revision exists (REVISION_DRAFT_EXISTS); GET /plan-revisions/{id}; PATCH /plan-revisions/{id} with book_id?, available_from?, draft only (REVISION_NOT_DRAFT), with no item list (D-407): a new book_id remaps each item to the new book's entry of the same (SKU, charge kind, period), bumping the item's version, and an unmatched item keeps its entry (the checks then show ITEM_BOOK_FOREIGN); DELETE /plan-revisions/{id} draft only, with a delete op for every item reference (D-414), and the last revision of a never-published plan takes the plan with it in the same transaction, freeing its code (D-417); GET /plan-revisions/{id}/checks answers { checks, ready, sale_date } from fresh SKU reads (D-408); POST /plan-revisions/{id}/submit (plan submit, D-418, as POST /prices/{id}/submit is price submit; no body) makes an unlocked draft a plan_revision unit (201 { applied, unit, revision }; REVISION_NOT_DRAFT otherwise), judged by the checks of GET …/checks built by the same function from fresh SKU reads: a red check is 400 REVISION_CHECKS_RED with the red checks (code, label, detail, blocked_by) in the problem detail and no unit; its lock is the conditional pending_unit_id, a lost one 409 ROW_LOCKED_PENDING; quorum 0 publishes at once; POST /plans/{id}/clone with code, name (plan author, Idempotency-Key; 201 with the new plan, as POST /plans answers) makes a new plan whose draft rev 1 copies the source's published revision (book, availability, items) under D-413, without the source's decisions, approval identity or pins; a deprecated SKU is carried and the new plan's checks show ITEM_SKU_DEPRECATED (D-408). Deferred by the owner: grants and bundle_sku_id in the revision PATCH (D-411); POST /plans/{id}/retire with migration_request_id, and PLAN_RETIRING on a retiring plan (D-410) |
| Plan items | 3 | POST /plan-revisions/{id}/items with sku_id, price_book_entry_id?, treatment, included_qty?, qty_min? (a plan_item create op, D-407; at most 200 items per revision); PATCH /plan-items/{id} with treatment?, included_qty?, qty_min?, price_book_entry_id?, draft only and never a SKU change; DELETE /plan-items/{id} (a delete op); the revision's creator edits it and its items (D-404) |
| Read contract | 4 | GET /resolve?plan_revision_id&date&item_id?&pins? (plan read, D-419): a published or superseded revision on one date, each item with its chain matrix (default and every value, `binding` or `uncovered`), its SKU version as of the date and its resolved invoice inputs with their source (D-420, D-421); no totals, no promotion (D-409, D-415); pins are price_id or price_id:dim_value, at most 1 000. GET /prices/{id} (price read, D-422): an approved price of the tenant, whatever its window, with its entry's SKU, charge kind, period, book and currency, stored facts only. Both are reads: no audit row, no idempotency key, no binding |
| Promotions | deferred (D-409) | Deferred by the owner and not built in phase 3; the planned shape: POST /promotions with name, percent, from_date, to_date, plan_ids (at most 50), apply_to; GET /promotions; GET /promotions/{id} (the current approved version, the open version and the history); PATCH /promotions/{id} under If-Match on the promotion, editing the open draft version or creating it from the current approved one; POST /promotions/{id}/submit, /end-today, /cancel |
| Migrations | deferred (D-410) | Deferred by the owner and not built in phase 3; the planned shape: POST /plans/{id}/migrations with target_plan_id, target_revision_id, timing (next_renewal or date), at?, scope (all or listed) and subscriptions [{ subscription_id, current_plan_revision_id, current_period_end }] (at most 1000; caller-supplied, D-410) answers the request with its preview and its migration unit; GET /migration-requests/{id}; GET /plans/{id}/migrations |

The consumer surface named by spec §7.1, `GET /pricing/v1/resolve` and `GET /pricing/v1/prices/{id}`, is
`GET /bss-pricing/v1/resolve` and `GET /bss-pricing/v1/prices/{id}` below the gear's base (D-419, D-422; the Read contract
row above); phase 4 mounts them with golden contracts. `GET /pricing/v1/quote`, planned for the Studio, is not built,
and the Studio is not wired to the API (D-415).
Fields/query parameters are snake_case, including plan_revision_id, item_id, pins, price_id, dim_value, dim_used,
pinned_from, uncovered, sku_version, billing_timing, rounding_policy, promotion_id and promotion_version (the last two
deferred with promotions, D-409). Resolve returns inputs, not totals; slice 07 §6 gives the response field by field.

| Condition | Response |
| --- | --- |
| Products unavailable before reservation/write | 503 REGISTRY_UNAVAILABLE; no entry |
| SKU fenced / retiring or retired / deprecated / draft for a new entry | 409 SKU_FENCED (Products' reserve refusal, passed through) / SKU_RETIRING / SKU_DEPRECATED / SKU_DRAFT |
| Bundle SKU, or a SKU type that no longer matches the charge kind | 409 BUNDLE_SKU_NOT_PRICEABLE / CHARGE_KIND_SKU_TYPE |
| Products refuses a SKU read a rule needs (a dated metering read, a plan check's read; for example no SKU read) | Products' own status and code, at submit, approve, reject and the checks door; only unavailability is 503 REGISTRY_UNAVAILABLE (D-402, D-416); a descriptor read refuses nothing and records "descriptors": "unavailable" (D-416) |
| Usage-chain structure changed | 400 CHAIN_MODEL_CHANGED (D-403) |
| A temporary's return (or closed end) no longer matches the approved chain | 400 PAIR_RETURN_STALE at submit; APPLY_REFUSED at apply (D-391) |
| A price that starts inside a temporary window; a temporary whose window contains another price's start | 400 PRICE_INSIDE_TEMPORARY; 400 TEMPORARY_SPANS_A_CHANGE, at the draft door and at submit; APPLY_REFUSED at apply (D-406) |
| Invalid dimension or price window | DIM_KEY_INVALID, DIM_VALUES_FEW, DIM_VALUE_UNKNOWN, DIM_NOT_DECLARED, WINDOW_START_IN_PAST or WINDOW_OVERLAP; validation rejection |
| Money sent as a JSON number; NUL in body text | 400 AMOUNT_INVALID; 400 VALIDATION |
| Removing a registry key an entry names / a value a price uses | 409 DIMENSION_KEY_IN_USE / DIM_VALUE_IN_USE |
| Edit or delete of a price that is not an unlocked draft (a pending price included) | 409 PRICE_NOT_DRAFT |
| Stale If-Match, or a conditional write that lost its version | 409 STALE_REVISION |
| A price another pending unit owns, at submit; a plan revision whose lock is lost at submit; a contended unit | 409 PRICE_LOCKED_PENDING; 409 ROW_LOCKED_PENDING (phase 3); 409 UNIT_CONTENDED |
| Transaction still contended after its bounded retries | 409 CONTENDED (an entry create that fails so, or with a 500, after its reserve is cancelled before the answer: no entry, key free, receipt released); UNIT_CONTENDED at an approval-unit door (submit, publish-changes, approve, reject, withdraw) |
| Author approval; a draft price edited or deleted by anyone but its author, or an entry delete that would take another author's draft | 403 SOD_VIOLATION; 403 NOT_DRAFT_AUTHOR (D-404) |
| Generation changed or content drift | 400 GENERATION_MISMATCH or committed UNIT_STALE with current generation |
| Duplicate vote / terminal unit / wrong withdrawer | 409 DUPLICATE_VOTE / UNIT_ALREADY_DECIDED; 403 NOT_SUBMITTER |
| Apply environment changed | APPLY_REFUSED, transaction rolls back |
| Released receipt on confirm | 409 REFERENCE_RELEASED from Products, or 404 for a reservation Products does not know; the entry stays confirmation_pending and a rereserve op re-reserves it; lost only when the SKU is fenced, retiring or retired (D-401) |
| Phase 3: a red plan check at submit; an item refused at its door; an entry a plan item names, deleted | 400 REVISION_CHECKS_RED with the red checks, no unit; 400 ITEM_BOOK_FOREIGN, ITEM_ENTRY_SKU_MISMATCH, ITEM_ENTRY_MISSING, ITEM_SKU_DEPRECATED, ITEM_BUNDLE_SKU, REVISION_ITEMS_TOO_MANY, TREATMENT_INVALID, INCLUDED_QTY_INVALID or QTY_MIN_INVALID, 409 ITEM_SKU_TAKEN; 409 SKU_FENCED, SKU_RETIRING or SKU_DRAFT from the item's create op (Products' reserve refusal or the re-read); 409 ENTRY_IN_USE (D-408) |
| Phase 3: an item delete, or a draft revision delete, while an item's confirm is outstanding | 409 ITEM_CONFIRMATION_PENDING; retry once the confirm completes |
| Phase 3: a blank plan code; a plan code taken; a copy of a plan with no published revision; a clone of one | 400 PLAN_CODE_REQUIRED; 409 PLAN_CODE_TAKEN; 409 PLAN_UNPUBLISHED; 409 CLONE_SOURCE_UNPUBLISHED |
| Phase 4: a resolve of a draft or pending revision; a date that is not YYYY-MM-DD; a pin that names no approved default or own-chain price of an entry this revision's items name (a :dim_value pin on a value-chain price included); two pins for one (item, value); more than 1 000 pins | 409 REVISION_NOT_PUBLISHED; 400 DATE_INVALID; 400 PIN_FOREIGN for the whole request (a :dim_value is not checked against today's registry); 400 PIN_DUPLICATE; 400 PINS_TOO_MANY (D-419) |
| Phase 4: an unknown or another tenant's revision; an item_id the revision does not have; a price that is not an approved price of the tenant; a price id that is not an id | 404, before any Products read; 404; 404 with one body for a draft, pending, rejected, unknown or foreign price (D-422); 400 ID_INVALID |
| Phase 4: which resource a read-contract refusal names | every GET /resolve refusal (query, grant, revision, item, state, pins) names resource type cf.bss.pricing.plan.v1~; every GET /prices/{id} refusal (id, grant, price) names cf.bss.pricing.price.v1~ (phase 4 review F1). The authoring doors still name cf.bss.pricing.price_book.v1~ for every refusal; one resource type per label there is owed |
| Phase 4: a resolve whose SKU version read Products cannot answer, refuses, or answers 404 | 503 REGISTRY_UNAVAILABLE; Products' own status and code; sku_version null, like no version on the date, never a pass-through 404 (D-421) |
| Phase 4: a chain that no price covers on the date | not an error: uncovered: true and no binding (D-420, PRD AC #18) |
| Deferred: migration, retirement and promotion refusals | MIGRATION_TARGET_UNPUBLISHED, MIGRATION_TARGET_CURRENT, MIGRATION_TARGET_RETIRING, MIGRATION_CURRENCY_MISMATCH, MIGRATION_SUBSCRIPTION_PENDING and RETIRE_MIGRATION_REQUIRED wait with migration requests and retirement (D-410); PROMOTION_VERSION_OPEN and PROMOTION_NOT_STARTED with promotions (D-409) |

Canonical toolkit RFC-9457 Problem carries code, field and message, retaining typed DbErr for retry classification.
Malformed body/precondition failures occur before domain work. A body string (value or key) that contains a NUL
character is 400 VALIDATION at parse_body, before any database work, so SQLite and Postgres answer alike. All four existing route censuses must agree with
mounted operations, permissions and preconditions; no Json<T> extractor replaces the retained pricing door shape.

### 3.4 Internal Dependencies

bss-approval supplies the engine, policy selection, Store contract, generation-aware decisions and prefixed DDL.
Repositories accept &impl DBRunner so state, items, replay, audit and outbox share the caller's transaction.
Toolkit database retries use per-attempt cloned inputs. Broker TypedEvent defines the durable event envelope;
outbox_migrations_with_prefix("bss_pricing_outbox") belongs in DatabaseCapability, with a live dispatcher.

### 3.5 External Dependencies

Products registers LocalReferenceRegistry::for_owner("pricing") during init under the
PricingReferenceRegistry ClientHub key. Pricing lazily resolves ReferenceRegistryV1 at each use; absence
returns 503 REGISTRY_UNAVAILABLE and does not prevent boot. Ownership is bound by Products, never by
request input. This is an explicit same-binary, same-deployment trust boundary. Calls use the caller's
subject for audit and record the bound owner. PRICING_SYSTEM_ACTOR with subject type bss-pricing.system
uses only the operation tenant's scope and is audited as system; other system subjects are refused.
A future out-of-process transport uses the REST reference_principals mapping and pricing's configured
service_principal_id credentials with the same semantics; that transport is not implemented here.
The fresh sku_for_write read accepts every lifecycle; sku_version_as_of resolves the version in force
at each price start for the D-402 pair guard.
Reserve is idempotent on the live logical reference, not on a released receipt. The same tenant and SKU must be
bound to the receipt and object. Products versions?as_of provides descriptor history in phase 4. There is no
SkuChanged listener/local SKU read model in phase 2 (D-399). Subscriptions migration execution and Rating
adaptation are independent programmes; approval here cannot claim a subscription moved.

### 3.6 Interactions & Sequences

#### Publish changes

**ID**: `cpt-cf-bss-pricing-seq-publish-changes`

```mermaid
sequenceDiagram
  actor Author
  actor Reviewer
  participant Pricing
  participant DB
  Author->>Pricing: Select draft prices and common_effective_date
  Pricing->>DB: Replay check; transaction claim, collect, validate, unit, items, conditional ownership
  Pricing->>DB: Submission audit; quorum zero applies in same transaction
  Pricing-->>Author: Unit snapshot and generation
  Reviewer->>Pricing: Approve with generation and new client key
  Pricing->>DB: Conditional unit version; check SoD; recollect fingerprint
  alt Content drift
    Pricing->>DB: Refresh items/snapshot/hash; bump generation; stale earlier votes; commit receipt
    Pricing-->>Reviewer: UNIT_STALE with new generation
  else Quorum reached
    Pricing->>DB: Revalidate chains; normalize windows; approve; audit and toolkit outbox; commit
    Pricing-->>Reviewer: Approved
  else More votes needed
    Pricing->>DB: Commit current-generation vote
    Pricing-->>Reviewer: Pending
  end
```

A generation mismatch counts no vote. Apply failures roll back. Unit writes use version predicates and entry
chains are acquired in ascending id before prices, preventing inverse ordering across batches. Rejection and
withdrawal clear only owned pending locks and emit the terminal event without PricesPublished.

#### Reserve, write, confirm

**ID**: `cpt-cf-bss-pricing-seq-reserve-write-confirm`

```mermaid
sequenceDiagram
  participant Caller
  participant Pricing
  participant Products
  participant DB
  participant Retry
  Caller->>Pricing: Create entry, Idempotency-Key
  Pricing->>DB: Replay first; Tx A claim key, mint price_book_entry_id, create op reserving
  Pricing->>Products: Reserve SKU reference (price_book_entry, price_book_entry_id), idempotently
  Products-->>Pricing: reservation_id or fence/unavailable error
  Pricing->>Products: Re-read SKU type and lifecycle
  alt Write permitted
    Pricing->>DB: Tx B entry + reservation_id + reference_state confirmation_pending; op written
    Pricing->>Products: Confirm receipt
    alt Confirmation success
      Pricing->>DB: Tx C entry confirmed, op done, key answered
    else Timeout or transient failure
      Retry->>Products: Resume written op with bounded backoff; never release on timeout
    else REFERENCE_RELEASED or 404 unknown reservation
      Pricing->>DB: Entry stays confirmation_pending, rereserve op, op done, key answered
    end
  else Refusal after reserve
    Pricing->>DB: Op cancelling, persist outcome
    Retry->>Products: Release after cancellation is durable
    Retry->>DB: Op done, key answered
  end
```

Concurrent replays resolve to the same logical object or a nonmutating conflict. Tx A durably names the
reference before reserve: a crash between reserve and Tx B is recoverable by repeating the idempotent reserve.
An unknown commit outcome is reconciled before cancellation. Deletion removes the entry and inserts a
delete op in releasing in one transaction, then release finishes the op. Every op not done is retried
with bounded backoff and never dropped. The ticker also checks confirmed entries through states(): a released
receipt on a live entry starts a rereserve op when the SKU is not fenced, else the entry becomes lost,
new prices fail ENTRY_REFERENCE_LOST and PriceBookEntryReferenceLost is emitted. A receipt released before its confirm
starts the same op; lost entries are re-reserved once their SKU admits a reservation again. No timeout
releases a reservation.

#### Temporary pair

**ID**: `cpt-cf-bss-pricing-seq-temporary-pair`

```mermaid
sequenceDiagram
  actor Author
  participant Prices
  participant Approvals
  participant DB
  Author->>Prices: Draft temporary_until for one chain
  Prices->>DB: Read existing chain and versionAt at end
  alt Existing chain
    Prices->>DB: Persist promo plus return with same dim_value and pair links
  else Previously unowned value
    Prices->>DB: Persist one closed price; no return copy of default
  end
  Author->>Approvals: Submit atomic set; optional common date
  Approvals->>DB: Shift both dates preserving duration; validate; conditional lock
  Approvals->>DB: On apply re-read chains; normalize per value; audit/outbox; commit
```

#### Blocked revision (phase 3)

**ID**: `cpt-cf-bss-pricing-seq-blocked-revision`

```mermaid
sequenceDiagram
  actor Manager
  participant Plans
  participant Prices
  participant Approvals
  Manager->>Plans: Check draft revision for sale date
  Plans->>Prices: Coverage for every item and dimension value
  Prices-->>Plans: Uncovered values and pending_unit_id candidates
  Plans-->>Manager: ITEM_UNCOVERED with computed blocked_by
  Manager->>Approvals: Publish covering prices unit
  Approvals->>Prices: Approve money independently
  Manager->>Plans: Recheck and submit revision
  Plans->>Approvals: Separate plan_revision unit only when checks pass
```

blocked_by is never a stored dependency graph or automatic submission trigger. Plan and price approval outcomes
remain independent. Rejecting the revision leaves the approved money visible to existing revisions on the book.

### 3.7 Database schemas & tables

This is the target Postgres schema shape in bss; SQLite omits bss., maps uuid/date/timestamptz/jsonb to text and
bytea to blob, preserving checks and indexes. Mutable tenant entities carry concurrency versions and timestamps.
Tenant-scoped parent checks accompany entity-id foreign keys. Approval children are accessed through scoped units.
The actual migrations are authored in phase 2c with schema goldens on both backends, not in Part 2a.

The four approval tables are exactly `bss_approval::ddl::up` with prefix `pricing_` (spec §6 with §2.2
corrections: no unit idempotency_key or unique key index); the schema goldens pin that shape on both backends.
The shared DDL supports all Pricing subject kinds; only prices is executable in phase 2. The plan, revision
and item tables are phase 3's; they follow the reference-op prose below and slice 04 describes them. The promotion
(D-409) and migration-request (D-410) tables are deferred by the owner.

The chain starts with the schema guard m0000_pricing_refuse_a_legacy_or_stale_schema (D-423). Its name sorts it
before every other migration of the gear, including the coordination, broker and outbox ones, and it creates
nothing. It reads the catalog only and refuses to migrate a database that holds a legacy pricing_* table (the
pre-PriceBook chain's tables minus today's, a constant in the guard) or a stale shape: pricing_reference_op
without ref_kind, pricing_price_row, or pricing_price without price_book_entry_id. Boot then fails with
"bss-pricing: this database holds a legacy|stale bss-pricing schema (…); PriceBook does not migrate it — start
from an empty data root / empty bss-pricing tables". Fresh databases and databases migrated by today's chain pass.

```sql
CREATE TABLE bss.pricing_settings (
  tenant_id uuid PRIMARY KEY, default_timing text NOT NULL CHECK (default_timing IN ('advance','arrears')),
  default_rounding text NOT NULL, default_gl text, default_tax_category text,
  invoice_line_templates jsonb NOT NULL, version bigint NOT NULL DEFAULT 1,
  created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL
);
CREATE TABLE bss.pricing_dimension_key (
  tenant_id uuid NOT NULL, key text NOT NULL, "values" jsonb NOT NULL,
  version bigint NOT NULL DEFAULT 1, PRIMARY KEY (tenant_id, key)
);
CREATE TABLE bss.pricing_price_book (
  id uuid PRIMARY KEY, tenant_id uuid NOT NULL, code text NOT NULL, name text NOT NULL,
  currency char(3) NOT NULL, valid_from date, valid_until date, version bigint NOT NULL DEFAULT 1,
  created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL,
  UNIQUE (tenant_id, code), UNIQUE (tenant_id, id),
  CHECK (valid_from IS NULL OR valid_until IS NULL OR valid_from < valid_until)
);
CREATE TABLE bss.pricing_approval_policy (
  tenant_id uuid NOT NULL, kind text NOT NULL, quorum integer NOT NULL CHECK (quorum >= 0),
  PRIMARY KEY (tenant_id, kind)
);
CREATE TABLE bss.pricing_approval_unit (
  id uuid PRIMARY KEY, tenant_id uuid NOT NULL, kind text NOT NULL, ref_type text NOT NULL, ref_id uuid NOT NULL,
  state text NOT NULL CHECK (state IN ('pending','approved','rejected','withdrawn')),
  common_effective_date date, quorum_required integer NOT NULL,
  generation integer NOT NULL DEFAULT 1, submitted_by uuid NOT NULL, submitted_at timestamptz NOT NULL,
  decided_at timestamptz, decided_note text, snapshot jsonb NOT NULL, snapshot_hash text NOT NULL,
  version bigint NOT NULL DEFAULT 1
);
CREATE INDEX ix_pricing_approval_unit_queue ON bss.pricing_approval_unit USING btree
  (tenant_id, state, kind, submitted_at);
CREATE TABLE bss.pricing_approval_unit_item (
  unit_id uuid NOT NULL REFERENCES bss.pricing_approval_unit(id), tenant_id uuid NOT NULL, item_type text NOT NULL,
  item_id uuid NOT NULL, created_by uuid NOT NULL, before_json jsonb, after_json jsonb NOT NULL,
  PRIMARY KEY (unit_id, item_type, item_id)
);
CREATE TABLE bss.pricing_approval_decision (
  unit_id uuid NOT NULL REFERENCES bss.pricing_approval_unit(id), tenant_id uuid NOT NULL, actor uuid NOT NULL,
  generation integer NOT NULL, decision text NOT NULL CHECK (decision IN ('approve','reject')), note text,
  at timestamptz NOT NULL, stale boolean NOT NULL DEFAULT false, PRIMARY KEY (unit_id, actor, generation)
);
CREATE TABLE bss.pricing_price_book_entry (
  id uuid PRIMARY KEY, tenant_id uuid NOT NULL, book_id uuid NOT NULL REFERENCES bss.pricing_price_book(id),
  sku_id uuid NOT NULL, charge_kind text NOT NULL CHECK (charge_kind IN ('recurring','usage','one_time')),
  period text, dimension_key text, invoice_line_override text,
  reservation_id uuid NOT NULL,
  reference_state text NOT NULL CHECK (reference_state IN ('confirmation_pending','confirmed','lost')),
  version bigint NOT NULL DEFAULT 1,
  created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL,
  FOREIGN KEY (tenant_id, dimension_key) REFERENCES bss.pricing_dimension_key(tenant_id, key),
  CHECK ((charge_kind = 'recurring' AND period IS NOT NULL AND period IN ('month','year'))
    OR (charge_kind IN ('usage','one_time') AND period IS NULL))
);
CREATE UNIQUE INDEX pricing_price_book_entry_key ON bss.pricing_price_book_entry (book_id, sku_id, charge_kind, coalesce(period, ''));
CREATE TABLE bss.pricing_price (
  id uuid PRIMARY KEY, tenant_id uuid NOT NULL, price_book_entry_id uuid NOT NULL REFERENCES bss.pricing_price_book_entry(id),
  version_no integer NOT NULL, dim_value text,
  model text NOT NULL CHECK (model IN ('flat','per_unit','graduated','volume','package')), price_json jsonb NOT NULL,
  min_fee text CHECK (min_fee ~ '^[0-9]+(\.[0-9]+)?$'), eligibility text NOT NULL CHECK (eligibility IN ('all','new')),
  effective_from date NOT NULL, effective_to date, keep_for_bound boolean NOT NULL DEFAULT false,
  closed_explicitly boolean NOT NULL DEFAULT false,
  temporary_until date, paired_price_id uuid REFERENCES bss.pricing_price(id),
  return_of_price_id uuid REFERENCES bss.pricing_price(id),
  state text NOT NULL CHECK (state IN ('draft','pending','approved','rejected')),
  pending_unit_id uuid REFERENCES bss.pricing_approval_unit(id),
  approved_by_unit_id uuid REFERENCES bss.pricing_approval_unit(id), note text, created_by uuid NOT NULL,
  approved_at timestamptz, version bigint NOT NULL DEFAULT 1,
  created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL,
  UNIQUE (price_book_entry_id, version_no), CHECK (dim_value IS NULL OR dim_value <> ''),
  CHECK (effective_to IS NULL OR effective_from < effective_to)
);
CREATE UNIQUE INDEX pricing_price_approved_start
  ON bss.pricing_price (price_book_entry_id, coalesce(dim_value, ''), effective_from) WHERE state = 'approved';
CREATE INDEX pricing_price_chain
  ON bss.pricing_price (price_book_entry_id, dim_value, effective_from) WHERE state = 'approved';
CREATE TABLE bss.pricing_reference_op (
  op_id uuid PRIMARY KEY, tenant_id uuid NOT NULL,
  kind text NOT NULL CHECK (kind IN ('create','delete','rereserve','attach')),
  ref_kind text NOT NULL CHECK (ref_kind IN ('price_book_entry','plan_item')),
  ref_id uuid NOT NULL, sku_id uuid NOT NULL, reservation_id uuid,
  idempotency_key text,
  state text NOT NULL CHECK (state IN ('reserving','written','cancelling','releasing','done')),
  outcome text, attempts integer NOT NULL DEFAULT 0, next_attempt_at timestamptz NOT NULL,
  last_error text, created_by uuid NOT NULL, created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL
);
CREATE INDEX pricing_reference_op_due ON bss.pricing_reference_op (state, next_attempt_at) WHERE state <> 'done';
```

Policy kind is '*' or a registered pricing kind; a missing default fails safe to quorum 1. Subject validation
owns kind legality, quorum snapshots and item typing. The pending-unit column plus version predicate admits
one unit per element. Same-start uniqueness is enforced by the index; general non-overlap is enforced by the
serializable approve transaction re-reading each chain, with entries and prices ordered by id. Approved money
cannot be edited/deleted; only controlled window normalization and keep_for_bound changes are allowed.
The reference op is durable before reserve and has no FK to its reference: it survives cancellation and removal.
An op names its reference as (ref_kind, ref_id), a price book entry or a plan item (D-407); phase 3 edited
m20260926_000006 in place for this (D-412).
The ticker resumes every op not done with bounded backoff and never drops one. It reconciles confirmed entries
through states(): released receipts are re-reserved when the SKU is not fenced, otherwise the entry becomes
lost, refuses new prices with ENTRY_REFERENCE_LOST and emits PriceBookEntryReferenceLost (D-401). Plan items
use the same machine, with their own cursor: an item's write re-reads its revision (an unlocked draft) and its
entry (of the revision's book, for the item's SKU), so SSI orders it against a submit, a book change or a
delete (D-407); an attach of a copied item admits a deprecated SKU, a losing refusal makes the item lost and
any other refusal is retried, as for a rereserve, unless Products gave it to the ticker's system actor, which
Products authorizes to the tenant: that refusal is about the SKU and loses the item (D-413);
a lost item emits PlanReferenceLost and is re-reserved once its SKU admits a reservation again.
Settings and dimension values are versioned direct edits; invalid keys/value lists fail domain validation.

Phase 3 adds m20260926_000010 to m20260926_000012 (D-412). Every partial unique index is its own CREATE UNIQUE
INDEX … WHERE statement on both dialects, never inline. included_qty is canonical decimal text, like min_fee. A
plan item's reference starts unreserved when it is copied (D-413), and reservation_id is null until a reserve
answers; the reference columns of an item in a published or superseded revision change only through the
reference machine.

```sql
CREATE TABLE bss.pricing_plan (
  id uuid PRIMARY KEY, tenant_id uuid NOT NULL, code text NOT NULL, name text NOT NULL, published_rev integer,
  version bigint NOT NULL DEFAULT 1, created_by uuid NOT NULL,
  created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL
);
CREATE UNIQUE INDEX pricing_plan_code ON bss.pricing_plan (tenant_id, code);
CREATE TABLE bss.pricing_plan_revision (
  id uuid PRIMARY KEY, tenant_id uuid NOT NULL, plan_id uuid NOT NULL REFERENCES bss.pricing_plan(id),
  rev_no integer NOT NULL, book_id uuid NOT NULL REFERENCES bss.pricing_price_book(id), state text NOT NULL,
  available_from date, pending_unit_id uuid REFERENCES bss.pricing_approval_unit(id),
  approved_by_unit_id uuid REFERENCES bss.pricing_approval_unit(id), published_at timestamptz,
  version bigint NOT NULL DEFAULT 1, created_by uuid NOT NULL,
  created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL,
  CONSTRAINT pricing_plan_revision_no UNIQUE (plan_id, rev_no),
  CONSTRAINT chk_pricing_plan_revision_state CHECK (state IN ('draft','pending','published','superseded'))
);
CREATE UNIQUE INDEX pricing_plan_revision_open ON bss.pricing_plan_revision (plan_id)
  WHERE state IN ('draft','pending');
CREATE UNIQUE INDEX pricing_plan_revision_published ON bss.pricing_plan_revision (plan_id)
  WHERE state = 'published';
CREATE TABLE bss.pricing_plan_item (
  id uuid PRIMARY KEY, tenant_id uuid NOT NULL, revision_id uuid NOT NULL REFERENCES bss.pricing_plan_revision(id),
  sku_id uuid NOT NULL, price_book_entry_id uuid REFERENCES bss.pricing_price_book_entry(id),
  treatment text NOT NULL, included_qty text, qty_min integer, reservation_id uuid, reference_state text NOT NULL,
  version bigint NOT NULL DEFAULT 1, created_by uuid NOT NULL,
  created_at timestamptz NOT NULL, updated_at timestamptz NOT NULL,
  CONSTRAINT pricing_plan_item_sku UNIQUE (revision_id, sku_id),
  CONSTRAINT chk_pricing_plan_item_treatment CHECK (treatment IN ('paid','optional','included')),
  CONSTRAINT chk_pricing_plan_item_entry CHECK (treatment = 'included' OR price_book_entry_id IS NOT NULL),
  CONSTRAINT chk_pricing_plan_item_included_qty CHECK (included_qty ~ '^[0-9]+(\.[0-9]+)?$'),
  CONSTRAINT chk_pricing_plan_item_qty_min CHECK (qty_min >= 0),
  CONSTRAINT chk_pricing_plan_item_reference_state
    CHECK (reference_state IN ('unreserved','confirmation_pending','confirmed','lost'))
);
```

Unique conflicts map to stable codes: PLAN_CODE_TAKEN, REVISION_NO_TAKEN, REVISION_DRAFT_EXISTS (the open-revision
index), REVISION_PUBLISHED_EXISTS and ITEM_SKU_TAKEN. SQLite names only the columns of a partial index, so the two
single-column revision indexes are told apart by the state the write sets. Deferred by the owner and not in the
chain: the promotion tables (D-409), the migration-request table and the plan's retiring state (D-410), and the
plan's bundle SKU with its unique index, the revision's grants and its sold-as columns (D-411); slices 04 and 06
keep their planned shape.

Audit and idempotency use the Products document's retained shapes, renamed pricing_. The audit trigger permits
only the reserved one-way seal transition without changing record facts. The following is schema, not a new
pricing-owned outbox: the actual event schema comes from toolkit migrations under bss_pricing_outbox (D-400).

```sql
CREATE TABLE bss.pricing_audit (
            audit_id          uuid        NOT NULL,
            tenant_id         uuid        NOT NULL,
            actor_ref         uuid        NOT NULL,
            action            text        NOT NULL,
            subject_kind      text        NOT NULL,
            subject_id        uuid,
            subject_revision  bigint,
            error_code        text,
            attempted_key     text,
            reason            text,
            correlation_id    text,
            written_at        timestamptz NOT NULL,
            session_id        uuid,
            ceremony_ref      uuid,
            seal_state        text        NOT NULL,
            chain_id          uuid,
            seq               bigint,
            prev_hash         bytea,
            row_hash          bytea,
            CONSTRAINT pricing_audit_pkey PRIMARY KEY (audit_id),
            CONSTRAINT chk_pricing_audit_seal_state CHECK (seal_state IN ('unsealed', 'sealed')),
            CONSTRAINT chk_pricing_audit_seal_group CHECK (
                (seal_state = 'unsealed' AND chain_id IS NULL AND seq IS NULL AND prev_hash IS NULL AND row_hash IS NULL)
                OR
                (seal_state = 'sealed' AND chain_id IS NOT NULL AND seq IS NOT NULL AND row_hash IS NOT NULL)
            ),
            CONSTRAINT chk_pricing_audit_seq CHECK (seq IS NULL OR seq >= 0),
            CONSTRAINT chk_pricing_audit_subject_ref CHECK (subject_id IS NOT NULL OR attempted_key IS NOT NULL OR session_id IS NOT NULL)
        );

CREATE INDEX idx_pricing_audit_tenant_time ON bss.pricing_audit USING btree (tenant_id, written_at);

CREATE INDEX idx_pricing_audit_subject ON bss.pricing_audit USING btree (tenant_id, subject_kind, subject_id, written_at);

CREATE INDEX idx_pricing_audit_actor ON bss.pricing_audit USING btree (tenant_id, actor_ref, written_at);

CREATE OR REPLACE FUNCTION bss.pricing_audit_append_only() RETURNS trigger AS $$
        BEGIN
          IF TG_OP = 'DELETE' THEN
            RAISE EXCEPTION 'pricing_audit is append-only: DELETE is not permitted';
          END IF;

          IF OLD.seal_state = 'unsealed'
             AND NEW.seal_state = 'sealed'
             AND NEW.chain_id IS NOT NULL
             AND NEW.seq IS NOT NULL
             AND NEW.row_hash IS NOT NULL
             AND NEW.audit_id IS NOT DISTINCT FROM OLD.audit_id
             AND NEW.tenant_id IS NOT DISTINCT FROM OLD.tenant_id
             AND NEW.actor_ref IS NOT DISTINCT FROM OLD.actor_ref
             AND NEW.action IS NOT DISTINCT FROM OLD.action
             AND NEW.subject_kind IS NOT DISTINCT FROM OLD.subject_kind
             AND NEW.subject_id IS NOT DISTINCT FROM OLD.subject_id
             AND NEW.subject_revision IS NOT DISTINCT FROM OLD.subject_revision
             AND NEW.error_code IS NOT DISTINCT FROM OLD.error_code
             AND NEW.attempted_key IS NOT DISTINCT FROM OLD.attempted_key
             AND NEW.reason IS NOT DISTINCT FROM OLD.reason
             AND NEW.correlation_id IS NOT DISTINCT FROM OLD.correlation_id
             AND NEW.written_at IS NOT DISTINCT FROM OLD.written_at
             AND NEW.session_id IS NOT DISTINCT FROM OLD.session_id
             AND NEW.ceremony_ref IS NOT DISTINCT FROM OLD.ceremony_ref
          THEN
            RETURN NEW;
          END IF;

          RAISE EXCEPTION 'pricing_audit is append-only: % is not permitted', TG_OP;
        END;
     $$ LANGUAGE plpgsql;

CREATE TRIGGER trg_pricing_audit_append_only BEFORE DELETE OR UPDATE ON bss.pricing_audit FOR EACH ROW EXECUTE FUNCTION bss.pricing_audit_append_only();
```

```sql
CREATE TABLE bss.pricing_idempotency (
            tenant_id       uuid        NOT NULL,
            endpoint        text        NOT NULL,
            client_key      text        NOT NULL,
            state           text        NOT NULL,
            payload_hash    bytea       NOT NULL,
            response_status integer,
            response_body   jsonb,
            expires_at      timestamptz NOT NULL,
            entity_ref      uuid,
            CONSTRAINT pricing_idempotency_pkey PRIMARY KEY (tenant_id, endpoint, client_key),
            CONSTRAINT chk_pricing_idempotency_state CHECK (state IN ('claimed', 'answered')),
            CONSTRAINT chk_pricing_idempotency_response_group CHECK (
                (state = 'claimed' AND response_status IS NULL AND response_body IS NULL)
                OR
                (state = 'answered' AND response_status IS NOT NULL AND response_body IS NOT NULL)
            )
        );

CREATE INDEX idx_pricing_idempotency_expires ON bss.pricing_idempotency USING btree (tenant_id, expires_at);
```

The replay store retains responses for 24 hours and rejects payload mismatches. An unknown transaction outcome
is reconciled by replay/entity_ref before compensation. Audit inserts and terminal events use the same transaction;
no separate connection can make a failed act look successful. Implement append-only audit guards on SQLite too.

## 4. Additional context

The old 22-file set remains on bss/products-backup and in history. Seven slices map one-to-one to FEATUREs.
Part 2a writes all phases' contracts; phase 2b removes the legacy implementation; phase 2c builds only the core.
The prototype is the pure-model reference except where the spec corrects it, especially half-open tier bands.

Known limits (accepted by the owner). In broker mode (an EventBrokerApi is registered) the event-broker SDK's
ProducerOutbox::enqueue turns the outbox's database error into a string, so a contended outbox insert fails the
act with 500 instead of being retried by the transaction; Products has the same limit, and the SDK stays unchanged.

## 5. Traceability

| Slice | Feature | Requirements / delivery |
| --- | --- | --- |
| 01 Foundation | foundation | `cpt-cf-bss-pricing-nfr-authz`, `cpt-cf-bss-pricing-nfr-audit`, `cpt-cf-bss-pricing-nfr-tenant-isolation`, `cpt-cf-bss-pricing-nfr-two-backends`, `cpt-cf-bss-pricing-nfr-idempotency-concurrency`; phase 2. |
| 02 Books & Entries | books-entries | `cpt-cf-bss-pricing-fr-dimension-registry`, `cpt-cf-bss-pricing-fr-price-book`, `cpt-cf-bss-pricing-fr-entry-key`, `cpt-cf-bss-pricing-fr-book-export`, `cpt-cf-bss-pricing-fr-settings`; phase 2. |
| 03 Prices, Windows & Dimension | prices-windows-dimension | `cpt-cf-bss-pricing-fr-price`, `cpt-cf-bss-pricing-fr-chain-windows`, `cpt-cf-bss-pricing-fr-pair-guard`, `cpt-cf-bss-pricing-fr-min-fee`, `cpt-cf-bss-pricing-fr-temporary-pair`, `cpt-cf-bss-pricing-fr-reference-protocol`; phase 2. |
| 04 Plans | plans | `cpt-cf-bss-pricing-fr-plans`; phase 3. |
| 05 Approvals | approvals | `cpt-cf-bss-pricing-fr-publish-changes`, `cpt-cf-bss-pricing-fr-approval-units`; phase 2. |
| 06 Promotions & Migrations | promotions-migrations | `cpt-cf-bss-pricing-fr-promotions`, `cpt-cf-bss-pricing-fr-migrations`; deferred by the owner (D-409, D-410). |
| 07 Read Contract & Events | read-contract-events | `cpt-cf-bss-pricing-fr-events`, `cpt-cf-bss-pricing-fr-resolve`, `cpt-cf-bss-pricing-fr-price-read`, `cpt-cf-bss-pricing-fr-quote`; phase 4 (core events in phase 2; quote not built, D-415). |

All four ADRs are cited in §1.2. [PRD](PRD.md) owns requirements; [DECISIONS](DECISIONS.md) owns D-384–D-425.
Source: `docs/superpowers/specs/2026-09-24-pricebook-model-design.md`, §2.2, §5–§8, §12–§13.
