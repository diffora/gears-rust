# Feature: Reference Signal

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-reference-signal-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-reference-signal`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Ingest a watermark](#ingest-a-watermark)
  - [Evaluate the predicate](#evaluate-the-predicate)
  - [Register or retire a producer](#register-or-retire-a-producer)
  - [Correct a bucket-ii field](#correct-a-bucket-ii-field)
  - [Break-glass correction](#break-glass-correction)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [Watermark storage and freshness](#watermark-storage-and-freshness)
  - [Error taxonomy](#error-taxonomy)
- [4. States (CDSL)](#4-states-cdsl)
- [5. Definitions of Done](#5-definitions-of-done)
  - [The watermark store](#the-watermark-store)
  - [The producer registry](#the-producer-registry)
  - [The break-glass evidence store](#the-break-glass-evidence-store)
  - [The watermark contract, in `bss-products-sdk`](#the-watermark-contract-in-bss-products-sdk)
  - [The watermark door and its four refusals](#the-watermark-door-and-its-four-refusals)
  - [The reference predicate, with its per-producer detail](#the-reference-predicate-with-its-per-producer-detail)
  - [Producer registration as a governed live op](#producer-registration-as-a-governed-live-op)
  - [The symmetric snapshot ride](#the-symmetric-snapshot-ride)
  - [The correction door, and the three gates it admits on](#the-correction-door-and-the-three-gates-it-admits-on)
  - [The correction re-publish, and the validator that re-checks the lane](#the-correction-re-publish-and-the-validator-that-re-checks-the-lane)
  - [Break-glass arm (a): the signal is unavailable](#break-glass-arm-a-the-signal-is-unavailable)
  - [Break-glass arm (b): the unresolvable target](#break-glass-arm-b-the-unresolvable-target)
  - [The tripwire](#the-tripwire)
  - [The error taxonomy, wired into `DomainError`](#the-error-taxonomy-wired-into-domainerror)
  - [The authz surface, and the four rosters it reddens](#the-authz-surface-and-the-four-rosters-it-reddens)
  - [The four config knobs, and the clock this feature owes `04-lifecycle`](#the-four-config-knobs-and-the-clock-this-feature-owes-04-lifecycle)
  - [The events and the alarms](#the-events-and-the-alarms)
  - [The audit trail for this feature's acts](#the-audit-trail-for-this-features-acts)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Carried verbatim from `design/07` §6](#carried-verbatim-from-design07-6)
  - [Raised here rather than carried](#raised-here-rather-than-carried)
  - [Owed to other documents, recorded and deliberately not edited](#owed-to-other-documents-recorded-and-deliberately-not-edited)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

Whether anything downstream still points at a SKU, answered honestly enough to gate a retirement.
This feature owns the registry's only **inbound liveness contract** and everything that leans on
it: watermark ingestion with per-producer full-set semantics, the reference predicate and its
per-producer detail surface, producer registration and its symmetric snapshot ride, the bucket-ii
correction door, the flag-gated break-glass correction, and the fail-safe tripwire that keeps
degraded operation from becoming normal.

### 1.2 Purpose

The registry must never falsely free a referenced SKU — at a retirement or at a correction — and
must never need dense zero-publishing at 10K-SKU scale. The watermark OR-predicate buys both. The
correction door exists because bucket-ii mistakes happen (a wrong `type`, a wrong meter) and the
only safe remedies are a **provably unreferenced** re-publish or an explicitly recorded emergency.

**Requirements**: `cpt-cf-bss-products-fr-reference-signal`,
`cpt-cf-bss-products-fr-reference-producer-registration`,
`cpt-cf-bss-products-fr-immutable-field-correction`,
`cpt-cf-bss-products-fr-failsafe-tripwire`

**Principles**: `cpt-cf-bss-products-principle-fail-closed`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Components**: `cpt-cf-bss-products-component-capability-handlers`

**Sequences**: `cpt-cf-bss-products-seq-reference-signal`

**Contracts terminated here**: `cpt-cf-bss-products-contract-sku-reference-count` — the inbound
watermark signal this feature's `WatermarkDoor` terminates, declared at PRD §9.2 and cited here by
id. `cpt-cf-bss-products-contract-reference-errors` is **declared by**
[`../design/07-reference-signal.md`](../design/07-reference-signal.md) §3.2 and stays there; a
`contract-` id is not FEATURE-definable.

**Out of scope**: what a producer counts, which is that producer's own contract — Contracts'
draft-and-quote question is recorded at its registration (PRD §15); retirement policy
(`04-lifecycle`); the ceremony machinery (`05-governance`); and erasure of watermark content
(`10-retention-erasure`), since the sets carry SKU ids only and no PII by construction.

**Not applicable**: no state machine is declared here — see §4.

**One roster this feature builds is wider than the design's own name for it.** DECOMPOSITION §2.7 and the PRD
call the predicate **three-state**; this feature builds **four** verdicts, because
`design/07`'s `inst-rp-freshzero` adds `no_producers` as a fail-safe, and `inst-pr-governed` flags
it as *"a design-introduced extension of the PRD's 3-state taxonomy"*. The fourth is defensive-unreachable in
v1 — `PRODUCER_SET_EMPTY_FORBIDDEN` refuses retiring the last producer — and is kept because an
empty producer set must never free anything. **Four is the number this feature implements**, and the
PRD's three is the number it extends, deliberately and with the extension named.

### 1.3 Actors

| Actor | Role in Feature |
|-------|-----------------|
| `cpt-cf-bss-products-actor-plan-price` | The v1 producer (**P-D-03**): posts watermarks from its live plan→SKU references |
| `cpt-cf-bss-products-actor-subscriptions` | Future producer; registers at its own build, GA-gated on producing |
| `cpt-cf-bss-products-actor-contracts` | Future producer, on the same footing as Subscriptions |
| `cpt-cf-bss-products-actor-catalog-admin` | Requests corrections; drives the break-glass emergency path and producer-registration ceremonies |

### 1.4 References

- **PRD**: [PRD.md](../PRD.md) — §6.1 (`fr-reference-signal`, `fr-immutable-field-correction`),
  §6.13 (`fr-reference-producer-registration`, `fr-failsafe-tripwire`), §9.2
  (`contract-sku-reference-count`), §17.1 (freshness 15 min interim; tripwire > 5/30 days);
  AC #2 (predicate half), #3, #4, #41, #43
- **Design**: [DESIGN.md](../DESIGN.md); the granular module boundary is
  [`../design/07-reference-signal.md`](../design/07-reference-signal.md) (361 lines)
- **Decisions**: [DECISIONS.md](../DECISIONS.md) — **P-D-03** (the v1 producer set),
  **P-D-05** (correction re-resolution), **P-D-11** (`N` floor 0), **P-D-13** (the quorum
  shorthand's fifth and sixth sites), **P-D-16** (the unresolvable-target arm), **P-D-32**
  (`ILLEGAL_FIELD_MUTATION`), **P-D-41** (the publish door's third argument), **P-D-48** (the
  unresolvable arm carries no flag of its own), **P-D-50** (the PII gate on every operator reason).
  Cross-gear: pricing PRD §15, the mirrored producer obligation
- **Dependencies**: `cpt-cf-bss-products-feature-foundation`,
  `cpt-cf-bss-products-feature-lifecycle`

**The dependency line above is DECOMPOSITION's build order, not this feature's authoring seams.**
The entry names two; the slice's instruction steps cite **eight** foreign ids across **five**
slices. Both statements are correct about different things, and conflating them is what mis-chose a
build order two features ago.

**The declaration site, and what this document may define.** `design/07`'s five `flow-` and one
`algo-` declarations **moved here**; each of its sections now carries a pointer at this file and
keeps its own instruction steps, which stay normative. One definition site per id.

This document defines only `flow`, `algo`, `dod` and `featstatus` ids. **It declares no `inst-`
id** — §4 mints none because this feature has no state machine (see §4), so every `inst-` reference
below resolves into a design slice.

`design/07` §3.2 carried only `cpt-cf-bss-products-contract-reference-errors`, so
**`cpt-cf-bss-products-algo-reference-errors` is minted here** and §3.2 points at it — the sixth
time a `contract-`-only §3 section has needed this.

**This document does not copy the slice's instruction steps.** §2 and §3 carry the actor, the
scenarios and the boundary; the steps stay at their single source. §5 does restate storage shapes,
because a DoD must name the columns it obliges — **`design/07` §3.1 and §4 govern on any
column-level fact**, and the slice's own optionality and roster markers are kept here deliberately.

**Censusing `design/07` §4 alone undercounts that slice's tables by half.** Two of the four —
`products_reference_watermark` and `products_reference_member` — are declared in its **§3.1's
`inst-wm-tables`**, not its §4, which then refers to them as *"§3.1's two tables"*.

**The nine foreign instruction ids this feature reaches, and all nine are written.**

| id | slice | FEATURE written? |
|---|---|---|
| `inst-fd-save-txn` | `01-foundation` | yes — `features/foundation.md` |
| `inst-av-pii-block` | `02-taxonomy-attributes` | yes |
| `inst-av-pii-reason` | `02-taxonomy-attributes` | yes |
| `inst-rt-flip-guard` | `04-lifecycle` | yes |
| `inst-lc-undeprecate` | `04-lifecycle` | yes |
| `inst-mt-inputs` | `05-governance` | yes |
| `inst-bg-open` | `05-governance` | yes |
| `inst-fz-force` | `06-catalog-version` | yes |
| `inst-ar-failure` | `04-lifecycle` | yes |

**Zero into unwritten** — the first capability feature for which that is true, and it became true
this session: `inst-fz-force` was 07's last unwritten seam until `features/catalog-version.md`
landed. The slice's own instruction steps cite **eight**; the ninth, `inst-ar-failure`, is reached
by §2's predicate boundary in this document and not by the slice's steps. Three of the nine are
citations rather than dependencies: `inst-bg-open` is named in order to
say this lane is **not** a `BreakGlassSession` and does not inherit its fixed platform floor, and
`inst-lc-undeprecate` is named as a sibling site of the same **P-D-13** quorum clause.

**Positive findings against the shipped crate.** Byte-verified at `19a81a406` and recorded so a
later reader does not re-measure them.

- **The Foundation already names this feature's door, in twelve places, and one of them carries the
  route.** `api/rest/products.rs`'s `correctable_after_publish` builds a shipped
  `DomainError::IllegalFieldMutation` reading *"after first publish it is writable only through the
  correction door (POST .../corrections, slice 07), which this door names rather than forwards
  to"*. The other sites are `domain/bucket.rs`'s module doc and its `Correctable` variant,
  `infra/storage/repo.rs`, three more in `products.rs`, two in `api/rest/skus.rs`, and
  `migrations_tests.rs`. **The posture is uniform and deliberate** — *"one door, one effect"*: the
  head doors **name** this door and do not forward to it, so nothing has to be un-wired when it
  lands.
- **The bucket classification ships and is compile-checked.**
  `domain::bucket::FieldBucket` carries all four variants — `Structural`, **`Correctable`** (this
  feature's), `MaterialMutable`, `Descriptive` — with two of them empty, and a member added without
  a decision behind it fails a test.
- **`ILLEGAL_FIELD_MUTATION` and `STALE_REVISION`, two of the four foreign codes this feature cites,
  are armed** — both are `DomainError` variants today with their wire codes on `code()`.

**One shipped test's message misattributes this feature's scope, and it matters.**
`bucket_tests::buckets_ii_and_iv_have_no_members_today` is green and asserts the `Correctable`
member count is **zero**, with the message *"bucket-ii columns arrive with slice 07"*. That message
is wrong, and `domain/bucket.rs`'s module doc says so — *"03 owns the columns and their registration
while slice 07 owns the correction door"* — in a different file from the assertion, whose own
neighbouring doc comment repeats the same misattribution. So the test reddens when
**`03-sku-classification`**'s code lands, not this feature's. **This feature claims the door, never
the columns.** Recorded as owed to the crate in §7.

## 2. Actor Flows (CDSL)

Each flow below is **declared here and stepped in
[`../design/07-reference-signal.md`](../design/07-reference-signal.md) §2**, whose steps are the
normative ones. What this section carries is the triggering actor, the scenarios and the boundary.

### Ingest a watermark

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-watermark`

**Actor**: `cpt-cf-bss-products-actor-plan-price`

