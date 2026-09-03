<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./02-taxonomy-attributes.md, ./03-sku-classification.md, ./04-lifecycle.md | Owners: BSS Product Catalog team -->

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
field class, **live re-validation** of every vocabulary reference (unit, tier, category, attribute definitions,
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

- [`../PRD.md`](../PRD.md) §6.10 (`fr-clone`), AC #34, AC #38 (the two rows
  **P-D-44** split: "authoring/cloning against a **de-listed** unit" and
  "authoring/cloning against a **deprecated** unit");
  [`../DECISIONS.md`](../DECISIONS.md) P-D-04 (name uniqueness — the
  revival-rename interaction); slices 01 (identity/doors), 02 (attributes,
  categories, the metadata map, the PII hook), 03 (vocabularies), 04 (deprecation, `replacedBy`).

### 1.5 Scope

**In**:
- the clone door for Product and SKU (single and product-with-SKUs)
- the disposition table
- live re-validation
- `clonedFrom`
- the revival-rename rule.

**Out**:
- bulk cloning (§6 — 09's resolver produces no copies)
- pricing/plan content (never copied — PRD)
- approval (a clone lands as `draft`; its publish is the ordinary 05-gated act).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Source may be `draft`, `published`, **`deprecated`** or `retired` (the revival path); the source is never affected. `deprecated` is a **governed sub-state of `published`**, so it was inside the PRD's second term rather than missing from it — `fr-clone` and AC #34 now name it explicitly (branch review, which found the row asserting a PRD defect where a reading would do, and a test written from AC #34 covering three states while the design admitted four). It is the state every entity occupies for the **whole retirement lead window** — including while the operator builds the successor that the retirement's `replacedBy` may name, which is the clone's most predictable use (item 37 of the review) | PRD `fr-clone` |
| C2 | New `productId`/`skuId`; a new `skuCode`/`productCode` — system-suggested, operator-overridable, atomically reserved through 01's `ReservationIndex` (`skuCode`, §4.2) and §4.1's `product_code` partial unique index | PRD `fr-clone` |
| C3 | Lifecycle and version counters reset (`draft`, `published_version = 0`, `internal_revision = 1`); pricing/plan content never copied | PRD `fr-clone` |
| C4 | Cloned vocabulary references re-validate against the **live** registries: a de-listed/`deprecated` unit, a **`deprecated`/retired** tier (H1 — the PRD's dropped word restored, delivered by 03's new `PLAN_TIER_DEPRECATED`), a retired category, an unknown code, or a **deprecated/narrowed attribute definition** (M5) **fails, and the refusal names every field class that failed** — never copied silently. **One outcome vocabulary** (**P-D-49**): the act refuses and collects, rather than refusing on the first class, which is the only reading under which §5's single fixture can yield three named failures; the PRD's "forces re-selection" is the operator's next act on that answer, not a second outcome on the wire. This is the donor's shape — its validation rules "append and never short-circuit" and its report carries the whole list | PRD `fr-clone`, AC #38 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `DispositionTable` | The normative per-field copy/reset/re-validate matrix (§3.1) |
| `CloneDoor` | The one endpoint; internally it drives the ordinary 01 create door |

## 2. Actor Flows (CDSL)

### Clone an entity

Declared by [`../features/clone.md`](../features/clone.md) §2 as `cpt-cf-bss-products-flow-clone`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p3` - `POST /bss-products/v1/{products|skus}/{id}/clone` (`product|sku × write`; a product-with-SKUs clone requires **both** grants — L4 — and **every product clone is the family act**: the P-D-75 body carries no selector and no document names a lone-product clone, so the product door spends both grants unconditionally, authorization preceding the child count it cannot yet read — **P-D-79**): source resolved in-tenant, any C1 state. **Read surface (M1/M2)**: a `retired`, `published` or `deprecated` source reads entity content from its **last frozen version** — never a head's pending edits, which would leak in-flight unapproved content — with `clonedFrom` recording exactly that version; the metadata map comes from the beside-entity store (P-D-06 — outside frozen content, survives retirement); a `draft` source reads its head. The clone materializes through the ordinary 01 create door — same validators, same codes, no parallel path — → **201**, as **one transaction per entity** (create + values + metadata: the single-clone act is atomic, L2 — **the door is the side tables' second composite creator on P-D-46's precedent**: entity row, side rows, `internal_revision = 1`, no side-door events, no second grant, **P-D-75**). **The request body is the overrides and nothing else** (**P-D-75**): `{code?, name?, newParentId?, and optional replacement values for the five re-validated classes}` — absent means copy/reset per §3.1, and the replacement slots exist because a refused re-validation on an immutable source must be answerable in the retry. **A `discarded` source is refused `CLONE_SOURCE_DISCARDED` (409, declared here — P-D-75 on P-D-52's mint test**: `ENTITY_TERMINAL` means a head *write* and the clone writes nothing to the source; the bare 404 carries no code channel**)**. **The door is keyed with the ordinary idempotency semantics** (P-D-75 — P-D-72's family resume already presupposed the key; a keyless request skips the phase, a keyed retry replays the first clone) - `inst-cn-door`
2. [ ] - `p3` - Identity per C2: new ids minted; the suggested code is derived (`{source}-copy-N`), operator-overridable, reserved atomically — **`N` is the first free integer for the suggested string, decided by the index under the reservation** (**P-D-62**, on P-D-37's and P-D-42's mechanism: a reservation conflict moves to the next free integer and retries, so concurrent clones of one source are arbitrated by the index rather than racing a read, and no counter column exists to drift); a collision on an **operator-supplied** code is the ordinary `DUPLICATE_CODE`; a source Product with no `productCode` suggests none — the clone's stays null (L5) - `inst-cn-identity`
3. [ ] - `p3` - **The rename rule (P-D-04, reframed per L3)**: **every same-brand Product clone renames** — the uniqueness index holds the source's name in every non-`discarded` state, so a clone of a draft, published, deprecated or retired Product collides alike; the suggestion is `{name}-copy-N` (matching the code suggestion, `N` the first free integer under the reservation — **P-D-62**), `{name}-revived` flavored for a retired source — **and a second revival of one lineage suggests `{name}-revived-N`**, the same first-free rule over the `-revived` family, so the flavor survives and the suggestion path never produces a refusal (P-D-62; falling back to `-copy-N` was declined as silently dropping the one signal `-revived` carries); operator-overridable. A collision on an operator-supplied name is the ordinary `DUPLICATE_NAME`. Revival is why the rule is non-negotiable, not when it applies. Display attributes copy verbatim as to their values (still re-validated per §3.1) — the quasi-code renames, the storefront doesn't - `inst-cn-rename`
4. [ ] - `p3` - The `DispositionTable` (§3.1) is applied field-class by field-class; every re-validated reference that fails names the field and the live-registry verdict (C4) so the operator re-selects rather than guesses - `inst-cn-disposition`
5. [ ] - `p3` - `clonedFrom = (entity id, published_version | 'draft')` is recorded on the clone (immutable thereafter — lineage, not a live link); the clone rides `ProductCreated`/`SkuCreated` (P-D-21: the event stream is the audit of record for what succeeds, so a committed act that emits one writes no audit row) (explicit: no separate clone event — the lineage field is queryable) - `inst-cn-lineage`
6. [ ] - `p3` - Product-with-SKUs clone: children clone per the same table in one batch-like act, each child riding its own `SkuCreated` (no new event — §4) (per-child ledger of failures — per-row atomic acts honestly reported, the PRD's own §6.9 shape, which is how this squares with 01's no-partial-application rule: each act IS complete, L1); a child failing re-validation fails alone; **parent-plus-surviving-children is a valid, intended end state** (drafts are cheap — failed children are re-selectable and re-clonable); a failing **parent** creates nothing (children never attempted). **The family act answers `201`
with a per-child receipt** — `{source sku_id, disposition ∈ {created, failed}, new sku_id | code +
violations}`, codes the owning doors' verbatim — **and resumes from its own data** (**P-D-72**): the
door's claim joins the *parent's* transaction, a committed-but-unanswered claim means *in progress*,
and the same-key retry re-enters, skipping sources the new parent's children's `cloned_from` already
names, cloning the rest, storing the answer at completion — no ledger table, the store already
carrying the facts; **the re-entry's parent handle is the claim row's own `entity_ref` stamp**
(**P-D-79** — several family acts over one source make `cloned_from` alone ambiguous), written in
the parent's transaction, the answer deliberately **not** stored there but at completion; the
children are the source's **non-discarded** SKUs, a `discarded` child neither attempted nor
receipted (P-D-79) - `inst-cn-children`

## 3. Processes / Business Logic

### 3.1 The disposition table

Declared by [`../features/clone.md`](../features/clone.md) §3 as `cpt-cf-bss-products-algo-disposition`.
The table below is this slice's and is the normative one; the FEATURE carries the
Input, the Output and the boundary.

*Every re-validation row below refuses on failure and the refusal collects across rows (C4); a
clone either lands whole or lands not at all.* **The row order below is also the rules' registration
order** (**P-D-55**), and therefore their execution order within the phase and the precedence
`ValidationReport::audit_code` would answer with — a tie-break over an attribution channel, fixed
here so it is not settled by whoever registers first. It ranks **rows**: a row whose code is not yet
minted takes its place when it is.

| Field class | Applies to | Disposition |
|-------------|-----------|-------------|
| System identity (`productId`/`skuId`) | both | **Reset** (minted) |
| Codes (`skuCode`/`productCode`) | both — each its own kind's code | **Reset** (suggested, reserved — a source Product with no `productCode` suggests none, `inst-cn-identity`) |
| Canonical name | **Product** | **Copy + rename** per `inst-cn-rename` (every same-brand product clone) |
| `brand_id` | both | **Copy** — a clone never retargets brand; a cross-brand copy is a create, not a clone (M4; also what keeps the rename rule's collision premise sound) |
| `created_by` | both | **Reset** to the cloning actor's ref — a copied ref would misattribute authorship in audit projections (M4) |
| Structure (type, scope) | both — `type` is SKU-only | **Copy** (scope re-checked by the ordinary containment validator) |
| Parent link | **SKU** | **Copy** for a lone-SKU clone — requiring a parent that is neither `retired` nor `discarded` and holds no live retire intent, per the create door (`PARENT_TERMINAL`/`RETIREMENT_PENDING`), so a lone clone of a retired parent's SKU must name a new parent (M6, the C1 carve-out disclosed); **remap to the new parent** in a product-with-SKUs clone |
| Display/localized attributes + metadata map | both | **Copy + re-validate** (M5): a `deprecated` definition ⇒ re-select (`ATTRIBUTE_DEFINITION_DEPRECATED`), visibility-scope drift ⇒ re-select (`ATTRIBUTE_SCOPE_VIOLATION`), PII re-screened by 02's hook — a once-allowed value re-passes the current policy (`CONTENT_PII_BLOCKED`) |
| Category assignments | **Product** | **Copy + re-validate** (retired category ⇒ re-select) |
| `PlanTier` | **SKU** | **Copy + re-validate** (`deprecated`/retired tier ⇒ re-select — `PLAN_TIER_DEPRECATED`/`PLAN_TIER_UNKNOWN`, H1) |
| Metering declaration (`unit`, `usageTypeRef`) | **SKU** | **Copy + re-validate** (deprecated/de-listed unit ⇒ fail per AC #38; `usageTypeRef` re-resolution stays 03 `inst-mt-resolve`'s, at publish) |
| Accounting codes | **SKU** | **Copy + re-validate** against the live sets — a `deprecated` code ⇒ re-select (`ACCOUNTING_CODE_DEPRECATED`), a `removed` or unknown one likewise (`ACCOUNTING_CODE_UNKNOWN`); **P-D-47** |
| `sellable` | **SKU** | **Copy** (bucket-iii value, judged again at publish) |
| Lifecycle, versions, approvals, `compositionPending`, `replacedBy`, deprecation provenance | both — `compositionPending`/`replacedBy` SKU-only | **Reset** (C3 — state never copies; `compositionPending` to its `false` default, 01 P-D-35) |
| `tenant_id` | both | **Copy** — the source is resolved in-tenant (`inst-cn-door`) |
| `name_normalized` | **Product** | **Derived** from the renamed name, application-side (01 §4.1) |
| `cloned_from` | both | **Reset** — written to the *immediate* source per `inst-cn-lineage`, never copied from the source (01 makes it writable only in the creating statement, so a copied value is unrepairable) |
| `timestamps` | both | **Reset** by the create door |
| Pricing/plan anything | both | **Never** (not carried here at all — the boundary) |

## 4. Data / Storage

**Two columns (P-D-76)** — `cloned_from` and `cloned_from_version`, the P-D-50 convention (`NULL` version under a set source = read at the head), shape-CHECKed, in 01's §4.1/§4.2 rosters, create-only, a later write
fails `ILLEGAL_FIELD_MUTATION` (M3)) on both entity tables; no new tables; no new events.
Errors reuse the owning slices' codes — the per-field map (L6): unit → `UNRECOGNIZED_UNIT`/
`UNIT_DEPRECATED`; tier → `PLAN_TIER_UNKNOWN`/`PLAN_TIER_DEPRECATED`; category →
`CATEGORY_RETIRED`; accounting → `ACCOUNTING_CODE_UNKNOWN`/`ACCOUNTING_CODE_DEPRECATED` (**P-D-47** minted the second); definition →
`ATTRIBUTE_DEFINITION_UNKNOWN`/`ATTRIBUTE_DEFINITION_DEPRECATED`/`ATTRIBUTE_SCOPE_VIOLATION`; name → `DUPLICATE_NAME`;
code → `DUPLICATE_CODE` (**P-D-25**: one code covers both the `skuCode` and `productCode`
reservations).

**Problem responses (RFC 9457):** `CLONE_SOURCE_DISCARDED` (409, the one code this slice declares —
P-D-75); everything else is the owning slices' with their statuses as declared there —
`DUPLICATE_NAME`, `DUPLICATE_CODE`, `PARENT_TERMINAL`, `ENTITY_TERMINAL`, `IDEMPOTENCY_CONFLICT`,
`IDEMPOTENCY_KEY_IN_FLIGHT` (409, `design/01` §3.3); `SCOPE_NOT_CONTAINED`, `VALIDATION`,
`CONTENT_PII_BLOCKED` (422, same block); `RETIREMENT_PENDING` (409, `design/04`); the twelve
re-validation codes with their owners (`02`, `03`, `04`) when they are minted.

## 5. Testing posture (slice-local)

- Rename is not revival-only: clone a `published` Product and a `draft` Product — both rename per
  `inst-cn-rename`; a `deprecated` source too, the state C1 calls the most predictable use.
- The revival flagship: clone a `retired` Product — new ids, forced canonical rename, identical
  display name, source untouched, `clonedFrom` recorded.
- Re-validation matrix: one fixture with a deprecated unit + retired tier + retired category on
  the source — three named failures/re-selections, none silently copied (each with the
  clean-source positive control).
- PII re-screen: a value allow-listed at source-creation time but since de-listed blocks on
  clone (the policy of *today* governs).
- Child-ledger probe: one failing child, siblings land.

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-fr-clone`, AC #34; AC #38 (clone-against-deprecated-unit row); the clone-vs-P-D-04
interaction — resolved here by `inst-cn-rename`.

**Risks & open items** — ten, all raised by the first lens pass; the slice is deliberately thin,
which is why its gaps are omissions rather than contradictions:
- ~~**What is the clone door's request body?**~~
  **Answered (owner call, 2026-09-01 — P-D-75): the overrides and nothing else** — `{code?, name?, newParentId?, optional replacement values for the five re-validated classes}`, absent meaning copy/reset per §3.1; the replacement slots exist because a refused re-validation on an immutable source must be answerable in the retry, which is C4's own re-select. Original text: Three rules require operator input — an overridable
  code, an overridable name, a replacement parent — and a fourth ("forces re-selection") may require
  re-selected values. No slice declares a clone payload, and whether those arrive in the clone
  request or in a follow-up save changes the door's shape, its validator order and whether it can
  refuse for a vocabulary reason at all. Owner: this slice with 12. *(Raised by the slice-11 first lens pass.)*
- ~~**What writes the clone's category assignments, attribute values and metadata map?**~~
  **Answered (owner call, 2026-09-01 — P-D-75): the clone door itself, in its creating transaction** — `inst-cn-door`'s L2 atomicity read through P-D-46's precedent: the door is the side tables' second composite creator, `internal_revision = 1`, no side-door events, no second grant. The tables do not ship yet; the rule binds when they land. Original text: All three live
  in side tables whose only stated writers are the save door and the metadata door — both of which
  bump `internal_revision` (defeating C3's `= 1`), emit their own events (defeating
  `inst-cn-lineage`'s "no new events") and spend a grant this door does not name. 01's create flow
  writes the entity row and its outbox row and nothing else. **P-D-46** answered the general question
  for the **save** door — `inst-fd-save-txn` now writes content in its own transaction — but the clone
  lands through the **create** door, which that arm did not reach, so this slice's atomicity claim
  still has no writer; 01 §6 carries the narrowed question. Owner: 01's door owner with 02's, plus 05
  for the grant. *(Raised by the slice-11 first lens pass.)*
- ~~**Does the clone door need `metadata × write` beside `product|sku × write`?**~~ **Answered (P-D-128, 2026-09-03): no** — a new draft's map under the authoring pair; `metadata × write` guards in-place edits on a published entity. *The item's text stood as:* 05 split that grant
  because the map is mutable in place on a **published** entity; the clone writes a new draft's map,
  which that reason does not reach — but the pair is declared per resource, not per lifecycle state,
  and 05 lists no exemption. Owner: 05's owner with 02's. *(Raised by the slice-11 first lens pass.)*
- ~~**How are `-copy-N` and `-revived` generated?**~~
  **Answered (owner call, 2026-08-31 — P-D-62): the first free integer for the suggested string,
  decided by the index under the reservation** — P-D-37's and P-D-42's mechanism, no counter column
  and no read-then-suggest race — **and a second revival suggests `{name}-revived-N`** by the same
  rule, so the flavor survives and the suggestion path never refuses. The operator path is untouched.
  Original text: `N` is never defined (per-source counter, global,
  first free integer), `-revived` carries no counter at all, and the index admits one holder per name
  in every non-`discarded` state — so a second revival of one lineage produces a suggestion the
  registry must refuse, and concurrent clones computing `N` by a read race each other. Owner: this
  slice. *(Raised by the slice-11 first lens pass.)*
- ~~**What does the door answer for a `discarded` source?**~~
  **Answered (owner call, 2026-09-01 — P-D-75): `CLONE_SOURCE_DISCARDED`, 409, minted on P-D-52's test** — the door owes a classified refusal and nothing existing fits: `ENTITY_TERMINAL` means a head *write* while the clone writes nothing to the source, and the bare 404 carries no code channel. Original text: C1 admits four states and `discarded` is the
  fifth, reachable and addressable; nothing says whether it is a 404-class miss, a state refusal or
  admitted, and `ENTITY_TERMINAL` cannot be reused as-is because the clone writes nothing to the
  source while a `retired` source is explicitly allowed. Owner: the taxonomy owner with this slice.
  *(Raised by the slice-11 first lens pass.)*
- ~~**Which surface answers the reverse lineage lookup — what was cloned from a given entity?**~~ **Answered (P-D-128, 2026-09-03): `08`'s timeline, from the `clonedFrom` column at render time**; no clone event. *The item's text stood as:* The absence of a clone event is justified by
  the lineage field being "queryable", and the field appears in no read model and no SDK shape; a
  clone is a draft, which the browse projection cannot see at all. Owner: 08's and 12's owners —
  expose it, or withdraw the justification. *(Two lenses raised it independently.)*
- ~~**Where does the per-child ledger of a product-with-SKUs clone live?**~~
  **Answered (owner call, 2026-09-01 — P-D-72): in the data itself.** The new parent's children carry
  their own `cloned_from` pointers, so the same-key retry re-enters the family act and resumes —
  skip, clone the rest, answer at completion; no table is built and the response receipt stays a
  receipt. The claim semantics extension is named on P-D-42. Original text: §4 declares no tables and no
  events, so the ledger is response-only and a crash between children leaves an unreported half-clone
  with no resumption path — 09, whose shape this cites, has both a table and a resume rule. Owner:
  this slice with 09's storage owner. *(Raised by the slice-11 first lens pass.)*
- **Which role holds the clone grant?** §1.3 gives it to the product manager and the PRD's own §11
  console gives clone to an Operator/Platform owner, while the door spends the authoring pair. The
  PRD disagrees with itself and no document maps roles to grants. Owner: the PRD owner with 05.
  *(Raised by the slice-11 first lens pass.)*
- ~~**May a slice restate a decision whose propagation field does not name it?**~~ **Answered (P-D-128, 2026-09-03): yes** — the field names where a decision was filed, not every citer. *The item's text stood as:* This slice's central
  rule leans on P-D-04, whose surface names `design/01-foundation.md`,
  `design/02-taxonomy-attributes.md`, `design/04-lifecycle.md` and `design/09-bulk-promotion.md` —
  not this file; the same holds for P-D-05 and P-D-06, while P-D-21, P-D-25 and P-D-35 name this
  file explicitly. The register's own standard
  is that a propagation field describes what a document says — which makes these register omissions
  rather than defects here, but nothing states whether the citing side owes an entry. Owner: the
  register's owner. *(Raised by the slice-11 first lens pass.)*
- ~~**Who owns mass cloning?**~~ **Answered (P-D-128, 2026-09-03): nobody, on purpose — out of v1**; `09`'s conflict arm naming clone is the intended revival path. *The item's text stood as:* This slice puts it Out and pointed at 09, whose resolver is total over
  identity and produces no copies — it classifies such a row as a no-op, an update to the source, or
  (for revival) a conflict naming clone as the only path. So the case is claimed by nobody. Owner:
  the design-set owner with 09. *(Raised by the slice-11 first lens pass.)*

The thinness itself is not a risk: its one design-introduced
rule (revival rename) resolves a flagged interaction rather than creating one. If product later
wants name **transfer** on revival (retire frees the name to its clone), that is a P-D-04
amendment, not a clone feature — named so nobody builds it here.

**Filed elsewhere** — two of this slice's questions live at their owners and are **not** restated
here: `clonedFrom`'s physical storage in `design/01-foundation.md` §6, and whether
`{source}-copy-N` is a legal `skuCode` in `PRD` §15.
