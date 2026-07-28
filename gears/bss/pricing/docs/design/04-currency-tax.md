<!-- CONFLUENCE_TITLE: [BSS]: Pricing — Multi-Currency, Regions & Tax Display (Design, Slice 4) -->
<!-- Related: ../PRD.md, ../DESIGN.md, ./01-foundation.md | Owners: BSS Product Catalog team -->

# DESIGN — Multi-Currency, Regions & Tax Display (Slice 4)

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
  - [Author Multi-Currency Price Rows](#author-multi-currency-price-rows)
  - [Preview a Base Price](#preview-a-base-price)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Region and Brand Taxonomy Validation](#region-and-brand-taxonomy-validation)
  - [Tax Display Basis and Policy](#tax-display-basis-and-policy)
  - [Single-Currency-per-Invoice Binding](#single-currency-per-invoice-binding)
- [4. States (CDSL)](#4-states-cdsl)
  - [Tax-Inclusive Sellability State Machine](#tax-inclusive-sellability-state-machine)
- [5. API Surface](#5-api-surface)
- [6. Data Model](#6-data-model)
- [7. Events & Alarms](#7-events--alarms)
- [8. Definitions of Done](#8-definitions-of-done)
  - [Multi-Currency Rows](#multi-currency-rows)
  - [Taxonomy Validation](#taxonomy-validation)
  - [Tax Display Basis](#tax-display-basis)
  - [Currency Binding](#currency-binding)
- [9. Acceptance Criteria](#9-acceptance-criteria)
- [10. Non-Functional Considerations](#10-non-functional-considerations)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice owns the **market axes of a price row**: independent per-`(currency, region)` rows
(no FX derivation, ever), the tenant-configured **region and brand taxonomies** with
membership validation before publish, the **`taxInclusive` display basis** + `taxCategory`
reference governed by the fail-closed tenant tax-display policy (with the tax-inclusive
**not-sellable-GA** gate while Tax Engine is post-MVP), and the
**single-currency-per-invoice binding** checks that reject configurations forcing
mixed-currency lines onto one invoice. It registers its rules into the Foundation pipeline;
tax **scheme determination and calculation** are explicitly not here (Tax Engine).

**Traces to**: `cpt-cf-bss-pricing-fr-multi-currency-rows`,
`cpt-cf-bss-pricing-fr-region-brand-taxonomy`, `cpt-cf-bss-pricing-fr-tax-display-basis`,
`cpt-cf-bss-pricing-fr-invoice-currency-binding`, `cpt-cf-bss-pricing-fr-price-amount-validation`

### 1.2 Purpose

Let a tenant sell one plan in many markets — ≥ 20 currencies per plan as a guaranteed floor —
with every market row first-class and independently authored, while making the two classic
failure classes impossible at publish: a price resolved through implicit FX (silently wrong
amount) and an invoice forced to mix currencies (unpostable downstream). Tax display stays a
catalog concern; everything else about tax is delegated.

### 1.3 Actors

| Actor | Role in Slice |
|-------|---------------|
| `cpt-cf-bss-pricing-actor-finance-manager` | Authors per-`(currency, region)` rows and `taxInclusive` flags |
| `cpt-cf-bss-pricing-actor-catalog-admin` | Configures the region/brand taxonomies and the tax-display policy |
| `cpt-cf-bss-pricing-actor-tax-engine` | (Post-MVP) consumes `taxInclusive` + `taxCategory`; maps `region` → tax jurisdiction |
| `cpt-cf-bss-pricing-actor-billing` | Depends on the single-currency-per-invoice invariant |
| `cpt-cf-bss-pricing-actor-partner` | Reads the base-price preview (with the preview grant, Slice 5) |
| `cpt-cf-bss-pricing-actor-subscriptions` | Owns currency selection at activation (consumes covered currencies) |

### 1.4 References

- **PRD**: [PRD.md](../PRD.md) — §6.4, §4.1 (environment: precision, taxonomies), §17.4 (currency-coverage / precision / tax-basis rules), §15 (Tax Engine post-MVP)
- **Design**: [01-foundation.md](./01-foundation.md) — money constraint, scope key (§4.1), policy objects
- **Dependencies**: Foundation (Slice 1), plan-definition (Slice 2). The brand taxonomy authored here is consumed by price-overlays (Slice 9: brand-scoped `PriceOverlay`); currency-coverage checks extend to bundles in Slice 8.

### 1.5 Scope

**In scope**: per-`(currency, region)` price rows + the ≥ 20-currency floor; region taxonomy
(price-row axis) and brand taxonomy (PriceOverlay scope value) validation; `taxInclusive` /
`taxCategory` persistence + the tenant tax-display policy (default fail-closed); the
tax-inclusive not-sellable-GA gate; the three enumerated currency-binding rejections; the
fail-closed base-price preview read on `(currency, region)`.

**Out of scope**: FX math and any currency conversion (Tariffs/PLAL; `currencyFallbackPolicy`
is Future); tax scheme/calc and `region` → jurisdiction mapping (Tax Engine, post-MVP);
currency selection at activation (Subscriptions); brand-scoped `PriceOverlay` authoring
(Slice 9); bundle currency coverage detail (Slice 8, reuses this slice's checker).

### 1.6 Constraints & Assumptions

Inherits Foundation C-set (ISO 4217 minor units, `≥ 0`, no implicit FX, UTC, tenant isolation). Slice-4-specific:

| # | Topic | Assumption (default) | Source |
|---|-------|----------------------|--------|
| C1 | Currency floor | ≥ 20 currencies per plan guaranteed; the read SLO (p95 < 100ms) holds at that floor | PRD §6.4 |
| C2 | Taxonomies are tenant config | `region` and `brand` value sets are tenant-configured taxonomies; membership is validated at save/publish (unknown value fails before publish) | PRD §4.1 |
| C3 | Tax Engine post-MVP | MVP sells **tax-exclusive**; a `taxInclusive=true` row is authorable but flagged **not-sellable-GA** (per row / market) until Tax Engine GA (ETA ~8 months) | PRD §15 |
| C4 | Tax-display policy default | Fail-closed for **all** tenants: `taxInclusive=true` without region tax readiness, and `taxInclusive=false` in a region with no configured `taxCategory`, both block publish unless the tenant policy explicitly selects warn. The check's input is the **`RegionTaxReadiness` lookup** — `(tenant, region) → { taxCategory, ratePresent }`, fail-closed on unknown; MVP provider = tenant-declared columns on `pricing_region_taxonomy` (§6), post-GA provider = Tax Engine-backed (contract lands in the Tax Engine PRD) | PRD §17.6; D-01 |
| C5 | Region ≠ authz region | Pricing `region` is a commercial territory; the IdP authorization-region claim governs *who may mutate*, enforced in Slice 5 | PRD §1.4 |

### 1.7 Naming & Design-Introduced Names

Reuses the PRD glossary; inherits Foundation mechanics. Not restated.

Design-introduced names (Slice 4):

| Name | Meaning |
|------|---------|
| `TaxonomyValidator` | Registered rules: `region` membership on price rows; `brand` membership on brand-scoped `PriceOverlay`s (rule shared with Slice 9) |
| `TaxDisplayValidator` | Registered rules: `taxInclusive`/`taxCategory` completeness under the tenant tax-display policy (C4) + the GA gate (C3) |
| `CurrencyBindingChecker` | Registered rules: the three enumerated mixed-currency rejection configs (§3); reused by Slice 8 for bundles |
| `not_sellable_ga` | Read-model flag on a tax-inclusive **price row** (⇒ per `(currency, region)` market) while Tax Engine is pre-GA: authorable, previewable, **not sellable** on that market |
| `RegionTaxReadiness` | The C4 input port: `(tenant, region) → { taxCategory, ratePresent }`, fail-closed on unknown. MVP provider: tenant-declared columns on `pricing_region_taxonomy`; post-GA provider: Tax Engine-backed (sync lookup or event-fed mirror — decided in the Tax Engine PRD) |

### 1.8 Context & Dependencies

```mermaid
flowchart TB
    subgraph s4["Slice 4 — Currency & Tax Display"]
        TXV["TaxonomyValidator"]
        TDV["TaxDisplayValidator"]
        CBC["CurrencyBindingChecker"]
    end
    CFG[("Tenant taxonomies<br/>region · brand<br/>+ tax-display policy")]
    FND["Foundation (Slice 1)<br/>ValidationPipeline · pricing_policy_object"]
    TAX["Tax Engine (post-MVP)<br/>scheme · region→jurisdiction"]
    CFG --> s4
    TXV --> FND
    TDV --> FND
    CBC --> FND
    FND -. taxInclusive · taxCategory .-> TAX
```

**Consumed:** tenant region/brand taxonomies + the tax-display policy
(`pricing_policy_object`, Foundation). **Produced:** validated per-market rows, the
`not_sellable_ga` flag, and the currency-coverage guarantee the sellability gate (Slice 7)
and bundles (Slice 8) build on.

## 2. Actor Flows (CDSL)

### Author Multi-Currency Price Rows

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-flow-multicurrency-author`

**Actor**: `cpt-cf-bss-pricing-actor-finance-manager`

**Success Scenarios**:
- Independent rows per `(currency, region)` attach to one `planId` (distinct scope keys), `PriceCreated` per row; ≥ 20 currencies supported per plan (C1)
- Each row's amount validates at its own currency's ISO 4217 minor unit (Foundation)

**Error Scenarios**:
- Unknown `region` → `REGION_UNKNOWN` (422, before publish)
- Precision above the currency's minor unit → `PRECISION_EXCEEDED` (422, Foundation)

**Steps**:
1. [ ] - `p1` - API: POST /v1/pricing/plans/{planId}/prices per `(currency, region)` (Slice 3 flow; this slice adds the market-axis rules) - `inst-mc-create`
2. [ ] - `p1` - `TaxonomyValidator` checks `region` membership at save **and** publish (C2) - `inst-mc-region`
3. [ ] - `p1` - No FX derivation ever: a missing `(currency, region)` row is simply absent — preview/publish paths fail closed on it, no base-currency fallback (Future `currencyFallbackPolicy` only) - `inst-mc-nofx`
4. [ ] - `p1` - **RETURN** 201 per row - `inst-mc-return`

### Preview a Base Price

- [ ] `p2` - **ID**: `cpt-cf-bss-pricing-flow-price-preview`

**Actor**: `cpt-cf-bss-pricing-actor-partner`, `cpt-cf-bss-pricing-actor-finance-manager` (requires the catalog-preview read grant, Slice 5)

**Success Scenarios**:
- Returns the catalog **base list price** for a `(region, currency)`: amount, `taxInclusive` flag, tier summary, `displayTrialDays`, with an explicit disclaimer that Contract/`PriceOverlays` apply at purchase (Tariffs evaluates)

**Error Scenarios**:
- No row for the requested `(currency, region)` → `PRICE_ROW_ABSENT` (404, fail closed — no FX)
- Principal without the preview grant → denied (403, audited; Slice 5)

**Steps**:
1. [ ] - `p2` - API: GET /v1/pricing/plans/{planId}/preview?currency=&region= - `inst-pv-api`
2. [ ] - `p2` - Resolve from the **published read model only** (no draft read); base `priceOverlay` only, overlays disclaimed - `inst-pv-resolve`
3. [ ] - `p2` - **RETURN** 200 (base price + disclaimer) or fail closed - `inst-pv-return`

## 3. Processes / Business Logic (CDSL)

### Region and Brand Taxonomy Validation

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-taxonomy`

**Input**: a price row's `region`; a brand-scoped `PriceOverlay`'s `brand` (Slice 9 registers into the same rule)
**Output**: pass, or a fail-closed violation naming the unknown value

**Steps**:
1. [ ] - `p1` - `region` MUST be a member of the tenant's configured region taxonomy; an unknown/invalid region fails validation **before** publish - `inst-tx-region`
2. [ ] - `p1` - `brand` is **not** a price-row field (Foundation §4.1): a brand-scoped `PriceOverlay`'s `brand` MUST be a member of the tenant's brand taxonomy, validated at save (rule owned here, exercised by Slice 9) - `inst-tx-brand`
3. [ ] - `p1` - Taxonomy mutation (add/retire a region/brand value) is tenant-admin config, audited; **retiring** a value is rejected while it is referenced by an active published price row (`region`) **or an active brand-scoped `PriceOverlay` scope (`brand`)** — referential integrity over both referencing shapes - `inst-tx-mutation`

### Tax Display Basis and Policy

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-tax-display`

**Input**: a row's `taxInclusive` + `taxCategory` + the tenant tax-display policy + the `RegionTaxReadiness` lookup (C4)
**Output**: pass / warn / fail per policy; the `not_sellable_ga` flag where applicable

**Steps**:
1. [ ] - `p1` - Persist `taxInclusive` (display basis) and the `taxCategory` reference **only** — no scheme determination, no calculation, no `region` → jurisdiction mapping (Tax Engine) - `inst-td-persist`
2. [ ] - `p1` - Policy check (C4): `taxInclusive=true` in a region whose `RegionTaxReadiness` has `ratePresent=false` **and** `taxInclusive=false` in a region with no configured `taxCategory` are both governed by the tenant tax-display policy — default **fail-closed**, explicit warn allowed. Readiness is resolved per `(tenant, region)`; an unknown region fails closed - `inst-td-policy`
2a. [ ] - `p1` - **Readiness provider (D-01):** at MVP `RegionTaxReadiness` reads the tenant-declared `tax_category`/`tax_rate_present` columns on `pricing_region_taxonomy` (CatalogAdmin, `config × write`, audited) — it catches **configuration** mistakes; rate correctness is unverifiable before Tax Engine. Once Tax Engine GAs, the provider becomes Tax Engine-backed and the tenant-declared markers are **reconciled** against its registry: a divergence (declared ready, engine disagrees) flags affected published rows + raises `pricing.tax.readiness_divergent` (Warn); remediation is a re-publish — never a silent retro-change - `inst-td-readiness`
3. [ ] - `p1` - **GA gate (C3):** while Tax Engine is pre-GA a `taxInclusive=true` **row** MAY be authored and previewed but publishes with the read-model flag `not_sellable_ga` — the flag is **per price row, hence per `(currency, region)` market**, not per plan: a plan selling tax-exclusive in US and tax-inclusive in EU is gated **only** on its EU market(s); the sellability gate (Slice 7) evaluates the flag per scope key; MVP sells tax-exclusive - `inst-td-gagate`
4. [ ] - `p1` - When Tax Engine GAs, clearing `not_sellable_ga` is a re-publish (goes through the pipeline + approval), not a silent flag flip - `inst-td-clear`

### Single-Currency-per-Invoice Binding

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-algo-currency-binding`

**Input**: a publishing plan + its add-ons / overrides / bundle composition
**Output**: pass, or the enumerated rejection naming the uncovered currency and the offending component

**Steps**:
1. [ ] - `p1` - **(i)** An add-on / price-override / **required**-add-on row lacking a row in a currency the base plan publishes → reject (the subscription could not resolve all lines in one bound currency) - `inst-cb-addon`
2. [ ] - `p1` - **(ii)** A `sum_of_parts` bundle whose component rows do not cover **every** currency the bundle sells → reject (Slice 8 invokes this rule with bundle context) - `inst-cb-bundle-sum`
3. [ ] - `p1` - **(iii)** An `own_price` bundle without a matching-currency component set → reject - `inst-cb-bundle-own`
4. [ ] - `p1` - Currency **selection** at activation is Subscriptions-owned; this slice guarantees only that every sellable currency is fully covered. `invoiceGroupingKey` is a layout hint and MUST NOT override this invariant (Billing splits currencies regardless) - `inst-cb-boundary`
5. [ ] - `p1` - **Region binding (normative, joint with Subscriptions):** like currency, the pricing `region` binds **once at activation** — Subscriptions resolves it from the payer's commercial profile (never from a client-supplied parameter) and the bound `(currency, region)` pair freezes into `pricingSnapshotRef`; every subsequent resolution (windows, renewals, overlays) uses the frozen pair. A region re-bind is a plan change, not a drift - `inst-cb-region-binding`

## 4. States (CDSL)

### Tax-Inclusive Sellability State Machine

- [ ] `p2` - **ID**: `cpt-cf-bss-pricing-state-tax-sellability`

**States** (per price row / market): sellable (tax-exclusive, or tax-inclusive post-Tax-Engine-GA), not_sellable_ga (tax-inclusive, pre-GA)
**Initial State**: per the row's `taxInclusive` and the Tax Engine GA status at publish

**Transitions**:
1. [ ] - `p1` - **FROM** not_sellable_ga **TO** sellable **WHEN** Tax Engine GAs **and** the row's plan is re-published through the pipeline (with approval); never a silent flip; per row/market - `inst-ts-ga`
2. [ ] - `p2` - The flag lives in the read model; the sellability gate (Slice 7) enforces it jointly with the window/version checks - `inst-ts-enforce`

## 5. API Surface

| Method | Path | Purpose | Idempotency |
|--------|------|---------|-------------|
| `GET` | `/v1/pricing/plans/{planId}/preview` | Base-price preview per `(currency, region)`; fail closed, overlay disclaimer | — |
| `GET/PUT` | `/v1/pricing/config/taxonomies/{region\|brand}` | Tenant taxonomy read/update (admin, audited) | ETag |
| `GET/PUT` | `/v1/pricing/config/tax-display-policy` | Tenant tax-display policy (fail-closed default) | ETag |

**Problem responses (RFC 9457):** `REGION_UNKNOWN` (422), `BRAND_UNKNOWN` (422),
`TAXONOMY_VALUE_IN_USE` (409, on retire), `TAX_BASIS_INCOMPLETE` (422, per policy),
`CURRENCY_NOT_COVERED` (422, naming component + currency), `PRICE_ROW_ABSENT` (404 preview,
fail closed). Price-row authoring codes are Slice 3's.

## 6. Data Model

Slice-owned tables (tenant-scoped, SecureORM; `pricing_` prefix per Foundation §3.7):

**`pricing_region_taxonomy`** / **`pricing_brand_taxonomy`** (PK `(tenant_id, value)`):

| Column | Type | Notes |
|--------|------|-------|
| `tenant_id` | `uuid` | RLS scope |
| `value` | `string` | the region / brand code |
| `display_name` | `string` | operator label |
| `state` | `enum` | `active \| retired`; retire rejected while referenced by an active published price row (`region`) or an active brand-scoped `PriceOverlay` scope (`brand`) |
| `tax_category` | `string` | **region taxonomy only** (D-01): the region's default tax category; a price row's `tax_category_ref` may override |
| `tax_rate_present` | `bool` | **region taxonomy only** (D-01): tenant-declared "a tax rate is configured for this region" — the MVP `RegionTaxReadiness` source; reconciled against Tax Engine post-GA |

**Tax-display policy** — a `pricing_policy_object` entry (Foundation-owned table):
`mode ∈ {fail_closed (default), warn}`; per-tenant.

**`pricing_price` (Foundation-owned; Slice-4 columns)** — `currency` (scope key), `region`
(scope key), `tax_inclusive` (bool), `tax_category_ref` (string), and the projected
`not_sellable_ga` flag in `pricing_read_model` (derived at publish, not authored).

Key constraints: FK-like validation (application-level, at save + publish) from
`pricing_price.region` to `pricing_region_taxonomy(active)`; the ≥ 20-currency floor is a
capacity guarantee (load-tested), not a schema constraint.

## 7. Events & Alarms

No new event names (`PriceCreated`/`PriceUpdated` per row; policy/taxonomy changes are
audited mutations). Alarm: `pricing.tax.not_sellable_ga_active` (Info gauge — count of
published tax-inclusive rows/markets awaiting Tax Engine GA; visibility for the GA-gate backlog,
PRD risk table).

## 8. Definitions of Done

### Multi-Currency Rows

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-multicurrency`

A plan **MUST** support independent price rows per `(currency, region)` on one `planId`
(≥ 20 currencies guaranteed, read SLO held at the floor), each validated at its currency's
ISO 4217 minor unit, with **no** FX derivation anywhere — a missing `(currency, region)` row
fails closed on preview and publish.

**Implements**: `cpt-cf-bss-pricing-flow-multicurrency-author`, `cpt-cf-bss-pricing-flow-price-preview`

**Touches**:
- API: `GET /v1/pricing/plans/{planId}/preview`
- DB: `pricing_price` (currency/region axes)
- Entities: `TaxonomyValidator`

### Taxonomy Validation

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-taxonomy`

`region` on price rows and `brand` on brand-scoped `PriceOverlay`s **MUST** validate against the
tenant taxonomies before publish (unknown value fails); retiring a referenced taxonomy value
**MUST** be rejected; taxonomy mutation is admin-scoped and audited.

**Implements**: `cpt-cf-bss-pricing-algo-taxonomy`

**Touches**:
- API: `GET/PUT /v1/pricing/config/taxonomies/*`
- DB: `pricing_region_taxonomy`, `pricing_brand_taxonomy`
- Entities: `TaxonomyValidator`

### Tax Display Basis

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-tax-display`

The catalog **MUST** persist `taxInclusive` + `taxCategory` only (no scheme/calc), govern the
two incomplete-basis cases by the tenant tax-display policy (default fail-closed), and flag
tax-inclusive rows `not_sellable_ga` (per market) until Tax Engine GA — cleared only by re-publish.

**Implements**: `cpt-cf-bss-pricing-algo-tax-display`, `cpt-cf-bss-pricing-state-tax-sellability`

**Touches**:
- API: `GET/PUT /v1/pricing/config/tax-display-policy`
- DB: `pricing_price` (tax columns), `pricing_policy_object`, `pricing_read_model` (`not_sellable_ga`)
- Entities: `TaxDisplayValidator`

### Currency Binding

- [ ] `p1` - **ID**: `cpt-cf-bss-pricing-dod-currency-binding`

Publish/preview **MUST** reject the three enumerated mixed-currency configurations
(add-on/override/required-add-on gap; `sum_of_parts` coverage gap; `own_price` mismatch),
naming the component and currency; `invoiceGroupingKey` never overrides the invariant.

**Implements**: `cpt-cf-bss-pricing-algo-currency-binding`

**Touches**:
- DB: `pricing_price`, `pricing_plan_addon_rule`
- Entities: `CurrencyBindingChecker`

## 9. Acceptance Criteria

Delta over the Foundation testing architecture.

Unit:

- [ ] Minor-unit precision matrix (JPY 0 / USD 2 / BHD 3; over-precision rejected); unknown region/brand rejection; the three currency-binding cases each rejected with the component named; tax-basis policy matrix (fail-closed vs warn × the two incomplete cases: `taxInclusive=true` with `tax_rate_present=false`, `taxInclusive=false` with no `tax_category`)

Integration (testcontainers):

- [ ] A plan with 20+ currency rows publishes and the preview reads each `(currency, region)` within the read SLO
- [ ] Preview of an absent `(currency, region)` fails closed (no base-currency fallback)
- [ ] A required add-on missing one of the base plan's currencies blocks publish (`CURRENCY_NOT_COVERED`)
- [ ] A mixed plan (EU `taxInclusive=true` with EU `tax_rate_present=true`, US exclusive) publishes with `not_sellable_ga` on the EU rows only — the US market stays sellable; the flag clears only via re-publish
- [ ] The same plan with EU `tax_rate_present=false` blocks publish under the default fail-closed policy (C4 precedes the C3 flag)
- [ ] Retiring a region referenced by an active published row is rejected (409)

API:

- [ ] RFC 9457 mapping for the §5 codes; preview carries the overlay disclaimer

## 10. Non-Functional Considerations

- **Performance**: the read/preview SLO (p95 < 100ms) holds at the 20-currency floor — preview is a single read-model lookup keyed by `(plan, currency, region)`; taxonomy checks are indexed lookups on the authoring path only.
- **Observability / metrics**: `pricing_preview_failclosed_total{reason}`, `pricing_currency_binding_blocks_total{case}`, `pricing_tax_not_sellable_ga` gauge.
- **Security & AuthZ**: taxonomy/policy mutation is CatalogAdmin-scoped and audited; preview requires the explicit preview grant (Slice 5); pricing `region` is decoupled from the authz-region claim (enforcement in Slice 5).
- **Risks & open items**: Tax Engine slip extends the `not_sellable_ga` window (PRD risk #1 — tracked on the program board); the MVP `RegionTaxReadiness` markers are self-declared — their post-GA reconciliation (divergence flag + `pricing.tax.readiness_divergent`) is part of the future Tax Engine contract (D-01); `currencyFallbackPolicy` deliberately deferred (fail-closed is the launch behavior); bundle currency coverage is exercised end-to-end only when Slice 8 lands.