**Success Scenarios**:
- A registered producer posts `(producer, watermark_at, complete skuId set)` and the member set is
  **replaced atomically** — no reader ever observes half a set.
- A replay of the same `watermark_at` with the same set is idempotent.
- Absence of a `skuId` under a fresh watermark is a **zero for that producer**, never a missing
  value. The set is complete by contract (C1), which is what removes dense zero-publishing at
  10K-SKU scale.

**Error Scenarios**:
- An unregistered producer is refused `PRODUCER_UNREGISTERED`; its silence pins nothing (C3).
- A `watermark_at` older than the stored one is refused `WATERMARK_REGRESSION`.
- An equal `watermark_at` carrying a **different** set is refused `WATERMARK_CONFLICT`.
- A `watermark_at` **above the receiving clock plus the configured skew** is refused
  `WATERMARK_FUTURE` and alerted. This bound is `p1` rather than hygiene, and
  `cpt-cf-bss-products-dod-watermark-door` carries why: one accepted future-dated post is
  unrecoverable, and its consequence is the inverse of the invariant this whole feature exists for.

**Boundary**: what a producer counts is its own contract. This feature owns ingestion, storage,
freshness and the refusals — never the semantics of the set's membership.

### Evaluate the predicate

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-predicate`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin` (through 04's retirement confirmation), and
`04-lifecycle`'s `ActivationRunner` as a system caller

**Success Scenarios**:
- The verdict is the **boolean OR** across every **registered** producer, and the detail is
  **per-producer** — which is what a retirement confirmation screen shows.
- **Fresh-zero** — every registered producer fresh **and** omitting the SKU — is the only verdict
  that unlocks a retirement flip and the correction door's **ordinary** gate.
- A fresh watermark containing the SKU gives `referenced(producer)`; one omitting it gives zero for
  that producer.

**Error Scenarios**: none. The predicate is a read and always answers. Its three conservative
answers are verdicts, not refusals:
- a **stale** watermark ⇒ `conservatively_referenced(stale, producer)` plus the
  `reference_watermark_stale` alarm;
- **never-received** ⇒ `conservatively_referenced(never_received, producer)` under a **distinct**
  flag, and deliberately **no** alarm (C2);
- **zero registered producers** ⇒ `no_producers`, distinct from fresh-zero — an empty producer set
  never frees anything.

**Boundary**: this feature answers; `04-lifecycle` decides. Its `inst-rt-flip-guard` defers a flip
on anything but fresh-zero, and its `inst-ar-failure` makes that deferral **unbounded** — C4 there
admits no force-retire door in v1. So this predicate can hold a retirement indefinitely, and that is
correct rather than a gap.

### Register or retire a producer

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-producer-registration`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin` — see §7 for whose act this actually is, which
the design set does not settle

**Success Scenarios**:
- Membership is a `GovernedLiveOp` on `reference_producer × write`, **material**, enumerated as one
  of this feature's kinds in `05-governance`'s material inputs (d). Each change emits
  `ReferenceProducerSetChanged` and audits.
- v1 seeds `{pricing}` per tenant at bootstrap (**P-D-03**).
- A registering producer's first watermark starts **never-received**, so onboarding can only tighten
  and never free.
- The producer set rides the `06-catalog-version` capture store per `CatalogVersion`, symmetrically
  with the freeze-participant set, so onboarding never retro-flips a historical mutability or
  retirement decision — past verdicts were computed against the then-registered set and stand.

**Error Scenarios**:
- Retiring the **last** registered producer is refused `PRODUCER_SET_EMPTY_FORBIDDEN`. This is what
  makes `no_producers` defensive-unreachable in v1 while keeping it as the fail-safe.
- Retiring a producer whose watermark is **stale or never-received** is refused
  `PRODUCER_RETIREMENT_WOULD_FREE` unless the retiring principal supplies the break-glass
  ceremony's own justification — **the retirement is admissible, its silence is not**. The reason
  passes 02's `inst-av-pii-block` before the row is written, a hit failing `CONTENT_PII_BLOCKED`.

**Boundary, and the case this flow does not close.** Retiring a producer **narrows the quantifier
the fresh-zero predicate runs over**, so retiring a *stale* producer can make the predicate answer
fresh-zero over the remaining fresh one — and a correction that `CORRECTION_REFERENCED` had blocked
would walk through the **normal** door with no flag, no override row and no tripwire increment.
Only the stale and last-producer cases are guarded. **A fresh producer's retirement can still free a
SKU**, the slice's own §6 records three attempts at that rule each introducing a contradiction, and
§7 carries it verbatim rather than drafting a fourth.

### Correct a bucket-ii field

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-correction`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios**:
- The `CorrectionDoor` (`sku × correct`) accepts `(skuId, field ∈ {type, meter declaration pair},
  new value, expected revision)` and admits on **one of three gates**: the ordinary fresh-zero
  gate, the unresolvable-target gate, or the break-glass admission.
- On approval it re-publishes the head as version **N+1** through `01-foundation`'s publish door,
  passing the field and new value as that door's **optional third argument** (**P-D-41**), so the
  correction and the `published_version` bump are **one statement** — the only form 01 §4.2 admits
  after first publish.
- The full validation pipeline runs, a corrected meter re-resolves `usageTypeRef` (**P-D-05**), and
  `SkuImmutableFieldCorrected` is emitted with `quorumReduced` recorded below the default of 2.

**Error Scenarios**:
- Structural identity is **never** correctable — bucket-i, and the door physically cannot write it.
- Not fresh-zero on the normal lane ⇒ `CORRECTION_REFERENCED`, **naming the per-producer detail**: a
  stale producer is named as the blocker, never hidden inside a boolean.
- An unpublished bucket-iii/iv edit on the head ⇒ `CORRECTION_DIRTY_HEAD`, because a correction is
  surgical and a co-published edit would misattribute the corrected version's content.
- An open approval on the subject ⇒ `CORRECTION_APPROVAL_OPEN`.
- A PII hit in the reason ⇒ `CONTENT_PII_BLOCKED`, checked before the row is written.

**Boundary**: the admission gate is **itself a registered validator on this publish** and re-runs
**inside the publish transaction** — the door-acceptance check is a fast-fail, never the last word.
And the validator re-checks **the lane's own** admission predicate, not always fresh-zero: naming
fresh-zero for every lane refuses every break-glass re-publish by construction, since that lane is
admissible only when no producer can answer fresh-zero. The ceremony machinery is `05-governance`'s;
this feature supplies the subject kind `sku_correction` and the gates.

### Break-glass correction

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-breakglass-correction`

**Actor**: `cpt-cf-bss-products-actor-catalog-admin`

**Success Scenarios** — **two admission arms, and only the first is flag-gated**:
- **Arm (a), the signal is entirely unavailable.** Admissible when the feature flag is ON **and**
  at least one producer is registered **and every** registered producer is stale or never-received.
  The quantifier cannot be vacuously true over an empty set, so the unavailability-evidence snapshot
  is non-empty by construction. Single-SKU only, through the same `CorrectionDoor`.
- **Arm (b), an unresolvable meter declaration** (**P-D-16**). When the subject's declared
  `usageTypeRef` **no longer resolves** — the resolver answers not-found, not a timeout — the door
  admits a meter-declaration correction **regardless of the reference predicate**. The reference is
  real, and that is the reason to repair the declaration rather than the reason to refuse. This arm
  is **not** behind the flag (**P-D-48**): its admission predicate is a resolver fact rather than
  operator discretion, and the tripwire already counts it.
- Both arms keep the full `N`-governed ceremony with `quorumReduced` recorded, a **mandatory**
  reason, and a `SkuCorrectionOverride` row carrying **the admitting arm's evidence** —
  per-producer unavailability on (a), `unresolvable-target` on (b) — emitted alongside
  `SkuImmutableFieldCorrected`.
- Every break-glass correction increments the `TripwireCounter`.

**Error Scenarios**:
- The flag OFF ⇒ `BREAKGLASS_CORRECTION_DISABLED`, a 403. The flag governs **arm (a) only**.
- A single fresh producer routes back to the normal gate ⇒ `CORRECTION_SIGNAL_AVAILABLE`, unless
  arm (b) admits.
- A PII hit in the reason ⇒ `CONTENT_PII_BLOCKED`.

**Boundary**: this lane is explicitly **not** a §6.8 `BreakGlassSession` — that mechanism never
authorizes writes — so it does **not** inherit `inst-bg-open`'s fixed platform floor. Its principal
is the tenant's own, so it follows the tenant's `N` as **P-D-13**'s sixth enumerated site, and it is
safe at `N = 0` by reason plus evidence plus tripwire rather than by a floor.

**Why arm (b) exists at all, stated because it reads as an exception and is not.** A sold SKU whose
`UsageType` the collector deleted is otherwise wedged in **every lane at once**: `fresh > 0` refuses
the normal door, the signal *being* available refuses arm (a), and retire-and-clone is blocked
because 04's flip guard defers on anything but fresh-zero with no force-retire door in v1. The PRD
confirms a collector can delete a referenced usage type, so this is a reachable state rather than a
hypothetical.

## 3. Processes / Business Logic (CDSL)

Each process below is **declared here and stepped in
[`../design/07-reference-signal.md`](../design/07-reference-signal.md) §3**, whose steps are the
normative ones.

### Watermark storage and freshness

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-watermark-storage`

**Input**: a `(producer, watermark_at, complete skuId set)` post from a registered producer, and the
receiving clock.

**Output**: `products_reference_watermark` at `(tenant_id, producer)` carrying `watermark_at` and
`posted_at`; `products_reference_member` at `(tenant_id, producer, sku_id)` **replaced as a set**;
and, at read time, a freshness verdict per producer.

**Boundary and the two clocks, because they are easy to conflate.** Freshness is evaluated **against
`watermark_at`**, never `posted_at` — the producer's claim instant is the semantic one. `posted_at`
is written from the **receiving** clock, which is the operand the future bound of the ingest flow is
evaluated against, and the **stored** value is read by nothing: it is the audit record of the clock,
not an input to any later rule. The staleness alarm keys on the **registered** set, so a retired
producer stops alarming.

Membership lookup is an index hit and the predicate is **O(producers)**, which is the whole point of
the full-set replacement: the cost scales with the producer count, not the catalog.

### Error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-reference-errors`

**Input**: a refusal raised by any door of this feature.

**Output**: a `DomainError` variant carrying its wire code, and an RFC 9457 problem response at the
mapped status.

**The roster is eleven**, and it is normative at
[`../design/07-reference-signal.md`](../design/07-reference-signal.md) §3.2, which keeps
`cpt-cf-bss-products-contract-reference-errors`:

| code | status | raised by |
|---|---|---|
| `PRODUCER_UNREGISTERED` | 403 | the watermark door, on an unregistered poster |
| `WATERMARK_REGRESSION` | 409 | the watermark door, on an older `watermark_at` |
| `WATERMARK_CONFLICT` | 409 | the watermark door, on an equal `watermark_at` with a different set |
| `WATERMARK_FUTURE` | **422 architectural → 400 on the wire** | the watermark door, above the receiving clock plus skew |
| `PRODUCER_SET_EMPTY_FORBIDDEN` | 409 | producer retirement, on the last registered producer |
| `PRODUCER_RETIREMENT_WOULD_FREE` | 409 | producer retirement, on a stale or never-received producer |
| `CORRECTION_REFERENCED` | 409 | the correction door's ordinary gate |
| `CORRECTION_DIRTY_HEAD` | 409 | the correction door, on an unpublished bucket-iii/iv edit |
| `CORRECTION_APPROVAL_OPEN` | 409 | the correction door, on an open approval |
| `CORRECTION_SIGNAL_AVAILABLE` | 409 | break-glass arm (a), when a producer is fresh |
| `BREAKGLASS_CORRECTION_DISABLED` | 403 | break-glass arm (a), when the flag is OFF |

**Four further codes appear in the slice and are foreign**: `ILLEGAL_FIELD_MUTATION` and
`STALE_REVISION` are `01-foundation`'s, `CONTENT_PII_BLOCKED` is `02-taxonomy-attributes`', and
`STALE_VERSION` is the **pricing** gear's — it reaches the slice only inside §3.2's quotation of
D-141, and no door of this feature raises it. A reader counting screaming-case tokens in that file arrives at fifteen;
the roster this feature owns is eleven.

**The status of every code is pinned by the slice, and §5's DoD transcribes it rather than deriving
one.** `design/07` §3.2 carries a `**Problem responses (RFC 9457):**` block covering all eleven, and
its architectural-422 note: *"no `CanonicalError` category renders 422, so each reaches the wire as a
400 carrying its code, and no endpoint may declare a 422 for an error **carrying a registry code** in
`OpenAPI`"*. The status column of §3's table above is that block, transcribed.

## 4. States (CDSL)

**No state machine is declared by this feature, and that is a measurement rather than an omission.**

`design/07` declares no `state-` id. The one value roster this feature stores is a storage column,
normative at that slice's §4: `products_reference_producer.state ∈ {registered, retired}`.

Because §4 declares nothing, **this feature mints no `inst-` id at all** — as on
`features/catalog-version.md`, and unlike `features/lifecycle.md`, whose §4 machine required one
step id per transition row. Every `inst-` reference in this document resolves into a design slice.

**And the predicate's four verdicts are deliberately not a machine.** They are the slice's own
four — `referenced`, **`unreferenced`** (fresh-zero), `conservatively_referenced` carrying C2's
**distinct** `stale` / `never_received` flag, and `no_producers` — and they
are **computed at read time** from the **registered** rows of `products_reference_producer`, from
`watermark_at` against the freshness threshold, and from the member set — they are stored nowhere
and no act transitions between them. Modelling them as states
would invent a store the design set does not have. §7 carries what *is* open about the producer
row's own two values: what happens to a retired producer's watermark and member rows, and to one
that re-registers.

## 5. Definitions of Done

Every DoD below names types, functions, tables and tests **that exist at `19a81a406`** wherever one
exists, rather than inventing a shape. Where the shipped seam cannot host what this feature needs,
the DoD says so and §7 carries the question.

### The watermark store

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-watermark-tables`

The system **MUST** create `products_reference_watermark`, keyed `(tenant_id, producer)`, carrying
`watermark_at`, `posted_at` and **`set_hash`** (**P-D-71**: `SHA-256` over the member `sku_id`s
sorted bytewise, **stored at ingestion** — recomputation from 10K member rows per comparison
declined); and `products_reference_member`, keyed
`(tenant_id, producer, sku_id)`, on **both** engines. Both **MUST** be tenant-scoped.
**A registered producer that has never posted has no watermark row** — `never-received` is the
row's absence, registration writing only `products_reference_producer` (P-D-71). **Member ids are
accepted unvalidated, counted and alarmed** (`reference_unknown_member`, P-D-71): a producer's
catalog lags erasure legitimately, and erasure leaves member rows untouched until the next full-set
post replaces them.

The member set **MUST** be **replaced as a set per ingestion**, atomically, so that no concurrent
reader observes a half-set. Membership lookup **MUST** be an index hit rather than a scan.

`posted_at` **MUST** be written from the **receiving** clock and **MUST NOT** be read by any
freshness evaluation — freshness reads `watermark_at`. The column is the audit record of the clock
the future bound was evaluated against.

**Implements**: `cpt-cf-bss-products-algo-watermark-storage`

**Constraints**: `cpt-cf-bss-products-constraint-tenant-isolation`

**Touches**:
- DB Table: `products_reference_watermark`, `products_reference_member`
- Entities: `ReferenceWatermark`, `ReferenceMember`, `WatermarkDoor`

### The producer registry

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-producer-table`

The system **MUST** create `products_reference_producer`, keyed `(tenant_id, producer)`, carrying
`state ∈ {registered, retired}`, `registered_at`, the ceremony reference, and the **declaration
payload** — the reserved field where Contracts' draft-and-quote-counting answer lands at its own
registration (PRD §15).

It **MUST** be seeded per tenant with `{pricing}` at bootstrap (**P-D-03**), and it is the operand
of the predicate and the source of the `06-catalog-version` capture-store ride.

**Implements**: `cpt-cf-bss-products-flow-producer-registration`

**Touches**:
- DB Table: `products_reference_producer`
- Entities: `ReferenceProducer`

### The break-glass evidence store

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-override-table`

The system **MUST** create `products_correction_override` carrying the SKU, the field, the
**mandatory** reason (the ceremony's), **the admitting arm's evidence** — a per-producer
unavailability snapshot on arm (a), `unresolvable-target` on arm (b) — the ceremony reference and the
instant. It **MUST** be append-only, being evidential.

The `TripwireCounter` **MUST** be a **windowed count over this table** and **MUST NOT** be a
separate counter column or row. There is no second piece of state to drift from the evidence.

**Implements**: `cpt-cf-bss-products-flow-breakglass-correction`

**Touches**:
- DB Table: `products_correction_override`
- Entities: `CorrectionOverride`

### The watermark contract, in `bss-products-sdk`

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-watermark-port`

The system **MUST** publish the watermark contract as a **client trait in `bss-products-sdk`**, a
typed contract a producer resolves from `ClientHub` rather than an implementation package, with the
in-process binding as the default deployment mode. The contract **MUST** carry
`(producer, watermark_at, complete skuId set)`.

**This is the second write method this design set puts on that SDK**, and the shape question was
**answered once for both** (**P-D-81** arm 4, `features/catalog-version.md` §7 row 28): a **trait
of its own** beside `ProductsClient`, which stays the read contract its own doc scopes it to. This
DoD cited that row rather than duplicating it, and the answer governs both features — the
increment contract and this one — as it was meant to.

**Implements**: `cpt-cf-bss-products-flow-watermark`

**Touches**:
- Entities: `ReferenceWatermark`

### The watermark door and its four refusals

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-watermark-door`

The door **MUST** be the out-of-process binding of the contract above and the authz door **both**
bindings pass (S2S). It **MUST** refuse an unregistered poster `PRODUCER_UNREGISTERED`, an older
`watermark_at` `WATERMARK_REGRESSION` — the equal-`watermark_at` comparison reading the stored
`set_hash` (**P-D-71**) — an equal `watermark_at` carrying a different set
`WATERMARK_CONFLICT`, and a `watermark_at` above the receiving clock plus the configured skew
`WATERMARK_FUTURE` — the last **alerted** as well as refused.

**The future bound is `p1` and its probe MUST test the chain, not the refusal.** One accepted
future-dated post from a registered producer is unrecoverable, and the chain is: that producer reads
**permanently fresh**, so the staleness alarm never fires; every later legitimate post is refused
`WATERMARK_REGRESSION`, so its member set is **frozen**; and **every SKU outside that frozen set
then reads fresh-zero** — the one verdict that unlocks retirement flips and the ordinary correction
gate. That is the *"never falsely free a referenced SKU"* invariant inverted by one bad timestamp,
which is why the slice calls the bound *"`p1`, not hygiene"*.

An idempotent replay — the same `watermark_at` with the same set — **MUST** be admitted.

**Implements**: `cpt-cf-bss-products-flow-watermark`

**Touches**:
- DB Table: `products_reference_watermark`, `products_reference_member`
- Entities: `ReferenceWatermark`, `ReferenceMember`

### The reference predicate, with its per-producer detail

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-reference-predicate`

The system **MUST** evaluate over every **registered** producer and return **both** the OR verdict
and the **per-producer detail**. A fresh watermark containing the SKU gives `referenced`; a fresh
watermark omitting it gives zero for that producer; a stale watermark gives
`conservatively_referenced(stale)` — the condition `reference_watermark_stale` fires on, the
evaluation itself emitting nothing (**P-D-59**); never-received gives
`conservatively_referenced(never_received)` under a **distinct** flag and **no** alarm.

**Fresh-zero MUST require every registered producer to be fresh AND omitting the SKU.** With zero
registered producers the answer **MUST** be `no_producers` — conservative, and **distinct** from
fresh-zero.

The verdict **MUST** be a boolean OR and **MUST NOT** be a sum. A count would make two producers
each holding the same SKU look like two references, and the contract is a complete set per producer,
not a tally.

**Four verdicts, where the PRD names three.** The fourth, `no_producers`, is a design-introduced
extension and is defensive-unreachable in v1 because the last-producer retirement is refused. It
**MUST** still be implemented: an empty producer set must never free anything, and the guard that
makes it unreachable is a rule that a later decision could relax.

**Implements**: `cpt-cf-bss-products-flow-predicate`

**Touches**:
- DB Table: `products_reference_watermark`, `products_reference_member`, `products_reference_producer`
- Entities: `ReferencePredicate`

### Producer registration as a governed live op

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-producer-registration`

Membership changes **MUST** be `GovernedLiveOp`s on `reference_producer × write`, **material**, and
each **MUST** emit `ReferenceProducerSetChanged` and audit. A registering producer's first watermark
**MUST** start `never-received`, so onboarding can only tighten.

Retiring the **last** registered producer **MUST** be refused `PRODUCER_SET_EMPTY_FORBIDDEN`.

Retiring a producer whose watermark is **stale or never-received** **MUST** be refused
`PRODUCER_RETIREMENT_WOULD_FREE` unless the retiring principal supplies the break-glass ceremony's
own justification, whose reason **MUST** pass 02's PII gate before the row is written.

**The rule this DoD does not state, because three attempts to state it each introduced a
contradiction.** Retiring a producer narrows the quantifier fresh-zero runs over, so a **fresh**
producer's retirement can still free a SKU and walk a previously blocked correction through the
**normal** door — no flag, no override row, no tripwire increment. §7 carries it verbatim from the
slice, which registered it rather than drafting a fourth rule.

**Implements**: `cpt-cf-bss-products-flow-producer-registration`

**Touches**:
- DB Table: `products_reference_producer`, `products_audit_log`
- Entities: `ReferenceProducer`

### The symmetric snapshot ride

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-producer-snapshot`

The registered producer set **MUST** ride `06-catalog-version`'s capture store per
`CatalogVersion`, symmetrically with the freeze-participant set, under its own `capture_kind`. A
historical verdict **MUST** be evaluated against the **then-registered** set, so onboarding a
producer never retro-flips a past mutability or retirement decision.

This is the counterpart of `inst-pr-snapshot`, which `features/catalog-version.md` names as one of
its five foreign seams — the two halves are one obligation seen from two sides, and 06's
`cpt-cf-bss-products-dod-snapshot-builder` obliges the capture that this DoD supplies.

**Implements**: `cpt-cf-bss-products-flow-producer-registration`

**Touches**:
- DB Table: `products_reference_producer`; and **`products_catalog_version_capture`**, the capture
  store having been resolved onto a table of its own by **P-D-60** (`features/catalog-version.md`
  §7 row 9, closed) — never `products_catalog_version_entry`, whose every row references an entity
  version
- Entities: `ReferenceProducer`

### The correction door, and the three gates it admits on

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-correction-door`

The system **MUST** serve a correction door on `sku × correct` accepting
`(skuId, field ∈ {type, meter declaration pair}, new value, expected revision)`, and it **MUST** be
the **only** door that writes a bucket-ii column **after first publish**. Below first publish the
admitting door is `01-foundation`'s `inst-fd-save-txn` (**P-D-41**).

**The Foundation already names this door and does not forward to it, and the DoD must keep that
posture.** `api/rest/products.rs`'s `correctable_after_publish` builds a shipped
`DomainError::IllegalFieldMutation` whose text is *"after first publish it is writable only through
the correction door (POST .../corrections, slice 07), which this door names rather than forwards
to"*, and eleven further sites across `bucket.rs`, `repo.rs`, `skus.rs` and `migrations_tests.rs`
carry the same posture — *"one door, one effect"*. So this DoD **adds** a door; it **MUST NOT**
re-route the head doors' refusal into it.

Structural identity **MUST NOT** be writable here — bucket-i, and the door must be unable to write
it rather than refuse to.

**Three admission gates, and the third is the one a two-gate reading loses**: the ordinary
fresh-zero gate, the unresolvable-target gate, and the break-glass admission. A SKU with a deleted
`UsageType` and `fresh > 0` is refused by the first and by break-glass arm (a) both, and reaches the
validator that admits it only through the second.

**Preconditions**: the head **MUST** be clean — no unpublished bucket-iii/iv edits,
`CORRECTION_DIRTY_HEAD`, because a correction is surgical and a co-published edit would misattribute
the corrected version's content — and the subject **MUST** carry no open approval,
`CORRECTION_APPROVAL_OPEN`. The `ApprovalRecord`'s subject kind is `sku_correction`, and the reason
**MUST** pass 02's `inst-av-pii-block` before the row is written, a hit failing
`CONTENT_PII_BLOCKED`.

**Implements**: `cpt-cf-bss-products-flow-correction`

**Touches**:
- API: `POST /bss-products/v1/skus/{id}/corrections` — **the shape the shipped refusal announces;
  the design set pins no route, see §7**
- DB Table: `products_sku`, `products_entity_version`, `products_approval`
- Entities: `Sku`, `CorrectionOverride`, `CorrectionDoor`

### The correction re-publish, and the validator that re-checks the lane

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-correction-republish`

The correction **MUST** be a governed **material** act through `05-governance`'s quorum at the
tenant's configured `N`, with **`quorumReduced` recorded on the `ApprovalRecord` and on the emitted
`SkuImmutableFieldCorrected`** whenever the effective count is below the default of 2 (**P-D-13**,
the same clause `inst-lc-undeprecate` and `inst-fz-force` carry).

On approval it **MUST** re-publish the head as version **N+1** through `01-foundation`'s publish
door, **passing the field and new value as that door's optional third argument** (**P-D-41**), so
the correction and the `published_version` bump are **one statement** — the only form 01 §4.2 admits
after first publish. It **MUST** run the full validation pipeline, and a corrected meter **MUST**
re-resolve `usageTypeRef` (**P-D-05**).

**The admission gate MUST re-run inside the publish transaction**, not only at the door: the
door-acceptance check is a fast-fail, never the last word, and a reference arriving between
submission and approval still refuses.

**How it re-runs is §7's, because the shipped pipeline cannot host it as a registered rule.**
`ValidationRule<S>::evaluate(&self, subject: &S, report)` is synchronous and judges the subject row
alone, while this re-check reads three other tables and, on arm (b), calls the resolver. The publish
path already states the consequence in its own words — such a check *"runs as a continuation of the
same identity phase … rather than as a `Phase::Identity` rule that cannot reach"* its operand — and
`features/lifecycle.md` §7 row 20 registers the general question. **This DoD therefore obliges the
re-run and not its host.**

**And that validator MUST re-check the lane's own predicate, not always fresh-zero.** On the normal
lane, fresh-zero; on break-glass arm (a), its own predicate — every registered producer still stale
or never-received; on arm (b), its own — the target still unresolvable. Naming fresh-zero for all
three has only bad readings: taken literally it refuses **every** break-glass re-publish, since that
lane is admissible only when no producer can answer fresh-zero; skipped instead, nothing re-checks
admission at commit and a correction could land after a producer recovered and reported the SKU,
with an override row recording unavailability evidence **already false at the instant it was
written**.

**Implements**: `cpt-cf-bss-products-flow-correction`,
`cpt-cf-bss-products-flow-breakglass-correction`

**Touches**:
- DB Table: `products_sku`, `products_entity_version`, `products_approval`
- Entities: `Sku`, `EntityVersion`

### Break-glass arm (a): the signal is unavailable

- [ ] `p2` - **ID**: `cpt-cf-bss-products-dod-breakglass-unavailable`

Admissible **only** when `breakglass_correction_enabled` is `true` (**P-D-71**) **and** at least one
producer is registered **and
every** registered producer is stale or never-received — `never-received` being **the absence of the
producer's watermark row** (P-D-71), never a sentinel value. Single-SKU only, through the same correction
door.

The registered-producer clause **MUST** be part of the predicate rather than implied: it is what
stops the quantifier being vacuously true over an empty set, and it is what makes the
unavailability-evidence snapshot non-empty by construction.

A single fresh producer **MUST** route back to the ordinary gate with `CORRECTION_SIGNAL_AVAILABLE`,
unless arm (b) admits. The flag OFF **MUST** raise `BREAKGLASS_CORRECTION_DISABLED` as a **403**.

**The flag governs this arm only.** Arm (b) is not behind it.

**Implements**: `cpt-cf-bss-products-flow-breakglass-correction`

**Touches**:
- DB Table: `products_correction_override`
- Entities: `CorrectionOverride`

### Break-glass arm (b): the unresolvable target

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-breakglass-unresolvable`

When the subject's declared `usageTypeRef` **no longer resolves** — the resolver answers
**not-found**, not a timeout — the door **MUST** admit a meter-declaration correction **regardless
of the reference predicate** (**P-D-16**).

This arm **MUST NOT** be behind the break-glass flag (**P-D-48**): its admission predicate is a
resolver fact rather than operator discretion, a default-OFF flag would withhold the only exit the
decision exists to provide, and the tripwire already counts it.

It **MUST** keep the full ceremony — `N`-governed with `quorumReduced` recorded, **not** a bare
two-person count, which would re-block the `N = 0` tenant on the one door that can repair a deleted
`UsageType` — plus the mandatory reason, and a `SkuCorrectionOverride` recording
**`unresolvable-target`** rather than unavailability evidence. It **MUST** increment the same
`TripwireCounter`: a broken cross-gear reference is escalated, never normalized.

**Note the `p1`, against arm (a)'s `p2`.** Arm (a) is an operator-enabled emergency; arm (b) is the
only exit from a state the PRD confirms is reachable — a collector deleting a referenced usage type
wedges the SKU in every other lane at once.

**Implements**: `cpt-cf-bss-products-flow-breakglass-correction`

**Touches**:
- DB Table: `products_correction_override`
- Entities: `CorrectionOverride`

### The tripwire

- [ ] `p2` - **ID**: `cpt-cf-bss-products-dod-tripwire`

Every break-glass correction — **both arms** — **MUST** increment the `TripwireCounter`, which
**MUST** be a windowed count over `products_correction_override` rather than stored state. Past the
configured rate (interim > 5 per 30 days) it **MUST** raise **`reference_breakglass_tripwire`**
(**P-D-71** named it on the stale alarm's convention, beside `reference_watermark_future` and
`reference_unknown_member`) — the escalation alarm **and** flip the
standing `signal_delivery_release_blocker` status surface.

Degraded operation is escalated, never normalized (C6).

**What the counter's population is, and what clears the status surface, are both open** — §7 carries
them from the slice's own §6.

**Implements**: `cpt-cf-bss-products-flow-breakglass-correction`

**Touches**:
- DB Table: `products_correction_override`
- Entities: `CorrectionOverride`, `TripwireCounter`

### The error taxonomy, wired into `DomainError`

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-reference-error-taxonomy`

The system **MUST** add a `DomainError` variant for each of the **eleven** codes in §3, and each
**MUST** carry its wire code through `DomainError::code`.

**None of the eleven exists today.** One gate is a compile error and **four more are hand-written**,
named here as lines for the reason the authz DoD gives — a blanket criterion is ticked by
inspection:

- `DomainError::code` is *"deliberately exhaustive rather than a catch-all"*, so a variant added
  without a code **fails to compile**. That is the only automatic gate.
- `infra::error_mapping`'s `From<DomainError> for CanonicalError` — the module's own *"SINGLE
  authoritative … ladder"* — is an exhaustive `match` with no catch-all: eleven arms **MUST** be
  added there, carrying the statuses §3's table pins.
- `error_mapping_tests::DOMAIN_ERROR_VARIANTS` is `14` and its doc says *"Bump it in the same edit
  that adds the variant to both"* — it becomes **25**.
- `error_mapping_tests::one_of_every_variant` is a hand-written roster and gains eleven members.
- `error_mapping_tests::declared_status_and_code` is a hand-written per-variant status and gains
  eleven rows.

Two of the four **foreign** codes this feature cites — `ILLEGAL_FIELD_MUTATION` and
`STALE_REVISION` — are armed today.

**`CONTENT_PII_BLOCKED` guards three of this feature's operator reasons, across two of its doors,
and is armed nowhere.** It is
`02-taxonomy-attributes`' code, it has no `DomainError` variant, and this feature raises it on the
correction reason, the break-glass reason and the producer-retirement justification. This DoD
**MUST NOT** mint it — that would make this feature the second author of another slice's code — and
§7 routes it to its owner.

Each code **MUST** carry the RFC 9457 problem response **`design/07` §3.2 pins for it** — the block
§3's table transcribes — and **MUST NOT** re-derive one from a class rule. Two of the eleven are
**422 architectural**, which is not a wire status: the slice states that *"no `CanonicalError`
category renders 422, so each reaches the wire as a 400 carrying its code, and no endpoint may
declare a 422 for an error **carrying a registry code** in `OpenAPI`"*. `WATERMARK_FUTURE` is one of
them, and so is the foreign `CONTENT_PII_BLOCKED`. Declaring either as a literal 422 is what
`error_mapping_tests::the_products_owned_422_codes_stay_wire_400_by_design` exists to catch.

**Implements**: `cpt-cf-bss-products-algo-reference-errors`

**Touches**:
- Entities: `ReferenceWatermark`, `ReferenceProducer`, `Sku`

### The authz surface, and the four rosters it reddens

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-reference-authz`

The system **MUST** declare the authz labels and actions this feature's three doors spend —
`reference_producer × write`, `sku × correct`, and the watermark door's **`reference_signal ×
post`** — the pair `design/07` §2 states — **MUST** declare
one `crate::authz::resource_types` descriptor per new label, and **MUST** register a permission
instance per `(resource_type, action)` pair.

**Four shipped roster tests bound this, one of them positionally**, and each **MUST** be updated in
the same change — named as lines because a blanket criterion is ticked by inspection:

- `authz_tests.rs` asserts `labels::ALL == [labels::PRODUCT, labels::SKU]`, a **positional**
  equality, so a third label reddens it.
- `gts::permissions`' `EXPECTED_PERMISSION_IDS` holds exactly six ids, compared as a set **in both
  directions**, with a separate length check that catches a duplicate registration.
- `catalog_resource_types_match_authz_labels_all` asserts the catalog's distinct `resource_type`s are
  **exactly** `labels::ALL`.
- `catalog_actions_are_declared_action_constants` compares each instance's action against a
  **hard-coded** `known = [READ, WRITE, PUBLISH]`, so `correct` and the watermark action must be
  added to **that array** as well as to `actions`.

**The descriptor clause is not optional and nothing reddens without it.** `authz.rs` requires that
*"every authoring door passes one of these, never a bare label string, to `access_scope`"*, and its
descriptor test asserts only the two labels that exist while the stub type-schemas derive from
`labels::ALL`. So the labels could land, the instances could land, all four rosters could go green,
and this feature's doors would have no `ResourceType` to hand the gate.

**Whether `sku × correct` is a new action on the existing `sku` label or a new label is a question
`05-governance`'s roster owns**, and §7 carries it rather than deciding it here.

**Implements**: `cpt-cf-bss-products-flow-watermark`,
`cpt-cf-bss-products-flow-producer-registration`, `cpt-cf-bss-products-flow-correction`

**Touches**:
- Entities: `ReferenceProducer`, `Sku`

### The four config knobs, and the clock this feature owes `04-lifecycle`

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-reference-config`

`ProductsConfig` **MUST** gain four fields: the **freshness threshold** (interim 15 min), the
**ingestion clock-skew tolerance** (interim 5 min), the **tripwire rate** (interim > 5 per 30 days),
and **`breakglass_correction_enabled: bool`, default `false`** (**P-D-71** named the flag
enable-positive; **per-deployment and boot-time** — a policy gate, not an incident tool, the
emergency surface being `05`'s read elevation, and a runtime or per-tenant toggle needing machinery
no slice declares).

*(Measured at `19a81a406`, `ProductsConfig` shipped exactly two fields and the words* freshness,
watermark, tripwire *and* break-glass *appeared in `config.rs` zero times — there was no shaped
hole here, in contrast to `06-catalog-version`, which that file named twice as the owner of a
missing export. **P-D-87** arm 1 settled the four homes and they ship.)*

**The freshness threshold MUST be exported, because another feature already depends on reading it.**
`04-lifecycle`'s `ActivationRunner` re-evaluates a deferred flip by **polling on that interval** —
by design, since no event exists for a watermark, which is state rather than history. And
`features/lifecycle.md` §7 row 8 already records this as *"a fourth clock this feature does not
own"*, beside sibling clocks of which it says *"neither carries a value, a default or a config
home"*. **This
DoD is the export that row waits on**; §7 cites the row rather than raising the question again.

**The break-glass flag's name and polarity are open**, and this DoD deliberately names no constant:
the slice refers to the flag only through the refusal code `BREAKGLASS_CORRECTION_DISABLED`, so *"the
flag is OFF"* and *"the arm is disabled"* are the same words for opposite polarities and the slice's
own §5 and C5 have read it both ways. §7 carries it; the code stays the 403.

**Implements**: `cpt-cf-bss-products-algo-watermark-storage`,
`cpt-cf-bss-products-flow-breakglass-correction`

**Touches**:
- Entities: `ReferenceWatermark`

### The events and the alarms

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-reference-events`

The system **MUST** emit `SkuImmutableFieldCorrected`, `SkuCorrectionOverride` and
`ReferenceProducerSetChanged`, each in the **same transaction** as the act it announces and each
carrying a **versioned** schema reference.

**Watermark ingestion emits NO broker event**, and that is normative rather than an omission:
`inst-ws-no-event` says so and gives the reason — *"watermarks arrive continuously and are queryable
state, not domain history"*. A per-post event would redden both rosters below and put a
continuous-cadence stream on the broker.

**Both `THE_EIGHT` rosters redden and MUST be extended in the same change.**
`infra::events::events_tests::THE_EIGHT` pins the payload roster exact in both directions plus a
length assertion; `infra::broker::broker_tests::THE_EIGHT` is a second, wider roster — payload
token, `TYPE_ID` and the `SUBJECT_TYPE` that id must carry — with a hand-written counterpart list
beside it. **Neither is derived from the code**, both stating the same reason: *"a list built from
the code under test could only prove the code equals itself."*

`SkuImmutableFieldCorrected` and `SkuCorrectionOverride` are SKU-subjected and fit
`EventBodyCore`'s shape. **`ReferenceProducerSetChanged`'s aggregate is the tenant's producer set
itself, `aggregate_id = tenant_id`** (**P-D-71**: a per-tenant singleton, so per-`(tenant, aggregate)`
ordering serializes set changes per tenant — `FreezeParticipantSetChanged` is the same class, its
subject question staying `06`'s row). **`ReferenceProducerSetChanged` does not** — a producer set has no
`entityKind`, no `entityId`, no `internalRevision` and no `lifecycleState`, and `EntityKind` is
exactly `Product | Sku`. It needs the same entity-less core
`features/catalog-version.md` §7 row 27 registers for three of its own events, and its
`SUBJECT_TYPE` is the same open question that document's **row 47** raises. **Cite those rows; do
not re-raise them.** Its **`aggregate_id`** — which `infra::events::enqueue` requires and
`partition_for` consumes — is raised by neither, and is registered in §7 here.

The **alarms** are separate from the events and **MUST** be raised as alarms:
`reference_watermark_stale` — **an alerting rule over a gauge** (**P-D-59**): the gear exposes
`now − watermark_at` per `(tenant_id, producer)` over the **registered** set, the rule's condition
references the exported freshness threshold, and **no fired-state is stored** because nothing is
raised per call. Deregistration removes the series rather than silencing an alarm. The mechanism
transfers to the other two once §7 row 28 names them —
the future-watermark alert, and the tripwire escalation. **Never-received
is a verdict flag and MUST NOT raise an alarm** (C2) — the distinction is deliberate, because a
producer that has never posted is a deployment state rather than an incident.

**Implements**: `cpt-cf-bss-products-flow-correction`,
`cpt-cf-bss-products-flow-producer-registration`,
`cpt-cf-bss-products-flow-breakglass-correction`

**Touches**:
- Entities: `Sku`, `ReferenceProducer`, `CorrectionOverride`

### The audit trail for this feature's acts

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-reference-audit`

Every producer registration and retirement, every correction on either lane, and every watermark
post — **accepted or refused** — **MUST** write an audit row through `01-foundation`'s audit plane.
Ingestion is audit-plane by design: `inst-ws-no-event` states it and gives the reason —
*"watermarks arrive continuously and are queryable state, not domain history"*. An accepted post
that left no row would make the `WATERMARK_FUTURE` chain uninvestigable, since the table it
overwrote is the only other record of what the producer claimed.

A break-glass correction's row **MUST** carry the ceremony reference, the same value
`products_correction_override` stores, so the ceremony and the evidence are joinable from either
side.

**Implements**: `cpt-cf-bss-products-flow-producer-registration`,
`cpt-cf-bss-products-flow-correction`,
`cpt-cf-bss-products-flow-breakglass-correction`

**Touches**:
- DB Table: `products_audit_log`, `products_correction_override`
- Entities: `CorrectionOverride`

## 6. Acceptance Criteria

**The predicate matrix as one fixture**

- [ ] Six fixture cases in a single fixture — fresh-zero, fresh > 0, stale, never-received,
      unregistered-silence, zero-producers — each with its **per-producer detail** asserted, not just
      the OR.
- [ ] Every conservative refusal is paired with its **fresh-zero positive control**, so no assertion
      can pass because the fixture could not reach the permissive branch.
- [ ] `never-received` puts no series in the staleness gauge while `stale` does, which is what
      `reference_watermark_stale` fires on (**P-D-59** — the predicate raises nothing itself); asserted
      apart, because the distinction is the design's and a merged assertion would hide it.
- [ ] `no_producers` is distinguishable from fresh-zero in the returned verdict, not only in a log
      line.

**Watermark ingestion**

- [ ] The member set is replaced **atomically** — a concurrent reader never observes a half-set,
      driven by real concurrency rather than read-then-assert.
- [ ] A regression is refused and the stored watermark is **unmoved**.
- [ ] An idempotent replay — same `watermark_at`, same set — is admitted and changes nothing.
- [ ] An equal `watermark_at` with a **different** set is refused `WATERMARK_CONFLICT`.

**The future bound, over the chain rather than the refusal**

- [ ] A post with `watermark_at > now + skew` is refused `WATERMARK_FUTURE`, the stored watermark
      unmoved, and the alert raised.
- [ ] **The chain probe.** Force-accept a future-dated watermark in a fixture, then assert every link
      the slice names: that producer reads **permanently fresh**; the staleness alarm does **not**
      fire; a later legitimate post is refused `WATERMARK_REGRESSION`; and **a SKU outside the frozen
      set reads fresh-zero**. Asserting only the refusal leaves the consequence untested, and the
      consequence is the invariant this feature exists for.

**Corrections**

- [ ] **RED first**: a referenced SKU is refused `CORRECTION_REFERENCED` **naming the blocking
      producer**, then fresh-zero goes green through the full quorum, the re-publish and the
      `usageTypeRef` re-resolution.
- [ ] A dirty head is refused `CORRECTION_DIRTY_HEAD`; the same correction on a clean head succeeds.
- [ ] An open approval on the subject is refused `CORRECTION_APPROVAL_OPEN`.
- [ ] A reference arriving **between submission and approval** still refuses at commit — the
      registered validator, not the door's fast-fail, is what catches it.
- [ ] The validator re-checks **the lane's own** predicate: a break-glass arm-(a) re-publish succeeds
      while no producer is fresh and is refused once one recovers, and an arm-(b) re-publish succeeds
      while the target is unresolvable and is refused once it resolves. A fresh-zero-for-all-lanes
      reading fails this criterion in both directions, which is why it is written as two.
- [ ] Structural identity cannot be submitted to this door at all.

**Break-glass**

- [ ] One fresh producer ⇒ `CORRECTION_SIGNAL_AVAILABLE`; every producer stale or never-received and
      the flag ON ⇒ admitted with a per-producer unavailability snapshot recorded.
- [ ] The flag OFF ⇒ `BREAKGLASS_CORRECTION_DISABLED` at **403**, on arm (a) **only**: an arm-(b)
      correction succeeds with the flag OFF.
- [ ] Arm (b): a `fresh > 0` SKU whose `usageTypeRef` no longer resolves is admitted for a
      meter-declaration correction, records **`unresolvable-target`**, and increments the counter.
- [ ] A resolver **timeout** does **not** admit arm (b) — only not-found does. Asserted, because the
      two are one call away and the wrong one turns an outage into a write path.
- [ ] The tripwire trips on the **sixth** within the window and the `signal_delivery_release_blocker`
      surface flips.

**Producer registration**

- [ ] Retiring a **stale** producer ⇒ `PRODUCER_RETIREMENT_WOULD_FREE`, with the **fresh-producer
      control** — which is the control that shows the guard is about silence rather than about
      retirement.
- [ ] Retiring the **last** producer ⇒ `PRODUCER_SET_EMPTY_FORBIDDEN`.
- [ ] Onboarding probe: registering a producer flips a previously fresh-zero SKU to conservative
      (never-received), and **no historical decision re-opens** — a past version's verdict still
      reads against the then-registered set.

**Positive controls, one line per declared code** — eleven codes, eleven lines. A blanket criterion
here is ticked by inspection.

- [ ] `PRODUCER_UNREGISTERED` — a post from an unregistered producer is refused; the same post after
      registration succeeds.
- [ ] `WATERMARK_REGRESSION` — an older `watermark_at` is refused; a newer one succeeds.
- [ ] `WATERMARK_CONFLICT` — an equal `watermark_at` with a different set is refused; with the same
      set it is an idempotent success.
- [ ] `WATERMARK_FUTURE` — above `now + skew` refused; at `now` accepted.
- [ ] `PRODUCER_SET_EMPTY_FORBIDDEN` — retiring the only producer refused; retiring one of two
      succeeds.
- [ ] `PRODUCER_RETIREMENT_WOULD_FREE` — retiring a stale producer refused; retiring a fresh one
      succeeds.
- [ ] `CORRECTION_REFERENCED` — a referenced SKU refused naming the producer; the same SKU at
      fresh-zero succeeds.
- [ ] `CORRECTION_DIRTY_HEAD` — a head with an unpublished bucket-iii edit refused; a clean head
      succeeds.
- [ ] `CORRECTION_APPROVAL_OPEN` — an open approval refused; after it closes, succeeds.
- [ ] `CORRECTION_SIGNAL_AVAILABLE` — arm (a) with one fresh producer refused; with all stale,
      succeeds.
- [ ] `BREAKGLASS_CORRECTION_DISABLED` — arm (a) with the flag OFF refused **403**; with it ON,
      succeeds.

**Controls on the shipped seam**

- [ ] `bucket_tests::buckets_ii_and_iv_have_no_members_today` **still passes** after this feature
      lands. It reddens on `03-sku-classification`'s columns, not on this door, and a change here
      that reddens it means the door has grown a column it does not own.
- [ ] A bucket-ii write attempted through a **head** door is still refused by
      `correctable_after_publish`, unchanged — this feature adds a door and does not re-route that
      refusal.

## 7. Known unknowns

**The arithmetic of this section.** Thirty-eight rows: **sixteen carried verbatim** from
[`../design/07-reference-signal.md`](../design/07-reference-signal.md) §6 — the slice's full count,
not a selection — and **twenty-two raised here**: five while authoring, from reading the crate,
twelve by the three-lens review of this document, and five (rows 34–38) by the three-lens review of
commit `e939953ee`, each an operand or a contract the shipped code went looking for and could not
find. Of the thirty-eight, **nineteen block no DoD in this
document** (rows 3, 4, 17, 31, 33 and 38, plus row 27, which **P-D-59 resolved on 2026-08-31**, and
rows 1, 13, 25, 26, 28, 29 and 30, which **P-D-71 resolved on 2026-09-01**, and rows 7, 12, 16, 19
and 32, which **P-D-87 resolved** the same day, freeing `cpt-cf-bss-products-dod-reference-config`,
`dod-producer-table` and `dod-watermark-door` — all kept in place
rather than struck; row 32 was listed here before its own `Blocks` field named a DoD, and P-D-87
made the listing true rather than leaving the two at odds); the other nineteen each name the DoD
they block. **Rows 34–36 land on `dod-reference-audit` and row 35 also on
`dod-reference-events`** — both were free of a live blocker before this pass, and both are now
measurably blocked, which is what the crate found when it went to build them.

**Carried, not answered.** A question is registered against **its owner's** register. Where the
owner is another document, the row carries a one-line pointer and nothing more.

**One departure from verbatim, declared so the claim is checkable.** The slice ends each item with a
provenance marker (*"(Raised by the slice-07 first lens pass.)"*) and an inline `Owner:` sentence;
both are converted here into this document's `**Owner**:` field, and row 5's *"§6's four
sub-questions"* is re-aimed to *"row 2's"* because §6 is the slice's numbering. Nothing else is
altered: the carried text was diffed against `design/07` §6 sentence by sentence, mechanically.

**And three questions this feature deliberately does NOT raise, because a sibling FEATURE already
owns them.** A duplicate open item is worse than a missing one, since closing the general question
leaves the specific ones looking open:

- whether `ProductsClient` widens or a second SDK trait arrives — `features/catalog-version.md` §7
  row 28, cited by `cpt-cf-bss-products-dod-watermark-port`;
- the entity-less event body core and its `SUBJECT_TYPE` — that document's §7 rows **27 and 47**,
  cited by `cpt-cf-bss-products-dod-reference-events`; its `aggregate_id` is raised by neither and
  is row 25 below;
- the freshness threshold's config home — `features/lifecycle.md` §7 row 8, which names it as *"a
  fourth clock this feature does not own"*, cited by `cpt-cf-bss-products-dod-reference-config`;
- the `sku_correction` subject's representation at the governance seam —
  `features/governance.md` §7 row 29, which measured that **four of the five subject kinds cannot
  cross** it because the seam's kind enum is exactly `Product | Sku`, cited by
  `cpt-cf-bss-products-dod-correction-republish`;
- whether §6 owes one criterion per DoD or is a selected set — `features/catalog-version.md` §7
  row 50.

### Carried verbatim from `design/07` §6

1. ~~**OPEN — the break-glass arm's feature flag has no name of its own.**~~
    **Answered in the slice (owner call, 2026-09-01 — P-D-71 arm 1): `breakglass_correction_enabled:
   bool`, default `false`** — enable-positive, so the polarity ambiguity dies; the refusal code stays
   the 403.
    Original text: It is referred to by the
   refusal code `BREAKGLASS_CORRECTION_DISABLED`, so "the flag is OFF" and "the arm is disabled" are
   the same words for opposite polarities, and `design/07` §5's probe and its C5 have read it both ways. A flag needs
   a name and a stated polarity; the code stays the 403.
    **Blocks**: no DoD — **resolved by P-D-71**; `cpt-cf-bss-products-dod-reference-config` and `dod-breakglass-unavailable` carry the name.
    **Owner**: was this feature; **closed**.

2. ~~**OPEN (third full-review pass) — a fresh producer's retirement can still free a SKU, and three
   attempts to write the rule have each introduced a contradiction. Registered rather than drafted a
   fourth time.**~~ **Answered (P-D-129, 2026-09-03): a producer retires only with an empty current watermark** — a live one posts an empty set first; a dead one retires under `05`'s break-glass elevation at the retirement door, with one `producer_unavailable` override row per freed SKU as the evidence, and those rows feed the tripwire as they should. *The item's text stood as:* `inst-pr-retirement` guards the case its own sentence names: a **stale or
   never-received** producer, whose *silence* is what pins the set. It does **not** dispose of a
   **fresh** producer that is the only one reporting SKU `X`: removing it drops the only non-zero
   vote and `X` goes fresh-zero, opening the bucket-ii correction door on it through the normal lane.
   Four sub-questions the owner has to settle together, because each answer constrains the next:
   1. **Which test binds.** "Refused whenever removal would move any SKU to fresh-zero" is a property
      of the current constellation and lets a retirement through when a *second* producer is stale —
      the SKU only falls later, when that one posts. "Refused whenever the retiring producer's
      current watermark is non-empty" is a property of the producer alone and has no such delay, at
      the cost of refusing retirements that free nothing.
   2. **What preserves what is removed — and this half is already settled by the PRD.** A retired
      producer is unregistered, and **AC #43** reads: *"only **registered** producers' signals or
      silence MUST factor in; an unregistered producer's absence MUST NOT pin SKUs
      conservatively-referenced"*. The first clause is the binding one: a retired producer's signals
      must not factor into the predicate at all, so **keeping its watermark alive in the predicate is
      not available** — which is what the mechanism struck from this rule tried to do. §6.13 carries
      the same clause in its own words (`fr-reference-producer-registration`), so the FR and the AC
      agree. So the choice is narrower than it looked: either the freed SKUs are cleared one by one
      **before** the retirement, or a new evidential record kind is defined for them.
   3. **Which record, and what it does to the tripwire.** `SkuCorrectionOverride` rows feed the
      `TripwireCounter`, and C6 trips the `signal_delivery_release_blocker` above five in thirty
      days. One governed
      retirement freeing six SKUs would block the release; the never-received case names the whole
      catalogue. So the retirement evidence needs either its own record kind or its own window.
   4. **Which lane.** The break-glass lane in this slice is the *correction* lane — single-SKU,
      `sku × correct`, gated by `inst-cr-door`. It is not a retirement door: retirement is
      `reference_producer × write`, a different resource and action.

   **Until this is settled the guarded case is guarded and the fresh case is not**, which is the
   honest state and is why it is written here rather than papered over in the rule.
   **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-producer-registration`, and constrains)*
   `cpt-cf-bss-products-dod-correction-door`, whose normal lane is what the freed SKU walks through.
   **Owner**: this feature, jointly with the tripwire's and the PRD's owners — sub-questions 3 and 4
   reach both.

3. **The pricing watermark is a joint build** (**P-D-03**): the producer-side query ("complete live
   plan→SKU reference set") and its cadence are pricing's to design; this slice's door and the §15
   mirror are ready — the joint fixture belongs to slice 12's seam suite.
   **Blocks**: no DoD — the registry side is complete without it.
   **Owner**: pricing, with `12-consumer-contracts` for the fixture.

4. **Watermark set size**: full-set replacement at 10K SKUs × cadence is fine as rows, but the door
   should accept a compressed set representation from day one (wire-level; no semantic change) —
   implementation note.
   **Blocks**: no DoD — it is a wire-level note, and `cpt-cf-bss-products-dod-watermark-port` is
   free to satisfy it.
   **Owner**: this feature at implementation.

5. ~~**`PRODUCER_RETIREMENT_WOULD_FREE`'s exception has no lane.**~~ **Answered (P-D-129, 2026-09-03): the exception exists for a dead producer only**, at the retirement door under break-glass elevation, evidenced by `producer_unavailable` override rows. *The item's text stood as:* The refusal is buildable; its escape
   hatch — "the break-glass ceremony's own justification" — has no admission predicate, no evidence
   record and no grant, and this slice's only break-glass lane is the single-SKU correction door,
   which is not a retirement door. No document defines a retirement-lane ceremony. *(All three lenses
   raised it independently.)*
   **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-producer-registration`.)*
   **Owner**: the owner of row 2's four sub-questions — decide whether the exception exists in v1 at
   all.

