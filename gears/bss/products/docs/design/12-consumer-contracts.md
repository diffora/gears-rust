<!-- Related: ../DESIGN.md, ../PRD.md, ../DECISIONS.md, ./01-foundation.md, ./06-catalog-version.md, ./07-reference-signal.md | Owners: BSS Product Catalog team -->

# DESIGN — Consumer Contracts & the Seam Suite (Slice 12)

<!-- toc -->

- [1. Context](#1-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
  - [1.5 Scope](#15-scope)
  - [1.6 Constraints & Assumptions](#16-constraints--assumptions)
  - [1.7 Naming & Design-Introduced Names](#17-naming--design-introduced-names)
- [2. Actor Flows / Contract Surfaces](#2-actor-flows--contract-surfaces)
  - [The seam suite](#the-seam-suite)
  - [The consumer-obligation register](#the-consumer-obligation-register)
  - [Event versioning, replay & bootstrap](#event-versioning-replay--bootstrap)
  - [The SDK / §9 surface](#the-sdk--9-surface)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 Monetization traceability (AC #37)](#31-monetization-traceability-ac-37)
  - [3.2 The completeness checks (`CoverageChecks`)](#32-the-completeness-checks-coveragechecks)
- [4. Data / Storage](#4-data--storage)
- [5. Testing posture (slice-local)](#5-testing-posture-slice-local)
- [6. Traces to / Risks & Open items](#6-traces-to--risks--open-items)

<!-- /toc -->

## 1. Context

### 1.1 Overview

The closing slice owns everything that makes the registry's boundaries **enforced contracts
rather than assumptions**: the registry↔plan-price **seam suite** (the CI fixture set every
"CI-verified" and "seam-suite-asserted" phrase in this design set has been pointing at), the
**shared schema pin**, **event schema versioning + replay/bootstrap**, the **SDK surface**
(PRD §9), the **monetization-traceability** doc obligation (AC #37), and the design set's own
**completeness checks** — the machine that catches an unclaimed FR, an unowned error code, an
unpaired door, or an unnamed event before a reviewer has to.

### 1.2 Purpose

Every cross-gear promise made in slices 01–11 converges here as an *assertable* artifact: a
fixture, a pinned schema, or a lint. A promise that cannot be asserted is re-labeled an open —
this slice is where "the seam suite will verify it" stops being future tense.

### 1.3 Actors

| Actor | Role |
|-------|------|
| `cpt-cf-bss-products-actor-plan-price` | The primary seam counterparty (schema pin, obligations, joint fixtures) |
| `cpt-cf-bss-products-actor-events-audit` | Transport of the versioned events the replay contract rides |
| All consumer actors | Bound by the consumer-obligation list (§2) |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.12 (`fr-plan-price-seam`, `fr-monetization-traceability`), §6.7
  (`fr-event-versioning-replay`), §9 (all **seven** id-bearing blocks across §9.1/§9.2 — `contract-increment-request` included, item 29 of the 2026-08-26 review), AC #29, #36, #37;
  §15 (seam-suite owner/home — proposed `api-contracts` CI, final owner unassigned; event-log
  retention ≥ the bootstrap gap)
- [`../DECISIONS.md`](../DECISIONS.md) P-D-01 (envelope discipline the versioning rides),
  P-D-03 (the joint watermark fixture); every slice's "seam-suite" and "slice 12" pointers

### 1.5 Scope

**In**:
- the seam-suite specification (fixtures, home, pin mechanics)
- the consumer-obligation register
- event schema versioning + the replay/bootstrap contract
- the SDK/§9 surfaces incl. the studio-inbox envelope cross-check
- the completeness checks
- §17.2 traceability (AC #37).

**Out**:
- the counterparts' implementations (each obligation names its owing gear)
- the broker's transport (Common Core)
- the §15 opens this slice can only *assert once closed* (freeze acks, composition signal, watermark delivery).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | A shared schema-version pin whose **membership is derived, not listed** (P-D-12): it covers exactly the operands the §2.2 `ObligationRegister`'s guards read. v1 set — `skuId`, `type`, the meter declaration pair **+ `usageTypeRef`**, `PlanTier`, `status` **with its value vocabulary**, `sellable`, `compositionPending`; `CatalogVersion` pinned as a surface, not a field; `skuCode`/`name` deliberately out (pick-list display, drift cosmetic). CI test **fails on divergence**; a runtime divergence fails closed (the dependent plan publish is rejected, pricing-side) | PRD `fr-plan-price-seam`; P-D-12 |
| C2 | Every event carries a versioned schema ref; a `vN` consumer deserializes `vN+1` (new fields optional with defaults); CI-guarded on every schema change | PRD AC #29, NFR #9, P-D-01 |
| C3 | Bootstrap = latest `CatalogVersion` + the event tail; a consumer checkpoint predating the available tail **fails loudly**; the event-log retention MUST cover the bootstrap gap (§15 open owns the number) | PRD `fr-event-versioning-replay` |
| C4 | A consumer-side assertion is authorable only once the referenced counterpart AC exists — the suite grows with the counterparts, and an unauthorable assertion stays listed as OWED, never silently dropped | PRD `fr-plan-price-seam` |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `SeamSuite` | The joint fixture set + schema-pin checks, one CI job, failing closed on divergence |
| `SchemaPin` | The versioned, committed serialization of the C1 joint fields both gears' CI compares against |
| `ObligationRegister` | §2.2's table — every consumer-side duty, its owing gear, its fixture status (asserted / owed) |
| `CoverageChecks` | The design-set lints of §3.2 |

## 2. Actor Flows / Contract Surfaces

### The seam suite

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-seam-suite`

1. [ ] - `p1` - Home: one CI job over a shared fixture crate (the §15 "proposed: `api-contracts` CI" — final owner still a §15 open; the suite is designed to run in either home unchanged); it consumes both gears' SDKs and the `SchemaPin`, and **fails on any divergence** in the C1 fields (C1) - `inst-ss-home`
2. [ ] - `p1` - The `SchemaPin` is a committed artifact versioned with the SDK: registry-side changes to a pinned field bump it through the ordinary review of BOTH gears (a one-sided bump fails the other side's CI — that asymmetry is the enforcement) - `inst-ss-pin`
3. [ ] - `p1` - Joint fixtures (grown per C4): the P-D-03 **watermark fixture** (pricing produces, registry's predicate answers — the retirement joint contract end-to-end); the **adoption-block fixture** (pricing AC #82 on **its own `When` — retirement or unpublishing**; the `SkuDeprecated` emitted by a *plain* deprecation has no counterpart AC, so by C4 that arm of the fixture is **not authorable yet** and the register carries the ask — 2026-08-26 branch review); the **usage-binding fixture** (pricing `inst-cmp-usagetype` against a registry declaration, incl. the deprecated-bound-unit reject/warn arm — M2); the **grandfathered-resolution fixture** (a frozen snapshot resolves byte-identically after registry churn); the **correction fixture** (`SkuImmutableFieldCorrected` ⇒ pricing re-validates) - `inst-ss-fixtures`

### The consumer-obligation register

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-obligations`

| Obligation | Owing consumer | Source | **Operand (the pinned field its guard reads — lint 9's only input)** | Status |
|------------|----------------|--------|--------|--------|
| Declare intent (`browse` vs `posted`) on resolution | every resolver | 06 `inst-rv-intent` | `CatalogVersion` (surface) | assertable now (registry-side refusal is built into the API) |
| Refuse adoption of `deprecated` SKUs | pricing | PRD AC #36; pricing AC #82 **covers only its own `When` — retirement or unpublishing — so a *plain* deprecation (04 `inst-lc-deprecate`, no retirement behind it) has no pricing-side counterpart AC** (item 35 of the 2026-08-26 review); + pricing PRD §15 row | `status` + its value vocabulary | **owed — partly NOT yet authorable**: the retirement-driven arm is authorable against AC #82 today; the plain-deprecation arm needs a pricing AC that does not exist and is now the §15 ask |
| Refuse adoption of `compositionPending` SKUs | pricing | PRD AC #36 | `compositionPending` | **owed — NOT yet authorable** (M3: pricing's PRD carries neither the flag nor the signal; tied to the §15 `BundleCompositionCompleted` open, per C4 it stays listed until that lands) |
| Enforce `sellable = false` as sellability-gate predicate 6 for standalone lines | pricing | D-46; pricing **D-167 records predicate 6 as needing this registry gear** | `sellable` | **owed, not assertable yet** (item 35 of the 2026-08-26 review corrected M7's reading: the pricing side is designed but its operand is this gear's `sellable`, so the joint assertion cannot precede the seam it asserts) |
| Re-validate on registry tier/meter divergence (`tier_divergent`/`meter_binding_divergent`) | pricing | pricing `inst-cmp-tier-drift`/`inst-cmp-usagetype` (**specified, not built** — 2026-08-26: pricing raises neither code and its own design calls the binding deferred) | `PlanTier` + the metering-unit declaration | **OWED** (M7) — the pricing side of this assertion is unbuilt (`inst-cmp-tier-drift` / `inst-cmp-usagetype` raise no code), so a joint fixture written today passes vacuously |
| Usage-binding checks (unbound meter; priced dimension ⊆ `metadata_fields`; **reject/warn on a `deprecated` bound unit** — M2, the FR's sixth clause restored) | pricing | P-D-05, pricing `inst-cmp-usagetype` | the metering-unit declaration + `usageTypeRef` | **owed** — **the pricing side is specified, not built** (2026-08-26: `inst-cmp-usagetype` raises neither `METER_USAGE_TYPE_UNBOUND` nor `METER_DIMENSION_UNDECLARED`; pricing's `design/01-foundation.md` calls the binding deferred). A joint fixture written today passes vacuously, so this row is OWED on both sides |
| Resolve grandfathered refs against the frozen snapshot | pricing / subscriptions | 06 `inst-gf-invariant` | `CatalogVersion` (surface) | **owed** |
| Re-validate on `SkuImmutableFieldCorrected` | pricing | 07 `inst-cr-republish` | `type` + the metering-unit declaration (the bucket-ii pair) | **owed** |
| Act on the surfaced binding diff `(boundVersion, resolvedVersion, diffRef)` | freeze participants | 06 `inst-sn-binding-diff` | `CatalogVersion` (surface) | **owed** |
| Refuse `not_frozen(forced)` participants' content for posted use | pricing / Billing | 06 `inst-fz-force` | `CatalogVersion` (surface) | **owed**, and **no longer the only enforcement** — P-D-19 put the fail-closed default back on the registry's own resolver (`VERSION_FORCED_INCOMPLETE`), because this row was booked against pricing *and Billing, which has no gear*, so a stated safe default was enforced on neither side |
| Release a `CatalogVersion` when the last live reference is gone (`catalog_version × release`) | every freeze participant — v1 = pricing / Contracts / Billing | 06 `inst-fz-liveness`, 10 `inst-rt-gc` | `CatalogVersion` (surface) | **owed** — P-D-18; one idempotent release per `(participant, version)`, and GC waits for all of them (this row absorbed a duplicate booking of the same door on 2026-08-26); snapshot GC is gated on it, so a participant that never releases pins storage indefinitely. First obligation on this list whose *absence* costs storage rather than correctness |
| Take the latest `fromVersion` on a re-announced `SkuRetired`, keyed `(skuId, effectiveAt)` | pricing / subscriptions | 04 `inst-rt-initiate` | `SkuRetired` payload | **owed** — P-D-20; `SkuRetired` is no longer at-most-once per entity, since a publish during the lead window re-announces it |
| Produce the `SkuReferenceCount` watermark | pricing (v1), then subscriptions/contracts | P-D-03, 07 | `skuId` | **owed — the P-D-03 joint build** |
| Consume `mustMigrateBy` | subscriptions | 04 EOL lockout | none in v1 (the field is never populated — 04 `inst-rt-eol-lockout`) | **deferred with post-v1 EOL** |

Every row is either a fixture in the suite or an explicitly OWED line — the register is the
suite's backlog, reviewed whenever a counterpart lands an AC (C4).

### Event versioning, replay & bootstrap

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-replay`

1. [ ] - `p1` - Every event schema is a versioned artifact in `products-sdk`; the CI compatibility test runs C2's actual direction — **an old (`vN`) consumer deserializing a `vN+1` payload** carrying the new optional fields with defaults (the reverse direction, new code reading old fixtures, is the trivial half and is also asserted) on every schema change — the 04 EOL field (`mustMigrateBy` present-but-unpopulated) is the standing example; **the 09 export-artifact schema joins the same corpus** (L3 — the discipline it cites is now exercised, not borrowed) - `inst-rc-compat`
2. [ ] - `p1` - Dedup/ordering detection beyond the idempotency window rides `(tenant, aggregate, sequence)`, where **`sequence` is the toolkit outbox's `seq`, published on the envelope beside `partition_id` (P-D-27)** — `partition = hash(tenant_id, aggregate_id) mod N`, so every event of one aggregate shares a partition and `seq` is monotonic within it; detection needs monotonicity, not density, so the gaps left by neighbouring aggregates in the same partition are expected and must not be read as loss. The consumer contract states it and the suite fixtures a duplicate + an out-of-order delivery - `inst-rc-dedup`
2a. [ ] - `p1` - **Replay reads the body core** every Foundation event carries — `{tenantId, entityKind, entityId, internalRevision, lifecycleState}`, with `publishedVersion` additionally on `*Published` (**P-D-27**) — and `internalRevision` is the value **as committed by the act** (**P-D-29**), so a consumer correlating an event to an ETag compares it directly rather than adjusting by one. `lifecycleState` is the discriminator on `ProductHeadSaved`/`SkuHeadSaved`, which cover a save on a `draft`, `published` or `deprecated` head alike - `inst-rc-body`
3. [ ] - `p1` - Bootstrap (C3): a published-scope consumer initializes from the latest `CatalogVersion` (06's resolver, `browse` intent) + the event tail from that version's instant — **or, in a tenant with zero published versions, from the empty catalog plus the whole retained tail** (the anchorless arm, stated because 08's projection tables are called rebuildable "without loss" and this is the only case with no anchor to rebuild from — item 35 of the 2026-08-26 review); a checkpoint older than the retained tail fails loudly with the named remedy (re-bootstrap) — the same contract 08's projector obeys internally, so the gear's own projector is the contract's first consumer and its permanent conformance probe *(renamed from `inst-rp-bootstrap` — H1: it collided with 08's read-projection id, and the lint that should have caught it is #6 below)* - `inst-rc-bootstrap`

*Replay of a refusal (**P-D-26**): a refused request is a finished request — the door answers its
idempotency key with the refusal as the stored response (`response_status`/`response_body`,
**P-D-29**), so a retry on the same key replays that refusal rather than re-running a rule whose
verdict cannot change. A consumer must therefore expect an idempotent replay to reproduce a 4xx as
faithfully as a success, and `IDEMPOTENCY_KEY_IN_FLIGHT` (409) to mean the first attempt is still
running.*

### The SDK / §9 surface

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-sdk`

1. [ ] - `p1` - `products-sdk` mirrors §9: the authoring/publish client (idempotency keys + `If-Match` + intent semantics are **part of the contract**, breaking = major), the read-model client, **the increment-request client (`PRD` §9.2 `…-contract-increment-request`) and the watermark client (`…-contract-sku-reference-count`) — **every** inbound machine contract of §9.2 (**P-D-15**) — four of them, the two named here plus the freeze acknowledgment with its release half (P-D-18) and the bundle composition-completed signal — typed clients resolved from `ClientHub` with the REST/S2S doors as their out-of-process bindings; the increment client's three-way error taxonomy (not wired / unreachable / unusable) is part of the contract**, the event payload types, the error-code enum (01 §3.3 + every slice's registered codes — renames breaking) - `inst-sdk-surface`
2. [ ] - `p1` - The catalog read shape is **`CatalogSku`-superset-compatible** (pricing's `ProductCatalogClientV1` trait consumes it — L2): `sku_id`, `sku_code`, `name`, `metering_unit`, `status`, `plan_tier` — plus `sellable`, `usage_type_ref`, `composition_pending`, `type` (the members pricing's copy lacks land consumer-side as additive). **The `status` wire vocabulary is pinned** (M4): browse serves `published|deprecated` only (draft never served, retired history-only — 08 C2); the SDK enum documents all five states with the wire subset named; pricing's opaque-string tolerance is a courtesy the pin replaces — its own doc calls the field "verbatim, **not an enum**", which is the right tolerance for display and exactly the wrong one for a guard, so a renamed `deprecated` or an added blocking state would leave pricing's adoption guard **accepting rather than erroring**. `status` and its vocabulary are **normative pin members** as of P-D-12 (2026-08-26; previously a flagged design widening beyond the FR's list) - `inst-sdk-catalogsku`
3. [ ] - `p2` - The approval-queue envelope is asserted against pricing's queue shape (the studio single-inbox contract, 05 `inst-gv-queue`) — a field-name drift fails the suite, not a UI sprint - `inst-sdk-inbox`

## 3. Processes / Business Logic

### 3.1 Monetization traceability (AC #37)

- [ ] `p2` - **ID**: `cpt-cf-bss-products-contract-traceability`

The §17.2 map is the deliverable and it exists in the PRD; this slice's duty is keeping it
true: the completeness checks assert that no registry surface grows a monetization-model
marker (the absence is intentional — a new SKU column matching the §17.2 left column is a
lint failure, not a feature).

### 3.2 The completeness checks (`CoverageChecks`)

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-coverage`

Doc-plane lints over this design set + PRD (spec-check-class. **Gated by nothing today** — corrected 2026-08-26: this read "run in CI with the docs job", and `.github/workflows/docs.yml` holds one job, `Check Markdown Links`; the `Spec Invariants` job that ran `make spec-check` was deleted on 2026-07-29 together with its Makefile target. These nine are specified here and run on demand until someone adds the job back, which is itself owed):

1. [ ] - `p1` - **Requirement coverage**: every `p1`/`p2` requirement-bearing PRD id — `fr-*`, `nfr-*`, `interface-*`, `contract-*` (M5: the universe is **all seven** id-bearing §9 blocks and §7, not FRs alone — `contract-increment-request` was added after this count was written, item 29 of the 2026-08-26 review) — is claimed by exactly one **owner per clause** — one slice for a whole requirement, or one slice per scope-qualified clause where a requirement is deliberately split (the fourteen pairs below)'s Traces-to. **NFRs are claimed by id, never by position** (item 30 of the 2026-08-26 review: slices cited "NFR #1, #2, #7, #10" and this lint keys on `nfr-*` ids, so it reported zero claims for all ten; every slice's Traces-to now carries the id alongside the number). **Qualifier grammar (M6)**: a claim is `id (<qualifier>)`; qualifiers compare as normalized strings; an unqualified claim conflicts with every other claim of the id; two identical qualifiers fail. **The id harvest is scoped to `PRD.md`** — the design set declares fourteen `cpt-cf-bss-products-contract-*` ids of its own (counted 2026-08-26: the prose said thirteen) (error taxonomies, `contract-rbac`, `contract-obligations`, `contract-sdk`, `contract-traceability`), and an unscoped `contract-*` glob would pull them into the requirement universe. **`usecase-*` ids join the universe** (seven §10 use cases; exactly one was cited by id anywhere before the 2026-08-26 branch review, though all seven have a substantive home). **And an AC-existence check**: every **unqualified** `AC #N` cited in a slice must resolve to a §12 heading of *this* PRD; a citation qualified by its gear (`pricing AC #82`, which this set carries ten times) resolves against that gear or is out of scope — added 2026-08-26, the unscoped rule was red on day one on ten correct citations — a one-line regex that catches nothing today but guards the class that five sites attributing pricing AC #82 a `When` it does not have belongs to, which no lint in this set could see. **What this lint still cannot do, stated so nobody reads it as more:** it checks that a claim is *well-formed and unique*, never that the slice covers the requirement, and it says nothing about whether what a slice asserts *about* a cited AC is true. Adopting the grammar requires one sweep of the existing Traces-to lines — owed with the lint's build - `inst-cc-fr`

   *Deliberate divergence, stated 2026-08-26: fourteen requirements are owned by **two**
   slices each, so the generic one-slice reading — `spec-check`'s `P2/fr-multiply-claimed` —
   reports all fourteen. That is expected here and is not a defect to sweep. Split ownership
   is legal in this set **when every claim carries a scope qualifier**, which is what the
   qualifier grammar below enforces and what all fourteen pairs now carry. The invariant is
   one owner per **clause**, not one slice per id.*
2. [ ] - `p1` - **AC #38 map**: every enumerated failure row **that a registry door can refuse** maps to exactly one error code with **exactly one raising *phase*** — a phase, not a door (**P-D-24**: the unit changed when the `state` phase was added, and §3.1's roster now runs **seven**, `idempotency resolution → precondition → shape → state → identity → registered validators → governance gate`, with authorization a **pre-pipeline gate** outside them all, P-D-30). A phase runs at several doors by design, which is why the door-shaped form was red by construction on every multi-door code. **The map gains three codes and a status class from the 2026-08-27 round (P-D-25)**: `DUPLICATE_CODE` (renamed from `DUPLICATE_SKU_CODE`, one code covering both the `skuCode` and `productCode` reservations), `ENTITY_TERMINAL` (409 — a save on a `retired`/`discarded` head) and `AUDIT_UNAVAILABLE` (**503** — the refusal's audit row could not be written), the second 503 in the gear beside 08's `READ_MODEL_OVERLOADED`. **Three of the fifteen rows are explicitly outside the universe — the retention-orphan alarm (10 `inst-rt-gc`), the `compositionPending` adoption duty (a consumer obligation with no registry door) and AC #38's post-v1 EOL row, whose only candidate code refuses the feature rather than the named condition — named here so the lint is buildable rather than perpetually red** (item 32 of the 2026-08-26 review): the retention-orphan row is deliberately an **alarm**, not an API error (10 `inst-rt-gc`), and "adopting a `compositionPending` bundle" is a **consumer duty with no registry door** (§2.2). The lint asserts the exclusion list is exactly those three — an unexplained fourth exclusion fails it - `inst-cc-errors`
3. [ ] - `p1` - **Door×grant pairing**: every REST/S2S door named in any slice appears in 05's RBAC catalog - `inst-cc-rbac`
4. [ ] - `p1` - **Event bookkeeping**: every state-changing instruction names its event or an explicit no-event; 09's coalesced summary event is **additive** (row-level domain events all emit — the H1-corrected rule) - `inst-cc-events`
5. [ ] - `p2` - **Register hygiene**: every P-D propagation list names only documents that restate the decision, and every restating document appears (the L2-class lint) - `inst-cc-register`
6. [ ] - `p1` - **Id uniqueness**: every `cpt-*`/`flow`/`inst-*`/actor id is **declared** exactly once across the design set, where a **declaration** is the id trailing its own numbered instruction row and a **re-use** is any other mention (H1's live violator — the 08/12 `inst-rp-bootstrap` collision — was fixed in the same commit that added this lint). **The grammar is what makes it buildable** (item 32 of the 2026-08-26 review): `design/01-foundation.md` legitimately carries `inst-fd-idempotency`, `inst-fd-create-txn`, `inst-fd-etag` and `inst-fd-publish-pin` on two rows each, where the second row continues the same instruction — those rows carry the id **parenthesized** (`(cont. inst-fd-etag)`), the same device lint 1's qualifier grammar uses, so a continuation can never read as a duplicate declaration - `inst-cc-ids`
7. [ ] - `p1` - **Identity materialization**: no table or projection other than 10's `IdentityRefMap` stores an operator identity — the lint 10's erasure guarantee names (H2: it existed only as a citation until here) - `inst-cc-identity`
8. [ ] - `p2` - **No monetization marker** (AC #37): a registry schema surface matching the §17.2 left column fails the lint — the §3.1 prose now has its buildable artifact (M1) - `inst-cc-monetization`
9. [ ] - `p1` - **Obligation×pin coupling** (P-D-12): every §2.2 `ObligationRegister` row whose guard reads a **catalog field** has that field in the `SchemaPin` — a row whose operand is an **event payload** (the P-D-20 `SkuRetired` row) is outside the pin by construction, since the pin covers the entity surface and not the event surface, and is excluded by that reason rather than by omission (added 2026-08-26) — and every pinned field is either an obligation operand or carries a recorded exclusion reason. **The register carries an explicit `Operand` column and the lint reads only that** (item 32 of the 2026-08-26 review — reading operands out of the rows' prose yielded ten fields against the pin's eight, and left `PlanTier` pinned with no register row naming it: `PlanTier`'s operand row is the tier-divergence obligation, now stated as such). This is what makes C1's membership a rule rather than a list — the FR it derives from previously stated three obligations (`deprecated` adoption, `compositionPending` adoption, usage binding) while pinning none of their operands, and no lint could see it - `inst-cc-pin`

## 4. Data / Storage

None owned — this slice's artifacts are the SDK crate, the fixture crate, the `SchemaPin`
file, and the lints. That absence is by design.

## 5. Testing posture (slice-local)

This slice IS testing posture; its own meta-probes: the `SchemaPin` divergence RED (mutate a
pinned field one-sided — both CIs must fail); the vN→vN+1 RED (remove a default — the compat
test must fail); the bootstrap RED (age a checkpoint past the tail — loud failure, named
remedy); one OWED row flipped to asserted end-to-end (the watermark fixture, first — it is the
P-D-03 joint build's acceptance).

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-fr-plan-price-seam`, `cpt-cf-bss-products-fr-event-versioning-replay`,
`cpt-cf-bss-products-fr-monetization-traceability`; AC #29, #36, #37; **NFR #9 `cpt-cf-bss-products-nfr-backward-compatible-evolution`** by id. **§9's seven ids are claimed by their owning slices, not here** — `interface-authoring-publish` + `contract-registry-events` → 01, `interface-read-model` → 08, `contract-sku-reference-count` → 07, `contract-increment-request` + `contract-freeze-ack` + `contract-bundle-composition-signal` → 06. This slice specifies the *suite* over that surface, which is not a coverage claim (2026-08-26 branch review: the ids were claimed nowhere at all, so lint 1 would have reported zero for all seven on its first run); the
"CI-verified once the suite exists" phrases of `cpt-cf-bss-products-fr-deprecation` (the consumer-side adoption-guard duty only)/`cpt-cf-bss-products-fr-freeze-atomicity` (the consumer obligation half) — this
slice is that suite's specification.

**Risks & open items**:
- **The suite's final owner/home is a §15 open** (proposed `api-contracts` CI) — the design is
  home-agnostic, but an unowned CI job is an unrun one; this is the set's last
  organizational dependency.
- **Most obligations are OWED** by construction (C4): the register makes the debt legible, and
  the P-D-03 watermark fixture is deliberately first — it unblocks retirement, the highest-value
  seam.
- **Event-log retention ≥ bootstrap gap** needs its number (§15) before the replay contract is
  more than words; named as the replay contract's single config dependency.
- **SchemaPin widening — RESOLVED 2026-08-26 (P-D-12)** (was: L1, owed a decision). The pin's
  membership became a **rule** — the operands the §2.2 guards read — rather than a list, and
  `fr-plan-price-seam` was amended to say so. Measuring the register against the FR's list found
  **four** operands outside the pin, not the two this item named: `status`, `compositionPending`,
  `sellable`, `usageTypeRef`, three of whose obligations that very FR states in its own sentence.
  The list also named two items that are not comparable fields of the consumer's shape at all
  (`bundle` type is absent from pricing's shipped `CatalogSku`; `CatalogVersion` is a surface),
  so of five pinned items only three could ever be compared. `inst-cc-pin` now lints the
  coupling in both directions. **Honest limit**: the seam suite itself does not exist yet (§15
  owns its home and owner), so this widens a specification rather than a running gate — cheaper
  before the job is built than after.
