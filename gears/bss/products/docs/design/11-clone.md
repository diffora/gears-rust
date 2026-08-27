<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./03-sku-classification.md | Owners: BSS Product Catalog team -->

# DESIGN — Clone & Templating (Slice 11)

<!-- toc -->

- [1. Context](#1-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
  - [1.5 Scope](#15-scope)
  - [1.6 Constraints & Assumptions](#16-constraints--assumptions)
  - [1.7 Naming & Design-Introduced Names](#17-naming--design-introduced-names)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Clone an entity](#clone-an-entity)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 The disposition table](#31-the-disposition-table)
- [4. Data / Storage](#4-data--storage)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

Clone is the registry's acceleration tool and its **only sanctioned revival path** for
`retired` entities: a new `draft` with new identity, an explicit copy/reset disposition per
field class, **live re-validation** of every vocabulary reference (unit, tier, category,
codes), a `clonedFrom` lineage pointer, and — under P-D-04's absolute name uniqueness — the
rename rule that revival forces.

### 1.2 Purpose

Copying structure is cheap; copying **stale classifications** is how a defect from last year
republishes itself. The disposition table and the live re-validation are the whole design: what
copies, what resets, and what must be re-proven against today's vocabularies.

### 1.3 Actors

| Actor | Role |
|-------|------|
| `cpt-cf-bss-products-actor-product-manager` | Clones drafts/published/retired sources into new drafts |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.10 (`fr-clone`), AC #34, AC #38 ("authoring/cloning against a
  de-listed/deprecated unit"); [`../DECISIONS.md`](../DECISIONS.md) P-D-04 (name uniqueness —
  the revival-rename interaction flagged in 01 §6); slices 01 (identity/doors), 03
  (vocabularies).

### 1.5 Scope

**In**:
- the clone door for Product and SKU (single and product-with-SKUs)
- the disposition table
- live re-validation
- `clonedFrom`
- the revival-rename rule.

**Out**:
- bulk cloning (09's import covers mass cases)
- pricing/plan content (never copied — PRD)
- approval (a clone lands as `draft`; its publish is the ordinary 05-gated act).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Source may be `draft`, `published`, **`deprecated`** or `retired` (the revival path); the source is never affected. `deprecated` is a **governed sub-state of `published`**, so it was inside the PRD's second term rather than missing from it — `fr-clone` and AC #34 now name it explicitly (2026-08-26 branch review, which found the row asserting a PRD defect where a reading would do, and a test written from AC #34 covering three states while the design admitted four). It is the state every entity occupies for the **whole retirement lead window** — including while the operator builds the successor that the retirement's `replacedBy` must name, which is the clone's most predictable use (item 37 of the 2026-08-26 review) | PRD `fr-clone` |
| C2 | New `productId`/`skuId`; a new `skuCode`/`productCode` — system-suggested, operator-overridable, atomically reserved through the 01 `ReservationIndex` | PRD `fr-clone` |
| C3 | Lifecycle and version counters reset (`draft`, `published_version = 0`, `internal_revision = 1`); pricing/plan content never copied | PRD `fr-clone` |
| C4 | Cloned vocabulary references re-validate against the **live** registries: a de-listed/`deprecated` unit, a **`deprecated`/retired** tier (H1 — the PRD's dropped word restored, delivered by 03's new `PLAN_TIER_DEPRECATED`), a retired category, an unknown code, or a **deprecated/narrowed attribute definition** (M5) **fails or forces re-selection** — never copied silently | PRD `fr-clone`, AC #38 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `DispositionTable` | The normative per-field copy/reset/re-validate matrix (§3.1) |
| `CloneDoor` | The one endpoint; internally it drives the ordinary 01 create/save doors |

## 2. Actor Flows (CDSL)

### Clone an entity

- [ ] `p3` - **ID**: `cpt-cf-bss-products-flow-clone`

1. [ ] - `p3` - `POST …/{entity}/clone` (`product|sku × write`; a product-with-SKUs clone requires **both** grants — L4): source resolved in-tenant, any C1 state. **Read surface (M1/M2)**: a `retired`, `published` or `deprecated` source reads entity content from its **last frozen version** — never a head's pending edits, which would leak in-flight unapproved content — with `clonedFrom` recording exactly that version; the metadata map comes from the beside-entity store (P-D-06 — outside frozen content, survives retirement); a `draft` source reads its head. The clone materializes through the ordinary 01 create door — same validators, same codes, no parallel path — as **one transaction per entity** (create + values + metadata: the single-clone act is atomic, L2) - `inst-cn-door`
2. [ ] - `p3` - Identity per C2: new ids minted; the suggested code is derived (`{source}-copy-N`), operator-overridable, reserved atomically — a collision is the ordinary `DUPLICATE_SKU_CODE`; a source Product with no `productCode` suggests none — the clone's stays null (L5) - `inst-cn-identity`
3. [ ] - `p3` - **The rename rule (P-D-04, reframed per L3)**: **every same-brand Product clone renames** — the uniqueness index holds the source's name in every non-`discarded` state, so a clone of a draft, published, or retired Product collides alike; the suggestion is `{name}-copy-N` (matching the code suggestion), `{name}-revived` flavored for a retired source; operator-overridable. Revival is why the rule is non-negotiable, not when it applies. Display attributes copy verbatim — the quasi-code renames, the storefront doesn't - `inst-cn-rename`
4. [ ] - `p3` - The `DispositionTable` (§3.1) is applied field-class by field-class; every re-validated reference that fails names the field and the live-registry verdict (C4) so the operator re-selects rather than guesses - `inst-cn-disposition`
5. [ ] - `p3` - `clonedFrom = (entity id, published_version | 'draft')` is recorded on the clone (immutable thereafter — lineage, not a live link); audit + the clone rides `ProductCreated`/`SkuCreated` (explicit: no separate clone event — the lineage field is queryable) - `inst-cn-lineage`
6. [ ] - `p3` - Product-with-SKUs clone: children clone per the same table in one batch-like act (per-child ledger of failures — per-row atomic acts honestly reported, the PRD's own §6.9 shape, which is how this squares with 01's no-partial-application rule: each act IS complete, L1); a child failing re-validation fails alone; **parent-plus-surviving-children is a valid, intended end state** (drafts are cheap — failed children are re-selectable and re-clonable); a failing **parent** creates nothing (children never attempted) - `inst-cn-children`

## 3. Processes / Business Logic

### 3.1 The disposition table

- [ ] `p3` - **ID**: `cpt-cf-bss-products-algo-disposition`

| Field class | Disposition |
|-------------|-------------|
| System identity (`productId`/`skuId`) | **Reset** (minted) |
| Codes (`skuCode`/`productCode`) | **Reset** (suggested, reserved) |
| Canonical name | **Copy + rename** per `inst-cn-rename` (every same-brand product clone) |
| `brand_id` | **Copy** — a clone never retargets brand; a cross-brand copy is a create, not a clone (M4; also what keeps the rename rule's collision premise sound) |
| `created_by` | **Reset** to the cloning actor's ref — a copied ref would misattribute authorship in audit projections (M4) |
| Structure (type, scope) | **Copy** (scope re-checked by the ordinary containment validator) |
| Parent link | **Copy** for a lone-SKU clone — requiring a live (non-retired) parent per the create door, so a lone clone of a retired parent's SKU must name a new parent (M6, the C1 carve-out disclosed); **remap to the new parent** in a product-with-SKUs clone |
| Display/localized attributes + metadata map | **Copy + re-validate** (M5): a `deprecated` definition ⇒ re-select (`ATTRIBUTE_DEFINITION_DEPRECATED`), visibility-scope drift ⇒ re-select (`ATTRIBUTE_SCOPE_VIOLATION`), PII re-screened by 02's hook — a once-allowed value re-passes the current policy |
| Category assignments | **Copy + re-validate** (retired category ⇒ re-select) |
| `PlanTier` | **Copy + re-validate** (`deprecated`/retired tier ⇒ re-select — `PLAN_TIER_DEPRECATED`/`PLAN_TIER_UNKNOWN`, H1) |
| Metering declaration (`unit`, `usageTypeRef`) | **Copy + re-validate** (deprecated/de-listed unit ⇒ fail per AC #38; `usageTypeRef` re-resolves per P-D-05) |
| Accounting codes | **Copy + re-validate** against the live sets |
| `sellable` | **Copy** (bucket-iii value, judged again at publish) |
| Lifecycle, versions, approvals, `compositionPending`, `replacedBy`, deprecation provenance | **Reset** (C3 — state never copies) |
| Pricing/plan anything | **Never** (not carried here at all — the boundary) |

## 4. Data / Storage

One column (`cloned_from`, nullable — now in 01's §4.1/§4.2 rosters, create-only, a later write
fails `ILLEGAL_FIELD_MUTATION` (M3)) on both entity tables; no new tables; no new events.
Errors reuse the owning slices' codes — the per-field map (L6): unit → `UNRECOGNIZED_UNIT`/
`UNIT_DEPRECATED`; tier → `PLAN_TIER_UNKNOWN`/`PLAN_TIER_DEPRECATED`; category →
`CATEGORY_RETIRED`; accounting → `ACCOUNTING_CODE_UNKNOWN`; definition →
`ATTRIBUTE_DEFINITION_DEPRECATED`/`ATTRIBUTE_SCOPE_VIOLATION`; name → `DUPLICATE_NAME`;
code → `DUPLICATE_SKU_CODE`.

## 5. Testing posture (slice-local)

- The revival flagship: clone a `retired` Product — new ids, forced canonical rename, identical
  display name, source untouched, `clonedFrom` recorded.
- Re-validation matrix: one fixture with a deprecated unit + retired tier + retired category on
  the source — three named failures/re-selections, none silently copied (each with the
  clean-source positive control).
- PII re-screen: a value allow-listed at source-creation time but since de-listed blocks on
  clone (the policy of *today* governs).
- Child-ledger probe: one failing child, siblings land.

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-fr-clone`, AC #34; AC #38 (clone-against-deprecated-unit row); the 01 §6
clone-vs-P-D-04 flag — resolved here by `inst-cn-rename`.

**Risks & open items**: none open — the slice is deliberately thin; its one design-introduced
rule (revival rename) resolves a flagged interaction rather than creating one. If product later
wants name **transfer** on revival (retire frees the name to its clone), that is a P-D-04
amendment, not a clone feature — named so nobody builds it here.