6. ~~**OWED — the tighter row-image predicate 01 books against this slice.**~~ **Answered (P-D-129, 2026-09-03): the bump **and** `correction_ref` set in the same statement** — a nullable uuid on `products_sku` only the correction re-publish writes; `01`'s migration, the lead's build. *The item's text stood as:* 01 §4.2 says twice that
   the physical guard carries the interim predicate "with a tighter one still **owed by 07**"; this
   slice carries no such item. Until it is supplied, door identity for bucket-ii head-row writes is
   an application guarantee only, and **any** publish carrying the third argument passes the guard.
   *Re-measured at `19a81a406`: the phrase occurs **twice** in `design/01`, but once under §3.1's
   `inst-fd-bucket-ii-refusal` and once under §4.2's `products_sku` — not twice in §4.2 as the row
   reads. The obligation is unaffected; the citation is.*
   **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-correction-door`,)*
   `cpt-cf-bss-products-dod-correction-republish`.
   **Owner**: this feature.

7. ~~**The `WATERMARK_FUTURE` skew tolerance has no config home.**~~
    **Answered (owner call, 2026-09-01 — P-D-87 arm 1): `watermark_skew_tolerance_minutes` on
   `ProductsConfig`, interim 5** — per-deployment and boot-time, P-D-84 arm 5's posture, now
   precedent rather than invention; row 19 takes the other three the same way.
    Original text: `inst-ws-not-future` introduces a
   "configured tolerance, interim 5 min"; `PRD` §17.1 has no row for it, and §1.4's reference line
   claims only the freshness and tripwire interims. It is the one configurable in this slice with no
   home. *(Two lenses raised it independently.)*
    **Blocks**: no DoD — **resolved by P-D-87**.
    **Owner**: was the §17.1 policy owner. *See row 19: measured at `19a81a406`, three of this feature's; **closed**.

8. ~~**What population does the tripwire count?**~~ **Answered (P-D-129, 2026-09-03): two counters, one window** — `producer_unavailable` feeds the release blocker, `unresolvable_target` its own alarm; the PRD owner may narrow. *The item's text stood as:* C6 counts break-glass corrections per window, and the
   unresolvable-target arm "increments the same `TripwireCounter`" — but that arm is admissible while
   the signal is fully available, and `fr-failsafe-tripwire` scopes the requirement to operating "in
   `SkuReferenceCount`-unavailable fail-safe mode". Six deleted-`UsageType` repairs in a month would
   reclassify signal *delivery* as a release blocker. P-D-16 amended the correction FR and AC #4 and
   did not touch the tripwire FR. *(Two lenses raised it independently.)*
   **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-tripwire`,)*
   `cpt-cf-bss-products-dod-breakglass-unresolvable`.
   **Owner**: the tripwire's §17.1 owner.

