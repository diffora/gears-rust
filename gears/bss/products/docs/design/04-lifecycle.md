<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md | Owners: BSS Product Catalog team -->

# DESIGN — Lifecycle Policy (Slice 4)

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
  - [Deprecate / un-deprecate](#deprecate--un-deprecate)
  - [Schedule a publish (`publishAt`)](#schedule-a-publish-publishat)
  - [Retire a SKU (scheduled transition + joint contract)](#retire-a-sku-scheduled-transition--joint-contract)
  - [Retire a Product (cascade with deferred intent)](#retire-a-product-cascade-with-deferred-intent)
  - [Parent-child integrity (publish ordering + containment)](#parent-child-integrity-publish-ordering--containment)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 The activation runner](#31-the-activation-runner)
  - [3.2 Error taxonomy (slice-owned codes)](#32-error-taxonomy-slice-owned-codes)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

Slice 01 owns the state-machine **floor** (the edge list, terminality, the physical trigger
guard); this slice owns the **policy on the edges**: deprecation with `direct`/`cascaded`
provenance, governed un-deprecation, scheduled publish (`publishAt`) and scheduled retirement
(`effectiveAt` with the ≥ 30-day lead), the `replacedBy` successor pointer, parent↔child
integrity (publish ordering, scope containment — the P-D-04 residue — and cascade-retire with
deferred intent), and the retirement joint contract with pricing (D-47: never flip to `retired`
while the `SkuReferenceCount` predicate reads referenced — the predicate itself is slice 07's).

### 1.2 Purpose

Lifecycle is where the registry's promises to downstream live or die: `deprecated` must block
new adoption without touching existing references, retirement must be un-surprising (lead time,
successor pointer, grandfathered snapshots untouched), and no hierarchy operation may ever
orphan published content or leak a child outside its parent's scope.

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-product-manager` | Deprecates; schedules publishes |
| `cpt-cf-bss-products-actor-catalog-admin` | Un-deprecates (two-person), initiates retirement/cascades, resumes deferred cascades |
| `cpt-cf-bss-products-actor-plan-price` | The AC #82 counterpart on **its own `When` — retirement or unpublishing**: flags referencing plans and blocks new adoption. A *plain* deprecation (`inst-lc-deprecate`, no retirement behind it) has no pricing-side counterpart AC yet — slice 12's `ObligationRegister` carries the ask |
| `cpt-cf-bss-products-actor-subscriptions` | Owns live-subscription migration; consumes `replacedBy` (and `mustMigrateBy`, post-v1) |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.5 (`fr-lifecycle-transitions` scheduling clauses,
  `fr-parent-child-integrity`, `fr-deprecation`, `fr-undeprecation`, `fr-retirement-eol`);
  AC #14–#18; AC #38 (parent/scope/EOL rows)
- [`../DECISIONS.md`](../DECISIONS.md) P-D-04 (containment is the region algebra's only
  survivor); pricing D-47 (the retirement joint contract; AC #82 is pricing's half)
- [`./01-foundation.md`](./01-foundation.md) §2 "Transition an entity", §3.1 (registered
  validators on edges), §4.2 (`deprecation_provenance`, `replaced_by_sku_id` columns)

### 1.5 Scope

**In**:
- policy validators on every lifecycle edge
- deprecation provenance
- un-deprecation rules
- scheduled transitions (publish + retirement) and their activation mechanics
- the retirement flip guard against the slice-07 predicate
- `replacedBy`
- parent-child publish ordering, scope containment (final rule), cascade-retire + deferred intent
- the v1 EOL lockout.

**Out**:
- the edge list itself and terminality (01)
- the reference predicate (07)
- approval ceremonies the edges invoke (05)
- grandfathered-snapshot immutability (06)
- live-subscription migration (Subscriptions)
- the consumer-side adoption block (pricing AC #82, retirement/unpublish arm only; verified by the slice-12 seam suite).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | Forward-only: no unpublish, no in-place rollback; retraction = deprecate/retire + a new version | PRD `fr-lifecycle-transitions` |
| C2 | Retirement lead time ≥ 30 days (interim §17.1 policy); all scheduling in UTC | PRD §4.1 |
| C3 | **v1 = plain retirement + grandfathering only**; EOL-with-`mustMigrateBy` is defined-but-deferred, disabled until the subscriptions-lifecycle AC exists and is referenced by number | PRD `fr-retirement-eol` |
| C4 | The registry never flips a SKU to `retired` while the `SkuReferenceCount` predicate reads referenced (fresh > 0 or stale/never-received) — D-47 joint contract | PRD `fr-retirement-eol` |
| C5 | Scope containment: flat region/brand value sets, containment = subset, not-provably-subset ⇒ fail-closed (`SCOPE_NOT_CONTAINED`) — the final form of 01's interim check. **Containment is over restrictions** (01 **P-D-39**): the empty set means *unrestricted*, so an unrestricted parent contains every child and an unrestricted child is contained only by an unrestricted parent | P-D-04; 01 P-D-39 |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `ScheduledTransition` | The persisted intent `(entity, kind ∈ {publish, retire}, at, approval_ref, state)` executed by the activation runner |
| `ActivationRunner` | The due-job executor: re-validates fail-closed, then drives the Foundation door |
| `CascadePlan` | The computed disposition of a Product retirement over its children: retire / leave-and-list / auto-discard, with provenance |
| `DeferredRetireIntent` | The tracked, queryable record that a Product's retirement is pending on listed children |

### 1.8 Context & Dependencies

**Consumed**: Foundation edges + publish door (01); slice-05 gate (un-deprecation two-person;
retirement confirmation; materiality of lifecycle transitions); slice-07 predicate (at
retirement initiation for the confirmation count, and at flip time as the guard). **Produced**:
`SkuDeprecated`/`SkuUndeprecated`/`SkuRetired`/`ProductRetired` (+ `ProductDeprecated`/
`ProductUndeprecated`), `PublishScheduled`/`RetirementScheduled` audit events; the deferred-intent query surface (owned here; slice 08
only projects it); the deprecation mark pricing consumes — through AC #82 when a retirement is behind it, and through no counterpart AC at all for a plain deprecation (slice 12 register).

## 2. Actor Flows (CDSL)

### Deprecate / un-deprecate

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-deprecation`

1. [ ] - `p1` - `published → deprecated`: records provenance `direct` (operator act) or `cascaded` (parent-driven); **a plain Product deprecation cascades `cascaded` deprecation onto its children, with a stated disposition for each state** (explicit per the 08 review L5 — the un-deprecation reversal rule has nothing to reverse otherwise; previously stated only inside the retirement `CascadePlan`, and previously keyed on "non-terminal children", which was two defects at once — Blocking 4 and item 8 of the 2026-08-26 review). The dispositions: a **`published`** child is deprecated `cascaded`; an **already-`deprecated`** child is **left untouched — its `deprecation_provenance` is never re-stamped**, because `direct` re-stamped as `cascaded` makes `inst-lc-provenance-reversal` revive on the parent's un-deprecation exactly the child AC #17 says it must not (and a `cascaded` child needs no second stamp); a **`draft`** child is **skipped and listed**, not transitioned — the Foundation admits no `draft → deprecated` edge and any failure rejects the whole mutation (01 `inst-fd-fail-closed`), so keying on "non-terminal" made deprecating a Product with one draft SKU fail `ILLEGAL_TRANSITION` with no remedy; drafts are not adoptable, so leaving them is correct and the listing is what the operator sees. `retired`/`discarded` children are terminal and outside it. emits `SkuDeprecated` (`ProductDeprecated`) with provenance in the payload; the registry marks and exposes — the new-adoption block is the consumer's (pricing AC #82), CI-verified by the slice-12 seam suite once it exists - `inst-lc-deprecate`
2. [ ] - `p1` - `deprecated → published` (un-deprecation) is **two-person** (slice-05 gate registered on this edge; `N`-governed like any transition to `published` and `quorumReduced` recorded on the record and on `SkuUndeprecated` below the default of 2 — P-D-13: a fixed floor here would contradict P-D-11's own enumeration of this very edge as material), re-opens adoption, emits `SkuUndeprecated`; it is **refused (`RETIREMENT_PENDING`) while a live retire intent exists on the entity *or on any child this un-deprecation would revive*** (item 10 of the 2026-08-26 review: checking only the subject's own intent let a parent's cancel-then-un-deprecate revive `cascaded` children to `published` while their own retire intents stayed live — and at `effectiveAt` the runner then needs a `published → retired` edge that does not exist, after `SkuRetired` was already announced). Aborting a retirement is its own explicit act: a governed cancel of the `ScheduledTransition` (state `superseded`, audited) **for the parent and for every child leg the reversal touches — the refusal names them**, then un-deprecate (M5 fix — un-deprecation is never a silent retirement abort). **The cancel is a `GovernedLiveOp` kind registered material by this slice** (05 `inst-mt-inputs` (d)) — without that registration the `MaterialityEvaluator` judges it non-material and `inst-gv-materiality` sets `required = min(N, 1)`, i.e. one approver at the default and none at `N = 0`, for the act that is the only way to unwind a cascade. *(It stopped being the only way to publish during a lead window when P-D-20 struck the freeze from the publish door on 2026-08-26; the head stays publishable and re-announces `SkuRetired`.)* It follows `N` with `quorumReduced` recorded, like every other ceremony on P-D-13's list - `inst-lc-undeprecate`
3. [ ] - `p1` - Un-deprecating a **Product** reverses **only `cascaded`** child deprecations; a child's `direct` deprecation survives its parent's reversal (the provenance column is the operand) - `inst-lc-provenance-reversal`
4. [ ] - `p1` - `retired` is never reversible (01's physical terminality; revival = clone, slice 11) - `inst-lc-terminal`

### Schedule a publish (`publishAt`)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-scheduled-publish`

1. [ ] - `p1` - Scheduling pins the approval at scheduling time (the slice-05 approval snapshot rides the `ScheduledTransition`); the entity stays `draft` until activation - `inst-sp-pin`
2. [ ] - `p1` - `ActivationRunner` at `publishAt` (UTC) drives the ordinary Foundation publish door **in `PreAuthorized(approvalId)` mode**, resolving 01's idempotency key as the reserved lane **`internal:scheduled-activation`** with the transition id as `client_key` (**P-D-26** — a caller with no wire surface writes a lane name rather than an endpoint, so two internal lanes cannot collide on one key) (01 `inst-fd-publish-*`, the composite-act half of 05 `inst-gv-one-shot`) — full pipeline re-validation, pinned-revision check included, and the gate verifies the initiation's consumed record rather than demanding a second `satisfied` one — an entity edited after scheduling fails `SCHEDULE_STALE_APPROVAL` (the edit already invalidated the approval per 01 `inst-fd-approval-hook`) and the transition lands `failed` with an operator alert, never a partial publish - `inst-sp-activate`
3. [ ] - `p2` - Activation is idempotent (keyed by the transition id); a runner crash replays to the identical outcome - `inst-sp-idempotent`

### Retire a SKU (scheduled transition + joint contract)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-retire-sku`

1. [ ] - `p1` - Initiation requires explicit confirmation with the **active-reference count shown** (the slice-07 predicate's current answer, including its conservative states); the operator confirms against what is known, not against silence - `inst-rt-confirm`
2. [ ] - `p1` - On confirmation: the SKU is **forced `deprecated` immediately** — emitting `SkuDeprecated` (pricing's AC #82 trigger — this is the retirement arm, which is the one AC #82 actually covers) with provenance `direct` for an operator-initiated retirement or `cascaded` when driven by a `CascadePlan` (the plan passes provenance through — H1 fix, 2026-08-25 review); adoption block starts now. `RetirementScheduled` is recorded with `effectiveAt` honoring the **configured** lead-time policy (§17.1, interim ≥ 30 days; `RETIREMENT_LEAD_TIME` otherwise), and `SkuRetired` is emitted **at initiation** with the full v1 payload `{skuId, fromVersion, reason, replacedBy?, effectiveAt}` — **`fromVersion` is the entity's `published_version` at the initiation instant** (defined here; it was defined nowhere in the set). It stays truthful by **re-announcement, not by a freeze** (**P-D-20**): the head stays open to publishes for the whole lead window, and a publish that moves the version **re-emits `SkuRetired`** with the new `fromVersion`, the same `effectiveAt` and the same retirement identity — consumers key on `(skuId, effectiveAt)` and take the latest. *(Item 16 of the 2026-08-26 review found the real defect — an operator could publish versions 8, 9, 10 after telling consumers the SKU retires from version 7 — and fixed it by refusing the publish `RETIREMENT_PENDING` for the whole ≥ 30-day window. That refusal is **struck**: it was a product-visible constraint no PRD requirement carried, and its escape hatch routed through a cancel ceremony that was registered material nowhere.)* **Saves stay legal** — they touch the head, which no consumer reads (01 `inst-fd-save-txn`). `mustMigrateBy` exists in the schema and is never populated in v1 (C3). The lead-time window IS the consumer handoff - `inst-rt-initiate`
3. [ ] - `p1` - `replacedBy`, when given, MUST name a `published` SKU (`REPLACED_BY_NOT_PUBLISHED`); the registry is its SoR. **Validated once, and the row is terminal at the flip, so the pointer can come to name a later-retired SKU** — refusing to retire a live replacement target would make it un-retirable forever, so instead: retiring a SKU that any live `replaced_by_sku_id` names raises a **`replacement_chain_broken`** alert listing the pointing SKUs, and the read surface **resolves the pointer transitively** to the first non-`retired` successor (or reports the chain's end). The break is a stored fact with a consumer-usable resolution rather than a silent dangling pointer; repairing the frozen pointer itself is out of scope in v1, stated rather than implied (item 36 of the 2026-08-26 review) - `inst-rt-replacedby`
4. [ ] - `p1` - At `effectiveAt` the flip runs the **D-47 guard**: if the slice-07 predicate reads anything but fresh-zero (fresh > 0, stale, never-received, or the defensive `no_producers`), the flip is **deferred** — state stays `deprecated`, a `retirement_held` alert names the blocking producers, and the runner re-evaluates on the predicate's freshness cadence; the flip happens only on a fresh all-zero — so the flip **may trail the announced `effectiveAt`**, and consumers key the state change on `SkuRetirementEffective`, never on the clock (L5 fix). C4 is unconditional: there is no force-retire door in v1 - `inst-rt-flip-guard`
5. [ ] - `p1` - **EOL lockout (C3)**: `mustMigrateBy` and the consumer-acknowledgment machinery are refused in v1 (`EOL_DISABLED`) behind a feature flag OFF by default; the payload field exists in the event schema (vN-compatible widening later), never populated in v1 - `inst-rt-eol-lockout`

### Retire a Product (cascade with deferred intent)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-retire-product`

1. [ ] - `p1` - A Product retirement over non-`retired` SKUs requires confirmed **cascade-retire** (an unconfirmed request fails `CASCADE_CONFIRMATION_REQUIRED` — L1 fix); the `CascadePlan` computed at confirmation lists each child's disposition: retire (schedules the child per flow 3, provenance `cascaded`), **leave-and-list** (children whose flip guard cannot clear — referenced SKUs — **deprecated (`cascaded`) and listed**, so new adoption stops growing during the deferral; the PRD's "left un-retired" permits this — M1 fix), **auto-discard** (never-published drafts, releasing their codes, emitting `SkuDiscarded`). Computing the plan **supersedes, for every child in all three arms, that child's live `publish` intent — and its live `retire` intent, replaced by this cascade's own leg and audited as such** (state `superseded`, audited; item 36 of the 2026-08-26 review: superseding only `publish` intents collided with the one-live-intent-per-kind UNIQUE for any child already holding a retire intent) — the runner never publishes into a cascade (M3 fix) - `inst-cp-plan`
2. [ ] - `p1` - **The parent's own path (H2 fix)**: at confirmation the parent Product is forced `deprecated` (provenance `direct`) and gets its **own** retire `ScheduledTransition` under the same configured lead; `ProductRetired` is emitted at initiation (payload analogous to the SKU's); the parent's flip guard is **all children `retired`/`discarded`** — there is no `published→retired` edge for it any more than for a SKU - `inst-cp-parent`
3. [ ] - `p1` - Cascades are **partial by design**: when any child is left, the parent's flip defers and a `DeferredRetireIntent` is recorded — tracked, **queryable through this slice's own surface** (04 owns `products_deferred_retirement`; slice 08 only projects it into read models — M6 fix), resumable by an operator once the listed children clear - `inst-cp-deferred`
4. [ ] - `p1` - The no-orphan invariant is re-checked at flip, not only planned at confirmation - `inst-cp-no-orphan`

### Parent-child integrity (publish ordering + containment)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-parent-child`

1. [ ] - `p1` - A SKU publish under a non-`published` parent fails `PARENT_NOT_PUBLISHED` (the validator registered on the SKU `→ published` edge; the code was named in 01 §3.3 for AC #38 completeness) - `inst-pc-ordering`
2. [ ] - `p1` - **Containment (C5, final rule)**: a SKU's brand/region scope must be a subset of its parent's — flat value-set subset, evaluated on save and re-evaluated on publish; anything not provably a subset fails `SCOPE_NOT_CONTAINED`; the empty set is *unrestricted*, so an unrestricted parent contains every child and an unrestricted child needs an unrestricted parent (01 **P-D-39**, C5) - `inst-pc-containment`
3. [ ] - `p1` - A **scope-narrowing Product publish** fails closed (`SCOPE_NARROWING_BLOCKED` — L1 fix) while any **non-terminal** child (`draft`/`published`/`deprecated`) would fall outside the narrowed scope — the validator names the falling-out children; widening is always admissible. **Non-terminal, not "non-`retired`"** (item 17 of the 2026-08-26 review): `discarded` is terminal at the physical layer (01 `inst-fd-terminal`) and is the routine output of the cascade's auto-discard arm, so the old operand let one discarded draft block that Product's narrowing permanently - `inst-pc-narrowing`

## 3. Processes / Business Logic

### 3.1 The activation runner

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-activation-runner`

1. [ ] - `p1` - One runner drives both transition kinds off `products_scheduled_transition`; due rows are claimed atomically (state CAS `pending → running` with `claimed_at`; a `running` row past the claim **lease** is reclaimed `running → pending` with `attempt += 1` — a crash never wedges the entity's one live-intent slot, and re-execution is safe because the doors are idempotent — M4 fix), executed through the ordinary Foundation doors, and finished `applied|failed|deferred` with the reason recorded — the runner adds **no** privileged path around the pipeline - `inst-ar-claim`
2. [ ] - `p1` - Failure posture: the runner is **its own raising door** (01's one-door rule, M2 fix) — it wraps the publish door's refusal (`STALE_REVISION`/`APPROVAL_REQUIRED`) into `SCHEDULE_STALE_APPROVAL` on the transition; `failed` is terminal for that transition (operator reschedules explicitly — a stale approval cannot be silently re-armed); `deferred` re-evaluates automatically, and it carries **two** populations, not one: the flip guard (`inst-rt-flip-guard`) and **transient dependency unavailability** — today `USAGE_TYPE_UNAVAILABLE` (03 `inst-cd-once`), bounded by a per-transition attempt budget after which it lands `failed`. Anything else is terminal. Without that arm a collector blip burned a pinned approval on a lane with no operator to retry it (item 37 of the 2026-08-26 review) - `inst-ar-failure`
3. [ ] - `p2` - Observability: gauges for due-but-unclaimed and deferred counts; the `retirement_held` alert carries the blocking producers from the slice-07 predicate - `inst-ar-observe`

### 3.2 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-lifecycle-errors`

`PARENT_NOT_PUBLISHED` (named in 01, registered here), `SCOPE_NOT_CONTAINED` (**named in 01, its final semantics registered here** — P-D-34 reads C5's "the final form of 01's interim check" literally: this slice replaces the operand inside 01's `identity` phase rather than registering a validator, so the code stays declared in 01 — **P-D-36** withdrew the phase unit and the declaring slice is now the unit),
`SCOPE_NARROWING_BLOCKED`, `RETIREMENT_LEAD_TIME`, `REPLACED_BY_NOT_PUBLISHED`,
`SCHEDULE_STALE_APPROVAL` (raised by the `ActivationRunner` — its own door), `CASCADE_CONFIRMATION_REQUIRED`, `RETIREMENT_PENDING` (**declared here**; both arms are this slice's validators — P-D-30, 2026-08-27; 01 lists it for its response map only, P-D-34: the
un-deprecation edge, and **this slice's validator registered on 01's create door**, whose operand
is the live retire intent in `products_scheduled_transition`, a table this slice owns. P-D-20
struck the code from the publish door. An earlier note read the create-door arm as slice 01's own
guard, which would have put the Foundation's floor in the business of reading lifecycle policy
against §1.1; both arms are therefore this slice's, and this slice declares the code
(**P-D-36** withdrew the phase unit; the declaring slice is the unit). **Owed: the registering instruction row in this slice**), `EOL_DISABLED`. AC #38 rows mapped:
"publishing a SKU under a non-published parent", "a SKU scope falling outside its parent",
"an indeterminate parent-child region-containment", "EOL retirement without an acknowledged
migration consumer (post-v1)".

**Problem responses (RFC 9457):** `SCOPE_NARROWING_BLOCKED`, `SCHEDULE_STALE_APPROVAL`, `RETIREMENT_PENDING`, `PARENT_NOT_PUBLISHED` (409); `CASCADE_CONFIRMATION_REQUIRED`, `EOL_DISABLED`, `SCOPE_NOT_CONTAINED`, `RETIREMENT_LEAD_TIME`, `REPLACED_BY_NOT_PUBLISHED` (422).

*`PARENT_NOT_PUBLISHED` moved 422 → 409 by **P-D-24** (2026-08-27): it is a refusal by the
parent's current state, which is the class §3.3's discriminator assigns to 409 — the same reading
that already put `PARENT_TERMINAL` there. Statuses added 2026-08-26, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, 2026-08-02, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"* — the citation was right the first time;
a 2026-08-26 pass re-pointed it at D-186 and was wrong to, D-186 being a later amendment scoped to
one config route) and where an earlier pass here wrongly wrote
412 and called that pricing's convention — **403** where the caller may not perform the act at
all, **404** only where a path segment names a resource this tenant has none of. **503** where retry
is the remedy is this gear's own addition — pricing's set carries no 503 at all, so that one
class is not "checked against it". **The 422s here are architectural, not wire** — see 01 §3.3, which quotes the
platform rule: no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for a **canonical** error in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.
  Codes listed here for the response map but **declared elsewhere**: `PARENT_NOT_PUBLISHED` (slice 01), `SCOPE_NOT_CONTAINED` (slice 01) — the status is repeated, not a second declaration, so the one-declaration rule stands.*

## 4. Data / Storage (normative shape; DDL in migrations)

- **`products_scheduled_transition`** — `transition_id` (PK) · `tenant_id` · `entity_kind` /
  `entity_id` · `kind ∈ {publish, retire}` · `at` (UTC) · `approval_ref` (the pinned slice-05
  snapshot) · `state ∈ {pending, running, applied, failed, deferred, superseded}` · `reason` ·
  timestamps. Partial `UNIQUE (tenant_id, entity_kind, entity_id, kind) WHERE state IN
  ('pending','running','deferred')` — one live intent per entity per kind; a re-schedule
  supersedes explicitly.
- **`products_deferred_retirement`** — `(tenant_id, product_id, cascade_ref)` PK (`cascade_ref`
  = the parent's `ScheduledTransition` id) · the leave-and-list snapshot (children + reasons,
  JSON) · `created_by` · `resolved_at` · `resolution ∈ {children_cleared, cascade_cancelled}` ·
  timestamps; resolved rows flip `resolved_at`, never delete (audit continuity). Partial
  **`UNIQUE (tenant_id, product_id) WHERE resolved_at IS NULL`** — at most one live deferral per
  Product, and **a cancelled cascade resolves its row** `cascade_cancelled` (item 36 of the
  2026-08-26 review: on the old `(tenant_id, product_id)` PK a cancelled cascade left an
  unresolved row forever and a second cascade on the same Product collided on the PK).
- Columns on `products_sku` (carried by 01): `replaced_by_sku_id`. On **both** entity tables:
  `deprecation_provenance`.
  *(Corrected 2026-08-27: this list had `replaced_by_sku_id` in both places at once. It is
  `products_sku` only — the column names a SKU, `inst-rt-initiate` requires `replacedBy` to name a
  `published` SKU, and this slice's own §4.2 pointer says so. Raised by the slice-01 third lens
  wave, where one lens proposed adding the column to 01 §4.1 on the strength of the wrong half.)*
- **Events**: `SkuDeprecated`/`SkuUndeprecated`/`ProductDeprecated`/`ProductUndeprecated`,
  `SkuRetired`/`ProductRetired` (at initiation, with `effectiveAt`), plus the state flip at
  `effectiveAt` riding a `SkuRetirementEffective` (design-named; consumers that only care about
  the handoff use the initiation event, the flip event is for read models/audit). Scheduling
  acts are audited; `PublishScheduled`/`RetirementScheduled` are audit-plane records, explicit
  "no broker event" per 01 §4.5.

## 5. Testing posture (slice-local)

- Cascade partiality probe: one Product, three children (referenced / clean / never-published)
  → leave-and-list + retire + auto-discard in one plan; the parent stays non-`retired` and the
  intent is queryable.
- Provenance reversal probe: parent un-deprecate revives the `cascaded` child, the `direct`
  sibling stays deprecated (positive + negative in one fixture).
- Flip-guard probes: fresh > 0, stale, never-received — all three defer; fresh all-zero flips;
  the alert names producers.
- Schedule-then-edit RED: edit after scheduling → activation fails `SCHEDULE_STALE_APPROVAL`,
  nothing published, transition `failed`.
- Narrowing probe: Product scope narrow with one child outside → fail-closed naming the child;
  widening passes.
- EOL lockout: any `mustMigrateBy` in v1 refused; the event schema still round-trips the absent
  field (vN compatibility).

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-usecase-lifecycle-deprecation` (§10 use case, claimed by id here 2026-08-26 — all seven were in lint 1's universe and none was claimed); `cpt-cf-bss-products-fr-lifecycle-transitions` (scheduling clauses), `cpt-cf-bss-products-fr-parent-child-integrity` (the final containment rule plus publish ordering; the interim check is slice 01's),
`cpt-cf-bss-products-fr-deprecation` (the deprecation/un-deprecation machine and its cascades), `cpt-cf-bss-products-fr-undeprecation`, `cpt-cf-bss-products-fr-retirement-eol`; AC #14–#18; AC #38 rows above;
pricing D-47 (joint contract), P-D-04 (containment residue).

**Risks & open items**:
- **`SkuRetired` at initiation — CONFIRMED by the 2026-08-25 slice review**: PRD §17.1 defines
  the lead as "≥ 30 days between **event** and effective hide", which mandates emission at
  initiation. Flag closed.
- **Deferred flips can hold indefinitely** while a producer watermark stays stale — correct
  (C4) but operationally invisible without slice 08's surfacing + the `retirement_held` alert;
  the §15 fail-safe tripwire (slice 07) bounds the corrections debt, nothing yet bounds held
  retirements. Candidate for an operator report, not a new mechanism.
- **Cascade + scheduled child publishes**: a `pending` scheduled publish on a child of a
  retiring Product is superseded by the cascade (auto-discard or listed) — stated here, but the
  supersession ordering deserves a probe when built.
- **Owed: the instruction row registering the create-door live-retire-intent validator.**
  **P-D-30 settled whose it is — this slice's**, its operand being the live retire intent in
  `products_scheduled_transition`, which this slice owns; the contrary note reading it as 01's own
  guard is corrected in §3.2 above. No `inst-lc-*`/`inst-rt-*`/`inst-cp-*` row registers it yet,
  and until one does nobody builds the guard — leaving item 36's hole open: a draft SKU created
  under a Product with a live retire intent defers that retirement indefinitely. *(Raised by the
  slice-01 fifth-pass review, 2026-08-27.)*
- **Does any runner write the reserved lane `internal:cascade-leg`?** 01 §3.2 reserves three
  `internal:` lane names and names this slice as the writer of the cascade one, keyed by "the
  leg's" id — but a repo-wide grep finds the lane only in 01, this slice's cascade rows name no
  lane, and its one lane use routes legs through the runner on `internal:scheduled-activation`
  with the transition id. P-D-26 records the restatement obligation as discharged. Either the legs
  ride the activation lane and 01 reserves a name nothing uses, or this slice owes the row that
  writes it — and the `client_key` it is keyed by, which no table here supplies. Owner: this slice
  with 01. *(Raised by the slice-01 fourth lens wave, 2026-08-27.)*
- **What announces a Product's `deprecated→retired` flip?** 01 §4.5 asserts this slice announces
  all three floor edges, naming `SkuRetirementEffective` on `deprecated→retired`. This slice gives
  the parent Product its own retire `ScheduledTransition` on that edge (H2 fix) and emits
  `ProductRetired` at *initiation*, but its Events list names no Product analogue for the flip
  itself and records no explicit "no event" for it — which §4.5's own rule and slice 12's
  completeness check both require. Naming one would invent normative content. Owner: the lifecycle
  owner, with the events/audit consumer set. *(Raised by the slice-01 fifth-pass review,
  2026-08-27.)*
- EOL (post-v1) will need: the subscriptions-lifecycle AC by number, the consumer-ack contract,
  and `SkuEolSuspended` — the schema field is already vN-compatible.
