# Feature: Lifecycle Policy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-lifecycle-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-lifecycle`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Deprecate and un-deprecate](#deprecate-and-un-deprecate)
  - [Schedule a publish](#schedule-a-publish)
  - [Retire a SKU](#retire-a-sku)
  - [Retire a Product](#retire-a-product)
  - [Parent-child integrity](#parent-child-integrity)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [The activation runner](#the-activation-runner)
  - [Lifecycle error taxonomy](#lifecycle-error-taxonomy)
- [4. States (CDSL)](#4-states-cdsl)
  - [ScheduledTransition State Machine](#scheduledtransition-state-machine)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Scheduled-transition store](#scheduled-transition-store)
  - [Deferred-retirement store](#deferred-retirement-store)
  - [Lifecycle columns on the entity tables](#lifecycle-columns-on-the-entity-tables)
  - [Deprecation with provenance](#deprecation-with-provenance)
  - [Deprecation cascade dispositions](#deprecation-cascade-dispositions)
  - [Un-deprecation](#un-deprecation)
  - [Provenance reversal](#provenance-reversal)
  - [Scheduled-publish approval pin](#scheduled-publish-approval-pin)
  - [Activation runner and its claim protocol](#activation-runner-and-its-claim-protocol)
  - [Runner failure posture](#runner-failure-posture)
  - [Retirement initiation](#retirement-initiation)
  - [Lead-window re-announcement](#lead-window-re-announcement)
  - [`replacedBy` and its chain](#replacedby-and-its-chain)
  - [Retirement flip guard](#retirement-flip-guard)
  - [EOL lockout](#eol-lockout)
  - [Cascade plan](#cascade-plan)
  - [The cascading parent's own path](#the-cascading-parents-own-path)
  - [Deferred intent and its surface](#deferred-intent-and-its-surface)
  - [No-orphan invariant](#no-orphan-invariant)
  - [Publish ordering](#publish-ordering)
  - [Scope containment, final rule](#scope-containment-final-rule)
  - [Scope narrowing](#scope-narrowing)
  - [Registered-validator host](#registered-validator-host)
  - [Lifecycle error taxonomy](#lifecycle-error-taxonomy-1)
  - [Lifecycle events](#lifecycle-events)
  - [Audit trail for lifecycle acts](#audit-trail-for-lifecycle-acts)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Raised here rather than carried](#raised-here-rather-than-carried)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

`01-foundation` owns the state-machine **floor** — the edge list, terminality, and the head-row
trigger that states the same rule a second time. This feature owns the **policy on the edges**:
deprecation with `direct`/`cascaded` provenance, governed un-deprecation, scheduled publish and
scheduled retirement with their activation mechanics, the retirement flip guard against
`07-reference-signal`'s predicate, the `replacedBy` successor pointer, parent↔child publish
ordering and scope containment, cascade-retire with deferred intent, and the v1 EOL lockout.

**The seam it fills is named in code, and so is the wall behind it.** `api/rest/skus.rs` says of
the publish re-validation pipeline: *"**`RegisteredValidators` is empty, and that is a real gap, not
a passing phase.** The `→ published` validators the instruction names are `04-lifecycle`'s and
`05-governance`'s, and neither exists at this commit; …"* `Phase::RegisteredValidators` is this
feature's slot.

**What the code does not give it is the keying.** `ValidationRule<S>` carries `name()`, `phase()`
and `evaluate(&self, subject: &S, report)` — and no kind, transition, target-state or field-set
operand. **P-D-97** settles that: the phase is a **slot** filled by a registered rule or a
continuation; the "keying" is the insertion site. Most of this feature's rules read other rows and
run as continuations. See §7 row 20 (closed).

**This feature needs no sixth edge and no sixth state.** `ADMITTED_EDGES` is pinned at five by
`transition_tests::the_five_admitted_edges_are_admitted`, whose message reads *"a sixth needs a
decision behind it"*; `ALL_STATES` is the five-member constant that module's 5×5 sweep quantifies
over, carrying **no assertion on its length**, so a sixth state would pass it silently. On the
edges: retirement forces `deprecated` first and flips
`deprecated → retired`, the parent's path says in its own words that there is no `published →
retired` edge, auto-discard rides `draft → discarded`, and a `draft` child is skipped from a
cascade *because* the floor admits no `draft → deprecated`. EOL is a feature flag, not a state.

**Two shipped tests do go red, and both are registered rather than waved through.**
`events_tests::THE_EIGHT` is an exact roster in both directions, so the first event this feature
adds reddens it (§7 row 25).

### 1.2 Purpose

Lifecycle is where the registry's promises to downstream live or die: `deprecated` must block new
adoption without touching existing references, retirement must be un-surprising — lead time,
successor pointer, grandfathered snapshots untouched — and no hierarchy operation may ever orphan
published content or leak a child outside its parent's scope.

**Requirements** — carried from [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.4:

- Whole: `cpt-cf-bss-products-fr-undeprecation`, `cpt-cf-bss-products-fr-retirement-eol`
- Shared: `cpt-cf-bss-products-fr-deprecation` — the deprecation and un-deprecation machine and
  its cascades; **the consumer-side adoption block is `12-consumer-contracts`'**, which claims the
  same id for that half in §2.12.
  `cpt-cf-bss-products-fr-lifecycle-transitions` — the **scheduling clauses**; the machine
  core is `01-foundation`'s. `cpt-cf-bss-products-fr-parent-child-integrity` — the **final
  containment rule** and publish ordering; the interim check is `01-foundation`'s
- Surfaces: `cpt-cf-bss-products-usecase-lifecycle-deprecation` — listed in §2.4's own Requirements
  Covered block, so this line is carried rather than claimed

**Principles**: `cpt-cf-bss-products-principle-forward-only`.

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`.

**Component**: `cpt-cf-bss-products-component-capability-handlers`.

**Sequence**: `cpt-cf-bss-products-seq-retirement-cascade`.

**Not applicable or delegated**: **Authentication** is the platform IdP's. **Authorization** is
`05-governance`'s RBAC catalog — this feature declares no grant pair of its own, and
[`./governance.md`](./governance.md) §7 row 24 records that the catalog nonetheless carries
`scheduled_transition × write|cancel|read` rows whose door this feature does not declare. **The approval ceremonies** the edges invoke are
`05-governance`'s; this feature **pins** an approval at scheduling and validates it at activation,
and registers its `ScheduledTransition` cancel as a material `GovernedLiveOp` kind — it evaluates
no materiality itself. **The reference predicate** the flip guard reads is
`07-reference-signal`'s; this feature owns only the guard that consults it. **Grandfathered-snapshot
immutability** is `06-catalog-version`'s. **Live-subscription migration** is Subscriptions'. **The
consumer-side adoption block** is the consumer's — the registry marks and exposes, and
`12-consumer-contracts`' seam suite verifies the counterpart. **Observability** primitives and the
outbox are `01-foundation`'s; this feature contributes the `retirement_held` alert and
the two gauges the slice's `inst-ar-observe` names — due-but-unclaimed count and deferred count. **Read-model projection** of the deferred-intent surface is `08-read-models`' —
this feature owns the surface, 08 only projects it. **Operator-facing message wording** is the API
layer's; the seven declared codes are the contract. **Rollout** is forward-only migration per
`01-foundation`, with one feature flag of its own: the EOL lockout, **OFF by default**.
**Presentation** is the studio's. **Rate limiting** is the platform gateway's. **Erasure** of the
operator text this feature stores is `10-retention-erasure`'s; this feature guarantees only that
the reason columns pass `02-taxonomy-attributes`' content-PII write block at the door.

**Out of scope**, mirroring [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.4: the edge list itself
and terminality (`01-foundation`); the reference predicate (`07-reference-signal`); the approval
ceremonies the edges invoke (`05-governance`); grandfathered-snapshot immutability
(`06-catalog-version`); live-subscription migration, which is Subscriptions'; and the consumer-side
adoption block, verified by `12-consumer-contracts`' seam suite.

### 1.3 Actors

| Actor | Role in this feature |
|-------|----------------------|
| `cpt-cf-bss-products-actor-product-manager` | Deprecates; schedules publishes; publishes SKUs and changes Product scope, subject to the parent-child rules |
| `cpt-cf-bss-products-actor-catalog-admin` | Un-deprecates (two-person), initiates retirement and cascades, resumes deferred cascades |
| `cpt-cf-bss-products-actor-plan-price` | The counterpart on its **own** `When` — retirement or unpublishing: flags referencing plans and blocks new adoption. A *plain* deprecation has no counterpart yet; `12-consumer-contracts`' register carries the ask |
| `cpt-cf-bss-products-actor-subscriptions` | Owns live-subscription migration; consumes `replacedBy` (and `mustMigrateBy`, post-v1) |

### 1.4 References

- [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.4 — the entry this feature realizes
- [`../design/04-lifecycle.md`](../design/04-lifecycle.md) — the design slice. **This FEATURE is
  the declaration site of the five `flow-` ids and the two `algo-` ids**, and the slice's §2 and §3
  point here for them; there is one definition site per id. One `algo-` id —
  `cpt-cf-bss-products-algo-activation-runner` — moved here from the slice;
  `cpt-cf-bss-products-algo-lifecycle-errors` is **minted here**, because §3.2's code roster
  carried only a `contract-` id, which a FEATURE may not define.
  **The slice's step lists remain the normative ones and are not copied here**: re-spelling the 22
  instruction steps it owns would fork the set's own instruction register and leave two texts where
  only one can be true. §2 and §3 carry the actor, the scenarios and the boundary.
  - **§4's state machine is a second exception, and its ids are this document's.** The template
    requires a step id per transition row, and the slice expresses the `ScheduledTransition`'s
    states as a column domain in §4 rather than as rows, so no row can be reused. The seven
    `inst-st-*` ids and `cpt-cf-bss-products-state-scheduled-transition` are declared here and
    cited by no slice. **Where §4 and the slice differ on a rule, the slice governs.**
  - **§5 restates the slice's §4 storage shapes**, a third exception on the same terms: a
    Definition of Done must name the columns and constraints it obliges. **Where §5 and the slice's
    §4 differ on a column-level fact, the slice governs.** In particular the slice's §4 carries the
    correction that `replaced_by_sku_id` is on `products_sku` **only**, and its optionality markers
    (`claimed_at` nullable, `retirement_reason` nullable, `outcome_reason` nullable,
    `replacedBy?`) are reproduced rather than normalized away.
  - **`contract-` ids are cited but not defined here.** A FEATURE may **define** only `flow`,
    `algo`, `state`, `dod` and `featstatus` ids, plus the `inst-` steps of a state machine it
    declares. `cpt-cf-bss-products-contract-lifecycle-errors` remains the slice's and is cited by
    id, which survives a renumber where a section number does not.
  - **Seventeen `inst-*` ids this feature cites are owned elsewhere** and are referenced, never
    claimed. **Twelve are other slices'**: `inst-fd-approval-hook`,
    `inst-fd-containment-retire-intent`, `inst-fd-fail-closed`, `inst-fd-publish-reannounce`,
    `inst-fd-save-txn`, `inst-fd-terminal` (`01-foundation`); `inst-av-pii-block`,
    `inst-av-pii-reason` (`02-taxonomy-attributes`); `inst-cd-once` (`03-sku-classification`);
    `inst-gv-materiality`, `inst-gv-one-shot`, `inst-mt-inputs` (`05-governance`). **Five are this
    feature's own slice's**, cited by §4 and §7 rather than restated: `inst-sp-pin`,
    `inst-lc-terminal`, `inst-ar-observe`, `inst-pc-narrowing`, `inst-rt-confirm`
    (`design/04-lifecycle.md`).
- **Dependencies**: `cpt-cf-bss-products-feature-foundation` is the only build-time dependency —
  its edge list, publish door, save door and validation pipeline are what this feature registers
  into. `cpt-cf-bss-products-feature-governance` and
  `cpt-cf-bss-products-feature-reference-signal` are required **at integration only**: governance
  because the lifecycle edges invoke approval ceremonies validated at activation through the gate,
  and reference-signal because of the retirement flip guard, evaluated against that feature's
  predicate. The `lifecycle`↔`reference-signal` cycle is broken by phase — this feature's edge
  policy, deprecation provenance and scheduling build against foundation alone, and the guard is
  wired once 07 lands.
- [`../PRD.md`](../PRD.md) §6.5; §12 AC #14–#18, AC #38 (parent, scope and EOL rows); §4.1 (the
  ≥ 30-day lead); §17.1 (the interim lead-time policy)
- [`../DESIGN.md`](../DESIGN.md) §1.3 layering, §2.1 principles, §2.2 constraints
- [`../DECISIONS.md`](../DECISIONS.md) — P-D-04, P-D-13, P-D-20, P-D-24, P-D-25, P-D-26, P-D-30,
  P-D-32, P-D-34, P-D-35, P-D-36, P-D-39, P-D-44, P-D-46, P-D-48, P-D-49; pricing D-47, whose
  AC #82 is the joint contract's consumer half
- [`./foundation.md`](./foundation.md) — the edge list, the publish and save doors, the validation
  pipeline and the audit trail this feature registers into

## 2. Actor Flows (CDSL)

**Use cases**: `cpt-cf-bss-products-usecase-lifecycle-deprecation`

The step lists live in [`../design/04-lifecycle.md`](../design/04-lifecycle.md) §2 — see §1.4. Each
flow below names its actor, what success and failure look like, and where its boundary runs.

### Deprecate and un-deprecate

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-deprecation`

**Actor**: `cpt-cf-bss-products-actor-product-manager` deprecates;
`cpt-cf-bss-products-actor-catalog-admin` un-deprecates

**Success Scenarios**:
- `published → deprecated` records provenance `direct` for an operator act or `cascaded` for a
  parent-driven one, and emits the deprecation event with the provenance in its payload
- A plain **Product** deprecation cascades `cascaded` deprecation onto its children with a stated
  disposition per child state: a `published` child is deprecated `cascaded`, an already-`deprecated`
  child is **left untouched with its provenance never re-stamped**, and a `draft` child is
  **skipped and listed** rather than transitioned
- Un-deprecation is **two-person**, `N`-governed like any transition to `published`, re-opens
  adoption, and records `quorumReduced` below the default
- Un-deprecating a Product reverses **only `cascaded`** child deprecations; a child's `direct`
  deprecation survives its parent's reversal

**Error Scenarios**:
- Un-deprecation while a live retire intent exists on the entity **or on any child this
  un-deprecation would revive** — `RETIREMENT_PENDING`, and the refusal names them
- A head write on a `retired` or `discarded` row — `ENTITY_TERMINAL`, `01-foundation`'s refusal
  (`inst-fd-terminal`), reached before any edge question is asked

**Boundary**: the registry **marks and exposes**; the new-adoption block is the consumer's.
Aborting a retirement is its own explicit act — a governed cancel of the `ScheduledTransition`
that clears `replaced_by_sku_id` in the same statement for the parent and every child leg the
reversal touches, then un-deprecates. Un-deprecation is never a silent retirement abort. `retired`
is never reversible; revival is a clone, `11-clone`'s. The `draft`-child skip is not a courtesy:
the floor admits no `draft → deprecated` edge and any failure rejects the whole mutation, so
keying the cascade on "non-terminal" would make deprecating a Product with one draft SKU fail
`ILLEGAL_TRANSITION` with no remedy.

### Schedule a publish

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-scheduled-publish`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- Scheduling **pins** the approval at scheduling time and marks that record `consumed` in the
  scheduling transaction; the entity's lifecycle state is unchanged until activation
- At `publishAt` the runner drives `01-foundation`'s **ordinary** publish door in
  `PreAuthorized(approvalId)` mode, resolving the idempotency key as the reserved lane
  `internal:scheduled-activation` with the transition id as `client_key`
- The full pipeline re-runs, pinned-revision check included, and the gate verifies the
  initiation's consumed record rather than demanding a second satisfied one
- Activation is idempotent by transition id; a runner crash replays to the identical outcome

**Error Scenarios**:
- An entity edited after scheduling — `SCHEDULE_STALE_APPROVAL`; the transition lands `failed`
  with an operator alert and **never a partial publish**

**Boundary**: the runner adds **no privileged path** around the pipeline — it is a caller of the
same doors an operator uses. Because it has no wire surface it writes a **lane name** rather than
an endpoint, so two internal lanes cannot collide on one key. `failed` is terminal for that
transition: a stale approval cannot be silently re-armed, and rescheduling is an explicit operator
act. What `PreAuthorized` requires of a leg whose subject is not the pinned subject is **§7 row
22**, and it is not this document's to settle.

### Retire a SKU

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-retire-sku`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- Initiation requires explicit confirmation **with the active-reference count shown**, including
  the predicate's conservative states — the operator confirms against what is known, not against
  silence
- On confirmation the SKU is **forced `deprecated` immediately** and adoption blocking starts;
  on an already-`deprecated` SKU no transition is taken and the provenance is not re-stamped
- `RetirementScheduled` is recorded with an `effectiveAt` honoring the configured lead-time
  policy, and the retirement event is emitted **at initiation** carrying
  `{skuId, fromVersion, reason, replacedBy?, effectiveAt}`
- The head **stays open to publishes** for the whole lead window, and a publish that moves the
  version **re-emits** the retirement event with the new `fromVersion`, the same `effectiveAt` and
  the same retirement identity; consumers key on `(skuId, effectiveAt)` and take the latest
- `replacedBy` is an optional input of retirement initiation, written in that transaction —
  `null` → non-null, the P-D-49 predicate, which does **not** require a lifecycle-state change in
  the same statement, because on an already-`deprecated` SKU no transition is taken — and the read
  surface **resolves the pointer transitively** to the first non-`retired` successor
- At `effectiveAt` the flip runs only on a **fresh all-zero** reference predicate

**Error Scenarios**:
- An `effectiveAt` inside the configured lead — `RETIREMENT_LEAD_TIME`
- A `replacedBy` naming a SKU that is not `published` — `REPLACED_BY_NOT_PUBLISHED`
- Free text in the reason carrying prohibited personal data — `CONTENT_PII_BLOCKED`,
  `02-taxonomy-attributes`' code raised at this door
- Any `mustMigrateBy` in v1 — `EOL_DISABLED`, the flag being OFF by default

**Boundary**: the flip guard is **unconditional** — there is no force-retire door in v1. On a
predicate reading anything but fresh-zero the flip **defers**: the state stays `deprecated`, a
`retirement_held` alert names the blocking producers, and the runner re-evaluates on the
predicate's freshness cadence. **The flip may therefore trail the announced `effectiveAt`**, and
consumers key the state change on the flip event, never on the clock. Truthfulness of the
announced `fromVersion` is maintained by **re-announcement, not by a freeze**: the earlier design
refused publishes for the whole window, and that refusal was struck as a product-visible
constraint no requirement carried. `mustMigrateBy` exists in the schema and is never populated in
v1. Whether `effectiveAt` is an operator input or computed is **§7 row 14**, and the answer decides
whether `RETIREMENT_LEAD_TIME` has a raiser at all.

### Retire a Product

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-retire-product`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- A Product retirement over non-`retired` SKUs requires **confirmed cascade-retire**, and the plan
  computed at confirmation lists each child's disposition across three arms: **retire**,
  **leave-and-list** (deprecated `cascaded` and listed, so adoption stops growing during the
  deferral), and **auto-discard** (never-published drafts, releasing their codes)
- The parent gets its **own** retire `ScheduledTransition` under the same configured lead, is
  forced `deprecated` with provenance `direct`, and emits its retirement event at initiation
- Computing the plan **supersedes**, for every child in all three arms, that child's live publish
  intent **and** its live retire intent, the latter replaced by this cascade's own leg and audited
- Cascades are **partial by design**: where any child is left, the parent's flip defers and a
  deferred-retire intent is recorded — tracked, queryable through this feature's own surface, and
  resumable once the listed children clear

**Error Scenarios**:
- An unconfirmed cascade request — `CASCADE_CONFIRMATION_REQUIRED`
- Any failure during plan application — the **whole mutation** is rejected
  (`01-foundation`'s `inst-fd-fail-closed`)

**Boundary**: "partial by design" means **children left un-retired, never a partly-applied plan**;
application at confirmation is one transaction. The parent's flip guard is **all children
`retired`/`discarded`** — there is no `published → retired` edge for a Product any more than for a
SKU, which is why the parent is forced `deprecated` first. The no-orphan invariant is **re-checked
at flip**, not only planned at confirmation. Which act discharges a deferral, and who writes its
`children_cleared` resolution, is **§7 row 11**.

### Parent-child integrity

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-parent-child`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- A SKU publish under a non-`published` parent is refused, the validator being registered on the
  SKU's **`→ published` target state** rather than on the edge, so a re-publish re-runs it
  fail-closed
- A SKU's brand and region scope must be a **subset** of its parent's — flat value-set subset,
  evaluated on save and re-evaluated on publish
- A scope **widening** on a Product is always admissible

**Error Scenarios**:
- A SKU publish under a non-`published` parent — `PARENT_NOT_PUBLISHED`
- A child scope not provably a subset of its parent's — `SCOPE_NOT_CONTAINED`, declared in
  `01-foundation` and carrying its final semantics here
- A scope-narrowing Product publish while any **non-terminal** child would fall outside the
  narrowed scope — `SCOPE_NOT_CONTAINED` (P-D-96 withdrew `SCOPE_NARROWING_BLOCKED`; both
  directions share one code so they cannot word one refusal two ways)

**Boundary**: containment is **over restrictions**: the empty set means *unrestricted*, so an
unrestricted parent contains every child and an unrestricted child is contained only by an
unrestricted parent. The narrowing operand is **non-terminal**, not non-`retired`: `discarded` is
terminal at the physical layer and is the routine output of the cascade's auto-discard arm, so the
wider operand would let one discarded draft block that Product's narrowing permanently. **Four
facts about the shipped code bear on this flow**: the narrowing check exists, it runs on the save
door with no publish call site, it raises `SCOPE_NOT_CONTAINED`, and it deliberately does not name
the children. Only the third is carried by §7 row 19; the door and the naming are §7 rows 26 and
27.

## 3. Processes / Business Logic (CDSL)

The step lists live in [`../design/04-lifecycle.md`](../design/04-lifecycle.md) §3 — see §1.4.

### The activation runner

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-activation-runner`

**Input**: due rows of `products_scheduled_transition` for both kinds, the pinned approval each
carries, the reference predicate's current answer for a retirement flip, and the claim lease and
per-transition attempt budget — **neither of which carries a value anywhere** (§7 row 8)

**Output**: each claimed row finished `applied`, `failed` or `deferred` with its reason recorded in
`outcome_reason`, plus the two gauges the slice's `inst-ar-observe` names — **due-but-unclaimed
count** and **deferred count** — and the `retirement_held` alert

One runner drives both transition kinds. Due rows are claimed atomically by a state CAS with
`claimed_at`; a `running` row past the claim lease is reclaimed to `pending` with `attempt += 1`,
so a crash never wedges the entity's one live-intent slot, and re-execution is safe because the
doors are idempotent. Execution runs **through the ordinary Foundation doors** — the runner adds no
privileged path around the pipeline.

The runner is **its own raising door**: it wraps the publish door's refusal into
`SCHEDULE_STALE_APPROVAL` on the transition rather than letting the door's own code escape. `failed`
is terminal for that transition.

**Boundary**: `deferred` carries **two populations and not one** — the retirement flip guard, and
transient dependency unavailability — and **only the second is bounded** by the attempt budget
after which it lands `failed`. A flip-guard deferral is **unbounded** by constraint, which is
correct and is what makes §7 row 1 an operational question rather than a defect. Anything else is
terminal. The first reader of `01-foundation`'s `TransitionEffects::bumps_the_guard_owns()` — a
method its own doc records as having no production caller and expects this feature's transition
door to become — is this runner.

### Lifecycle error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-lifecycle-errors`

**Input**: a refused lifecycle act and the rule that refused it

**Output**: one canonical code carrying its declared RFC 9457 status

This feature **declares six** codes: `RETIREMENT_LEAD_TIME`,
`REPLACED_BY_NOT_PUBLISHED`, `SCHEDULE_STALE_APPROVAL`, `CASCADE_CONFIRMATION_REQUIRED`,
`RETIREMENT_PENDING` and `EOL_DISABLED`. **`SCOPE_NARROWING_BLOCKED` is withdrawn (P-D-96).**
**Two** appear in its response map and are declared in
`01-foundation`: `PARENT_NOT_PUBLISHED` and `SCOPE_NOT_CONTAINED` — the status is repeated, not a
second declaration. **Seven more** are cited from elsewhere and raised by their own owners:
`APPROVAL_REQUIRED`, `CONTENT_PII_BLOCKED`, `ILLEGAL_TRANSITION`, `PARENT_TERMINAL`,
`STALE_REVISION`, `STALE_VERSION`, `USAGE_TYPE_UNAVAILABLE`. Six, two and seven make the fifteen
distinct codes the slice names after the withdrawal. **One more is raised by a §2 scenario and named by no slice**:
`ENTITY_TERMINAL`, `01-foundation`'s refusal on a head write against a `retired` or `discarded`
row, which reaches every act this feature drives and so belongs in the response map — sixteen in
all.

**Boundary**: the roster and every status are specified by
`cpt-cf-bss-products-contract-lifecycle-errors`, which stays in the slice.
`SCHEDULE_STALE_APPROVAL`, `RETIREMENT_PENDING` and `PARENT_NOT_PUBLISHED` sit at 409;
`CASCADE_CONFIRMATION_REQUIRED`, `EOL_DISABLED`, `SCOPE_NOT_CONTAINED`, `RETIREMENT_LEAD_TIME` and
`REPLACED_BY_NOT_PUBLISHED` at 422 **architecturally**, reaching the wire as 400 carrying their
code, no `CanonicalError` category rendering 422 absent a transport override this design set does
not declare. **One of the six declared codes has a raise-path problem still carried**:
`RETIREMENT_LEAD_TIME` can never be raised if `effectiveAt` is computed rather than supplied
(§7 row 14). Narrowing's code question is closed (P-D-96).

## 4. States (CDSL)

### ScheduledTransition State Machine

- [ ] `p1` - **ID**: `cpt-cf-bss-products-state-scheduled-transition`

The seven rows below are this document's rendering of the slice's §4 column domain; see §1.4.

**States**: `pending`, `running`, `applied`, `failed`, `deferred`, `superseded`

**Initial State**: `pending`, written by the scheduling act — a scheduled publish
(`inst-sp-pin`), a retirement initiation, or a cascade leg. The row is **live** in `pending`,
`running` and `deferred`, which is the partial-unique predicate that holds one intent per entity
per kind.

**Transitions**:
1. [ ] - `p1` - **FROM** `pending` **TO** `running` **WHEN** the runner's poll finds the row due
   and wins the atomic state CAS, stamping `claimed_at`. Authorization: the runner's own identity,
   which holds no privileged path around the pipeline the doors run - `inst-st-claim`
2. [ ] - `p1` - **FROM** `deferred` **TO** `running` **WHEN** the same poll re-claims it; the
   slice calls this *"the only exit that state has"*, which **§7 row 23 shows cannot be literally
   true** while a cascade supersedes live intents and `deferred` counts as live. Authorization: as
   row 1 - `inst-st-reclaim`
3. [ ] - `p1` - **FROM** `running` **TO** `pending` **WHEN** the row is past its claim lease, with
   `attempt += 1`, so a crashed runner never wedges the entity's one live-intent slot; safe because
   the doors are idempotent. Authorization: as row 1; **the lease has no value anywhere** (§7 row
   8) - `inst-st-lease-reclaim`
4. [ ] - `p1` - **FROM** `running` **TO** `applied` **WHEN** the Foundation door the runner drove
   committed. Authorization: the **pinned** approval, verified at activation in `PreAuthorized`
   mode — the runner consumes nothing further, the record having been consumed at the initiation
   transaction - `inst-st-applied`
5. [ ] - `p1` - **FROM** `running` **TO** `deferred` **WHEN** either the retirement flip guard
   reads anything but fresh-zero — **unbounded** by constraint — or a transient dependency is
   unavailable, **bounded** by the attempt budget. `outcome_reason` records which population.
   Authorization: as row 1 - `inst-st-defer`
6. [ ] - `p1` - **FROM** `running` **TO** `failed` **WHEN** the door refused terminally — wrapped
   as `SCHEDULE_STALE_APPROVAL` — or the transient-deferral attempt budget is exhausted. Terminal:
   an operator reschedules explicitly, because a stale approval may not be silently re-armed.
   Authorization: as row 1 - `inst-st-failed`
7. [ ] - `p1` - **FROM** a live state **TO** `superseded` **WHEN** an explicit re-schedule, a
   governed cancel, or a cascade plan replaces the intent; audited in every case. Authorization:
   the cancel is a `GovernedLiveOp` kind **registered material by this feature**, so it follows the
   tenant's `N` with `quorumReduced` recorded — without that registration the evaluator judges it
   non-material and one approver, or none at `N = 0`, could unwind a cascade. **Which live states
   admit this edge is §7 row 23**, and **which actor performs the cancel is §7 row 15** - `inst-st-superseded`

**Terminal states**: `applied`, `failed`, `superseded`. No transition other than the seven above is
admitted, and no edge leaves a terminal state — a re-schedule after `applied` or `failed` is a
**new row**, which is what keeps the partial unique index sound.

## 5. Definitions of Done

**Twenty-six**, counted by `grep` on this file rather than from the plan that sized them.
**Twenty are separately testable.** Six are not, and each names what it needs:
`dod-flip-guard` reads `07-reference-signal`'s predicate, which does not exist, and `dod-no-orphan`
reaches it transitively — its own operands are child lifecycle states, but the flip it re-checks at
is guarded by that predicate — so both are testable only against a stub — and a stub that always answers fresh-zero passes the
guard while proving nothing, which is why a **four-state negative control** is part of the
obligation; `dod-undeprecation` and `dod-scheduled-publish-pin` need `05-governance`'s gate host and
approval store, the existing `RecordingGate` double being the shape; `dod-registered-validator-host` fills the phase slot under **P-D-97** (registered rule or
continuation); and `dod-lifecycle-events` depends on a broker payload shape this feature adds a
third body to, and on a roster test that is exact in both directions (§7 row 25).

### Scheduled-transition store

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-scheduled-transition-store`

The system **MUST** create `products_scheduled_transition` on both engines carrying `transition_id`
(PK), `tenant_id`, `entity_kind`/`entity_id`, `kind ∈ {publish, retire}`, `at` (UTC), `approval_ref`,
`state ∈ {pending, running, applied, failed, deferred, superseded}`, `claimed_at` (nullable, UTC),
`attempt` (integer, `NOT NULL`, default 0), `retirement_reason` (nullable — the **operator's** text)
and `outcome_reason` (nullable — the **runner's** outcome text), plus timestamps. **The two reason
columns MUST stay separate**: one column let a deferral's failure text overwrite the operator's. A
**partial** `UNIQUE (tenant_id, entity_kind, entity_id, kind) WHERE state IN ('pending','running','deferred')`
**MUST** hold one live intent per entity per kind. A schema-oracle golden **MUST** exist on both
engines with a perturbation case proving it can fail.

**Implements**: `cpt-cf-bss-products-flow-scheduled-publish`,
`cpt-cf-bss-products-flow-retire-sku`, `cpt-cf-bss-products-flow-retire-product`,
`cpt-cf-bss-products-state-scheduled-transition`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_scheduled_transition`
- Entities: `ScheduledTransition`

### Deferred-retirement store

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-deferred-retirement-store`

The system **MUST** create `products_deferred_retirement` keyed
`(tenant_id, product_id, cascade_ref)` — `cascade_ref` being the parent's `ScheduledTransition`
id — carrying the leave-and-list snapshot of children and reasons, `created_by`, `resolved_at`
(nullable — the partial index below is defined over its NULLs),
`resolution ∈ {children_cleared, cascade_cancelled}` and timestamps. Resolved rows **MUST** flip
`resolved_at` and **never delete**, for audit continuity. A **partial**
`UNIQUE (tenant_id, product_id) WHERE resolved_at IS NULL` **MUST** hold at most one live deferral
per Product, and a cancelled cascade **MUST** resolve its row `cascade_cancelled` — on the bare
composite key a cancelled cascade left an unresolved row forever and a second cascade on the same
Product collided.

**Implements**: `cpt-cf-bss-products-flow-retire-product`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_deferred_retirement`
- Entities: `DeferredRetireIntent`

### Lifecycle columns on the entity tables

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-lifecycle-columns`

The system **MUST** carry `deprecation_provenance` on **both** entity tables and
`replaced_by_sku_id` on **`products_sku` only** — the column names a SKU. Both are created by
`01-foundation`'s migrations and this feature is their first writer. `replaced_by_sku_id` **MUST**
be write-once **per retirement, not per row**: the head-row whitelist admits the governed cancel's
clearing write, and without it a cancelled, un-deprecated SKU stays `published` while permanently
naming a successor no admitted write could clear.

**Both columns ship with the row-image predicates P-D-34 pins, and an earlier revision of this
paragraph had it backwards.** It said the write-once property was the head guard's existing shape
rather than a new clause, because that guard admits everything else — but a guard admitting
everything else has **no** write-once property, and **P-D-34** lists these two among *"Four row-image predicates the first migration's trigger was missing"*. Both are installed now, in the
shape `composition_pending`'s predicate already had seven lines above them: `deprecation_provenance`
only in the same statement as a `lifecycle_state` change, and `replaced_by_sku_id` admitting
`null → non-null` and `non-null → null` and refusing between two non-nulls.

The probe moved with them. It stamped both columns on an **already-`retired`** row and called that
"by design"; `design/04` says the successor is *"written by that act in the same statement as its
`lifecycle_state` change"*, so the write rides the statement that **makes** the row terminal, and
the old probe asserted a write three normative texts refuse.

**The tick was withdrawn and is restored, 2026-09-01.** It went not because the columns were
unbuilt but because this `DoD`'s own paragraph asserted a property the code did not have, and a
tick resting on a false sentence is the one thing the register must not carry. Its two stated
return conditions are both met: `dod-append-only-guard`'s roster now says which of its clauses
have no column to govern, so `composition_pending`'s predicate can no longer be read as the
bucket-ii one; and §7 row 33 is closed on re-measurement, the third of its *three answers* having
been a stale attribution the crate corrected in an earlier wave. The predicates it describes are
the ones that ship.

Both are registered `Outside(Mechanical)` in the bucket registry, and that is **measured, not
chosen**: `design/01` §4.3 groups `deprecation_provenance` and `replaced_by_sku_id` with
`lifecycle_state` and `internal_revision` as the four that *"move on transitions, which write no
version row"* (**P-D-24** as **P-D-35** extended it), and the other two of that four were already
registered `Mechanical`. Two shipped censuses moved with them — the class counts, and the
fail-closed list that had been asserting these two columns' **absence**.

*(The Foundation migrations' own docs credited slice **03** with these columns. `design/03` names
neither and `design/04` §4.2 owns the pair, so that attribution was wrong rather than stale; it is
corrected in the same change, along with a count that predated `cloned_from` landing.)*

**Implements**: `cpt-cf-bss-products-flow-retire-sku`, `cpt-cf-bss-products-flow-deprecation`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Entities: `Product`, `SKU`

### Deprecation with provenance

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-deprecation-provenance`

The system **MUST** stamp `deprecation_provenance` as `direct` on an operator act and `cascaded`
on a parent-driven one, on the `published → deprecated` edge, and **MUST** emit the deprecation
event carrying the provenance in its payload. An **already-`deprecated`** entity **MUST NOT** be
re-stamped: `direct` re-stamped as `cascaded` would make a parent's un-deprecation revive exactly
the child the requirement says it must not.

**Implements**: `cpt-cf-bss-products-flow-deprecation`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Entities: `Product`, `SKU`
- API: `POST /bss-products/v1/skus/{id}/deprecate`

### Deprecation cascade dispositions

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-deprecation-cascade`

The system **MUST** cascade a plain Product deprecation onto its children with a stated disposition
per child state: `published` ⇒ deprecated `cascaded`; already-`deprecated` ⇒ **left untouched**;
`draft` ⇒ **skipped and listed**, never transitioned, because the floor admits no
`draft → deprecated` edge and any failure rejects the whole mutation. `retired` and `discarded`
children are terminal and outside the population. The listing **MUST** be what the operator sees.

**Implements**: `cpt-cf-bss-products-flow-deprecation`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_sku`
- Entities: `SKU`

### Un-deprecation

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-undeprecation`

The system **MUST** put `deprecated → published` behind the governance gate as an `N`-governed
two-person ceremony, recording `quorumReduced` below the default, and **MUST** refuse it
`RETIREMENT_PENDING` while a live retire intent exists on the entity **or on any child this
un-deprecation would revive**, the refusal naming them. Checking only the subject's own intent let
a parent's cancel-then-un-deprecate revive `cascaded` children whose own retire intents stayed
live, after which the runner needed a `published → retired` edge that does not exist. The
governed cancel that aborts a retirement **MUST** clear `replaced_by_sku_id` in the same statement,
for every child leg the reversal touches — ~~and for the parent~~, which is a Product and has no such
column (`dod-lifecycle-columns` pins it to `products_sku`; **P-D-114**, 2026-09-03).

**Implements**: `cpt-cf-bss-products-flow-deprecation`,
`cpt-cf-bss-products-state-scheduled-transition`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`, `products_scheduled_transition`
- Entities: `Product`, `SKU`, `ScheduledTransition`
- API: `POST /bss-products/v1/{products|skus}/{id}/undeprecate`, `POST /bss-products/v1/{products|skus}/{id}/retire/cancel`

### Provenance reversal

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-provenance-reversal`

The system **MUST** reverse **only `cascaded`** child deprecations when a Product is un-deprecated;
a child's `direct` deprecation **MUST** survive its parent's reversal, the provenance column being
the operand.

**Implements**: `cpt-cf-bss-products-flow-deprecation`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_sku`
- Entities: `SKU`
- API: `POST /bss-products/v1/{products|skus}/{id}/undeprecate`

### Scheduled-publish approval pin

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-scheduled-publish-pin`

The system **MUST** pin the approval at scheduling time onto the `ScheduledTransition` and mark
that record `consumed` **in the scheduling transaction**, leaving the entity's lifecycle state
unchanged until activation. Activation **MUST** verify that consumed record in `PreAuthorized`
mode rather than demanding a second satisfied one, and **MUST NOT** consume anything further.

**Implements**: `cpt-cf-bss-products-flow-scheduled-publish`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_scheduled_transition`
- Entities: `ScheduledTransition`

### Activation runner and its claim protocol

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-activation-runner`

The system **MUST** run one runner over both transition kinds, claiming due rows by an **atomic
state CAS** stamping `claimed_at`, re-claiming a `deferred` row on the same poll, and reclaiming a
`running` row past the claim **lease** to `pending` with `attempt += 1`. Execution **MUST** go
through the ordinary Foundation doors with **no privileged path** around the pipeline. Activation
**MUST** be idempotent by transition id, resolving the idempotency key as the reserved lane
`internal:scheduled-activation` with the transition id as `client_key`. Each row **MUST** finish
`applied`, `failed` or `deferred` with its reason in `outcome_reason`.

**Implements**: `cpt-cf-bss-products-algo-activation-runner`,
`cpt-cf-bss-products-state-scheduled-transition`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_scheduled_transition`
- Entities: `ScheduledTransition`

### Runner failure posture

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-runner-failure-posture`

The runner **MUST** be its **own raising door**, wrapping the publish door's `STALE_REVISION` or
`APPROVAL_REQUIRED` refusal into `SCHEDULE_STALE_APPROVAL` on the transition. `failed` **MUST** be
terminal for that transition. `deferred` **MUST** carry two populations distinguishably — the
retirement flip guard, **unbounded**, and transient dependency unavailability, **bounded** by a
per-transition attempt budget after which it lands `failed`. Everything else **MUST** be terminal.
Without the bounded arm a collector blip burns a pinned approval on a lane with no operator to
retry it.

**Implements**: `cpt-cf-bss-products-algo-activation-runner`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_scheduled_transition`
- Entities: `ScheduledTransition`

### Retirement initiation

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-retirement-initiation`

The system **MUST** require explicit confirmation showing the **active-reference count** including
its conservative states, then in one transaction force the SKU `deprecated` — taking no transition
and re-stamping no provenance where it already is — record `RetirementScheduled` with an
`effectiveAt` honoring the configured lead-time policy or refusing `RETIREMENT_LEAD_TIME`, run
`02-taxonomy-attributes`' content-PII write block over the free-text `reason` refusing
`CONTENT_PII_BLOCKED`, and emit the retirement event **at initiation** carrying
`{skuId, fromVersion, reason, replacedBy?, effectiveAt}` where `fromVersion` is the entity's
`published_version` at the initiation instant. **Saves MUST stay legal** during the window: they
touch the head, which no consumer reads.

**Implements**: `cpt-cf-bss-products-flow-retire-sku`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_sku`, `products_scheduled_transition`
- Entities: `SKU`, `ScheduledTransition`
- API: `POST /bss-products/v1/{products|skus}/{id}/retire`

### Lead-window re-announcement

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-lead-window-reannounce`

The head **MUST** stay open to publishes for the whole lead window, and a publish that moves the
version **MUST** re-emit the retirement event with the new `fromVersion`, the same `effectiveAt`
and the same retirement identity, enqueued in the publish's own transaction by `01-foundation`'s
publish door. A publish **outside** any window **MUST** emit none. Consumers key on
`(skuId, effectiveAt)` and take the latest. The alternative — refusing publishes for the window —
was a product-visible constraint no requirement carried and is **not** to be reintroduced.

**Implements**: `cpt-cf-bss-products-flow-retire-sku`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_sku`, `products_entity_version`, `products_scheduled_transition`
- Entities: `SKU`, `EntityVersion`, `ScheduledTransition`

### `replacedBy` and its chain

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-replaced-by`

`replacedBy` **MUST** be an optional input of retirement initiation, written to
`replaced_by_sku_id` **in the retirement-initiation transaction** — `null` → non-null, P-D-49's
predicate, which does not require a lifecycle-state change in the same statement, the initiation on
an already-`deprecated` SKU taking no transition at all — and when given **MUST**
name a `published` SKU or refuse `REPLACED_BY_NOT_PUBLISHED`. It is **validated once** and the row
is terminal at the flip, so the pointer may come to name a later-retired SKU: retiring a SKU that
any live pointer names **MUST** raise a `replacement_chain_broken` alert listing the pointing SKUs,
and the read surface **MUST** resolve the pointer transitively to the first non-`retired` successor
or report the chain's end. Repairing the frozen pointer is out of scope in v1. **Where the alert's
fact is stored, what bounds the walk, and what a cycle returns are §7 rows 12 and 13**, so this
obligation's storage half cannot be closed until they are answered.

**Implements**: `cpt-cf-bss-products-flow-retire-sku`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_sku`
- Entities: `SKU`

### Retirement flip guard

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-flip-guard`

At `effectiveAt` the flip **MUST** consult `07-reference-signal`'s predicate and **MUST** defer
where it reads anything but **fresh-zero** — fresh > 0, stale, never-received, or the defensive
no-producers state — leaving the state `deprecated`, raising a `retirement_held` alert naming the
blocking producers, and re-evaluating on the predicate's freshness cadence. The flip **MUST**
happen only on a fresh all-zero. The flip **MAY** therefore trail the announced `effectiveAt`, and
the flip event is what consumers key the state change on. There **MUST** be no force-retire door in
v1. A stub predicate **MUST** be exercised in all four deferring states as well as the passing one.

**Implements**: `cpt-cf-bss-products-flow-retire-sku`,
`cpt-cf-bss-products-algo-activation-runner`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_sku`, `products_scheduled_transition`
- Entities: `SKU`, `ScheduledTransition`

### EOL lockout

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-eol-lockout`

`mustMigrateBy` and the consumer-acknowledgment machinery **MUST** be refused in v1 with
`EOL_DISABLED`, behind a feature flag **OFF by default**. The payload field **MUST** exist in the
event schema for vN-compatible widening and **MUST** never be populated in v1, and the schema
**MUST** round-trip the absent field.

**Implements**: `cpt-cf-bss-products-flow-retire-sku`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_sku`
- Entities: `SKU`

### Cascade plan

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-cascade-plan`

A Product retirement over non-`retired` SKUs **MUST** require confirmed cascade-retire, refusing an
unconfirmed request `CASCADE_CONFIRMATION_REQUIRED`. The plan computed at confirmation **MUST**
list each child's disposition across exactly three arms — **retire** (scheduled per the SKU flow,
provenance `cascaded`, the plan passing provenance through), **leave-and-list** (deprecated
`cascaded` and listed), **auto-discard** (never-published drafts, releasing their codes). Plan
application **MUST** be one transaction in which any failure rejects the whole mutation. Computing
the plan **MUST** supersede, for every child in all three arms, that child's live publish intent
**and** its live retire intent, audited as such — superseding only publish intents collided with
the one-live-intent-per-kind unique index for any child already holding a retire intent.

**Implements**: `cpt-cf-bss-products-flow-retire-product`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_sku`, `products_scheduled_transition`
- Entities: `SKU`, `CascadePlan`, `ScheduledTransition`

### The cascading parent's own path

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-cascade-parent-path`

At confirmation the parent Product **MUST** be forced `deprecated` with provenance `direct` and
**MUST** get its **own** retire `ScheduledTransition` under the same configured lead, emitting its
retirement event at initiation. The parent's flip guard **MUST** be **all children
`retired`/`discarded`** — there is no `published → retired` edge for a Product any more than for a
SKU.

**Implements**: `cpt-cf-bss-products-flow-retire-product`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_scheduled_transition`
- Entities: `Product`, `ScheduledTransition`

### Deferred intent and its surface

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-deferred-intent`

Where any child is left un-retired the parent's flip **MUST** defer and a `DeferredRetireIntent`
**MUST** be recorded — tracked and **queryable through this feature's own surface**, which
`08-read-models` projects rather than owns — and resumable by an operator once the listed children
clear. "Partial by design" **MUST** mean children left un-retired and **never** a partly-applied
plan.

**Implements**: `cpt-cf-bss-products-flow-retire-product`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_deferred_retirement`
- Entities: `DeferredRetireIntent`

### No-orphan invariant

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-no-orphan`

No `published` SKU **MUST** exist under a `retired` Product, and the invariant **MUST** be
re-checked **at flip**, not only planned at confirmation.

**Implements**: `cpt-cf-bss-products-flow-retire-product`,
`cpt-cf-bss-products-flow-parent-child`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Entities: `Product`, `SKU`

### Publish ordering

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-publish-ordering`

A SKU publish under a non-`published` parent **MUST** fail `PARENT_NOT_PUBLISHED`, the validator
registered on the SKU's **`→ published` target state** and not on the edge, so a re-publish re-runs
it fail-closed. The code is declared in `01-foundation` and registered here; **which slice declares
it is open in that slice's own §6** and is not this document's to settle (§7 row 18).

**Implements**: `cpt-cf-bss-products-flow-parent-child`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Entities: `Product`, `SKU`

### Scope containment, final rule

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-scope-containment-final`

A SKU's brand and region scope **MUST** be a flat value-set subset of its parent's, evaluated on
save and re-evaluated on publish, anything not provably a subset failing `SCOPE_NOT_CONTAINED`.
Containment is **over restrictions**: the empty set means unrestricted, so an unrestricted parent
contains every child and an unrestricted child needs an unrestricted parent.

**This obligation MUST begin by measuring what is left to replace.** `01-foundation` ships
`domain::containment` implementing exactly these three clauses, wired at **four** call sites — the
SKU create door, the SKU save door, the SKU publish re-check and the Product save door. The slice describes this feature as replacing the
operand inside the identity phase rather than registering a validator; **what operand remains is
§7 row 21**, and a DoD that obliges a replacement of something already final would be a no-op
dressed as work.

**Implements**: `cpt-cf-bss-products-flow-parent-child`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Entities: `Product`, `SKU`

### Scope narrowing

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-scope-narrowing`

A scope-narrowing Product publish **MUST** fail closed while any **non-terminal** child —
`draft`, `published` or `deprecated` — would fall outside the narrowed scope, and the validator
**MUST** name the falling-out children. Widening is always admissible. The operand is
**non-terminal, not non-`retired`**: `discarded` is terminal at the physical layer and is the
routine output of the auto-discard arm, so the wider operand would let one discarded draft block
that Product's narrowing permanently.

**Four shipped facts bear on this, and each is registered**: the check exists as
`products::check_children_stay_contained` with the non-terminal operand already correct; it raises
**`SCOPE_NOT_CONTAINED`** — and **P-D-96 withdrew** the competing `SCOPE_NARROWING_BLOCKED`
declaration so both directions keep one code; it runs on the **save** door and has no publish call
site (§7 row 26); and its refusal **deliberately does not name the offending child**, on the stated
ground that naming one would be a second wording of a shared message (§7 row 27) — and that
standing choice holds for `PARENT_NOT_PUBLISHED` too.

**Implements**: `cpt-cf-bss-products-flow-parent-child`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Entities: `Product`, `SKU`

### Registered-validator host

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-registered-validator-host`

This feature **MUST** fill `01-foundation`'s `RegisteredValidators` phase for the `→ published`
target state, which the Foundation's own publish re-validation records as *"a real gap, not a
passing phase"*. It **MUST NOT** mint a parallel validation vocabulary: the `ValidationRule` trait,
the `Phase` enum and the pipeline are the Foundation's.

**Whether this feature's rules can be registered rules at all was §7 row 20; it is
closed as P-D-97.** `RegisteredValidators` is a phase **slot** with two admissible
fillings: a registered `ValidationRule` where the operand is subject-local or a
single fact the door can prefetch, **or** a **continuation** of that phase on the
same transaction (the position §4.1 asks for). The trait does not widen. **Residue
to honour**: a continuation raises a `DomainError` directly and does **not** append
to a `ValidationReport`, so it refuses on the first finding rather than collecting
several within its phase — every cross-row rule here is a single-condition refusal,
so nothing is lost, but the two fillings are not interchangeable in every respect.

**Implements**: `cpt-cf-bss-products-flow-parent-child`,
`cpt-cf-bss-products-flow-deprecation`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_product`, `products_sku`
- Entities: `Product`, `SKU`

### Lifecycle error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-lifecycle-errors`

The system **MUST** declare six codes — `RETIREMENT_LEAD_TIME`,
`REPLACED_BY_NOT_PUBLISHED`, `SCHEDULE_STALE_APPROVAL`, `CASCADE_CONFIRMATION_REQUIRED`,
`RETIREMENT_PENDING`, `EOL_DISABLED` — each carrying its declared RFC 9457 status and each raised by
**exactly one** rule. `PARENT_NOT_PUBLISHED` and `SCOPE_NOT_CONTAINED` appear in the response map
carrying `01-foundation`'s declarations, repeated and not re-declared. **`SCOPE_NARROWING_BLOCKED`
is withdrawn (P-D-96)** — narrowing rides `SCOPE_NOT_CONTAINED`. **`PARENT_NOT_PUBLISHED` and
`RETIREMENT_PENDING` are owned slots this feature fills in D7** (P-D-24 already prices the former
at 409; the mapping paragraph already assigns both to 04). `RETIREMENT_LEAD_TIME`
has no raiser at all if `effectiveAt` is computed (§7 row 14).

**Implements**: `cpt-cf-bss-products-algo-lifecycle-errors`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- Entities: `SKU`, `Product`, `ScheduledTransition`

### Lifecycle events

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-lifecycle-events`

The system **MUST** emit the deprecation, un-deprecation and retirement events for both entity
kinds, and the SKU retirement-effective flip event, through `01-foundation`'s outbox. Scheduling
acts **MUST** be audit-plane records with an **explicit "no broker event"** statement rather than
silence. **What announces a Product's `deprecated → retired` flip is §7 row 5** — the Events roster
names no Product analogue and records no explicit no-event for it, and `01-foundation`'s
`domain::transition` module doc independently records the same hole. The gear ships **eight**
payload types today on two body shapes — `ProductCreated`, `SkuCreated`, `ProductHeadSaved`,
`SkuHeadSaved`, `ProductPublished`, `SkuPublished`, `ProductDiscarded`, `SkuDiscarded`; the
retirement payload's five fields need a third body, and this DoD **MUST** widen the shape rather
than overload `EventBodyCore`. Each new event **MUST** also take its own versioned schema
reference, and `events_tests::THE_EIGHT` **MUST** be extended in the same change — that roster is
asserted exact in **both** directions, so the first event added without it reddens the suite.

**Implements**: `cpt-cf-bss-products-flow-deprecation`,
`cpt-cf-bss-products-flow-retire-sku`, `cpt-cf-bss-products-flow-retire-product`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: the toolkit outbox's own tables under `OUTBOX_TABLE_PREFIX` — **P-D-22 struck
  `products_outbox`** as a gear-authored table and this gear declares none
- Entities: `SKU`, `Product`

### Audit trail for lifecycle acts

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-lifecycle-audit`

Every lifecycle act and every refusal this feature raises **MUST** leave an audit row carrying its
reason — every flow this feature declares, not the runner alone — through `01-foundation`'s audit
plane and its pseudonymous actor refs. Supersessions,
governed cancels, deferrals and the `retirement_held` and `replacement_chain_broken` alerts
**MUST** each be recorded rather than inferred from a state change.

**Implements**: `cpt-cf-bss-products-algo-activation-runner`,
`cpt-cf-bss-products-flow-deprecation`, `cpt-cf-bss-products-flow-scheduled-publish`,
`cpt-cf-bss-products-flow-retire-sku`, `cpt-cf-bss-products-flow-retire-product`,
`cpt-cf-bss-products-flow-parent-child`

**Constraints**: `cpt-cf-bss-products-constraint-immutable-identity`

**Touches**:
- DB Table: `products_audit_log`
- Entities: `ScheduledTransition`, `SKU`, `Product`

## 6. Acceptance Criteria

- [ ] `ADMITTED_EDGES` still has exactly five members after this feature lands — the one shipped
      assertion that would catch an invented edge. `ALL_STATES` carries no length assertion, so a
      sixth state needs one written here
- [ ] A `published` child is deprecated `cascaded`, an already-`deprecated` child keeps its
      `direct` stamp, and a `draft` child is listed rather than transitioned — all three arms in
      one fixture
- [ ] Deprecating a Product with one `draft` SKU **succeeds** and lists the draft; the
      non-terminal keying that made it fail `ILLEGAL_TRANSITION` is red before the fix
- [ ] Un-deprecating a parent revives the `cascaded` child and leaves the `direct` sibling
      deprecated — positive and negative in one fixture
- [ ] Un-deprecation with a live retire intent on a **child** is refused and the refusal names the
      child; with no live intent it is admitted
- [ ] A governed cancel clears `replaced_by_sku_id` on the parent and every child leg, and the row
      is writable a second time only through that cancel
- [ ] Scheduling marks the approval `consumed` in the scheduling transaction and leaves the
      lifecycle state unchanged; activation consumes nothing further
- [ ] Editing an entity after scheduling makes activation fail `SCHEDULE_STALE_APPROVAL`, publishes
      nothing, and lands the transition `failed` — the positive control being an unedited entity
      that activates
- [ ] A crashed runner's `running` row past the lease returns to `pending` with `attempt` bumped,
      and re-execution reaches the identical outcome
- [ ] A `deferred` row is re-claimed on the next poll; a flip-guard deferral is **never** bounded
      by the attempt budget while a transient-dependency deferral lands `failed` when it is spent
- [ ] Retirement initiation shows the reference count including its conservative states — whether a
      confirmation against a stale count is *refused* is §7 row 34, and no declared code covers it
- [ ] Initiation on an already-`deprecated` SKU takes no transition, re-stamps no provenance, and
      emits no deprecation event — asserted as set equality over emitted events, not by inspection
- [ ] A publish during the lead window re-emits the retirement event with the new `fromVersion`,
      the same `effectiveAt` and the same identity; a publish outside any window emits none
- [ ] Saves during the lead window are **admitted** — the positive control on the struck freeze
- [ ] The flip defers on all four conservative predicate states and flips on fresh all-zero; the
      alert names the blocking producers
- [ ] The four-state deferral is asserted against a stub that can answer each state, not against
      one that always answers fresh-zero
- [ ] A `replacedBy` naming a non-`published` SKU is refused; naming a `published` one is admitted
- [ ] Retiring a SKU that a live pointer names raises `replacement_chain_broken` listing the
      pointing SKUs, and the read surface resolves transitively to the first non-`retired` successor
- [ ] A Product retirement over non-`retired` SKUs without confirmation is refused
      `CASCADE_CONFIRMATION_REQUIRED`; with confirmation it is admitted
- [ ] One Product with three children — referenced, clean, never-published — yields leave-and-list,
      retire and auto-discard in one plan; the parent stays non-`retired` and the intent is queryable
- [ ] A failure anywhere in plan application leaves **no** child transitioned — asserted over the
      whole child set, not over the failing child
- [ ] Computing a plan supersedes a child's live retire intent as well as its live publish intent,
      proven against the one-live-intent-per-kind unique index
- [ ] A cancelled cascade resolves its deferred-retirement row `cascade_cancelled`, and a second
      cascade on the same Product is admitted afterwards
- [ ] The parent's flip is refused while any child is non-terminal and admitted once all are
      `retired`/`discarded`; the no-orphan invariant is re-checked at flip and not only at
      confirmation
- [ ] A SKU publish under a non-`published` parent is refused `PARENT_NOT_PUBLISHED`; under a
      `published` parent it is admitted; a re-publish re-runs the check fail-closed
- [ ] An unrestricted parent contains every child; an unrestricted child is refused by a restricted
      parent; between two non-empty sets it is ordinary subset
- [ ] A Product narrowing with one non-terminal child outside is refused with
      `SCOPE_NOT_CONTAINED` (P-D-96); one whose only outside child is `discarded` is admitted; a
      widening is always admitted. The refusal does **not** name the offending child (§7 row 27)
- [ ] Each of the six declared codes is raised by exactly one rule and carries its declared
      status, with a **positive control per code**: an act that is admitted where that same rule
      would have refused it —
      `RETIREMENT_LEAD_TIME`: an `effectiveAt` at exactly the configured lead is admitted;
      `REPLACED_BY_NOT_PUBLISHED`: a `published` successor is admitted;
      `SCHEDULE_STALE_APPROVAL`: an unedited scheduled entity activates;
      `CASCADE_CONFIRMATION_REQUIRED`: a confirmed cascade proceeds;
      `RETIREMENT_PENDING`: un-deprecation with no live intent is admitted;
      `EOL_DISABLED`: a retirement carrying no `mustMigrateBy` is admitted
- [ ] The EOL flag OFF refuses `mustMigrateBy`; the event schema round-trips the absent field
- [ ] A schema-oracle golden exists for both new tables on both engines, each with a perturbation
      case proving it can fail
- [ ] The partial unique index admits one live intent per entity per kind and admits a second row
      once the first is `applied`, `failed` or `superseded`
- [ ] `retirement_reason` survives a deferral that writes `outcome_reason` — the two-column split
      asserted, with the single-column version red
- [ ] Scheduling acts emit **no** broker event, asserted as set equality over emitted events
- [ ] Every refusal this feature raises leaves an audit row carrying its reason
- [ ] A stub reference predicate is exercised in every state before `07-reference-signal` exists,
      and the suite fails if the stub is replaced by a constant
- [ ] No `#[ignore]`d test exists without a CI tier that runs it

## 7. Known unknowns

[`../design/04-lifecycle.md`](../design/04-lifecycle.md) §6 carries **18 open items**, and each is
carried below with its owner and, where it blocks one, the DoD it blocks — **row 17 blocks none**.
The table has **37 rows**, and the arithmetic is stated rather than left to a reader: §6's eighteen
are rows **1–18**, in the slice's own order; rows **19–37**, marked `**`, were raised by this
document's own reading of the crate — seven before the three-lens review of the FEATURE, ten by
it, one by the implementation of `inst-lc-deprecate`, and one by the lens pass over that
implementation — and are new.

**None of these is answered here.** Registering a question is this document's job; answering one
would be authoring, and **seven** of the eighteen below are questions the slice put to owners who
are not this feature — rows 1, 6, 8, 10, 16, 17 and 18.

| # | The question | Blocks | Owner |
|---|---|---|---|
| 1 | **Deferred flips can hold indefinitely** while a producer watermark stays stale — correct by constraint but operationally invisible without `08-read-models`' surfacing and the `retirement_held` alert; the fail-safe tripwire bounds the corrections debt, nothing yet bounds held retirements. Candidate for an operator report, not a new mechanism *(P-D-109: a live question, not a blocker — this row asks how a behaviour that is *correct by constraint* is surfaced, which `08` part-owns.)* | `dod-deferred-intent` | the ops owner |
| ~~2~~ | **Cascade + scheduled child publishes**: a `pending` scheduled publish on a child of a retiring Product is superseded by the cascade — stated, but **the supersession ordering deserves a probe when built** **Answered by `inst-cp-plan`'s own clauses (P-D-114 (2026-09-03))**: supersede every child's live publish intent, in one transaction — so the ordering is supersede-then-schedule, atomically, and a pending child publish cannot interleave because confirmation is the only writer. | ~~`dod-cascade-plan`~~ | **struck** |
| 3 | **Owed: the instruction row registering the create-door live-retire-intent validator.** Whose it is has been settled — this feature's, its operand being the live retire intent in `products_scheduled_transition`, which this feature owns. **No instruction row registers it yet, and until one does nobody builds the guard** — leaving the hole open: a draft SKU created under a Product with a live retire intent defers that retirement indefinitely *(P-D-109: a live question, not a blocker — this row asks for an **owed instruction row**, not for a store clause.)* | `dod-cascade-plan` | this feature — to add the row in `design/04` |
~~| 4~~ | **struck**
decision that reserves it is false at HEAD**: the crate reserves it in `api/rest.rs`, the
idempotency migration names it, and `products_tests` claims on it. What no artifact supplies is a
*design row that writes it* — this feature's cascade rows name no lane, and its one lane use routes legs through the runner on `internal:scheduled-activation` with the transition id. Either the legs ride the activation lane and `01-foundation` reserves a name nothing uses, or this feature owes the row that writes it — and the `client_key` it is keyed by, which no table here supplies | `dod-activation-runner` | this feature with 01 |
| ~~5~~ | **What announces a Product's `deprecated→retired` flip?** `01-foundation` §4.5 asserts this feature announces all three floor edges, naming the SKU flip event on `deprecated→retired`. This feature gives the parent Product its own retire `ScheduledTransition` on that edge and emits its retirement event at *initiation*, but its Events list names no Product analogue for the flip itself and records no explicit "no event" for it — which §4.5's own rule and `12-consumer-contracts`' completeness check both require. **Independently corroborated by the crate**: `domain::transition`'s module doc says the three edges are announced by slice 04 *"except the `Product` side of `deprecated -> retired`, which no slice announces"*. Naming one would invent normative content **Answered by P-D-115 (2026-09-03): `ProductRetirementEffective`**, the analogue of the shipped `SkuRetirementEffective` whose doc reads "No Product analogue (row 5)". `01` §4.5 owes it. A new type: its own `SCHEMA_REFS` entry and roster line. C's build. | ~~`dod-lifecycle-events`, `dod-cascade-parent-path`~~ | **struck** |
| 6 | **EOL (post-v1) will need** the subscriptions-lifecycle AC by number, the consumer-ack contract, and a suspension event — the schema field is already vN-compatible | `dod-eol-lockout` | the PRD owner with Subscriptions |
| 7 | **Does the create-door retire-intent validator also register on the save door?** `01-foundation`'s save instruction is the only door that may change a SKU's `product_id`, so a draft SKU can be **re-parented** under a retire-pending Product by a door neither arm covers — the hazard `inst-fd-containment-retire-intent` itself describes *(P-D-109: a live question, not a blocker — this row asks whether one further door also registers the validator — a coverage gap, not a defect in the host.)* | `dod-cascade-plan` | this feature |
| ~~8~~ | **What is the claim lease, and what is the per-transition attempt budget?** The claim instruction reclaims a `running` row "past the claim lease" and the failure instruction bounds retries by "a per-transition attempt budget"; **neither carries a value, a default or a config home**, and the interim-defaults table has no row for either. `03-sku-classification` relies on the same budget. **The runner's poll cadence and batch bound are a third knob of the same class**, named nowhere and delegated to nobody, while `dod-flip-guard` defers to "the predicate's freshness cadence" — a fourth clock this feature does not own. *(All three lenses of the slice's own review raised the first two independently.)* **Answered by P-D-113 arm 4 (2026-09-03), interim, in `ProductsConfig`**: `activation_claim_lease_secs` **60** — the loop ticks each second and a flip is one transaction, so a minute frees a crashed worker's row without racing a slow flip — and `activation_attempt_budget` **5**, bounding only the transient arm since a pin mismatch is terminal on its first try. Zero refused at boot. | ~~`dod-activation-runner`, `dod-runner-failure-posture`, `cpt-cf-bss-products-state-scheduled-transition`~~ | **struck** |
| ~~9~~ | **Is the cascade-retire trigger keyed on non-`retired` or non-terminal children?** The plan instruction fires over non-`retired` SKUs while the narrowing instruction rejects that exact operand for its sibling rule and records the narrowing by number. **A `discarded` child is inside the trigger population and fits none of the three plan arms.** The PRD carries the wider wording, so narrowing it is a deliberate deviation that owes a register entry **Answered by P-D-114 (2026-09-03): non-terminal, and the DoD's "non-`retired`" is corrected.** A `discarded` child is terminal and non-retired; firing over it would try to retire a row the machine admits no edge from, and none of the three arms gives it a disposition because it needs none. | ~~`dod-cascade-plan`~~ | **struck** |
| ~~10~~ | **Is `SCOPE_NARROWING_BLOCKED`'s operand a PRD deviation that owes an entry?** The narrowing instruction reads **non-terminal**, the requirement reads non-`retired`, and the reasoning is sound — but no decision entry records the change, the way the struck publish freeze was recorded **Moot since P-D-96 (2026-09-02)**, which withdrew `SCOPE_NARROWING_BLOCKED` entirely — narrowing stays on `SCOPE_NOT_CONTAINED`. A code that no longer exists owes no PRD entry for its operand. Struck by the lead 2026-09-03. | ~~`dod-scope-narrowing`~~ | **struck** |
| ~~11~~ | **Does a deferred cascade complete automatically or by an operator act, and who writes `resolution = children_cleared`?** Three mechanics are in play for one act: the failure instruction says `deferred` re-evaluates automatically, the deferred-intent instruction says the parent is "resumable by an operator once the listed children clear", and the storage roster has a named writer for `cascade_cancelled` and **none** for `children_cleared`. No listed child acquires a retire intent of its own, so "once the listed children clear" names no act that retires them *(P-D-109: the store `DoD` requires the **column** admitting `children_cleared`, not a **writer** for it — the resumption mechanics are `dod-deferred-intent`'s, which this row still and rightly holds)* **Answered by P-D-114 (2026-09-03): the operator resumes, and the resume writes `children_cleared`** — `inst-cp-deferred`'s own words. The failure instruction's "re-evaluates automatically" is the flip guard's re-check on the next poll, which decides whether the deferral still holds; resolution is an act with an actor. Two sentences about two things. | ~~`dod-deferred-intent`~~ | **struck** |
| 12 | **What does the transitive `replacedBy` resolution return on a cycle, which surface walks it, and what bounds the walk?** The pointer instruction has the read surface resolve to the first non-`retired` successor. **A cycle is constructible** on this feature's own admission that a cancelled, un-deprecated SKU keeps a successor no admitted write can clear. `08-read-models` claims no chain walk *(P-D-115 (2026-09-03): **two of three halves answered by the crate** — `resolve_replacement_chain(start, next, bound)` returns `Cycle {{ seen }}` on a repeat and `Bounded {{ seen }}` on exhaustion, so both are `replacement_chain_broken` with the path in hand. What remains is **which surface walks it**, and that is `08-read-models`', routed there with row 13 as one question.)* | `dod-replaced-by` | `08-read-models`, with row 13 |
| 13 | **Where is the `replacement_chain_broken` fact stored, and who reads it?** The pointer instruction calls it "a stored fact with a consumer-usable resolution rather than a silent dangling pointer"; the two tables hold no such row and the observability line names only gauges and the `retirement_held` alert. Alert only — and then strike "stored fact" — or a row with a table, key and consumer | `dod-replaced-by`, `dod-lifecycle-audit` | this feature with the observability owner |
| ~~14~~ | **Is `effectiveAt` an operator input or computed?** The initiation instruction has it "honoring the **configured** lead-time policy … `RETIREMENT_LEAD_TIME` otherwise", and the taxonomy declares the code with a status. **If the registry computes `now + policy` the code can never be raised** — a declared code with no raiser, which the completeness check reads as a defect; if the operator supplies a date, the door owes a date input, a fail-closed comparison and a timezone rule **Answered by the crate, and both readings were right.** `RetireSkuRequest.effective_at` is `Option<DateTime<Utc>>` with its own doc — *"Optional operator instant. Absent: now + interim lead (30 days)"* — and `early_effective_at_is_retirement_lead_time` pins the computed arm. Operator input **or** computed, defaulting to computed. Struck by the lead 2026-09-03. | ~~`dod-retirement-initiation`, `dod-lifecycle-errors`~~ | **struck** |
| ~~15~~ | **Which actor performs the governed cancel of a `ScheduledTransition`?** The un-deprecation instruction makes the cancel a `GovernedLiveOp` registered material by this feature; the actor roster gives it to nobody — the catalog-admin row carries "initiates retirement/cascades" and "resumes deferred cascades", both forward acts *(P-D-109: **who** performs the cancel is the actor roster's, and the `DoD` requires the ceremony, the `RETIREMENT_PENDING` refusal and the clear — no clause names an actor)* **Answered by P-D-114 (2026-09-03): the catalog-admin performs the governed cancel**, under the N-governed ceremony already registered — the actor who initiates a retirement or cascade may abort one. Roster clause added. | ~~`cpt-cf-bss-products-state-scheduled-transition`~~ | **struck** |
| 16 | **Does `leave-and-list` cover referenced children or only EOL-requiring ones?** The plan instruction scopes the arm to "children whose flip guard cannot clear — referenced SKUs"; the PRD and its AC both scope it to "EOL-requiring children left un-retired", and EOL is disabled in v1 — so **on the PRD's wording the arm has no v1 population at all** | `dod-cascade-plan` | the PRD owner, as a wording call |
| ~~17~~ | **`inst-lc-terminal` restates a rule the slice's own §1.5 puts out of scope.** The row's whole content is terminality, which §1.5 assigns to `01-foundation`. The one-declaration rule is stated for error codes, not for instruction rows, so nothing says whether a restating row is a second declaration **Answered (owner, 2026-09-03): `inst-lc-terminal` is reworded as an explicit pointer** — it declares nothing `01` does not and stays so a reader of the edge list finds `retired`'s fate beside the other edges. The one-declaration principle is kept; the restatement is gone. | — | **struck** |
| ~~18~~ | **Pointer**: which slice declares `PARENT_NOT_PUBLISHED` is open in `01-foundation` §6. This feature asserts one arm — "named in 01, registered here" — and **the answer is not this feature's to give** *(P-D-109: a live question, not a blocker — this row asks **where** a code is declared, which is a pointer.)* **Answered (owner, 2026-09-03), and `01` had answered itself**: its own §3.3 reads *"registered by slice 04 on the `→ published` target state … two raising arms, both are 04's"*, and P-D-97 landed the second arm. Declared in `01`'s ladder, raised by `04`. `01` §6 item 1 is struck on the same measurement. | ~~`dod-lifecycle-errors`~~ | **struck** |
| ~~19~~** | ~~**`SCOPE_NARROWING_BLOCKED` has no raiser…**~~ **Closed 2026-09-02 as P-D-96**: `SCOPE_NARROWING_BLOCKED` is **withdrawn** (narrowing stays on `SCOPE_NOT_CONTAINED`); `PARENT_NOT_PUBLISHED` is **admitted** as an owned slot (P-D-24 already prices it at 409). `RETIREMENT_PENDING` is settled on the same terms and is this feature's (D7 arm). `features/reference-signal.md`'s copy of the withdrawn code is owed to `07` **Closed in its own body since 2026-09-02 (P-D-96) and never struck** — the same propagation lag that left six of `02`'s rows open a day past their answers. Struck by the lead 2026-09-03. | ~~`dod-scope-narrowing`, `dod-lifecycle-errors` — **freed**~~ | **struck** |
| ~~20~~** | ~~**The validation pipeline cannot key a rule…**~~ **Closed 2026-09-02 as P-D-97**: `RegisteredValidators` is a phase **slot** with two admissible fillings — a registered `ValidationRule` (subject-local or one door-prefetched fact) **or** a **continuation** of that phase on the same transaction, positioned where §4.1 asks. The trait does not widen; "keying" is the insertion site. **Residue**: a continuation raises a `DomainError` directly and does **not** collect into a `ValidationReport` — it refuses on the first finding | `dod-registered-validator-host`, `dod-publish-ordering`, `dod-scope-narrowing`, `dod-scope-containment-final` — **freed** | was this feature with 01; **closed** |
| 21** | **What operand does this feature replace in the containment check?** The slice describes `SCOPE_NOT_CONTAINED`'s "final semantics" as registered here, replacing the operand inside `01-foundation`'s identity phase. But `domain::containment` **already implements the final restriction-based rule verbatim** — unrestricted parent contains every child, unrestricted child needs an unrestricted parent, ordinary subset between non-empty sets — wired at three call sites. If nothing is left to replace, the obligation is a no-op and the slice's C5 wording is stale | `dod-scope-containment-final` | this feature with 01 |
| ~~22~~ | **Answered by P-D-105 (2026-09-02)**, jointly with `05-governance` §7 row 27 — the two were one question. At a scheduled flip `PreAuthorized` verifies **the pin the row carries**: the named record is `consumed` and the row being flipped names it in its own `approval_ref`. The subject/revision equality is dropped there and kept everywhere else, because a cascade leg could never satisfy it — the row's `entity_id` is the child while the record names the parent. **Not the bearer token this row warned of**, and the difference is measured rather than argued: the forbidden form admits a *caller* that names a consumed record, while this operand is a stored column on a row the caller cannot write — `insert_scheduled_transition` has three call sites, all inside a `GovernanceGate`-run `run_retire`, counted and now guarded, and `inst-cp-plan` already makes that transaction atomic. The predicate is **B's** to write in `domain::approval`; the runner's call is **this feature's** in `domain::activation`; neither decides it. P-D-105 records one undischarged residue: `01` §3's `inst-fd-gate-mode-preauthorized` still words the clause as *"this subject at this revision"* and owes the scheduled-flip exception, which is the lead's | ~~`dod-scheduled-publish-pin`, `dod-cascade-plan`, `dod-activation-runner`~~ | **struck** |
| ~~23~~** | **Can a `deferred` row be superseded, and the slice says two things.** The claim instruction calls the re-claim to `running` *"the only exit that state has"*, while the cascade-plan instruction supersedes a child's **live** intents and the partial unique index counts `deferred` as live. Both cannot hold. §4 row 7 above records the edge as leaving "a live state" precisely because the source does not enumerate them *(P-D-109: **the `DoD`'s own clause already resolves it**: the partial UNIQUE is specified `WHERE state IN ('pending','running','deferred')`, so `deferred` **is** live and the claim instruction's *"the only exit that state has"* is the half that needs qualifying. The contradiction is between two instructions, not against this `DoD`)* **Answered (owner, 2026-09-03): the claim instruction is qualified** — the re-claim is the only **runner** exit; supersession by a confirmed cascade is the other, which is exactly why the DoD's partial UNIQUE counts `deferred` as live. Both instructions now say one thing. | ~~`cpt-cf-bss-products-state-scheduled-transition`, `dod-cascade-plan`~~ | **struck** |
| ~~24~~** | **Three of the four edges this feature owns have no admitted writer, so it owes a transition door the slice never describes.** `published → deprecated`, `deprecated → published` and `deprecated → retired` are what §2's flows and eight of §5's DoDs are made of, and no shipped door can write them: `published_state_after` maps `draft → published` and returns `from` for every other state, the discard door writes `discarded` only, and `SkuHeadSave`/`ProductHeadSave` carry no `lifecycle_state` column at all. The floor says so itself — *"There is also no door here … belongs to the transition and discard doors, which are a later slice's"* — and `TransitionEffects::bumps_the_guard_owns()`, documented as having no production caller, names *"slice 04's transition door"* as its expected first reader. **The activation runner is not that door**: §3 has it drive the ordinary Foundation doors with no privileged path. So the door is owed, its bump arithmetic is the method's, and neither the slice nor this document specifies it *(P-D-109: **the factual half is stale**: written 2026-08-31, and `01690816a` opened the deprecate, un-deprecate, retire and retire/cancel doors on 2026-09-02. Verified in code, not from the commit title — `run_deprecate`, `run_undeprecate` and `run_retire` exist in both door files over `deprecate_sku_head`, `undeprecate_product_head`, `undeprecate_sku_head` and `deprecate_sku_head_with_replacement`. All three edges have admitted writers)* **The design half may stand**: the doors are the *crate's*, and whether the *slice* describes them is row 36's live complaint. Narrowed, not struck. **Fully stale.** Its factual half — *"no shipped door can write them"* — was true on 2026-08-31 and false from `01690816a` on 2026-09-02; verified in code under P-D-109. Its design half — whether the *slice* describes the doors the *crate* built — is row 36's live complaint and lives there. Struck by the lead 2026-09-03. | ~~`dod-activation-runner`~~ | **struck** |
| 25** | **Seven broker events against eight shipped payload types, two body shapes and a roster test that is exact in both directions.** The gear ships `ProductCreated`, `SkuCreated`, `ProductHeadSaved`, `SkuHeadSaved`, `ProductPublished`, `SkuPublished`, `ProductDiscarded` and `SkuDiscarded` on `EventBodyCore` and `PublishedEventBody`, and `events_tests::THE_EIGHT` asserts both that every schema-carrying token is in the roster and that the roster's length is eight — so the first event this feature adds reddens it, and every one owes a versioned schema reference or it cannot reach the wire. This feature declares seven — deprecation and un-deprecation for both kinds, retirement for both kinds, and the SKU flip — of which the retirement payload carries five fields beyond the core. Nothing in the design set says whether the deprecation events carry a body beyond the core, and the retirement event needs a **third** body type that no artifact names | `dod-lifecycle-events` | this feature with 01 and 12 |
| ~~26~~** | **The narrowing check ships on the *save* door and has no publish call site**, while `inst-pc-narrowing` and this document's §2 both put it on a **publish**. `products::check_children_stay_contained` has exactly one production call site, inside `run_save`, at what that function's own comment calls "Phase 6, the registered-validators phase". Either the rule moves to publish, or it is registered on both, or the design's "publish" is wrong — and the third reading is the cheapest, since a narrowing is a head write and the save door is where head writes land **Answered by P-D-115 (2026-09-03): narrowing runs at publish — the placement `inst-pc-narrowing` and §2 both state, and where a head becomes visible — and the save-door check stays as an early refusal, not the obligation.** C's build. | ~~`dod-scope-narrowing`~~ | **struck** |
| ~~27~~** | **The document requires the narrowing refusal to name the falling-out children; the shipped door deliberately refuses to.** §5 obliges "the validator **MUST** name the falling-out children" and §6 asserts a refusal "**naming that child**", while `products.rs` records the opposite as a decision: *"The refusal therefore does **not** name the offending child: that message is the shared one, and naming a SKU in it would be a second wording."* No document in the tree carries this as an open item — the slice states the naming requirement flatly and its §6 registers nothing. Accept the shared unnamed message, or fork a second message for the parent's end **Answered by P-D-115 (2026-09-03): the refusal names the falling-out children**, as §5 obliges; `scope_not_contained_domain_err` gains them, since knowing *which* SKUs fall out is the refusal's whole operator value. C's build. | ~~`dod-scope-narrowing`~~ | **struck** |
| ~~28~~** | **`attempt` has one increment rule and two populations, so the bounded arm can never spend its budget.** §4 moves the counter only on the lease reclaim (row 3), while the budget it is measured against bounds the *transient-dependency* deferral (row 5). On the text as written a transient deferral never increments, so the budget is never spent and `failed` is unreachable by that path; meanwhile a lease reclaim during an unbounded **flip-guard** hold does bump the same counter toward a budget that arm is expressly not subject to. The slice has the identical shape. Whether these are one column or two, and which transitions move which, is unstated **Answered by P-D-113 arm 3 (2026-09-03): `attempt` increments on every claim, persisted by the claim statement.** Measured: it moved **nowhere** — `repo/lifecycle.rs` writes `Set(0)` at insert and nothing else touches it — so "one increment rule" was generous and the budget could never be spent. One counter, both populations: the budget is *how many times a worker picked this row up*. | ~~`cpt-cf-bss-products-state-scheduled-transition`, `dod-runner-failure-posture`, `dod-activation-runner`~~ | **struck** |
| ~~29~~** | **Nothing names what hosts the activation runner, or the identity it drives the doors under.** The gear implements exactly two capabilities — `DatabaseCapability` and `RestApiCapability` — and there is no background-job seam, while §2 states the runner "has no wire surface". The doors it must drive take a compiled `AccessScope` and a pseudonymous `actor_ref`, both request-derived, and `dod-lifecycle-audit` obliges every one of its acts to leave an audit row through those refs. Any answer that skips scope compilation is the "privileged path around the pipeline" §4 row 1 forbids **Answered by P-D-113 (2026-09-03): a fourth `activation_tick` in `gear.rs`'s existing lifecycle loop — the same 1s interval and cancel token that already host `batch_tick` — under a stable UUID-v5 system principal.** The measured half nobody had asked: `system_actor_ref` was `Uuid::new_v4()` **per process start** at `gear.rs:536`, so the gear's own acts changed principal on every restart. Replaced. | ~~`dod-activation-runner`, `dod-lifecycle-audit`, `cpt-cf-bss-products-state-scheduled-transition`~~ | **struck** |
| ~~30~~** | **The Product retirement event's payload is undefined, and it cannot be the SKU's.** §2 states one retirement payload — `{skuId, fromVersion, reason, replacedBy?, effectiveAt}` — and `dod-cascade-parent-path` has the parent emit "its retirement event at initiation" with no field list. One of the five fields has no Product-side source at all: `replaced_by_sku_id` is `products_sku` only. The slice says "payload analogous to the SKU's", which is not a field list. Row 5 is about the Product **flip**; this is the **initiation** event **Answered by the crate, recorded by P-D-115 (2026-09-03)**: the Product payload is `RetiredEventBody` with `replacedBy` absent — `{productId, fromVersion, reason, effectiveAt}` — as `events.rs` documents (*"`replaced_by` is SKU-only"*) and as C's re-announcement already emits. | ~~`dod-lifecycle-events`, `dod-cascade-parent-path`~~ | **struck** |
| ~~31~~** | **The cascade's auto-discard arm needs a pre-authorized discard, on a door whose mode was fixed shut because no such caller was believed to exist.** `dod-cascade-plan` runs the whole plan in one transaction, auto-discard included, while the discard door takes `GateMode::Gate` as a literal on a stated premise: *"No scheduled or cascaded **discard** exists in any slice, so there is no caller for a pre-authorized discard and no instruction asking for one."* This feature is that caller. The door also builds its idempotency key from a wire path, where §2 requires an internal caller to write a lane name. Row 4 asks which lane; row 22 asks what `PreAuthorized` requires of a leg's subject; neither reaches the discard door's mode **Answered by P-D-114 (2026-09-03): the auto-discard arm is not a second ceremony.** It runs inside the confirmed cascade-retire act's transaction, as `apply_cascade_plan` already does for scheduling the legs, under the gate that act passed. The discard *door*'s `Gate` literal is the wire surface and the cascade never calls it. P-D-105 was right not to reach for a pre-authorized discard. `apply_cascade_plan`'s single-caller invariant joins the P-D-105 writer guard. | ~~`dod-cascade-plan`~~ | **struck** |
| ~~32~~** | **The governed cancel is required to clear `replaced_by_sku_id` "for the parent", and the parent is a Product, which has no such column.** `dod-undeprecation` obliges the clear "for the parent and every child leg the reversal touches" while `dod-lifecycle-columns` pins the column to **`products_sku` only** — "the column names a SKU" — and §1.4 records that the slice was already corrected once for listing it on both tables. The slice carries the same pair, so the contradiction is inherited rather than introduced. Either the parent clause is scoped to the child legs, or Products get the column back *(P-D-109: `dod-lifecycle-columns` is the `DoD` being **contradicted**, not the one defeated — it pins the column to `products_sku` and that clause is satisfiable and satisfied. The unsatisfiable clause is `dod-undeprecation`'s, which this row keeps)* **Answered by P-D-114 (2026-09-03): the clause is corrected to "for every child leg the reversal touches"**, matching `dod-lifecycle-columns`, and `dod-undeprecation` is re-ticked — its other clauses were verified at the original tick and nothing moved them. | ~~`dod-undeprecation`~~ | **struck** |
| ~~33~~** | ~~**Three artifacts give three answers about whose migration creates `deprecation_provenance` and `replaced_by_sku_id`.**~~ **Closed on re-measurement 2026-09-01**: one answer is left and all three artifacts give it. `design/01` carries the columns inside slice 01's own table shapes, each tagged *slice 04* — `deprecation_provenance` in §4.1's Product shape and `replaced_by_sku_id` in §4.2's SKU shape, so the pair spans both sections rather than §4.2 alone, which is the one thing the row had imprecise. Both shipped entity migrations now credit slice **04** by name and record in their own docs that an earlier revision of them credited 03. And `design/03-sku-classification.md` names neither column, as `features/sku-classification.md` does not. So **01's migrations create the pair and 04 is its first writer** — what `dod-lifecycle-columns` said all along; the third answer was a stale attribution in the crate, corrected in an earlier wave. *Original text:* `dod-lifecycle-columns` says `01-foundation`'s migrations create them and this feature is their first writer; `design/01-foundation` §4.2 lists them in slice 01's own table shapes annotated with slice 04 as the *writer*; and both shipped entity migrations say in their own docs that **slice 03** brings them. `features/sku-classification.md` obliges no such column addition. Whoever owns it, one of the three is wrong | `dod-lifecycle-columns` — **freed** | was the design-set owner with 01 and 03; **closed** |
| ~~34~~** | **Is the active-reference count part of the retirement confirmation token, or only displayed?** `dod-retirement-initiation` and the slice's own `inst-rt-confirm` both stop at *showing* it, and none of the seven declared codes covers a stale-count refusal. If the count is part of the token, a code, a raiser and a staleness comparison are owed; if it is displayed only, the operator confirms against a number that may already have moved — which is the silence the requirement to show it exists to prevent **Answered by P-D-115 (2026-09-03): the confirmation stays a boolean and the count is displayed, not pinned.** A count-pinned token is a TOCTOU guard the PRD does not ask for; the guard the cascade needs is already physical — the plan supersedes every child's live intents at confirmation in one transaction. | ~~`dod-retirement-initiation`, `dod-lifecycle-errors`~~ | **struck** |
| ~~35~~** | **Does the cascade supersede a child's live *retire* intent in all three arms, or only in the retire arm?** `dod-cascade-plan` supersedes it "for every child in all three arms … the latter replaced by this cascade's own leg", but only the retire arm produces a leg, and the stated justification — a unique-index collision — can only arise where a leg is being inserted. So a leave-and-list or auto-discard child has its own scheduled retirement cancelled and replaced by nothing. Row 11 records the downstream symptom, that no listed child acquires a retire intent, but asks who writes the resolution rather than whether the cancellation was right **Answered by `inst-cp-plan`'s own text**: *"supersedes, for every child in all three arms, that child's live `publish` intent — and its live `retire` intent, replaced by this cascade's own leg."* Both intents, all three arms; the DoD says the same. There is no second reading to choose between. Struck by the lead 2026-09-03. | ~~`dod-cascade-plan`, `dod-deferred-intent`~~ | **struck** |
| 36** | **No document declares an entity-scoped door for the deprecation act.** `design/04` §2 `inst-lc-deprecate` is a `p1` instruction — an operator act stamping provenance `direct`, cascading onto the children — and `09 inst-bl-lifecycle` calls its per-row caller *"the ordinary 04 policy doors"* and *"each row's 04 transition door"*, running in 01's `PreAuthorized(approvalId)` mode. The act's **one declared carrier is batch-only**: `09`'s `POST /bss-products/v1/bulk/lifecycle` under its own `bulk_lifecycle × execute` grant. The set's entity-scoped `{products\|skus}/{id}/<act>` spans are **three** — `publish`, `discard`, `clone` (07's `POST /skus/{skuId}/corrections` is entity-scoped too but a different shape) — and `deprecate` is not among them; PRD §9.1's `interface-authoring-publish` names *lifecycle transitions* but defers the shape to Design, and no slice supplies it. The crate now performs the act and registers `POST /bss-products/v1/products/{id}/deprecate` under `product × write` plus a SKU-scoped `sku × write` for the child half, on the discard door's own reasoning — a normative act with no carrier is inert — but that path and those grants are **the crate's, not the set's** (05 §3.1 makes a transition to `deprecated` material exactly as publish is, and publish has its own `product × publish`, so a dedicated grant is at least arguable), and the crate's door takes `GateMode::Gate` as a literal where 09's lane needs `PreAuthorized` — row 31's class, reproduced here. All of that is why `dod-deprecation-provenance` and `dod-deprecation-cascade` carry bare markers rather than ticks. Either the span, its grant and its mode arrive in the set (P-D-87 arm 7's route census, **seventeen**, moves to eighteen — noting `design/05` §3.2 still reads *fourteen*, unpropagated), or the act is re-carried by a door that already exists *(P-D-109: a live question, not a blocker — this row asks who owns a door whose obligations `0b603dd19` verified clause by clause; its *bare markers* sentence was true on 2026-09-01 and stale from the tick the next day.)* | *(no DoD — P-D-109)* | the design-set owner with 01 and 05 — the span roster, the grant table and the gate mode |
| ~~37~~** | **Which declared code refuses the no-orphan flip?** `dod-no-orphan` obliges that no `published` SKU exist under a `retired` Product, re-checked at flip — and no code covers the refusal. `design/04` §3.2's slice-owned roster carries none for it, and `design/01` §3.3 scopes `PARENT_TERMINAL` to *"the parent's own state"* refusing a **child's** write — the reverse direction. The crate's `no_orphan_at_flip` (uncalled until the flip ships) raises `PARENT_TERMINAL` provisionally and says so in its own doc. Either the code is declared for this refusal, or the flip's slice mints one **Answered by P-D-113 arm 5 (2026-09-03): at the flip it is a deferral, not a wire refusal.** `defer_flip_guard` already finishes `Deferred` with `retention_orphan_blocked` as `outcome_reason`, and the runner is not a wire door — nothing receives a code. `no_orphan_at_flip`'s `ParentTerminal` is the wrong code pointing the wrong way, on a rule called from no door; it becomes the deferral's judge. | ~~`dod-no-orphan`~~ | **struck** |

*Rows marked `**` were raised by this document's own reading of the crate, not carried from the
slice. **§7 rows 19 and 20 are closed** as P-D-96 and P-D-97 (2026-09-02). Row 24 — three of the
four edges this feature owns having no admitted writer in any shipped door — remains load-bearing
and was invisible to a reading of the design set alone.*

### Raised here rather than carried

*Both bullets name the obligation they block, as the table's rows do.*

- **The flip guard's stub is the whole test surface until `07-reference-signal` exists.** *Blocks
  `dod-flip-guard` and `dod-no-orphan`.* The predicate has five answers and only one of them lets a
  flip proceed, so a stub that always answers fresh-zero passes every criterion while proving
  nothing about the four deferring states. Whether the stub is this feature's to own, or belongs in
  a shared fixture `07-reference-signal` later replaces, is unstated. *Owner: this feature with 07.*
- **`08-read-models` projects the deferred-intent surface this feature owns, and neither names the
  shape.** *Blocks `dod-deferred-intent`.* The slice is explicit that 04 owns the table and 08 only
  projects it, but no artifact declares what the query surface returns — a field-name drift here
  costs an adapter later, exactly as the sibling inbox envelope did. *Owner: this feature with 08.*