9. ~~**Where does `signal_delivery_release_blocker` live, and what clears it?**~~ **Answered (P-D-129, 2026-09-03): derived** — a rolling-window predicate over the override table, no row, no operator exit; C6's rate rule is a rolling window. *The item's text stood as:* Derived from the
   rolling window it clears itself thirty days later — normalizing degraded operation, which C6
   forbids; stored, it is a state with no exit and no table in `design/07` §4.
   **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-tripwire`.)*
   **Owner**: `fr-failsafe-tripwire`'s owner.

10. ~~**What carries the admitting lane into the publish transaction?**~~ **Answered (P-D-129, 2026-09-03): the envelope's `kind`** — `sku_correction` is a registered `GovernedLiveOp` kind and the lane is its arm. *The item's text stood as:* `inst-cr-republish` has the
    registered validator re-check "the lane's own admission predicate", and nothing tells it which
    lane admitted: 01's `PublishDoor` signature has no lane argument, 05's `ApprovalRecord` has no
    arm discriminator, and this slice's override row is written *by* the re-publish.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-correction-republish`.)*
    **Owner**: this feature with `01-foundation` and `05-governance` — a fourth door argument, a field
    on the approval, or a pre-written admission record.

11. ~~**Who sets `required` on a `sku_correction` `ApprovalRecord`?**~~ **Answered (P-D-129, 2026-09-03): `05`'s evaluator, and it returns material** once `sku_correction` is a registered `GovernedLiveOp` kind; `required = N`. *The item's text stood as:* This slice calls the correction a
    material act at the tenant's `N`; 05's evaluator returns material only on a bucket-iii touch, the
    enumerated ops, an affected-entity count, or a registered `GovernedLiveOp` kind — and 05
    explicitly removes the metering-unit field from the evaluator's view, while `sku_correction` is
    not a `GovernedLiveOp` kind. **As it stands the evaluator returns non-material and the correction
    closes on `min(N, 1)`.**
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-correction-republish`.)*
    **Owner**: `05-governance`'s owner.

12. ~~**What happens to a retired producer's watermark and member rows, and to one that re-registers?**~~
    **Answered (owner call, 2026-09-01 — P-D-87 arm 2): the watermark and member rows are
    DELETED in the retirement transaction**, the producer row staying with `state = retired`, so a
    re-registering producer starts `never-received` — which is what makes the DoD's own
    onboarding-can-only-tighten clause true rather than merely stated.
    Original text:
    `design/07` §4's producer row carries only a state, a registration instant, a ceremony ref and
    the declaration payload, with no clearing rule. If the rows survive, retire-then-re-register inside the freshness window makes the
    producer read **fresh** against a stale member set and frees every SKU that has since gained a
    reference — the opposite of "onboarding can only tighten, never free".
    **Blocks**: no DoD — **resolved by P-D-87**; `cpt-cf-bss-products-dod-producer-registration` keeps rows 2, 5, 15 and 16.
    **Owner**: was `fr-reference-producer-registration`'s owner; **closed**.

13. ~~**Where does `inst-ws-monotonic`'s set hash come from?**~~
    **Answered in the slice (owner call, 2026-09-01 — P-D-71 arm 3): a `set_hash` column on
    `products_reference_watermark`, stored at ingestion** — `SHA-256` over the member `sku_id`s
    sorted bytewise; recomputation from 10K member rows per comparison declined.
    Original text: An equal `watermark_at` with an identical
    set hash is an idempotent success and with a different set a refusal, while §4 declares no hash
    column and no rule states its derivation — canonical ordering, algorithm, stored at ingestion or
    recomputed from member rows at 10K SKUs.
    **Blocks**: no DoD — **resolved by P-D-71**; `cpt-cf-bss-products-dod-watermark-tables` and `dod-watermark-door` carry the column.
    **Owner**: was this feature with the schema owner; **closed**.

14. ~~**Is the correction door's `expected revision` the `If-Match` precondition or a body field?**~~ **Answered (P-D-129, 2026-09-03): `If-Match`** — P-D-33's convention for every mutating door. *The item's text stood as:*
    This pass gave the mismatch 01's `STALE_REVISION`; which surface carries it is still unstated,
    and it determines the door's declared response map.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-correction-door`.)*
    **Owner**: this feature with `01-foundation`.

