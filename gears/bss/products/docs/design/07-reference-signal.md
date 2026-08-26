<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./04-lifecycle.md, ./05-governance.md | Owners: BSS Product Catalog team -->

# DESIGN — Reference Signal & Corrections (Slice 7)

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
  - [Ingest a watermark](#ingest-a-watermark)
  - [Evaluate the predicate](#evaluate-the-predicate)
  - [Register / retire a producer](#register--retire-a-producer)
  - [Correct a bucket-ii field (the fresh-zero door)](#correct-a-bucket-ii-field-the-fresh-zero-door)
  - [Break-glass correction (emergency lane)](#break-glass-correction-emergency-lane)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 Storage & freshness mechanics](#31-storage--freshness-mechanics)
  - [3.2 Error taxonomy (slice-owned codes)](#32-error-taxonomy-slice-owned-codes)
- [4. Data / Storage (normative shape; DDL in migrations)](#4-data--storage-normative-shape-ddl-in-migrations)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

This slice owns the registry's only inbound liveness contract and everything that leans on it:
**`SkuReferenceCount` watermark ingestion** (per-producer full-set semantics), the **3-state
predicate** (fresh-zero ⇒ unreferenced; fresh > 0 ⇒ referenced; stale/never-received ⇒
conservatively referenced), **producer registration** (P-D-03: v1 = {pricing}), the
**bucket-ii correction door** (fresh-zero governed re-publish of `type`/meter declaration), the
flag-gated **break-glass correction** path, and the **fail-safe tripwire** that keeps degraded
operation from becoming normal.

### 1.2 Purpose

The registry must never falsely free a referenced SKU (retirement, corrections) and never need
dense zero-publishing at 10K-SKU scale — the watermark OR-predicate buys both. The correction
door exists because bucket-ii mistakes happen (wrong `type`, wrong meter) and the only safe
remedies are a **provably unreferenced** re-publish or an explicitly recorded emergency.

### 1.3 Actors

| Actor | Role in this slice |
|-------|--------------------|
| `cpt-cf-bss-products-actor-plan-price` | The v1 producer (P-D-03): posts watermarks from its live plan→SKU references |
| `cpt-cf-bss-products-actor-subscriptions` / `…-actor-contracts` | Future producers; register at their own build (GA gated on producing) |
| `cpt-cf-bss-products-actor-catalog-admin` | Requests corrections; the break-glass emergency path |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.1 (`fr-reference-signal`, `fr-immutable-field-correction`),
  §6.13 (`fr-reference-producer-registration`, `fr-failsafe-tripwire`); AC #2/#3/#4/#41/#43;
  §17.1 (freshness 15 min interim; tripwire > 5/30 days)
- [`../DECISIONS.md`](../DECISIONS.md) P-D-03 (producer set), P-D-05 (what a correction may
  touch); pricing PRD §15 (the mirrored producer obligation)
- [`./01-foundation.md`](./01-foundation.md) §3.1 (bucket-ii routing), §4.2 (the head-row
  guard that names this slice's door); [`./04-lifecycle.md`](./04-lifecycle.md) (flip guard +
  confirmation count — this slice's consumers); [`./05-governance.md`](./05-governance.md) C5
  (the ceremony-shape-only boundary)

### 1.5 Scope

**In**: the watermark door + storage + freshness; the predicate and its per-producer detail
surface (what 04's confirmation shows); producer registration + its symmetric snapshot ride;
the correction door; the break-glass correction; the tripwire.

**Out**: what producers count (their contract — Contracts' draft/quote question is recorded at
its registration, PRD §15); retirement policy (04); the ceremony machinery (05); erasure of
watermark content (10 — sets carry `skuId`s only, no PII by construction).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | A watermark is a **complete** set per producer: absence of a `skuId` under a fresh watermark ⇒ zero for that producer; `referenced` = boolean OR across registered producers; never summed | PRD `fr-reference-signal` |
| C2 | Freshness threshold: configured, interim 15 min; staler ⇒ conservatively referenced + `stale` alert; never-received ⇒ conservative + a **distinct** flag | PRD §17.1, AC #3 |
| C3 | Only **registered** producers factor in; unregistered silence pins nothing; onboarding never retro-flips history | PRD `fr-reference-producer-registration` |
| C4 | A correction touches bucket-ii fields only (`type`, the meter declaration pair) — never structural identity; it is a governed **`N`-governed** re-publish (version N+1, "two-person" per the PRD glossary shorthand = the tenant's configured quorum), `SkuImmutableFieldCorrected` — with **`quorumReduced` recorded on the `sku_correction` `ApprovalRecord` and on `SkuImmutableFieldCorrected`** whenever the effective count is below the default of 2 (P-D-13; this slice was the one of that decision's four `N`-governed ceremonies the 2026-08-26 sweep missed, because the register named slice 05's `inst-mt-inputs` as the door) | PRD `fr-immutable-field-correction` |
| C5 | The break-glass correction exists only behind a feature flag OFF by default, only while the signal is **entirely unavailable**, with the `N`-governed quorum + mandatory reason + `SkuCorrectionOverride` recording the unavailability — and it is NOT a §6.8 `BreakGlassSession` (05 C5 boundary: ceremony shape only), so it inherits neither `inst-bg-open`'s fixed platform floor nor the ordinary door's disposition: it is **P-D-13's sixth enumerated site**, following `N` with `quorumReduced` recorded, safe at `N = 0` by the flag + reason + evidence + tripwire rather than by a floor | PRD `fr-immutable-field-correction` |
| C6 | Tripwire: > 5 break-glass corrections / 30 days (configured) ⇒ escalation alert + signal delivery reclassified a release blocker | PRD `fr-failsafe-tripwire` |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `WatermarkDoor` | The watermark ingestion contract: `(producer, watermark_at, complete skuId set)`. Like 06's increment request, the contract is the **`products-sdk` client** resolved from `ClientHub` (manifest §3.3.2 — transport-agnostic, in-process default); the S2S endpoint is its out-of-process binding and its authz door |
| `ReferencePredicate` | The 3-state evaluator over all registered producers, with per-producer detail |
| `CorrectionDoor` | The bucket-ii write door the 01 head-row guard names: fresh-zero gate + 05 quorum + re-publish |
| `TripwireCounter` | The rolling 30-day break-glass-correction counter behind C6 |

### 1.8 Context & Dependencies

**Consumed**: producer watermarks (pricing first — the P-D-03 joint build); the 05 gate
(correction quorum; producer-registration ops — enumerated in 05's material inputs (d));
config (freshness, tripwire, the break-glass flag). **Produced**: the predicate 04 consumes
(flip guard, confirmation counts) **plus the exported freshness-threshold config — 04's
`ActivationRunner` re-evaluates deferred flips by polling on that interval (F7: no event
exists by design; watermarks are state, not history)**; `SkuImmutableFieldCorrected`,
`SkuCorrectionOverride`, `ReferenceProducerSetChanged` events; the `stale` / `never-received`
/ tripwire alarms.

## 2. Actor Flows (CDSL)

### Ingest a watermark

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-watermark`

1. [ ] - `p1` - `WatermarkDoor` (`reference_signal × post`; the out-of-process binding is S2S and is accepted only from the producer's own service identity — the in-process binding carries the same identity through `SecurityContext`) receives `(producer, watermark_at, set)`; an unregistered producer is refused `PRODUCER_UNREGISTERED`, audited - `inst-ws-door`
2. [ ] - `p1` - Watermarks are **monotonic per producer** (F3 fix): `watermark_at` **<** the stored one is refused `WATERMARK_REGRESSION` (an out-of-order replay must not roll liveness backwards); an **equal** `watermark_at` with an identical set hash is an idempotent no-op success; equal with a **different** set is refused `WATERMARK_CONFLICT` (the same never-silent rule as 01's idempotency conflicts) - `inst-ws-monotonic`
3. [ ] - `p1` - **Bounded above by the receiving clock (Blocking 3 fix, 2026-08-26 review)**: `watermark_at` **>** `now + skew` (configured tolerance, interim 5 min) is refused **`WATERMARK_FUTURE`** and alerted — monotonicity alone is one-sided, and one future-dated post from a registered producer is unrecoverable: that producer reads permanently fresh so the staleness alarm never fires, every later legitimate post is refused `WATERMARK_REGRESSION` so its member set is frozen, and **every SKU outside that frozen set then reads fresh-zero** — the one state that unlocks retirement flips (04 `inst-rt-flip-guard`) and bucket-ii corrections (`inst-cr-freshzero`). That is precisely the "never falsely free a referenced SKU" this slice exists for, so the bound is `p1`, not hygiene. **`posted_at` is the operand** — already stored (`inst-wm-tables`) and, before this rule, read by nothing - `inst-ws-not-future`
4. [ ] - `p1` - The set replaces the producer's previous set **atomically** (one transaction: member rows swapped + the watermark row advanced); a reader never sees a half-replaced set - `inst-ws-atomic`
5. [ ] - `p1` - Ingestion is audit-plane (explicit **no broker event** — watermarks arrive continuously and are queryable state, not domain history) - `inst-ws-no-event`

### Evaluate the predicate

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-predicate`

1. [ ] - `p1` - `ReferencePredicate(skuId)` over every **registered** producer: any fresh watermark containing the SKU ⇒ `referenced(producer)`; a fresh watermark omitting it ⇒ zero for that producer; a stale watermark ⇒ `conservatively_referenced(stale, producer)` + the `reference_watermark_stale` alarm; never-received ⇒ `conservatively_referenced(never_received, producer)` with the distinct flag (C2); the verdict is the OR, the detail is per-producer (what 04's confirmation screen shows) - `inst-rp-eval`
2. [ ] - `p1` - **Fresh-zero** = every registered producer fresh AND omitting the SKU — the only state that unlocks corrections and retirement flips; with zero registered producers the predicate answers `no_producers` (fail-safe: conservative, distinct from fresh-zero — an empty producer set never frees anything) - `inst-rp-freshzero`

### Register / retire a producer

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-producer-registration`

1. [ ] - `p1` - Membership ops are `GovernedLiveOp`s (`reference_producer × write`; material — 05 input (d) enumerates this slice's kinds); v1 seeds {pricing} per tenant (P-D-03); **retiring the LAST registered producer is refused** (`PRODUCER_SET_EMPTY_FORBIDDEN` — F8 fix: the set changes members but never empties in v1, so the `no_producers` verdict is defensive-unreachable, kept as a fail-safe and flagged as a design-introduced extension of the PRD's 3-state taxonomy); each change emits `ReferenceProducerSetChanged` + audit - `inst-pr-governed`
1a. [ ] - `p1` - **Retirement can only tighten, never free — the converse of `inst-pr-onboarding`** (item 21 of the 2026-08-26 review). Retiring a producer **narrows the AND-quantifier the fresh-zero predicate runs over**, so with two producers, retiring the *stale* one makes the predicate answer fresh-zero over the remaining fresh one — and a correction that `CORRECTION_REFERENCED` had blocked walks through the **normal** door: no feature flag, no `SkuCorrectionOverride`, no `TripwireCounter` increment, none of the break-glass lane's evidence. Only the last-producer case was guarded. So: retiring a producer whose watermark is **stale or never-received** is refused **`PRODUCER_RETIREMENT_WOULD_FREE`** unless the retiring principal supplies the break-glass ceremony's own justification — the retirement is admissible, its *silence* is not. A producer retired while **fresh** frees nothing it was not already reporting zero for, and is unaffected - `inst-pr-retirement`
2. [ ] - `p1` - The producer set is **snapshotted symmetrically with the freeze-participant set** (PRD): it rides the 06 capture store per `CatalogVersion`, and onboarding a producer never retro-flips a historical mutability/retirement decision — past verdicts were computed against the then-registered set and stand - `inst-pr-snapshot`
3. [ ] - `p1` - A registering producer's first watermark starts as `never-received` (conservative) until it posts — onboarding can only tighten, never free - `inst-pr-onboarding`

### Correct a bucket-ii field (the fresh-zero door)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-correction`

1. [ ] - `p1` - `CorrectionDoor` (`sku × correct`) accepts `(skuId, field ∈ {type, meter declaration pair}, new value, expected revision)`; structural identity is never correctable (01 bucket i — the door physically cannot write it). **One door, two admission gates** (F4 fix): the normal fresh-zero gate, or the break-glass admission of `inst-bc-admission` — the 01 head-row whitelist admits bucket-ii writes via this door alone, both lanes included. **Preconditions** (F5/F6 fix): the head must be **clean** (no unpublished bucket-iii/iv edits — `CORRECTION_DIRTY_HEAD`: a correction is surgical, and co-published edits would misattribute the corrected version's content) and the subject must carry **no open approval** (`CORRECTION_APPROVAL_OPEN`); the correction's own `ApprovalRecord` subject kind is `sku_correction` - `inst-cr-door`
2. [ ] - `p1` - The gate: `ReferencePredicate` MUST answer **fresh-zero**; anything else fails `CORRECTION_REFERENCED` naming the per-producer detail (a stale producer is named as the blocker, not hidden inside a boolean) - `inst-cr-freshzero`
3. [ ] - `p1` - The correction is a governed **material** act through the 05 quorum — the tenant's configured `N`, with **`quorumReduced` on the `ApprovalRecord` and on the emitted `SkuImmutableFieldCorrected`** below the default of 2 (P-D-13, the same clause 04 `inst-lc-undeprecate` and 06 `inst-fz-force` carry); on approval it re-publishes the head as version N+1 through the 01 `PublishDoor` (the head-row guard admits the bucket-ii columns only via this door), runs the full pipeline — **the admission gate is itself a registered validator on this publish and re-runs inside the publish transaction** (F1 fix: a reference arriving between submission and approval still refuses; the door-acceptance check is a fast-fail, never the last word). **The validator re-checks the lane's own admission predicate, not always fresh-zero** (item 20 of the 2026-08-26 review): on the normal lane, fresh-zero; on the break-glass lane, `inst-bc-admission`'s predicate — every registered producer still stale/never-received, or the target still unresolvable. Naming fresh-zero for both was a contradiction with only bad readings: taken literally the validator refuses **every** break-glass re-publish, since that lane is admissible only when no producer can answer fresh-zero; skipped instead, nothing re-checks admission at commit and a correction could land after a producer recovered and reported the SKU — with `SkuCorrectionOverride` recording unavailability evidence already false at the instant it was written. Re-checking the lane's own predicate is the only reading that is both satisfiable and fail-closed, a corrected meter re-resolves `usageTypeRef` per P-D-05 — and emits `SkuImmutableFieldCorrected` - `inst-cr-republish`

### Break-glass correction (emergency lane)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-flow-breakglass-correction`

1. [ ] - `p2` - **First admission arm (signal unavailable).** Admissible only when the feature flag is ON (default OFF — `BREAKGLASS_CORRECTION_DISABLED`) **and ≥ 1 producer is registered and every registered producer is stale/never-received** (F8 fix: the quantifier cannot be vacuously true over an empty set, and the unavailability-evidence snapshot is non-empty by construction; a single fresh producer routes to the normal gate: `CORRECTION_SIGNAL_AVAILABLE`); single-SKU only, through the same `CorrectionDoor` (F4) - `inst-bc-admission`
1a. [ ] - `p1` - **Second admission arm: an unresolvable meter declaration** (item 19 of the 2026-08-26 review; authorised by **P-D-16**, which amended `fr-immutable-field-correction` and AC #4 to carry this arm and closed the PRD §15 row it had been silently answering — it stood against two `MUST`s until then). A sold SKU whose `UsageType` the collector deleted is otherwise wedged in every lane at once — fresh > 0 refuses the normal door `CORRECTION_REFERENCED`, the signal *being* available refuses break-glass `CORRECTION_SIGNAL_AVAILABLE`, and retire-and-clone is blocked because the flip guard defers on anything but fresh-zero with no force-retire door in v1 (04 C4). `PRD.md` §15 confirms the collector can delete a referenced usage type, so this is a reachable state, not a hypothetical. So: when the subject's declared `usageTypeRef` **no longer resolves** (03 `UsageTypeResolver` answers not-found, not a timeout), the correction door admits the meter-declaration correction **regardless of the reference predicate** — the reference is real, and that is the reason to repair the declaration rather than the reason to refuse. It keeps the full ceremony (two-person + mandatory reason + `SkuCorrectionOverride` recording *unresolvable-target* rather than unavailability evidence) and increments the same `TripwireCounter`: a broken cross-gear reference must be escalated, never normalized - `inst-bc-unresolvable`
2. [ ] - `p2` - The `N`-governed quorum + mandatory reason (the 05 ceremony shape — explicitly NOT a §6.8 `BreakGlassSession`, which never authorizes writes, and therefore **not** covered by `inst-bg-open`'s fixed platform floor: this lane's principal is the tenant's own, so it follows `N` as P-D-13's sixth site, with **`quorumReduced` on the record and on both emitted events** below the default of 2); the re-publish emits `SkuCorrectionOverride` recording the per-producer unavailability evidence alongside `SkuImmutableFieldCorrected` - `inst-bc-ceremony`
3. [ ] - `p2` - Every break-glass correction increments the `TripwireCounter`; past the configured rate (> 5/30 days) the tripwire raises the escalation alarm and flips the standing `signal_delivery_release_blocker` status surface — degraded operation is escalated, never normalized (C6) - `inst-bc-tripwire`

## 3. Processes / Business Logic

### 3.1 Storage & freshness mechanics

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-watermark-storage`

1. [ ] - `p1` - `products_reference_watermark` — `(tenant_id, producer)` → `watermark_at`, `posted_at`; `products_reference_member` — `(tenant_id, producer, sku_id)`, replaced as a set per ingestion (the atomic swap of `inst-ws-atomic`); membership lookup is an index hit, the predicate is O(producers) - `inst-wm-tables`
2. [ ] - `p1` - Freshness is evaluated at read time against `watermark_at` (never `posted_at` — the producer's claim instant is the semantic one); the staleness alarm keys on the registered set so a retired producer stops alarming. **`posted_at` has exactly one reader**: the ingestion-time future bound of `inst-ws-not-future`, which is why it is stored - `inst-wm-freshness`

### 3.2 Error taxonomy (slice-owned codes)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-reference-errors`

`PRODUCER_UNREGISTERED`, `WATERMARK_REGRESSION`, `WATERMARK_CONFLICT`,
**`WATERMARK_FUTURE`** (`inst-ws-not-future`), **`PRODUCER_RETIREMENT_WOULD_FREE`**
(`inst-pr-retirement`),
`CORRECTION_REFERENCED`, `CORRECTION_DIRTY_HEAD`, `CORRECTION_APPROVAL_OPEN`,
`CORRECTION_SIGNAL_AVAILABLE` (break-glass refused because the normal door is open),
`BREAKGLASS_CORRECTION_DISABLED`, `PRODUCER_SET_EMPTY_FORBIDDEN`. The correction door's quorum refusals ride the 05 gate's
codes; structural-identity attempts ride 01's `ILLEGAL_FIELD_MUTATION`.

## 4. Data / Storage (normative shape; DDL in migrations)

§3.1's two tables; **`products_reference_producer`** (F2 fix) — `(tenant_id, producer)` →
`state ∈ {registered, retired}`, `registered_at`, ceremony ref, and the **declaration payload**
(the reserved field where Contracts' draft/quote-counting answer lands at its registration —
PRD §15); seeded per tenant with {pricing} at bootstrap (P-D-03); the operand of `inst-rp-eval`
and the source of the 06 capture-store ride. `products_correction_override` — the break-glass
evidence rows (SKU, field, per-producer unavailability snapshot, ceremony ref, instant) feeding
the `TripwireCounter` (a windowed count over this table — no separate counter state to drift);
events per §1.8. All tenant-scoped, append-only where evidential.

## 5. Testing posture (slice-local)

- Predicate matrix as ONE fixture: fresh-zero / fresh > 0 / stale / never-received /
  unregistered-silence / zero-producers — six verdicts, each with the per-producer detail
  asserted (the fixture-grants lesson: every conservative refusal paired with the fresh-zero
  positive control).
- Watermark atomicity under concurrent read (no half-set observation); regression refusal +
  idempotent replay.
- Correction RED first: referenced SKU refused naming the blocking producer; then fresh-zero
  green through the full quorum + re-publish + `usageTypeRef` re-resolution.
- Break-glass admission: one fresh producer ⇒ `CORRECTION_SIGNAL_AVAILABLE`; flag OFF ⇒
  disabled; tripwire trips on the 6th within the window and the status surface flips.
- Onboarding probe: registering a producer flips a previously fresh-zero SKU to conservative
  (never-received) — tighten-only, and no historical decision re-opens.

## 6. Traces to / Risks & Open items

**Traces to**: **§9.2 by id** — `cpt-cf-bss-products-contract-sku-reference-count` (the inbound signal this slice's `WatermarkDoor` terminates; claimed by id here for the first time, 2026-08-26 branch review). `cpt-cf-bss-products-fr-reference-signal`, `cpt-cf-bss-products-fr-immutable-field-correction`,
`cpt-cf-bss-products-fr-reference-producer-registration`, `cpt-cf-bss-products-fr-failsafe-tripwire`; AC #2 (predicate half), #3, #4,
#41, #43; P-D-03, P-D-05 (correction re-resolution), P-D-13 (the quorum reach reaches both correction lanes), P-D-16 (the unresolvable-target admission arm).

**Risks & open items**:
- **The pricing watermark is a joint build** (P-D-03): the producer-side query ("complete live
  plan→SKU reference set") and its cadence are pricing's to design; this slice's door and the
  §15 mirror are ready — the joint fixture belongs to slice 12's seam suite.
- **Watermark set size**: full-set replacement at 10K SKUs × cadence is fine as rows, but the
  door should accept a compressed set representation from day one (wire-level; no semantic
  change) — implementation note.
- **Contracts' draft/quote question** (PRD §15) is answered at its registration, not before;
  the registration op is where that declaration is recorded — named here so the op's payload
  reserves the field.
