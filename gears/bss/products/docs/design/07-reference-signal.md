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
**`SkuReferenceCount` watermark ingestion** (per-producer full-set semantics), the **4-state
predicate** (the PRD's three plus the `no_producers` fail-safe of `inst-rp-freshzero`) (fresh-zero ⇒ unreferenced; fresh > 0 ⇒ referenced; stale/never-received ⇒
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
- [`../DECISIONS.md`](../DECISIONS.md) P-D-03 (producer set), P-D-05 (correction re-resolution), P-D-11 (`N` floor 0),
  P-D-13 (the quorum shorthand's fifth and sixth sites), P-D-16 (the unresolvable-target arm),
  P-D-32 (`ILLEGAL_FIELD_MUTATION`), P-D-41 (the publish door's third argument); pricing PRD §15 (the mirrored producer obligation)
- [`./01-foundation.md`](./01-foundation.md) §3.1 (bucket-ii routing), §4.2 (the head-row
  guard that names this slice's door); [`./04-lifecycle.md`](./04-lifecycle.md) (flip guard +
  confirmation count — this slice's consumers); [`./05-governance.md`](./05-governance.md) C5
  (the ceremony-shape-only boundary)

### 1.5 Scope

**In**:
- the watermark door + storage + freshness
- the predicate and its per-producer detail surface (what 04's confirmation shows)
- producer registration + its symmetric snapshot ride
- the correction door
- the break-glass correction
- the tripwire.

**Out**:
- what producers count (their contract — Contracts' draft/quote question is recorded at its registration, PRD §15)
- retirement policy (04)
- the ceremony machinery (05)
- erasure of watermark content (10 — sets carry `skuId`s only, no PII by construction).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | A watermark is a **complete** set per producer: absence of a `skuId` under a fresh watermark ⇒ zero for that producer; `referenced` = boolean OR across registered producers; never summed | PRD `fr-reference-signal` |
| C2 | Freshness threshold: configured, interim 15 min; staler ⇒ conservatively referenced + `stale` alert; never-received ⇒ conservative + a **distinct** flag | PRD §17.1, AC #3 |
| C3 | Only **registered** producers factor in; unregistered silence pins nothing; onboarding never retro-flips history | PRD `fr-reference-producer-registration` |
| C4 | A correction touches bucket-ii fields only (`type`, the meter declaration pair) — never structural identity; it is a governed **`N`-governed** re-publish (version N+1, "two-person" per the PRD glossary shorthand = the tenant's configured quorum), `SkuImmutableFieldCorrected` — with **`quorumReduced` recorded on the `sku_correction` `ApprovalRecord` and on `SkuImmutableFieldCorrected`** whenever the effective count is below the default of 2 (P-D-13; this slice was the one of that decision's four `N`-governed ceremonies the sweep missed, because the register named slice 05's `inst-mt-inputs` as the door) | PRD `fr-immutable-field-correction` |
| C5 | The correction door admits on one of **three** gates: the ordinary fresh-zero gate, and two exceptional ones — (a) the break-glass arm, **behind a feature flag that is OFF by default, meaning the arm is unavailable until an operator enables it** (the flag is **`breakglass_correction_enabled: bool`, default `false`** — **P-D-71** named it enable-positive, per-deployment and boot-time in `ProductsConfig`, a policy gate rather than an incident tool, the emergency surface being 05's read elevation; the refusal code stays `BREAKGLASS_CORRECTION_DISABLED`, the 403 raised while it is `false`), while the signal is **entirely unavailable**; and (b) P-D-16's unresolvable-target arm, **not** behind that flag (a default-OFF flag would withhold the only exit that decision exists to provide, and **P-D-48** settled that the arm carries no flag of its own either: its admission predicate is a resolver fact, not operator discretion, and the tripwire already counts it; the residual open write path is recorded on P-D-16), while the subject's declared `usageTypeRef` no longer resolves, **regardless of the reference predicate** (P-D-16, swept into this row: the arm was added to `inst-bc-admission` and this constraint kept the single-arm "only", which is the version an implementer reading the constraints table would have built), with the `N`-governed quorum + mandatory reason + `SkuCorrectionOverride` recording the unavailability on arm (a) and *unresolvable-target* on arm (b), per `inst-bc-unresolvable` and P-D-16 — and it is NOT a §6.8 `BreakGlassSession` (05 C5 boundary: ceremony shape only), so it does **not** inherit `inst-bg-open`'s fixed platform floor. It keeps the ordinary correction door's ceremony — `inst-cr-republish`, **P-D-13's fifth enumerated site** (the sixth is `inst-bc-ceremony`, which P-D-13 names; this cell had said "sixth", and P-D-13's propagation list never mentions `inst-bc-unresolvable`), following `N` with `quorumReduced` recorded, safe at `N = 0` by reason + evidence + tripwire rather than by a floor — plus, on arm (a) only, the flag | PRD `fr-immutable-field-correction` |
| C6 | Tripwire: > 5 break-glass corrections / 30 days (configured) ⇒ escalation alert + signal delivery reclassified a release blocker | PRD `fr-failsafe-tripwire` |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `WatermarkDoor` | The watermark ingestion contract: `(producer, watermark_at, complete skuId set)`. Like 06's increment request, the contract is the **`products-sdk` client** resolved from `ClientHub` (manifest §3.3.2 — transport-agnostic, in-process default); the S2S endpoint is its out-of-process binding and its authz door |
| `ReferencePredicate` | The 4-state evaluator over all registered producers (`no_producers` included), with per-producer detail |
| `CorrectionDoor` | The **after-first-publish** bucket-ii write door the 01 head-row guard names (P-D-41's second door; the first is 01 `inst-fd-save-txn`): fresh-zero gate + 05 quorum + re-publish |
| `TripwireCounter` | The rolling 30-day break-glass-correction counter behind C6 |

### 1.8 Context & Dependencies

**Consumed**: producer watermarks (pricing first — the P-D-03 joint build); the 05 gate
(correction quorum; producer-registration ops — enumerated in 05's material inputs (d));
config (freshness, the ingestion clock-skew tolerance, tripwire, the break-glass flag). **Produced**: the predicate 04 consumes
(flip guard, confirmation counts) **plus the exported freshness-threshold config — 04's
`ActivationRunner` re-evaluates deferred flips by polling on that interval (F7: no event
exists by design; watermarks are state, not history)**; `SkuImmutableFieldCorrected`,
`SkuCorrectionOverride`, `ReferenceProducerSetChanged` events; the `reference_watermark_stale`,
**`reference_watermark_future`** and **`reference_breakglass_tripwire`** alarms plus **`reference_unknown_member`** (P-D-71 named all three on `reference_watermark_stale`'s convention; never-received is a verdict flag, not an alarm — C2); the
`signal_delivery_release_blocker` status surface.

## 2. Actor Flows (CDSL)

### Ingest a watermark

Declared by [`../features/reference-signal.md`](../features/reference-signal.md) §2 as `cpt-cf-bss-products-flow-watermark`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `WatermarkDoor` (`reference_signal × post`; the out-of-process binding is S2S and is accepted only from the producer's own service identity — the in-process binding carries the same identity through `SecurityContext`) receives `(producer, watermark_at, set)`; an unregistered producer is refused `PRODUCER_UNREGISTERED`, audited - `inst-ws-door`
2. [ ] - `p1` - Watermarks are **monotonic per producer** (F3 fix): `watermark_at` **<** the stored one is refused `WATERMARK_REGRESSION` (an out-of-order replay must not roll liveness backwards); an **equal** `watermark_at` with an identical set hash is an idempotent no-op success; equal with a **different** set is refused `WATERMARK_CONFLICT` (the same never-silent rule as 01's idempotency conflicts) - `inst-ws-monotonic`
3. [ ] - `p1` - **Bounded above by the receiving clock (Blocking 3 fix)**: `watermark_at` **>** `now + skew` (configured tolerance, interim 5 min) is refused **`WATERMARK_FUTURE`** and alerted — monotonicity alone is one-sided, and one future-dated post from a registered producer is unrecoverable: that producer reads permanently fresh so the staleness alarm never fires, every later legitimate post is refused `WATERMARK_REGRESSION` so its member set is frozen, and **every SKU outside that frozen set then reads fresh-zero** — the one state that unlocks retirement flips (04 `inst-rt-flip-guard`) and the **ordinary** bucket-ii correction gate (`inst-cr-freshzero`). That is precisely the "never falsely free a referenced SKU" this slice exists for, so the bound is `p1`, not hygiene. **the receiving clock is the operand, and it is stamped into `posted_at`** — already stored (`inst-wm-tables`) and, before this rule, read by nothing - `inst-ws-not-future`
4. [ ] - `p1` - The set replaces the producer's previous set **atomically** (one transaction: member rows swapped + the watermark row advanced); a reader never sees a half-replaced set - `inst-ws-atomic`
5. [ ] - `p1` - Ingestion is audit-plane (explicit **no broker event** — watermarks arrive continuously and are queryable state, not domain history) - `inst-ws-no-event`
6. [ ] - `p1` - **Member ids are accepted unvalidated, counted and alarmed** (**P-D-71**): the set is the producer's authoritative claim, and an unknown `sku_id` can be legitimate — a producer's catalog lags `10`'s erasure until its next full-set post replaces the set — so refusing a 10K post for one such id would wedge the producer on this gear's lifecycle, while silence would hide a typo that silently frees a real SKU. Unknown ids are counted per post and raise **`reference_unknown_member`** (the fourth alarm, on the stale alarm's convention); erasure leaves member rows untouched; **no event** - `inst-ws-members`

### Evaluate the predicate

Declared by [`../features/reference-signal.md`](../features/reference-signal.md) §2 as `cpt-cf-bss-products-flow-predicate`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `ReferencePredicate(skuId)` over every **registered** producer: any fresh watermark containing the SKU ⇒ `referenced(producer)`; a fresh watermark omitting it ⇒ zero for that producer; a stale watermark ⇒ `conservatively_referenced(stale, producer)`, the condition the `reference_watermark_stale` alarm fires on — **the evaluation emits nothing** (**P-D-59**: the alarm is an alerting rule over the per-producer watermark-age gauge, so a polled predicate does not alarm once per call and no fired-state is stored); never-received ⇒ `conservatively_referenced(never_received, producer)` with the distinct flag (C2); the verdict is the OR, the detail is per-producer (what 04's confirmation screen shows) - `inst-rp-eval`
2. [ ] - `p1` - **Fresh-zero** = every registered producer fresh AND omitting the SKU — the only state that unlocks retirement flips and the correction door's **ordinary** gate (C5's two exceptional gates admit without it); with zero registered producers the predicate answers `no_producers` (fail-safe: conservative, distinct from fresh-zero — an empty producer set never frees anything) - `inst-rp-freshzero`

### Register / retire a producer

Declared by [`../features/reference-signal.md`](../features/reference-signal.md) §2 as `cpt-cf-bss-products-flow-producer-registration`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Membership ops are `GovernedLiveOp`s (`reference_producer × write`; material — 05 input (d) enumerates this slice's kinds); v1 seeds {pricing} per tenant (P-D-03); **retiring the LAST registered producer is refused** (`PRODUCER_SET_EMPTY_FORBIDDEN` — F8 fix: the set changes members but never empties in v1, so the `no_producers` verdict is defensive-unreachable, kept as a fail-safe and flagged as a design-introduced extension of the PRD's 3-state taxonomy); each change emits `ReferenceProducerSetChanged` — **its aggregate is the tenant's producer set itself, `aggregate_id = tenant_id`** (**P-D-71**: a per-tenant singleton, so per-`(tenant, aggregate)` ordering serializes set changes per tenant) + audit - `inst-pr-governed`
2. [ ] - `p1` - **Retirement of a silent producer is refused** (item 21 of the review; the row's earlier title, "can only tighten, never free", promised more than the body delivers — **a *fresh* producer's retirement can still free a SKU, and that case is registered in §6's open items, not disposed of here**). Retiring a producer **narrows the AND-quantifier the fresh-zero predicate runs over**, so with two producers, retiring the *stale* one makes the predicate answer fresh-zero over the remaining fresh one — and a correction that `CORRECTION_REFERENCED` had blocked walks through the **normal** door: no feature flag, no `SkuCorrectionOverride`, no `TripwireCounter` increment. Only the last-producer case was guarded. So: retiring a producer whose watermark is **stale or never-received** is refused **`PRODUCER_RETIREMENT_WOULD_FREE`** unless the retiring principal supplies the break-glass ceremony's own justification — the retirement is admissible, its *silence* is not. **The exception's lane is unsettled too**: the break-glass lane in this slice is the single-SKU correction door (`inst-cr-door`, `sku × correct`), which is not a retirement door — sub-question 4 of §6's open item, which the stale case needs answered as much as the fresh one; the reason passes 02's `inst-av-pii-block` before the row is written, a hit failing `CONTENT_PII_BLOCKED` (**P-D-50**; 02 `inst-av-pii-reason` enumerates this door) - `inst-pr-retirement`
3. [ ] - `p1` - The producer set is **snapshotted symmetrically with the freeze-participant set** (PRD): it rides the 06 capture store per `CatalogVersion`, and onboarding a producer never retro-flips a historical mutability/retirement decision — past verdicts were computed against the then-registered set and stand - `inst-pr-snapshot`
4. [ ] - `p1` - A registering producer's first watermark starts as `never-received` (conservative) until it posts — onboarding can only tighten, never free - `inst-pr-onboarding`

### Correct a bucket-ii field (the fresh-zero door)

Declared by [`../features/reference-signal.md`](../features/reference-signal.md) §2 as `cpt-cf-bss-products-flow-correction`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - `CorrectionDoor` (`sku × correct`) accepts `(skuId, field ∈ {type, meter declaration pair}, new value, expected revision)`; structural identity is never correctable (01 bucket i — the door physically cannot write it). **One door, three admission gates** (F4 fix; the third added — P-D-16's arm is a gate of this door and not only of the break-glass block, and while this row said "two" a SKU with a deleted `UsageType` and `fresh > 0` was refused here and never reached the validator that admits it): the normal fresh-zero gate, the unresolvable-target gate of `inst-bc-unresolvable`, or the break-glass admission of `inst-bc-admission` — the 01 head-row whitelist admits bucket-ii writes **after first publish** via this door alone, both lanes included (below first publish the admitting door is 01 `inst-fd-save-txn` — **P-D-41**). **Preconditions** (F5/F6 fix): the head must be **clean** (no unpublished bucket-iii/iv edits — `CORRECTION_DIRTY_HEAD`: a correction is surgical, and co-published edits would misattribute the corrected version's content) and the subject must carry **no open approval** (`CORRECTION_APPROVAL_OPEN`); the correction's own `ApprovalRecord` subject kind is `sku_correction`; the reason passes 02's `inst-av-pii-block` before the row is written, a hit failing `CONTENT_PII_BLOCKED` (**P-D-50**; 02 `inst-av-pii-reason` enumerates this door) - `inst-cr-door`
2. [ ] - `p1` - The **normal lane's** gate (the first of `inst-cr-door`'s three): `ReferencePredicate` MUST answer **fresh-zero**; anything else fails `CORRECTION_REFERENCED` unless `inst-bc-admission` or `inst-bc-unresolvable` admits naming the per-producer detail (a stale producer is named as the blocker, not hidden inside a boolean) - `inst-cr-freshzero`
3. [ ] - `p1` - The correction is a governed **material** act through the 05 quorum — the tenant's configured `N`, with **`quorumReduced` on the `ApprovalRecord` and on the emitted `SkuImmutableFieldCorrected`** below the default of 2 (P-D-13, the same clause 04 `inst-lc-undeprecate` and 06 `inst-fz-force` carry); on approval it re-publishes the head as version N+1 through the 01 `PublishDoor`, **passing the field and new value as that door's optional third argument** (01 **P-D-41**) so the correction and the `published_version` bump are one statement, which is the only form 01 §4.2 admits (after first publish the head-row guard admits the bucket-ii columns only via this door — **P-D-41**), runs the full pipeline — **the admission gate is itself a registered validator on this publish and re-runs inside the publish transaction** (F1 fix: a reference arriving between submission and approval still refuses; the door-acceptance check is a fast-fail, never the last word). **The validator re-checks the lane's own admission predicate, not always fresh-zero** (item 20 of the review): on the normal lane, fresh-zero; on the `inst-bc-admission` arm, its own predicate (every registered producer still stale/never-received); on the `inst-bc-unresolvable` arm, its own (the target still unresolvable). Naming fresh-zero for both was a contradiction with only bad readings: taken literally the validator refuses **every** break-glass re-publish, since that lane is admissible only when no producer can answer fresh-zero; skipped instead, nothing re-checks admission at commit and a correction could land after a producer recovered and reported the SKU — with `SkuCorrectionOverride` recording unavailability evidence already false at the instant it was written. Re-checking the lane's own predicate is the only reading that is both satisfiable and fail-closed, a corrected meter re-resolves `usageTypeRef` per P-D-05 — and emits `SkuImmutableFieldCorrected` - `inst-cr-republish`

### Break-glass correction (emergency lane)

Declared by [`../features/reference-signal.md`](../features/reference-signal.md) §2 as `cpt-cf-bss-products-flow-breakglass-correction`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p2` - **First admission arm (signal unavailable).** Admissible only when **`breakglass_correction_enabled` is `true`** (default `false` — P-D-71; the refusal is `BREAKGLASS_CORRECTION_DISABLED`) **and ≥ 1 producer is registered and every registered producer is stale/never-received** (F8 fix: the quantifier cannot be vacuously true over an empty set, and the unavailability-evidence snapshot is non-empty by construction; a single fresh producer routes to the normal gate: `CORRECTION_SIGNAL_AVAILABLE`, unless `inst-bc-unresolvable`'s arm admits); single-SKU only, through the same `CorrectionDoor` (F4); the flag governs **this arm only** — the second arm below is not behind it (**P-D-16**) - `inst-bc-admission`
2. [ ] - `p1` - **Second admission arm: an unresolvable meter declaration** (item 19 of the review; authorised by **P-D-16**, which amended `fr-immutable-field-correction` and AC #4 to carry this arm and closed the **registry-side half** of the PRD §15 row it had been silently answering (the cross-gear deletion-guard negotiation stays open — P-D-16, P-D-05's residue) — it stood against two `MUST`s until then). A sold SKU whose `UsageType` the collector deleted is otherwise wedged in every lane at once — fresh > 0 refuses the normal door `CORRECTION_REFERENCED`, the signal *being* available refuses break-glass `CORRECTION_SIGNAL_AVAILABLE`, and retire-and-clone is blocked because the flip guard defers on anything but fresh-zero with no force-retire door in v1 (04 C4). `PRD.md` §15 confirms the collector can delete a referenced usage type, so this is a reachable state, not a hypothetical. So: when the subject's declared `usageTypeRef` **no longer resolves** (03 `UsageTypeResolver` answers not-found, not a timeout), the correction door admits the meter-declaration correction **regardless of the reference predicate** — the reference is real, and that is the reason to repair the declaration rather than the reason to refuse. It keeps the full ceremony (`N`-governed with `quorumReduced` recorded — the ceremony of the correction door's step 3 above, unchanged; corrected, this arm had kept a bare "two-person" after P-D-11/P-D-13 retired the fixed count everywhere else in the slice, which would have re-blocked the `N = 0` tenant on the one door that can repair a deleted `UsageType` — plus mandatory reason + `SkuCorrectionOverride` recording *unresolvable-target* rather than unavailability evidence) and increments the same `TripwireCounter`: a broken cross-gear reference must be escalated, never normalized - `inst-bc-unresolvable`
3. [ ] - `p2` - The `N`-governed quorum + mandatory reason (the 05 ceremony shape — explicitly NOT a §6.8 `BreakGlassSession`, which never authorizes writes, and therefore **not** covered by `inst-bg-open`'s fixed platform floor: this lane's principal is the tenant's own, so it follows `N` as P-D-13's sixth site, with **`quorumReduced` on the record and on both emitted events** below the default of 2); the re-publish emits `SkuCorrectionOverride` recording **the admitting arm's evidence** per §4 (per-producer unavailability on arm (a), `unresolvable-target` on arm (b)) alongside `SkuImmutableFieldCorrected`; the reason passes 02's `inst-av-pii-block` before the row is written, a hit failing `CONTENT_PII_BLOCKED` (**P-D-50**; 02 `inst-av-pii-reason` enumerates this door) - `inst-bc-ceremony`
4. [ ] - `p2` - Every break-glass correction increments the `TripwireCounter`; past the configured rate (> 5/30 days) the tripwire raises the escalation alarm and flips the standing `signal_delivery_release_blocker` status surface — degraded operation is escalated, never normalized (C6) - `inst-bc-tripwire`

## 3. Processes / Business Logic

### 3.1 Storage & freshness mechanics

Declared by [`../features/reference-signal.md`](../features/reference-signal.md) §3 as `cpt-cf-bss-products-algo-watermark-storage`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
Input, the Output and the boundary.

1. [ ] - `p1` - `products_reference_watermark` — `(tenant_id, producer)` → `watermark_at`, `posted_at`, **`set_hash`** (**P-D-71**: `SHA-256` over the member `sku_id`s sorted bytewise, **stored at ingestion** — recomputing from 10K member rows at every idempotence comparison is the declined arm); **a registered producer that has never posted has no row here** — `never-received` is the absence of the row, registration writing only `products_reference_producer` (P-D-71); `products_reference_member` — `(tenant_id, producer, sku_id)`, replaced as a set per ingestion (the atomic swap of `inst-ws-atomic`); membership lookup is an index hit, the predicate is O(producers) - `inst-wm-tables`
2. [ ] - `p1` - Freshness is evaluated at read time against `watermark_at` (never `posted_at` — the producer's claim instant is the semantic one); the staleness alarm keys on the registered set so a retired producer stops alarming — **as a gauge of `now − watermark_at` per `(tenant_id, producer)` over that set** (**P-D-59**), so deregistration removes the series rather than silencing an alarm, and the alerting rule's condition **references** this gear's exported freshness threshold rather than restating it. **`posted_at` is written from the receiving clock the future bound of `inst-ws-not-future` was evaluated against**, and is read by nothing - `inst-wm-freshness`

### 3.2 Error taxonomy (slice-owned codes)

Declared by [`../features/reference-signal.md`](../features/reference-signal.md) §3 as `cpt-cf-bss-products-algo-reference-errors`.
The roster below is this slice's and is the normative one; the FEATURE carries the obligation and the boundary.

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-reference-errors`

`PRODUCER_UNREGISTERED`, `WATERMARK_REGRESSION`, `WATERMARK_CONFLICT`,
**`WATERMARK_FUTURE`** (`inst-ws-not-future`), **`PRODUCER_RETIREMENT_WOULD_FREE`**
(`inst-pr-retirement`),
`CORRECTION_REFERENCED`, `CORRECTION_DIRTY_HEAD`, `CORRECTION_APPROVAL_OPEN`,
`CORRECTION_SIGNAL_AVAILABLE` (break-glass refused because the normal door is open),
`BREAKGLASS_CORRECTION_DISABLED`, `PRODUCER_SET_EMPTY_FORBIDDEN`. The correction door's quorum refusals ride the 05 gate's
codes; a stale `expected revision` rides 01's `STALE_REVISION`; structural-identity attempts ride 01's
`ILLEGAL_FIELD_MUTATION`.

**Problem responses (RFC 9457):** `PRODUCER_UNREGISTERED`, `BREAKGLASS_CORRECTION_DISABLED` (403); `PRODUCER_SET_EMPTY_FORBIDDEN`, `WATERMARK_REGRESSION`, `WATERMARK_CONFLICT`, `PRODUCER_RETIREMENT_WOULD_FREE`, `CORRECTION_REFERENCED`, `CORRECTION_DIRTY_HEAD`, `CORRECTION_APPROVAL_OPEN`, `CORRECTION_SIGNAL_AVAILABLE` (409); `ILLEGAL_FIELD_MUTATION` (409 — moved from 422 by **P-D-32**, with its declaration in 01); `CONTENT_PII_BLOCKED` (422 architectural, declared by 02 — **P-D-50**);
`WATERMARK_FUTURE` (422 architectural — each reaches the wire as 400; see the note below).

*Statuses added, corrected the same day by the fix-wave review. The gear declared
its codes with no HTTP status and no problem-response block in any slice, against
`guidelines/DNA/README.md`'s RFC 9457 rule and `.cf-studio/config/rules/api-contracts.md`. The
mapping follows pricing's, checked against it code by code: **422** for content the door cannot
process, **409** where the current state refuses the act — including the ETag precondition,
which pricing maps to 409 rather than 412 (**D-141**, whose own decision text reads
*"A mismatch is `STALE_VERSION` (409, Foundation-owned)"*) — **403** where the caller may not
perform the act at all, **404** only where a path segment names a resource this tenant has none
of. **503** where retry is the remedy is this gear's own addition — pricing's set carries no 503
at all, so that one
class is not "checked against it". **The 422s here are architectural, not wire** — see 01 §3.3, which quotes the sibling
plan-price gear's rule (the `MUST NOT` being this gear's own choice, 01 §3.3): no `CanonicalError` category renders 422, so each reaches the wire as a 400
carrying its code, and no endpoint may declare a 422 for an error **carrying a registry code** in `OpenAPI` (the framework layer is the exception — a `Json<T>` schema violation, which carries no registry code). Proposed per
row and open to correction; the requirement is that every code carries one.
  Codes listed here for the response map but **declared elsewhere**: `ILLEGAL_FIELD_MUTATION` (slice 01) and `CONTENT_PII_BLOCKED` (slice 02, **P-D-50**) — the status is repeated, not a second declaration, so the one-declaration rule stands.*

## 4. Data / Storage (normative shape; DDL in migrations)

§3.1's two tables; **`products_reference_producer`** (F2 fix) — `(tenant_id, producer)` →
`state ∈ {registered, retired}`, `registered_at`, ceremony ref, and the **declaration payload**
(the reserved field where Contracts' draft/quote-counting answer lands at its registration —
PRD §15); seeded per tenant with {pricing} at bootstrap (P-D-03); the operand of `inst-rp-eval`
and the source of the 06 capture-store ride. `products_correction_override` — the break-glass
evidence rows (SKU, field, **reason** (mandatory, the ceremony's), **the arm's evidence** — a per-producer unavailability snapshot on arm (a),
`unresolvable-target` on arm (b) per P-D-16 and `inst-bc-unresolvable` — ceremony ref, instant) feeding
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
- Future bound: a post with `watermark_at > now + skew` is refused `WATERMARK_FUTURE`, the stored
  watermark unmoved; an equal `watermark_at` with a different set is refused `WATERMARK_CONFLICT`.
- Unresolvable-target arm: a `fresh > 0` SKU whose `usageTypeRef` no longer resolves is admitted for
  a meter-declaration correction, records `unresolvable-target`, and increments the `TripwireCounter`.
- Retiring a stale producer ⇒ `PRODUCER_RETIREMENT_WOULD_FREE`, with the fresh-producer control.
- Break-glass admission: one fresh producer ⇒ `CORRECTION_SIGNAL_AVAILABLE`; flag OFF ⇒
  disabled; tripwire trips on the 6th within the window and the status surface flips.
- Onboarding probe: registering a producer flips a previously fresh-zero SKU to conservative
  (never-received) — tighten-only, and no historical decision re-opens.

## 6. Traces to / Risks & Open items

**Traces to**: **§9.2 by id** — `cpt-cf-bss-products-contract-sku-reference-count` (the inbound signal this slice's `WatermarkDoor` terminates; claimed by id here for the first time). `cpt-cf-bss-products-fr-reference-signal`, `cpt-cf-bss-products-fr-immutable-field-correction`,
`cpt-cf-bss-products-fr-reference-producer-registration`, `cpt-cf-bss-products-fr-failsafe-tripwire`; AC #2 (predicate half), #3, #4,
#41, #43; P-D-03, P-D-05 (correction re-resolution), P-D-13 (the quorum reach reaches both correction lanes), P-D-16 (the unresolvable-target admission arm).

**Risks & open items**:

- ~~**OPEN — the break-glass arm's feature flag has no name of its own.**~~
  **Answered (owner call, 2026-09-01 — P-D-71): `breakglass_correction_enabled: bool`, default `false`** — enable-positive, per-deployment, boot-time in `ProductsConfig`; a policy gate, not an incident tool, the emergency surface being 05's read elevation. The code stays the 403. Original text: It is referred
  to by the refusal code `BREAKGLASS_CORRECTION_DISABLED`, so "the flag is OFF" and "the arm is
  disabled" are the same words for opposite polarities, and §5's probe and C5 have read it both
  ways. A flag needs a name and a stated polarity; the code stays the 403.

- ~~**OPEN (third full-review pass) — a fresh producer's retirement can still free a SKU,
  and three attempts to write the rule have each introduced a contradiction. Registered rather than
  drafted a fourth time.**~~ **Answered (P-D-129, 2026-09-03): a producer retires only with an empty current watermark**; a dead one under break-glass at the retirement door, with `producer_unavailable` override rows as evidence. *The item's text stood as:* `inst-pr-retirement` guards the case its own sentence names: a **stale or
  never-received** producer, whose *silence* is what pins the set. It does **not** dispose of a
  **fresh** producer that is the only one reporting SKU `X`: removing it drops the only non-zero vote
  and `X` goes fresh-zero, opening the bucket-ii correction door on it through the normal lane. Four
  sub-questions the owner has to settle together, because each answer constrains the next:
  1. **Which test binds.** "Refused whenever removal would move any SKU to fresh-zero" is a property
     of the current constellation and lets a retirement through when a *second* producer is stale —
     the SKU only falls later, when that one posts. "Refused whenever the retiring producer's current
     watermark is non-empty" is a property of the producer alone and has no such delay, at the cost
     of refusing retirements that free nothing.
  2. **What preserves what is removed — and this half is already settled by the PRD.** A retired
     producer is unregistered, and **AC #43** (`PRD.md`) reads: *"only **registered** producers'
     signals or silence MUST factor in; an unregistered producer's absence MUST NOT pin SKUs
     conservatively-referenced"*. The first clause is the binding one: a retired producer's signals
     must not factor into the predicate at all, so **keeping its watermark alive in the predicate is
     not available** — which is what the mechanism struck from this rule tried to do. §6.13 carries the same
     clause in its own words (`fr-reference-producer-registration`), so the FR and the AC agree.
     So the choice is narrower than it looked: either
     the freed SKUs are cleared one by one **before** the retirement, or a new evidential record kind
     is defined for them.
  3. **Which record, and what it does to the tripwire.** `SkuCorrectionOverride` rows feed the
     `TripwireCounter`, and C6 trips the `signal_delivery_release_blocker` above five in thirty days.
     One governed retirement freeing six SKUs would block the release; the never-received case names
     the whole catalogue. So the retirement evidence needs either its own record kind or its own
     window.
  4. **Which lane.** The break-glass lane in this slice is the *correction* lane — single-SKU,
     `sku × correct`, gated by `inst-cr-door`. It is not a retirement door: retirement is `reference_producer × write`, a different
     resource and action.
  **Until this is settled the guarded case is guarded and the fresh case is not**, which is the
  honest state and is why it is written here rather than papered over in the rule.
- **The pricing watermark is a joint build** *(P-D-133, 2026-09-04: scheduled by the Program Lead.)* (P-D-03): the producer-side query ("complete live
  plan→SKU reference set") and its cadence are pricing's to design; this slice's door and the
  §15 mirror are ready — the joint fixture belongs to slice 12's seam suite.
- ~~**Watermark set size**~~ **Struck as a note (P-D-134, 2026-09-04)**: booked to `dod-watermark-port`'s builder. *The item's text stood as:*: full-set replacement at 10K SKUs × cadence is fine as rows, but the
  door should accept a compressed set representation from day one (wire-level; no semantic
  change) — implementation note.
- ~~**`PRODUCER_RETIREMENT_WOULD_FREE`'s exception has no lane.**~~ **Answered (P-D-129, 2026-09-03): the exception exists for a dead producer only**, at the retirement door under break-glass elevation. *The item's text stood as:* The refusal is buildable; its escape
  hatch — "the break-glass ceremony's own justification" — has no admission predicate, no evidence
  record and no grant, and this slice's only break-glass lane is the single-SKU correction door,
  which is not a retirement door. No document defines a retirement-lane ceremony. Owner: the owner
  of §6's four sub-questions — decide whether the exception exists in v1 at all.
  *(All three lenses raised it independently.)*
- ~~**OWED — the tighter row-image predicate 01 books against this slice.**~~ **Answered (P-D-129, 2026-09-03): the bump **and** `correction_ref` set in one statement** — a new nullable uuid on `products_sku`; the lead's build. *The item's text stood as:* 01 §4.2 says twice that
  the physical guard carries the interim predicate "with a tighter one still **owed by 07**"; this
  slice carries no such item. Until it is supplied, door identity for bucket-ii head-row writes is
  an application guarantee only, and any publish carrying the third argument passes the guard.
  Owner: this slice. *(Raised by the slice-07 first lens pass.)*
- ~~**The `WATERMARK_FUTURE` skew tolerance has no config home.**~~
  **Answered (owner call, 2026-09-01 — P-D-87 arm 1): `watermark_skew_tolerance_minutes` on
  `ProductsConfig`** (interim 5), per-deployment and boot-time, landing beside the freshness,
  tripwire and break-glass knobs — P-D-84 arm 5's posture as precedent rather than invention.
  Original text: `inst-ws-not-future` introduces a
  "configured tolerance, interim 5 min"; `PRD` §17.1 has no row for it, and §1.4's reference line
  claims only the freshness and tripwire interims. It is the one configurable in this slice with no
  home. Owner: was the §17.1 policy owner; **closed**. *(Two lenses raised it independently.)*
- ~~**What population does the tripwire count?**~~ **Answered (P-D-129, 2026-09-03): two counters, one window** — only `producer_unavailable` feeds the release blocker. *The item's text stood as:* C6 counts break-glass corrections per window, and the
  unresolvable-target arm "increments the same `TripwireCounter`" — but that arm is admissible while
  the signal is fully available, and `fr-failsafe-tripwire` scopes the requirement to operating "in
  `SkuReferenceCount`-unavailable fail-safe mode". Six deleted-`UsageType` repairs in a month would
  reclassify signal *delivery* as a release blocker. P-D-16 amended the correction FR and AC #4 and
  did not touch the tripwire FR. Owner: the tripwire's §17.1 owner.
  *(Two lenses raised it independently.)*
- ~~**Where does `signal_delivery_release_blocker` live, and what clears it?**~~ **Answered (P-D-129, 2026-09-03): derived, no row, no operator exit** — C6's rate rule is a rolling window. *The item's text stood as:* Derived from the rolling
  window it clears itself thirty days later — normalizing degraded operation, which C6 forbids;
  stored, it is a state with no exit and no table in §4. Owner: `fr-failsafe-tripwire`'s owner. *(Raised by the slice-07 first lens pass.)*
- ~~**What carries the admitting lane into the publish transaction?**~~ **Answered (P-D-129, 2026-09-03): the `GovernedLiveOp` envelope's `kind`** — `sku_correction` is a registered kind. *The item's text stood as:* `inst-cr-republish` has the
  registered validator re-check "the lane's own admission predicate", and nothing tells it which
  lane admitted: 01's `PublishDoor` signature has no lane argument, 05's `ApprovalRecord` has no arm
  discriminator, and this slice's override row is written *by* the re-publish. Owner: this slice with
  01 and 05 — a fourth door argument, a field on the approval, or a pre-written admission record.
  *(Raised by the slice-07 first lens pass.)*
- ~~**Who sets `required` on a `sku_correction` `ApprovalRecord`?**~~ **Answered (P-D-129, 2026-09-03): `05`'s evaluator, returning material for the registered kind**; `required = N`. *The item's text stood as:* This slice calls the correction a
  material act at the tenant's `N`; 05's evaluator returns material only on a bucket-iii touch, the
  enumerated ops, an affected-entity count, or a registered `GovernedLiveOp` kind — and 05 explicitly
  removes the metering-unit field from the evaluator's view, while `sku_correction` is not a
  `GovernedLiveOp` kind. As it stands the evaluator returns non-material and the correction closes on
  `min(N, 1)`. Owner: 05's owner. *(Raised by the slice-07 first lens pass.)*
- ~~**What happens to a retired producer's watermark and member rows, and to one that
  re-registers?**~~
  **Answered (owner call, 2026-09-01 — P-D-87 arm 2): the retirement transaction DELETES the
  producer's watermark and member rows**, and a re-registering producer starts `never-received` —
  which is what makes "onboarding can only tighten" true. The producer row itself stays, its
  `state` moving to `retired`, so the registration history is not lost.
  Original text: §4's producer row carries only a state, a registration instant and a ceremony ref, with no clearing
  rule. If the rows survive, retire-then-re-register inside the freshness window makes the producer
  read **fresh** against a stale member set and frees every SKU that has since gained a reference —
  the opposite of "onboarding can only tighten, never free". Owner:
  was `fr-reference-producer-registration`'s owner; **closed**. *(Raised by the slice-07 first lens pass.)*
- ~~**Where does `inst-ws-monotonic`'s set hash come from?**~~
  **Answered (owner call, 2026-09-01 — P-D-71): a `set_hash` column on `products_reference_watermark`, stored at ingestion** — `SHA-256` over the member `sku_id`s sorted bytewise; recomputing from 10K member rows per comparison is the declined arm. Original text: An equal `watermark_at` with an identical
  set hash is an idempotent success and with a different set a refusal, while §4 declares no hash
  column and no rule states its derivation — canonical ordering, algorithm, stored at ingestion or
  recomputed from member rows at 10K SKUs. Owner: this slice with the schema owner. *(Raised by the slice-07 first lens pass.)*
- ~~**Is the correction door's `expected revision` the `If-Match` precondition or a body field?**~~ **Answered (P-D-129, 2026-09-03): `If-Match`**, P-D-33's convention. *The item's text stood as:* This
  pass gave the mismatch 01's `STALE_REVISION`; which surface carries it is still unstated, and it
  determines the door's declared response map. Owner: this slice with 01. *(Raised by the slice-07 first lens pass.)*
- ~~**Which actor performs `reference_producer × write`?**~~ **Answered (P-D-129, 2026-09-03): any principal holding the grant** — the quorum is on the approvers, not the submitter. *The item's text stood as:* §1.3 assigns producer registration and
  retirement to nobody: the producer actors "register at their own build", which reads either as the
  service registering itself or as an operator registering it — incompatible with a material governed
  op requiring a tenant quorum. Owner: this slice with 05. *(Raised by the slice-07 first lens pass.)*
- ~~**What transport and success responses do this slice's three doors have?**~~
  **Answered (owner call, 2026-09-01 — P-D-87 arm 3): the routes and success responses are
  fixed**, each from the set's nearest precedent — watermark post
  `POST /bss-products/v1/reference-watermarks` (**200**: state, not a minted resource); producer
  registration `POST /bss-products/v1/reference-producers` (**201**); retirement
  `POST /bss-products/v1/reference-producers/{producer}/retirements` (**200**); correction
  `POST /bss-products/v1/skus/{skuId}/corrections` (**202**: the door accepts, the write happens
  at approval).
  Original text: Only the watermark door
  is bound to one; the correction door and the membership ops name no route and no 2xx, and §3.2
  gives only refusals. Every comparable operator door in the set names both — 02 added its path and
  pair expressly because without them 12's lint could not see the door. Owner: was the design-set
  owner; **closed**. *(Raised by the slice-07 first lens pass.)*