15. ~~**Which actor performs `reference_producer × write`?**~~ **Answered (P-D-129, 2026-09-03): any principal holding `reference_producer × write`** — the quorum is on the tenant's approvers, so a service at deploy and an operator use one door. *The item's text stood as:* `design/07` §1.3 assigns producer
    registration and retirement to nobody: the producer actors "register at their own build", which
    reads either as the service registering itself or as an operator registering it — incompatible
    with a material
    governed op requiring a tenant quorum. *(This document's own §1.3 names
    `actor-catalog-admin` on the ceremony, which is an authoring choice made here and not an answer
    — the incompatibility above is untouched by it.)*
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-producer-registration`,)*
    `cpt-cf-bss-products-dod-reference-authz`.
    **Owner**: this feature with `05-governance`.

16. ~~**What transport and success responses do this slice's three doors have?**~~
    **Answered (owner call, 2026-09-01 — P-D-87 arm 3): four routes and their 2xx**, each from
    the set's nearest precedent — the watermark post 200, `POST /reference-producers` 201, `POST
    /reference-producers/{producer}/retirements` 200, and the correction door **adopting the shape
    the shipped crate already announces** (row 20), `POST /skus/{skuId}/corrections`, 202 (the
    write happens at approval).
    Original text: Only the watermark
    door is bound to one; the correction door and the membership ops name no route and no 2xx, and
    `design/07` §3.2 gives only refusals. Every comparable operator door in the set names both — 02 added its
    path and pair expressly because without them 12's lint could not see the door.
    **Blocks**: no DoD — **resolved by P-D-87**; `dod-correction-door` and `dod-producer-registration` keep their other rows.
    **Owner**: was the design-set owner. *See row 20: the shipped crate already announces one of the; **closed**.

### Raised here rather than carried

Five, all from reading the crate at `19a81a406`. Every quotation was byte-verified against source.

17. ~~**A shipped green test's message assigns this feature's scope to it wrongly, and the message is
    what a later reader will act on.**~~ **Answered (P-D-129, 2026-09-03): stale by measurement** — `bucket_tests` counts two `Correctable` members and carries no such message. *The item's text stood as:* `bucket_tests::buckets_ii_and_iv_have_no_members_today`
    asserts the `Correctable` member count is zero with the message *"bucket-ii columns arrive with
    slice 07"*. `domain/bucket.rs`'s module doc says the opposite and is right: *"03 owns the
    columns and their registration while slice 07 owns the correction door"* — and the
    misattribution sits in **two** lines of `bucket_tests.rs`, the assertion message and the doc
    comment above the test. So the test reddens on
    `03-sku-classification`'s code, not this feature's, and a reader who trusts the message will look
    for the columns here.
    **Blocks**: no DoD — it constrains one, in that
    `cpt-cf-bss-products-dod-correction-door` must not grow a column.
    **Owner**: `01-foundation`'s code, whose test it is. One-line pointer only.

18. ~~**`CONTENT_PII_BLOCKED` guards three of this feature's operator reasons — across two of its
    doors — and is armed nowhere.**~~ **Answered (P-D-129, 2026-09-03): stale by measurement** — `DomainError::ContentPiiBlocked` exists and `content_pii_block` is called at five doors; `07`'s three reasons call it when `07`'s doors are built. *The item's text stood as:* The
    correction reason, the break-glass reason and the producer-retirement justification all fail on a
    PII hit with that code (**P-D-50**), and `domain::error::DomainError`'s fourteen variants carry no
    arm for it — unlike `ILLEGAL_FIELD_MUTATION` and `STALE_REVISION`, the other foreign codes this
    feature cites, which are armed. A code with no raiser anywhere in the crate is the class
    `04-lifecycle` found in `SCOPE_NARROWING_BLOCKED`. **This feature must not mint it** — that would
    make it the second author of another slice's code.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-reference-error-taxonomy`.)*
    **Owner**: `02-taxonomy-attributes`, which declares the code.

19. ~~**Three of this feature's four config knobs have no home, not one.**~~
    **Answered (owner call, 2026-09-01 — P-D-87 arm 1): all four knobs land on
    `ProductsConfig`** — `reference_freshness_minutes` (15), `watermark_skew_tolerance_minutes`
    (5), `tripwire_max_overrides_per_30_days` (5) and `breakglass_correction_enabled` (`false`) —
    the freshness threshold exported through a getter, the shape
    `resolved_idempotency_retention_hours` already has, since `04-lifecycle`'s runner polls on it.
    Original text: Row 7 records the skew
    tolerance. Measured at `19a81a406`, `ProductsConfig` ships **exactly two fields** —
    `idempotency_retention_hours` and `require_broker` — and the words *freshness*, *watermark*,
    *tripwire* and *break-glass* appear in `config.rs` **zero times**. So the tripwire rate and the
    break-glass flag are in the same position as the skew tolerance, and only the **freshness
    threshold** has an owner already named — `features/lifecycle.md` §7 row 8, which needs it
    exported. This is the mirror image of `06-catalog-version`, whose export `config.rs` names
    **twice** with the `clamp` shaped for it.
    **Blocks**: no DoD — **resolved by P-D-87**; `cpt-cf-bss-products-dod-tripwire` keeps rows 8 and 9.
    **Owner**: was this gear's config owner with the §17.1 policy owner; **closed**.

20. ~~**The shipped crate already announces one of the three routes the design set declines to pin.**~~ **Answered (P-D-129, 2026-09-03): the announced shape is adopted** — `POST /bss-products/v1/skus/{id}/corrections`, declared in `DECOMPOSITION` §2.7. *The item's text stood as:*
    `api/rest/products.rs`'s `correctable_after_publish` builds a refusal reading *"writable only
    through the correction door (POST .../corrections, slice 07)"*, so callers are told a route shape
    that no artifact carries — DECOMPOSITION §2.7 says *"API: None declared in the design set"* and
    row 16 records the gap. Either the artifact adopts the announced shape or the message is changed;
    a third route, invented at implementation, would make the refusal a lie.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-correction-door`.)*
    **Owner**: the design-set owner — row 16's owner, with this measurement attached.

21. ~~**Is `sku × correct` a new action on the existing `sku` label, or a new label?**~~ **Answered (P-D-129, 2026-09-03): a new action `correct` on the existing `sku` label**; `sku × write` does not reach the door. *The item's text stood as:* The slice writes
    the pair and `05-governance`'s roster owns the catalog. Measured: `authz::labels::ALL` is exactly
    `[PRODUCT, SKU]` and `actions` is exactly `read | write | publish`, so either answer extends a
    positionally-asserted roster, and the two answers differ in whether an existing `sku × write`
    grant reaches this door. This feature cannot widen another slice's table.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-reference-authz`.)*
    **Owner**: 05's roster owner.


22. ~~**How does the admission gate re-run inside the publish transaction?**~~ **Answered (P-D-129, 2026-09-03): P-D-121 row 19's shape** — the door resolves the reads before the transaction and hands the phase a `Resolution`; inside, a continuation of the identity phase. *The item's text stood as:*
    `cpt-cf-bss-products-dod-correction-republish` obliges the re-run and deliberately names no host,
    because the shipped pipeline cannot be one: `ValidationRule<S>::evaluate(&self, subject: &S,
    report)` is synchronous and judges the subject row alone, while this re-check reads
    `products_reference_watermark`, `products_reference_member` and `products_reference_producer`
    and, on arm (b), calls the resolver. The publish path states the consequence itself — such a
    check *"runs as a continuation of the same identity phase … rather than as a `Phase::Identity`
    rule that cannot reach"* its operand. Whether the trait widens or the re-check runs as a
    continuation is `features/lifecycle.md` §7 row 20's, which registers the general question.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-correction-republish`,)*
    `cpt-cf-bss-products-dod-correction-door`.
    **Owner**: `01-foundation`, with this feature.

23. ~~**Where does the correction's payload live between submission and the approved re-publish?**~~ **Answered (P-D-129, 2026-09-03): in the `GovernedLiveOp` envelope**, whose bytes `05`'s snapshot pins; the apply writes head and override row in the re-publish transaction. *The item's text stood as:* The
    door accepts the new value and the write happens on approval, and each candidate store is closed
    by another document: the head, because this door is the only writer of a bucket-ii column after
    first publish; the `GovernedLiveOp` payload channel, because §7 row 11 records that
    `sku_correction` is not such a kind; and the approval's stored snapshot, because
    `05-governance` C3 makes it byte-identical to the head at that revision. No table in §5 holds a
    pending correction.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-correction-door`,)*
    `cpt-cf-bss-products-dod-correction-republish`.
    **Owner**: `05-governance`'s owner with this feature.

24. ~~**What measures "the head is clean"?**~~ **Answered (P-D-129, 2026-09-03): digest equality** — the head rendered through `domain::canonical` over the frozen roster carries its last version row's `content_digest`; one operand for all three guards. *The item's text stood as:* `CORRECTION_DIRTY_HEAD` refuses on an operand no document
    names. The cheap comparison is unavailable: the frozen content excludes `lifecycle_state`,
    `deprecation_provenance`, `replaced_by_sku_id` and `internal_revision`, so a whole-image diff
    against `published_version` is not the same question. `06-catalog-version` records that this
    guard has **three** instances — `CORRECTION_DIRTY_HEAD`/`CORRECTION_APPROVAL_OPEN` here and
    `PROMOTION_DIRTY_HEAD` in 09 — and none of the three defines it.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-correction-door`.)*
    **Owner**: `01-foundation`'s head/version model owner, since the guard is shared by three
    slices.

25. ~~**What `aggregate_id` does `ReferenceProducerSetChanged` carry?**~~
    **Answered (owner call, 2026-09-01 — P-D-71 arm 4): `aggregate_id = tenant_id`** — the tenant's
    producer set is a per-tenant singleton, so per-`(tenant, aggregate)` ordering serializes set
    changes per tenant. `FreezeParticipantSetChanged` is the same class; its subject question stays
    `06`'s row 47.
    Original text: `infra::events::enqueue`
    requires one and `partition_for` consumes it, and every shipped event passes its `entity_id`. A
    producer set has none. `features/catalog-version.md` §7 rows 27 and 47 raise the body core and
    the `SUBJECT_TYPE` for the same class of subject and **neither reaches the partition key**, so
    it is registered here rather than cited.
    **Blocks**: no DoD — **resolved by P-D-71**; `cpt-cf-bss-products-dod-reference-events` carries the key.
    **Owner**: was the events/audit owner, with this feature; **closed**.

26. ~~**What represents `never-received`?**~~
    **Answered (owner call, 2026-09-01 — P-D-71 arm 5): the absence of the watermark row** —
    registration writes only `products_reference_producer`, the watermark table gaining a row on
    first post; a sentinel timestamp is the poison-value class, and row-absence is what P-D-59's
    deregistration-removes-the-series already reads as.
    Original text: The value is obliged at registration while the verdicts are
    computed from `watermark_at`, the member set and the registered rows, and
    `products_reference_watermark` carries only `watermark_at` and `posted_at`. Absence of a row and
    a sentinel `watermark_at` are both consistent with the text and differ in whether registration
    writes a watermark row at all — which is also row 12's re-registration operand.
    **Blocks**: no DoD — **resolved by P-D-71**; `cpt-cf-bss-products-dod-watermark-tables` carries the rule.
    **Owner**: was this feature, alongside row 12; **closed**.

27. ~~**What raises `reference_watermark_stale`, and what stops it repeating?**~~
    **Answered (owner call, 2026-08-31 — P-D-59): an alerting rule over a gauge, and the second half
    dissolves.** The gear exposes `now − watermark_at` per `(tenant_id, producer)` over the
    **registered** set — an operand `design/07` §4 already stores — and the alarm is the observability
    owner's rule over that gauge, its condition **referencing** this gear's exported freshness
    threshold rather than restating it. **Nothing is raised per call, so there is no fired-state to
    store**: repetition, for-duration and grouping belong to the alerting side, and a polled predicate
    no longer alarms once per call. Deregistration removes the series rather than silencing an alarm,
    which is what `inst-wm-freshness` already promised. The predicate's verdict is unchanged —
    `conservatively_referenced(stale, producer)` stays exactly as `inst-rp-eval` states.
    Original text: The alarm is described
    both as an output of a read and as a property of the registered set, and no verdict is stored, so
    there is nowhere to record that it has already fired — while `04-lifecycle`'s runner polls the
    predicate on a cadence. Read-time emission alarms once per call.
    **Blocks**: no DoD — **resolved by P-D-59**; `cpt-cf-bss-products-dod-reference-predicate` is
    freed, while `dod-reference-events` stays blocked by rows 25 and 28.
    **Owner**: was this feature with the observability owner; **closed** for the named alarm. Row 28's
    two unnamed alarms are not settled here.

28. ~~**Two of this feature's three alarms have no names.**~~
    **Answered (owner call, 2026-09-01 — P-D-71 arm 6): `reference_watermark_future` and
    `reference_breakglass_tripwire`**, on the named alarm's convention — prefix, case and the
    alignment with the refusal code where one exists.
    Original text: `reference_watermark_stale` is named; the
    future-watermark alert and the tripwire escalation are described. An alarm name is a consumer
    contract the way an event name is, and this document names every event and every code.
    **Blocks**: no DoD — **resolved by P-D-71**; `cpt-cf-bss-products-dod-reference-events` and `dod-tripwire` carry the names.
    **Owner**: was the observability owner with this feature; **closed**.

29. ~~**Is the break-glass flag per deployment or per tenant, and can it be turned on without a
    restart?**~~
    **Answered (owner call, 2026-09-01 — P-D-71 arm 2): per-deployment, boot-time** — it lives in
    `ProductsConfig`, where `dod-reference-config` already put it, and **the flag is a policy gate,
    not an incident tool**: the emergency surface is `05`'s read elevation, and a runtime or
    per-tenant toggle needs machinery no slice declares. Enabling the arm costs a deploy,
    deliberately.
    Original text: `cpt-cf-bss-products-dod-reference-config` puts it in `ProductsConfig`, which
    `config.rs` calls *"The gear's boot configuration"* and which carries no tenant dimension, while
    C5 describes the arm as *"unavailable until an operator enables it"* — an act in the moment. Row
    1 asks only for the flag's name and polarity.
    **Blocks**: no DoD — **resolved by P-D-71**; `cpt-cf-bss-products-dod-reference-config` carries the posture.
    **Owner**: was this feature with this gear's config owner — the pairing row 19 names; **closed**.

30. ~~**Does ingestion validate the ids in the set?**~~
    **Answered (owner call, 2026-09-01 — P-D-71 arm 7): accepted, counted, alarmed** —
    `reference_unknown_member` per post. An unknown id can be legitimate (a producer's catalog lags
    `10`'s erasure until its next full-set post), so refusal would wedge the producer on this gear's
    lifecycle, and silence would hide a typo that silently frees a real SKU. Erasure leaves member
    rows untouched.
    Original text: The set is authoritative and
    `products_reference_member` is keyed `(tenant_id, producer, sku_id)` with no foreign key and no
    existence rule, and the watermark door's four refusals cover the producer, the timestamp and the
    set — never the members. Whether a `skuId` naming no SKU in this tenant is accepted, refused or
    accepted-and-alarmed is unstated, as is what happens to member rows when
    `10-retention-erasure` erases a SKU. Validating 10K ids per post is the cost the full-set design
    was chosen to avoid.
    **Blocks**: no DoD — **resolved by P-D-71**; `cpt-cf-bss-products-dod-watermark-tables` and `dod-watermark-door` carry the posture.
    **Owner**: was this feature with `fr-reference-signal`'s owner; **closed**.

31. ~~**Does "no config home" mean no `config.rs` field, or no PRD §17.1 policy row?**~~ **Answered (P-D-129, 2026-09-03): a knob's home is `ProductsConfig`; "homeless" means owed a `PRD` §17.1 row** — rows 7 and 19 restate as that. *The item's text stood as:* Row 7 uses the
    second sense — the skew tolerance is *"the one configurable in this slice with no home"* because
    §17.1 has no row for it — and row 19 uses the first, under which the tripwire rate has no home
    either though §17.1 carries its interim. Under one sense three knobs are homeless, under the
    other one is. No document defines the term.
    **Blocks**: no DoD; it decides which of rows 7 and 19 is restated.
    **Owner**: the §17.1 policy owner with this gear's config owner.

32. ~~**May `cpt-cf-bss-products-dod-reference-config` pin "default OFF" while row 1 calls that phrase
    ambiguous?**~~
    **Answered (owner call, 2026-09-01 — P-D-87 arm 1): it may, and the question dissolves** —
    **P-D-71 arm 1** already named the flag enable-positive, so "default OFF" and `false` are one
    fact; the DoD pins what row 1 deferred rather than against it.
    Original text: The DoD states the default in exactly the words row 1 says read both ways, so it
    both pins and defers the same fact. Naming the constant and its polarity settles both at once.
    **Blocks**: no DoD — **resolved by P-D-87**.
    **Owner**: was row 1's owner — this feature; **closed**.

33. ~~**Can a `p2` deliverable carry a `p1` arm's obligation?**~~ **Answered (P-D-125, 2026-09-03): `pN` is a per-id importance**; a `p2` DoD may carry a `p1` arm's obligation. *The item's text stood as:* `cpt-cf-bss-products-dod-tripwire` is
    `p2` while the `p1` arm-(b) DoD obliges *"It **MUST** increment the same `TripwireCounter`"*. The
    design set carries the same split — `inst-bc-unresolvable` is `p1` and `inst-bc-tripwire` `p2` —
    so this document did not introduce it and cannot resolve it: either the counter rises with arm
    (b), or arm (b) ships without its escalation.
    **Blocks**: no DoD; it decides whether one changes priority.
    **Owner**: the slice owner.

34. ~~**The retirement door's not-found refusal has no declared code, so it can carry no audit
    row.**~~ **Answered (P-D-129, 2026-09-03): `PRODUCER_UNREGISTERED` widens to the producer doors** — one slice declares it and raises it at two of its own doors. *The item's text stood as:* `dod-reference-audit` obliges a row for *"every producer registration and retirement …
    **accepted or refused**"*, and `design/01` §4.4 scopes the class to *"every refusal a registry
    door raises, not only the enumerated ones"*. Retiring a producer the tenant does not have is a
    refusal, and §3.3 names no code for it — `PRODUCER_UNREGISTERED` is declared for **the
    watermark door, on an unregistered poster** (§5's code table), so spending it here would be a
    false attribution under 12's one-declaring-slice rule. An audit row's `error_code` is the
    channel a consumer matches, so the refusal ships as a bare 404 with no row. Either the code
    widens to the producer doors or a second one is minted.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-reference-audit`.)*
    **Owner**: this feature with `12-consumer-contracts`. *(Raised by the three-lens review of
    `e939953ee`; two lenses independently.)*

35. ~~**Does an act that emits a broker event still owe an audit row?**~~ **Answered (P-D-129, 2026-09-03): P-D-21's rule stands** — when `ReferenceProducerSetChanged` lands, the registration and retirement audit rows go and `dod-reference-audit` narrows to refusals. *The item's text stood as:* The two rows this surface
    writes for registration and retirement are admissible today only because
    `ReferenceProducerSetChanged` is emitted nowhere in the crate. `design/01` §4.4 under
    **P-D-21** holds the table for *"only acts that emit no event"* and says a committed mutation
    that does emit *"writes no row here; its outbox event is the record"* — while
    `design/07` §3 and `dod-reference-events` require the membership ops to emit
    `ReferenceProducerSetChanged` **and** audit. The day the event lands, one of the two must give,
    and which decides whether these rows are removed.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-reference-audit`, `cpt-cf-bss-products-dod-reference-events`.)*
    **Owner**: `01-foundation`'s owner with this feature. *(Raised by the three-lens review of
    `e939953ee`; two lenses independently.)*

36. ~~**The audit side of the ceremony join has no column.**~~ **Answered (P-D-129, 2026-09-03): a nullable `ceremony_ref` on `products_audit_log`**, in the same in-place migration as P-D-118's `correlation_id`; the lead's build. *The item's text stood as:* `dod-reference-audit` requires a
    break-glass correction's row to *"carry the ceremony reference, the same value
    `products_correction_override` stores, so the ceremony and the evidence are joinable from
    either side"*. `products_audit_log`'s roster carries **no `ceremony_ref`** — its columns are
    `audit_id`, `tenant_id`, `actor_ref`, `action`, `subject_kind`, `subject_id`,
    `subject_revision`, `error_code`, `attempted_key`, `reason`, `correlation_id`, `written_at`,
    `session_id`. So the join is owed on both sides, not just on the corrections door's. Whether
    the value rides a new column or the existing `correlation_id` is the choice.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-reference-audit`.)*
    **Owner**: `01-foundation`'s schema owner with this feature. *(Raised by the three-lens review
    of `e939953ee`.)*

37. ~~**Is the tripwire's window edge inclusive?**~~ **Answered (P-D-129, 2026-09-03): inclusive**, as shipped and probed — `[now − 30 d, now]`. *The item's text stood as:* The shipped count filters `recorded_at >= since`
    and a probe pins that, but nothing normative says which: §5 gives only *"a windowed count over
    this table"* and `design/07` C6 gives the rate as *"> 5 break-glass corrections / 30 days
    (configured)"* with no edge rule. A rolling caller crossing a boundary gets a different answer
    under `>` — at exactly six overrides thirty days apart, one reading escalates and the other
    does not.
    **Blocks**: no DoD — **resolved by P-D-129** *(was: `cpt-cf-bss-products-dod-tripwire`.)*
    **Owner**: the tripwire's §17.1 owner. *(Raised by the three-lens review of `e939953ee`.)*

38. **May a retention collector delete from `products_correction_override`, and which document
    says?** `design/10` `inst-rt-gc` lists *"correction overrides (audit-grade, statutory max)"*
    among the stores whose expiry candidates it computes; this table's guard refuses every
    `DELETE` unconditionally. The chain holds **three shapes for one class**: the audit plane took
    a row-image retention predicate (**P-D-34**), `products_catalog_version_entry` took an interim
    message naming slice 10 as the future admitter, and `products_approval`,
    `products_breakglass_session` and this table took a flat refusal. A collector reaching a
    statutory-max row raises `P0001`, which is not retryable contention, so the sweep aborts and
    takes its other candidates with it.
    **Blocks**: no DoD here; it is `10-retention-erasure`'s to answer for the whole class.
    **Owner**: *(P-D-129, 2026-09-03: **routed to strand D** with a recommendation — the audit plane's row-image predicate, P-D-34, is the one shape for the class.)* `10-retention-erasure`'s owner. *(Raised by the three-lens review of `e939953ee`.)*

### Owed to other documents, recorded and deliberately not edited

Each is a one-line pointer into its owner's register. None was edited here.

- **`features/foundation.md` §1.4** claims `artifacts.toml` excludes design slices from
  autodetection. It is false — `[systems.autodetect.artifacts.DESIGN_SLICE]` carries
  `pattern = "design/*.md"` and `traceability = "FULL"`. It is the shape donor for the five unwritten
  FEATUREs, so the cost compounds.
- **`bucket_tests::buckets_ii_and_iv_have_no_members_today`**'s assertion message names slice 07
  where `domain/bucket.rs` names slice 03 — see row 17. Owner: `01-foundation`'s code.
- **`features/catalog-version.md`'s foreign-seam table** marks `inst-pr-snapshot` as **no** and
  counts two of its five seams unwritten. This document is that FEATURE, so one of the two is now
  written. Owner: that document's owner.
- **DECOMPOSITION §2.7's Purpose and its Scope bullet** both call the predicate *"three-state"*
  where this feature builds four, `no_producers` being the design-introduced fail-safe. Owner: the
  DECOMPOSITION owner; the entity roster is swept with this change, the two *"three-state"*
  sentences are not.
