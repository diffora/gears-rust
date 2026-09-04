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
  - [2.1 The seam suite](#21-the-seam-suite)
  - [2.2 The consumer-obligation register](#22-the-consumer-obligation-register)
  - [2.3 Event versioning, replay & bootstrap](#23-event-versioning-replay--bootstrap)
  - [2.4 The SDK / §9 surface](#24-the-sdk--9-surface)
- [3. Processes / Business Logic](#3-processes--business-logic)
  - [3.1 Monetization traceability (AC #37)](#31-monetization-traceability-ac-37)
  - [3.2 The completeness checks (`CoverageChecks`)](#32-the-completeness-checks-coveragechecks)
- [4. Data / Storage](#4-data--storage)
  - [4.1 The AC #38 row → code map](#41-the-ac-38-row--code-map)
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
fixture, a pinned schema, or a lint. A promise that cannot be asserted is re-labeled an open

### 1.3 Actors

| Actor | Role |
|-------|------|
| `cpt-cf-bss-products-actor-plan-price` | The primary seam counterparty (schema pin, obligations, joint fixtures) |
| `cpt-cf-bss-products-actor-events-audit` | Transport of the versioned events the replay contract rides |
| All consumer actors | Bound by the consumer-obligation list (§2) |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.12 (`fr-plan-price-seam`, `fr-monetization-traceability`), §6.7
  (`fr-event-versioning-replay`), §9 (all **seven** id-bearing blocks across §9.1/§9.2 — `contract-increment-request` included, item 29 of the review), AC #29, #36, #37;
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
| C1 | A shared schema-version pin whose **membership is derived, not listed** (P-D-12): it covers exactly the **catalog-field** operands the §2.2 `ObligationRegister`'s guards read
(event-payload operands are outside the pin — §3.2 lint 9). v1 set — `skuId`, `type`, the metering-unit declaration (`unit` **+** `usageTypeRef`, 03 `inst-mt-atomic-pair`), `PlanTier`, `status` **with its value vocabulary**, `sellable`, `compositionPending`; `CatalogVersion` pinned as a surface, not a field (**P-D-65**: a pin entry of kind `surface`, its comparison delegated to the port trait — the job neither compares it nor asserts its absence); `skuCode`/`name` deliberately out (pick-list display, drift cosmetic). CI test **fails on divergence for a member the pin marks comparable, and fails on the *presence* of one it does not** (**P-D-57** — the pin keeps every derived member and carries its comparability, so an unshipped member is asserted absent rather than compared); a runtime divergence fails closed (the dependent plan publish is rejected, pricing-side) | PRD `fr-plan-price-seam`; P-D-12 |
| C2 | Every event carries a versioned schema ref; a `vN` consumer deserializes `vN+1` (new fields optional with defaults); CI-guarded on every schema change | PRD AC #29, NFR #9, P-D-01 |
| C3 | Bootstrap = latest `CatalogVersion` + the event tail, or — in a tenant with zero published versions — the empty catalog + the whole retained tail (§2.3 `inst-rc-bootstrap`); a consumer checkpoint predating the available tail **fails loudly**; the event-log retention MUST cover the bootstrap gap (§15 open owns the number) | PRD `fr-event-versioning-replay` |
| C4 | A consumer-side assertion is authorable only once the referenced counterpart AC exists — the suite grows with the counterparts, and an unauthorable assertion stays listed as OWED, never silently dropped | PRD `fr-plan-price-seam` |

### 1.7 Naming & Design-Introduced Names

| Name | Meaning |
|------|---------|
| `SeamSuite` | The joint fixture set + schema-pin checks, one CI job, failing closed on divergence |
| `SchemaPin` | The versioned, committed serialization of the C1 joint fields both gears' CI compares against |
| `ObligationRegister` | §2.2's table — every consumer-side duty, its owing gear, its fixture status (asserted / owed / assertable now / deferred — §2's register is the roster) |
| `CoverageChecks` | The design-set lints of §3.2 |

## 2. Actor Flows / Contract Surfaces

### 2.1 The seam suite

Declared by [`../features/consumer-contracts.md`](../features/consumer-contracts.md) §2 as `cpt-cf-bss-products-flow-seam-suite`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Home: one CI job over **`cf-gears-bss-fixtures`** (**P-D-44**, path in §4.1's artifact table; the §15 "proposed: `api-contracts` CI" — final owner still a §15 open; the suite is designed to run in either home unchanged); it consumes both gears' SDKs and the `SchemaPin`, and is **two-sided** (**P-D-57**): it **fails on any divergence** in the C1 fields the pin marks comparable, and on the **presence** of any C1 field the pin marks not-yet-comparable — so a member that ships while still marked fails the job in the change that shipped it - `inst-ss-home`
2. [ ] - `p1` - The `SchemaPin` is a committed artifact versioned with the SDK: registry-side changes to a pinned field bump it through the ordinary review of BOTH gears (a one-sided bump fails the other side's CI — that asymmetry is the enforcement) - `inst-ss-pin`
3. [ ] - `p1` - Joint fixtures (grown per C4): the P-D-03 **watermark fixture** (pricing produces, registry's predicate answers — the retirement joint contract end-to-end); the **adoption-block fixture** (pricing AC #82 on **its own `When` — retirement or unpublishing**; the `SkuDeprecated` emitted by a *plain* deprecation has no counterpart AC, so by C4 that arm of the fixture is **not authorable yet** and the register carries the ask — branch review); the **usage-binding fixture** (pricing's meter-binding rule against a registry declaration, incl. the deprecated-bound-unit reject/warn arm — M2); the **grandfathered-resolution fixture** (a frozen snapshot resolves byte-identically after registry churn); the **correction fixture** (`SkuImmutableFieldCorrected` ⇒ pricing re-validates). **The transport under every event-bearing fixture is `event-broker-sdk`'s own `MockBroker` under its `test-util` feature, registered into `ClientHub` as `dyn EventBrokerApi`** (**P-D-58**) — the registration a production boot performs, with a different transport behind it, never a producer injected past the hub; so the fixtures do not wait on the missing production registration, and what a green suite licenses is that the contract holds over this gear's own path with a conforming transport - `inst-ss-fixtures`

### 2.2 The consumer-obligation register

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-obligations`

| Obligation | Owing consumer | Source | **Operand (what its guard reads — lint 9's only input; not every operand is a pinned field, see lint 9)** | Status |
|------------|----------------|--------|--------|--------|
| Declare intent (`browse` vs `posted`) on resolution | every resolver | 06 `inst-rv-intent` | `CatalogVersion` (surface) | assertable now (registry-side refusal is built into the API) |
| Refuse adoption of `deprecated` SKUs | pricing | PRD AC #36; pricing AC #82 **covers only its own `When` — retirement or unpublishing — so a *plain* deprecation (04 `inst-lc-deprecate`, no retirement behind it) has no pricing-side counterpart AC** (item 35 of the review); + pricing PRD §15 row | `status` | **owed — partly NOT yet authorable**: the retirement-driven arm is authorable against pricing AC #82 today; the plain-deprecation arm needs a pricing AC that does not exist and is now the §15 ask |
| Refuse adoption of `compositionPending` SKUs | pricing | PRD AC #36 | `compositionPending` | **owed — NOT yet authorable** (M3: pricing's PRD carries neither the flag nor the signal; tied to the §15 `BundleCompositionCompleted` open, per C4 it stays listed until that lands) |
| Enforce `sellable = false` as sellability-gate predicate 6 for standalone lines | pricing | D-46; pricing **D-167 records predicate 6 as needing this registry gear** | `sellable` | **owed, not assertable yet** (item 35 of the review corrected M7's reading: the pricing side is designed but its operand is this gear's `sellable`, so the joint assertion cannot precede the seam it asserts) |
| Re-validate on registry tier/meter divergence (`tier_divergent`/`meter_binding_divergent`) | pricing | pricing's tier-drift and meter-binding rules (**specified, not built** — pricing raises neither code and its own design calls the binding deferred) | `PlanTier`, `unit`, `usageTypeRef` | **OWED** (M7) — the pricing side of this assertion is unbuilt (neither rule raises a code), so a joint fixture written today passes vacuously |
| Usage-binding checks (unbound meter; priced dimension ⊆ `metadata_fields`; **reject/warn on a `deprecated` bound unit** — M2, the FR's sixth clause restored) | pricing | P-D-05, pricing's meter-binding rule | `unit`, `usageTypeRef` | **owed** — **the pricing side is specified, not built** (that rule raises neither `METER_USAGE_TYPE_UNBOUND` nor `METER_DIMENSION_UNDECLARED`; pricing's `design/01-foundation.md` calls the binding deferred). A joint fixture written today passes vacuously, so this row is OWED on both sides |
| Resolve grandfathered refs against the frozen snapshot | pricing / subscriptions | 06 `inst-gf-invariant` | `CatalogVersion` (surface) | **owed** |
| Re-validate on `SkuImmutableFieldCorrected` | pricing | 07 `inst-cr-republish` | `type`, `unit`, `usageTypeRef` (07 C4's bucket-ii set) | **owed** |
| Act on the surfaced binding diff `(boundVersion, resolvedVersion, diffRef)` | freeze participants | 06 `inst-sn-binding-diff` | `CatalogVersion` (surface) | **owed** |
| Refuse `not_frozen(forced)` participants' content for posted use | pricing (the v1 participant set — P-D-48) | 06 `inst-fz-force` | `CatalogVersion` (surface) | **owed**, and **no longer the only enforcement** — P-D-19 put the fail-closed default back on the registry's own resolver (`VERSION_FORCED_INCOMPLETE`), because this row was booked against pricing *and Billing, which has no gear*, so a stated safe default was enforced on neither side |
| Release a `CatalogVersion` when the last live reference is gone (`catalog_version × release`) | every freeze participant — **v1 = pricing** (P-D-48); Contracts and Billing when they register | 06 `inst-fz-liveness`, 10 `inst-rt-gc` | `CatalogVersion` (surface) | **owed** — P-D-18; one idempotent release per `(participant, version)`, and GC waits for all of them (this row absorbed a duplicate booking of the same door ); snapshot GC is gated on it, so a participant that never releases pins storage indefinitely. First obligation on this list whose *absence* costs storage rather than correctness |
| Take the latest `fromVersion` on a re-announced `SkuRetired`, keyed `(skuId, effectiveAt)` | pricing / subscriptions | 04 `inst-rt-initiate` | `SkuRetired` payload | **owed** — P-D-20; `SkuRetired` is no longer at-most-once per entity, since a publish during the lead window re-announces it |
| Produce the `SkuReferenceCount` watermark | pricing (v1), then subscriptions/contracts | P-D-03, 07 | `skuId` | **owed — the P-D-03 joint build** |
| Consume `mustMigrateBy` | subscriptions | 04 EOL lockout | none in v1 (the field is never populated — 04 `inst-rt-eol-lockout`) | **deferred with post-v1 EOL** |

Every row is a fixture in the suite, an explicitly OWED line, or one of the two rows marked
`assertable now` / `deferred with post-v1 EOL` — the register is the
suite's backlog, reviewed whenever a counterpart lands an AC (C4).

### 2.3 Event versioning, replay & bootstrap

Declared by [`../features/consumer-contracts.md`](../features/consumer-contracts.md) §2 as `cpt-cf-bss-products-flow-replay`.
The steps below are this slice's and are the normative ones; the FEATURE carries the
actor, the scenarios and the boundary.

1. [ ] - `p1` - Every event schema is a versioned artifact in `products-sdk`; the CI compatibility test runs C2's actual direction — **an old (`vN`) consumer deserializing a `vN+1` payload** carrying the new optional fields with defaults (the reverse direction, new code reading old fixtures, is the trivial half and is also asserted) on every schema change — the 04 EOL field (`mustMigrateBy` present-but-unpopulated) is the standing example; **the 09 export-artifact schema joins the same corpus** (L3 — the discipline it cites is now exercised, not borrowed) - `inst-rc-compat`
2. [ ] - `p1` - Dedup/ordering detection beyond the idempotency window rides `(tenant, aggregate, sequence)`, where **`sequence` is the broker's server-assigned read-side `sequence` per `(topic, partition)` (P-D-47, re-taking P-D-27's slot, which the broker's schema refuses)** — the gear sets no `partition_key`, so the broker's ADR-0002 default puts every event of one tenant on one partition, and `sequence` is monotonic across them; detection needs monotonicity, not density, so the gaps left by neighbouring aggregates in the same partition are expected and must not be read as loss. Within the window the dedup key is the event **`id`**, which the SDK mints once at enqueue and every delivery attempt repeats. The consumer contract states both and the suite fixtures a duplicate + an out-of-order delivery - `inst-rc-dedup`
3. [ ] - `p1` - **Replay reads the body core** every Foundation event carries — `{tenantId, entityKind, entityId, internalRevision, lifecycleState}`, with `publishedVersion` additionally on `*Published` (**P-D-27**) — and `internalRevision` is the value **as committed by the act** (**P-D-29**), so a consumer correlating an event to an ETag compares it directly rather than adjusting by one. `lifecycleState` is the discriminator on `ProductHeadSaved`/`SkuHeadSaved`, which cover a save on a `draft`, `published` or `deprecated` head alike - `inst-rc-body`
4. [ ] - `p1` - Bootstrap (C3): a published-scope consumer initializes from the latest `CatalogVersion` (06's resolver, `browse` intent) + the event tail from that version's instant — **or, in a tenant with zero published versions, from the empty catalog plus the whole retained tail** (the anchorless arm, stated because 08's projection tables are called rebuildable "without loss" and this is the only case with no anchor to rebuild from — item 35 of the review); a checkpoint older than the retained tail fails loudly with the named remedy (re-bootstrap) — the same contract 08's projector obeys internally, so the gear's own projector is the contract's first consumer and its permanent conformance probe *(renamed from `inst-rp-bootstrap` — H1: it collided with 08's read-projection id, and the lint that should have caught it is #6 below)* - `inst-rc-bootstrap`

*Refusals are never replayed (**P-D-38**, superseding P-D-26's arm): a refusal stores
nothing and releases the key, so a retry on the same key **runs** rather than replaying. A consumer
can therefore retry a refused request on the same key and get a fresh verdict — which is what a
client refused `STALE_REVISION` needs after re-reading the head — and an idempotent replay only
ever reproduces a **success**. `IDEMPOTENCY_KEY_IN_FLIGHT` (409) still means the first attempt is
running.*

### 2.4 The SDK / §9 surface

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-sdk`

1. [ ] - `p1` - `products-sdk` mirrors §9: the authoring/publish client (idempotency keys + `If-Match` + intent semantics are **part of the contract**, breaking = major), the read-model client, **the increment-request client (`PRD` §9.2 `…-contract-increment-request`) and the watermark client (`…-contract-sku-reference-count`) — **every** inbound machine contract of §9.2 (**P-D-15**) — four of them, the two named here plus the freeze acknowledgment with its release half (P-D-18) and the bundle composition-completed signal — typed clients resolved from `ClientHub` with the REST/S2S doors as their out-of-process bindings; the increment client's three-way error taxonomy (not wired / unreachable / unusable) is part of the contract**, the event payload types, the error-code enum (01 §3.3 + every slice's registered codes — renames breaking) - `inst-sdk-surface`
2. [ ] - `p1` - The catalog read shape is **`CatalogSku`-superset-compatible** (pricing's `ProductCatalogClientV1` trait consumes it — L2): `sku_id`, `sku_code`, `name`, `metering_unit`, `status`, `plan_tier` — plus `sellable`, `usage_type_ref`, `composition_pending`, `type` (the members pricing's copy lacks land consumer-side as additive). **The `status` wire vocabulary is pinned** (M4): browse serves `published|deprecated` only (draft never served, retired history-only — 08 C2); the SDK enum documents all five states with the wire subset named; pricing's opaque-string tolerance is a courtesy the pin replaces — its own doc calls the field "verbatim, **not an enum**", which is the right tolerance for display and exactly the wrong one for a guard, so a renamed `deprecated` or an added blocking state would leave pricing's adoption guard **accepting rather than erroring**. `status` and its vocabulary are **normative pin members** as of P-D-12 (previously a flagged design widening beyond the FR's list) - `inst-sdk-catalogsku`
3. [ ] - `p2` - The approval-queue envelope is asserted against pricing's queue shape (the studio single-inbox contract, 05 `inst-gv-queue`) — a field-name drift fails the suite, not a UI sprint - `inst-sdk-inbox`

## 3. Processes / Business Logic

### 3.1 Monetization traceability (AC #37)

Declared by [`../features/consumer-contracts.md`](../features/consumer-contracts.md) §3 as `cpt-cf-bss-products-algo-monetization-traceability`.
The obligation below is this slice's and is the normative one; the FEATURE carries the
Input, the Output and the boundary.

- [ ] `p2` - **ID**: `cpt-cf-bss-products-contract-traceability`

The §17.2 map is the deliverable and it exists in the PRD; this slice's duty is keeping it
true: the completeness checks assert that no registry surface grows a monetization-model
marker (the absence is intentional — a new SKU column matching §17.2's first left-column row is a
lint failure, not a feature).

### 3.2 The completeness checks (`CoverageChecks`)

Declared by [`../features/consumer-contracts.md`](../features/consumer-contracts.md) §3 as `cpt-cf-bss-products-algo-coverage`.
The lints below are this slice's and are the normative ones; the FEATURE carries the
Input, the Output and the boundary.

Doc-plane lints over this design set + PRD (spec-check-class. **Gated by nothing today** — corrected: this read "run in CI with the docs job", and `.github/workflows/docs.yml` holds one job, `Check Markdown Links`; the `Spec Invariants` job that ran `make spec-check` was deleted  together with its Makefile target. These nine are specified here and run on demand until someone adds the job back, which is itself owed):

1. [ ] - `p1` - **Requirement coverage**: every `p1`/`p2` requirement-bearing PRD id — `fr-*`, `nfr-*`, `interface-*`, `contract-*` (M5: the universe is **all seven** id-bearing §9 blocks and §7, not FRs alone — `contract-increment-request` was added after this count was written, item 29 of the review) — is claimed by exactly one **owner per clause** — one slice for a whole requirement, or one slice per scope-qualified clause where a requirement is deliberately split (the fourteen pairs below)'s Traces-to. **NFRs are claimed by id, never by position** (item 30 of the review: slices cited "NFR #1, #2, #7, #10" and this lint keys on `nfr-*` ids, so it reported zero claims for all ten; every slice's Traces-to now carries the id alongside the number). **Qualifier grammar (M6)**: a claim is `id (<qualifier>)`; qualifiers compare as normalized strings; an unqualified claim conflicts with every other claim of the id; two identical qualifiers fail. **The id harvest is scoped to `PRD.md`** — the design set declares fourteen `cpt-cf-bss-products-contract-*` ids of its own (counted: the prose said thirteen) (error taxonomies, `contract-rbac`, `contract-obligations`, `contract-sdk`, `contract-traceability`), and an unscoped `contract-*` glob would pull them into the requirement universe. **`usecase-*` ids join the universe** (seven §10 use cases; exactly one was cited by id anywhere before the branch review, though all seven have a substantive home). **And an AC-existence check**: every **unqualified** `AC #N` cited in a slice must resolve to a §12 bold-numbered acceptance-criterion item (`**N. <title>**`) of *this* PRD; a citation qualified by its gear (`pricing AC #82`, which this set carries ten times) resolves against that gear or is out of scope — added, the unscoped rule was red on day one on ten correct citations — a one-line regex that catches nothing today, 04's seven `AC #82` citations having been qualified but guards the class that five sites attributing pricing AC #82 a `When` it does not have belongs to, which no lint in this set could see. **What this lint still cannot do, stated so nobody reads it as more:** it checks that a claim is *well-formed and unique*, never that the slice covers the requirement, and it says nothing about whether what a slice asserts *about* a cited AC is true. Adopting the grammar requires one sweep of the existing Traces-to lines — owed with the lint's build - `inst-cc-fr`

   *Deliberate divergence, stated: fourteen requirements are owned by **two**
   slices each, so the generic one-slice reading — `spec-check`'s `P2/fr-multiply-claimed` —
   reports all fourteen. That is expected here and is not a defect to sweep. Split ownership
   is legal in this set **when every claim carries a scope qualifier**, which is what the
   qualifier grammar below enforces and what all fourteen — thirteen pairs and one triple, `nfr-scale-extensibility` claimed by `01`, `02` and `06` (**P-D-130**, 2026-09-03) — now carry. The invariant is
   one owner per **clause**, not one slice per id.*
2. [ ] - `p1` - **AC #38 map**: every enumerated failure row **that a registry door can refuse** maps to exactly one error code with **exactly one declaring *slice*** — a slice, not a door and not a pipeline phase (**P-D-36**: the phase unit **P-D-24** introduced was this set's own invention, carried contradictions no other gear has — the donor's `ValidationPipeline` has no stage concept at all — and is withdrawn. The door unit it had replaced was red by construction on every multi-door code; the slice unit is not, a code having exactly one declaring slice by construction, fixed by **P-D-35**'s rule. 01 §3.1's seven phases remain the execution order and are no longer a taxonomy). **The map gains three codes and a status class from the round (P-D-25)**: `DUPLICATE_CODE` (renamed from `DUPLICATE_SKU_CODE`, one code covering both the `skuCode` and `productCode` reservations), `ENTITY_TERMINAL` (409 — **any head write on a `retired`/`discarded` row**, save, publish or correction alike, 01 §3.3 / **P-D-32**) and `AUDIT_UNAVAILABLE` (**503** — the refusal's audit row could not be written), the third 503 in the gear beside 08's `READ_MODEL_OVERLOADED` and 03's `USAGE_TYPE_UNAVAILABLE`. **Three of the fifteen rows are explicitly outside the universe — the retention-orphan alarm (10 `inst-rt-gc`), the `compositionPending` adoption duty (a consumer obligation with no registry door) and AC #38's post-v1 EOL row, whose only candidate code refuses the feature rather than the named condition — named here so the lint is buildable rather than perpetually red** (item 32 of the review): the retention-orphan row is deliberately an **alarm**, not an API error (10 `inst-rt-gc`), and "adopting a `compositionPending` bundle" is a **consumer duty with no registry door** (§2.2). The lint asserts the exclusion list is exactly those three — an unexplained fourth exclusion fails it - `inst-cc-errors`
3. [ ] - `p1` - **Door×grant pairing**: every declared route appears in the `Doors` column of 05 §3.2's RBAC catalog. **The population is the declared routes** (**P-D-45**): the fourteen `` `METHOD /bss-products/v1/…` `` code spans, one machine-readable form. Doors named only in prose — "the fresh-zero door", "the watermark door", "the correction door" — are **outside** it, because a prose name has no enumerable form; that most grants carry no declared route is registered as a gap in 05 §6 rather than treated as a lint failure, the stated direction being door ⇒ grant - `inst-cc-rbac`
4. [ ] - `p1` - **Event bookkeeping**: every row of the **`EventRegister`** names its emitting instruction, and every instruction the register names carries the event on its own row; 09's coalesced summary event is **additive** (row-level domain events all emit — the H1-corrected rule). **The register is authored, never harvested** (**P-D-45**), and the reason is measured rather than asserted: five harvest passes over the same tree returned five different populations — **31, 24, 32 and 35** events under four patterns, one pass inventing donor-gear events (`PlanPublished`, `BundleCompositionCompleted`) and another dropping real ones (`SkuCreated`, `SkuRetired`) on a name-length filter, while the instruction attribution differed in 28 of 31 rows. This is lint 9's `Operand` lesson at a larger scale: an emitting instruction is not recoverable from prose, so each slice's owner writes their own rows and the lint reads only the table. **Authoring it is owed, per slice** (§6) - `inst-cc-events`
5. [ ] - `p2` - **Register hygiene**: every P-D **`Propagated`** list names only documents that restate the decision, and every restating document appears (the L2-class lint). **The grammar the lint reads** (**P-D-43**): the register carries **one** propagation field, spelled `- **Propagated**`; it names each document by its **repo-relative path** (`design/12-consumer-contracts.md`), never by an `S<NN>` abbreviation; and a document **restates** a decision exactly when it **cites the decision id**. That definition is deliberately the mechanical one, and the owner's round recorded its cost: a document can cite an id without carrying the claim, which is the blindness measured on **P-D-35** (10 cites it elsewhere, for a different clause, while the claim it was taken to settle never landed). The lint does not see that difference and is not meant to - `inst-cc-register`
6. [ ] - `p1` - **Id uniqueness**: every `inst-*` id is **declared** exactly once across the design set, where a **declaration** is the id trailing its own numbered instruction row and a **re-use** is any other mention (H1's live violator — the 08/12 `inst-rp-bootstrap` collision — was fixed in the same commit that added this lint). **The grammar is what makes it buildable** (item 32 of the review): `design/01-foundation.md` legitimately carries `inst-fd-create-txn`, `inst-fd-etag`, `inst-fd-publish-pin` and `inst-fd-name-unique` on two rows each and `inst-fd-idempotency` on six (recounted under this lint's own grammar: one declaration plus its `(cont. …)` markers), where each further row continues the same instruction — those rows carry the id **parenthesized** (`(cont. inst-fd-etag)`), the same device lint 1's qualifier grammar uses, so a continuation can never read as a duplicate declaration. **The domain is `inst-*` alone** (**P-D-43**): `cpt-*`/`flow` ids are declared on unnumbered bullets and an actor id is an Actors-table cell, so under the stated grammar both kinds had **zero** declarations and the lint was red on a correct set by construction — and an actor legitimately appears in every slice it acts in (`cpt-cf-bss-products-actor-plan-price` in five), while the set states no notion of an actor's owning slice to make "once" meaningful. Donor-gear ids are outside it as well, and now outside the set: P-D-43 struck every `inst-*` citation of the donor gear in favour of prose, so "across the design set" needs no cross-gear arm - `inst-cc-ids`
7. [ ] - `p1` - **Identity materialization**: no table or projection other than 10's `IdentityRefMap` stores an operator identity — the lint 10's erasure guarantee names (H2: it existed only as a citation until here). **The lint reads column names** (**P-D-45**): any column holding an operator identity is named `*_actor_ref`, which is the convention 10's own `products_identity_ref` already follows, and the lint asserts exactly one table declares such a column. **Its stated weakness**: a column named otherwise passes silently, so the lint is green over the very defect it exists to catch. It is a naming discipline enforced at review, not a proof; the alternative priced against it was an explicit `identity:` field on all 34 table declarations - `inst-cc-identity`
8. [ ] - `p2` - **No monetization marker** (AC #37): a registry schema surface matching §17.2's **first** left-column row (`flat, per-seat, tiered, volume, hybrid, commitment`) fails the lint — the `usage` row is excluded, AC #37 saying only `usage` leaves a footprint here — the §3.1 prose now has its buildable artifact (M1). **"Registry schema surface" means the table and column declarations of the slices' §4 sections** (**P-D-45**) — the six words being a fixed literal list, this lint needs a definition rather than an artifact and is executable as it stands. **Its stated blind spot**: a monetization marker arriving as an SDK field or an event payload rather than a column is outside §4, so the lint would be green while the footprint exists; widening it waits on the SDK shapes being declared structurally - `inst-cc-monetization`
9. [ ] - `p1` - **Obligation×pin coupling** (P-D-12): every §2.2 `ObligationRegister` row whose guard reads a **catalog field** has that field in the `SchemaPin` — a row whose operand is an **event payload** (the P-D-20 `SkuRetired` row) is outside the pin by construction, since the pin covers the entity surface and not the event surface, and is excluded by that reason rather than by omission (added) — and every pinned field is either an obligation operand or carries a recorded exclusion reason. **The register carries an explicit `Operand` column and the lint reads only that** (item 32 of the review — reading operands out of the rows' prose yielded ten fields against the pin's eight, and left `PlanTier` pinned with no register row naming it: `PlanTier`'s operand row is the tier-divergence obligation, now stated as such). **The cell's value grammar** (**P-D-43**, amended by **P-D-63**): **one token per pin member**, comma-separated, each either a catalog field name or one of three **non-field markers** — `(surface)` where the operand is a whole surface rather than a field, `none in v1` where the row has no operand yet, and `payload` for an event-payload operand — **and a non-field marker may be preceded by exactly one backticked identifier, which the marker consumes as its annotation** (P-D-63: `` `CatalogVersion` (surface) `` is one token; the identifier names the surface or payload and is never looked up in the pin). **`+` is not a separator**: the four cells that used it were normalized to comma-separated pin tokens rather than the grammar widened to a typographic habit. **Tokens are written backticked, and a backticked catalog field name is a token, never the ignorable prose** (P-D-63); a cell whose only token is `none in v1` is outside the coupling population by the marker rule. The lint reads **tokens only**: a field token must appear in the `SchemaPin`; a marker token is outside the **field-comparison population** by construction (**P-D-65** narrowed this from "outside the pin": a `` `CatalogVersion` (surface) `` token couples to the pin's `surface` entry in both directions, while `payload` and `none in v1` couple to nothing) and carries its exclusion reason in the row's prose. Any prose beside the tokens is ignored, so a cell is never judged by being read. This is what makes C1's membership a rule rather than a list — the FR it derives from previously stated three obligations (`deprecated` adoption, `compositionPending` adoption, usage binding) while pinning none of their operands, and no lint could see it - `inst-cc-pin`

## 4. Data / Storage

None owned as tables — this slice's artifacts are the SDK crate, the fixture crates, the
`SchemaPin` file, the AC #38 map below, and the lints. That absence of storage is by design.

**The named artifacts** (**P-D-44**):

| Artifact | Where |
|---|---|
| SDK crate | `products-sdk` (`cf-gears-bss-products-sdk`), per `DESIGN.md` |
| Fixture corpus | **`cf-gears-bss-fixtures`** at `gears/bss/fixtures/bss-fixtures/` — it already exists and the donor gear already depends on it; its own description names it "the only fixture crate a gear may take as a production dependency" |
| Fixture harness | **`cf-gears-bss-fixtures-conformance`** — runners and evaluator traits, a **dev-dependency only**, as pricing takes it |
| `SchemaPin` | **`gears/bss/products/products-sdk/schema-pin.toml`**, versioned with the SDK as §3.2 requires; TOML so a gate reads it without parsing prose |

### 4.1 The AC #38 row → code map

Lint 2's input set. The PRD enumerates **fifteen** rows; **twelve** are refusable by a registry
door and carry a code, and **three** are outside the universe for the reasons lint 2 states.

| # | AC #38 row | Code | Declaring slice |
|---|---|---|---|
| 1 | stale-revision write | `STALE_REVISION` | 01 |
| 2 | duplicate idempotency key with different body | `IDEMPOTENCY_CONFLICT` | 01 |
| 3 | taxonomy cycle | `TAXONOMY_CYCLE` | 02 |
| 4 | unrecognized metering unit without elevation | `UNRECOGNIZED_UNIT` | 03 |
| 5 | publish of an incomplete entity | `INCOMPLETE_ENTITY` | 01 |
| 6 | immutable-field change without a valid correction path | `ILLEGAL_FIELD_MUTATION` | 01 |
| 7 | reissue of a reserved `skuCode` and concurrent collision | `DUPLICATE_CODE` | 01 |
| 8 | EOL without an acknowledged migration consumer (post-v1) | **excluded** | — |
| 9 | publishing a SKU under a non-`published` parent | `PARENT_NOT_PUBLISHED` | 04 |
| 10 | a SKU scope falling outside its parent | `SCOPE_NOT_CONTAINED` | 04 |
| 11 | authoring/cloning against a **de-listed** unit | `UNRECOGNIZED_UNIT` | 03 |
| 12 | authoring/cloning against a **deprecated** unit | `UNIT_DEPRECATED` | 03 |
| 13 | a bulk row whose in-batch dependency failed | `BULK_DEPENDENCY_FAILED` | 09 |
| 14 | adopting a `compositionPending` bundle | **excluded** | — |
| 15 | retention orphaning a live grandfathered reference | **excluded** | — |

Three notes the map carries rather than hides:

- **Row 11's code holds under P-D-47, which closed the 03 question it rested on** — whether a
  `RecognizedSet` removal is a physical DELETE or a third state. It is a third state, and 03 §3.1
  defines the set as its `active` and `deprecated` rows, so a de-listed unit is outside the set and
  `inst-mt-recognized` refuses it `UNRECOGNIZED_UNIT` on either engine. The cell was re-read against
  that definition, as the earlier form of this note required.
- **Rows 4 and 11 share a code, and rows 9 and 10 do not.** Lint 2 requires one code **per row**,
  not one row per code, so a shared code passes; it is recorded here so a reader does not take the
  repetition for an editing slip.
- **The row count held at fifteen across P-D-44 while its membership changed** — the old
  "indeterminate parent-child region-containment" row was withdrawn as unreachable and the
  "de-listed/deprecated" row split in two. Any future citation of "fifteen rows" should be checked
  against this table rather than against the number.

## 5. Testing posture (slice-local)

This slice IS testing posture; its own meta-probes: the `SchemaPin` divergence RED (mutate a
pinned field one-sided — both CIs must fail); the vN→vN+1 RED (remove a default — the compat
test must fail); the bootstrap RED (age a checkpoint past the tail — loud failure, named
remedy); one OWED row flipped to asserted end-to-end (the watermark fixture, first — it is the
P-D-03 joint build's acceptance).

## 6. Traces to / Risks & Open items

**Traces to**: `cpt-cf-bss-products-fr-plan-price-seam`, `cpt-cf-bss-products-fr-event-versioning-replay`,
`cpt-cf-bss-products-fr-monetization-traceability`; AC #29, #36, #37; **NFR #9 `cpt-cf-bss-products-nfr-backward-compatible-evolution`** by id. **§9's seven ids are claimed by their owning slices, not here** — `interface-authoring-publish` + `contract-registry-events` → 01, `interface-read-model` → 08, `contract-sku-reference-count` → 07, `contract-increment-request` + `contract-freeze-ack` + `contract-bundle-composition-signal` → 06. This slice specifies the *suite* over that surface, which is not a coverage claim (branch review: the ids were claimed nowhere at all, so lint 1 would have reported zero for all seven on its first run); the
"CI-verified once the suite exists" phrases of `cpt-cf-bss-products-fr-deprecation` (the consumer-side adoption-guard duty only)/`cpt-cf-bss-products-fr-freeze-atomicity` (the consumer obligation half) — this
slice is that suite's specification.

**Risks & open items**:
- **The `EventRegister` is declared and empty.** P-D-45 made lint 4 read a table that does not yet have rows, and the measurement that forced it (five harvests, five populations) is also the reason nobody can fill it in one pass: an event's emitting instruction is only known to whoever wrote the rule. Each slice owes its own rows — event, emitting `inst-*`, and an explicit no-event where a state change emits nothing. Until it is written lint 4 is declared and inert, which is a better state than prose but is not a working gate. Owner: every slice owner, coordinated by this one. *(Raised by the P-D-45 round.)*
- ~~**The suite's final owner/home is a §15 open**~~ **Answered (P-D-132, 2026-09-03): no CI job, by the owner's decision** — the products-side crate, run on demand. *The item's text stood as:* (proposed `api-contracts` CI) — the design is
  home-agnostic, but an unowned CI job is an unrun one; this is the set's last
  organizational dependency.
- **Most obligations are OWED** by construction (C4): the register makes the debt legible, and
  the P-D-03 watermark fixture is deliberately first — it unblocks retirement, the highest-value
  seam.
- ~~**Event-log retention ≥ bootstrap gap**~~ **Answered (P-D-130, 2026-09-03): the broker's retention bounds the gap**; an older checkpoint rebuilds from the store. *The item's text stood as:* needs its number (§15) before the replay contract is
  more than words; named as the replay contract's single config dependency.
- ~~**`inst-cc-events` lints per instruction row, against P-D-34's act unit.**~~ **Answered (P-D-130, 2026-09-03): the unit is the act** — inheriting rows lint as one. *The item's text stood as:* **P-D-34** makes the
  event-declaration unit the *act*: a step inside a transaction whose event another row of that
  transaction names inherits the declaration. This row still lints per instruction *row*, so 01's
  `inst-fd-publish-freeze`, `inst-fd-publish-correction` and `inst-fd-publish-bump` — which inherit
  `inst-fd-publish-emit`'s declaration — are red by construction on a correct document. Owner: this
  slice. *(Filed from 01 §6 by the slice-01 eighth lens pass — the pointer claimed it was registered here and it was not.)*
- ~~**Does this slice owe an open-item reciprocity lint?**~~ **Answered (P-D-130, 2026-09-03): yes, a tenth lint**, declared and unenforced like the nine. *The item's text stood as:* Design 01 §6 used to restate its outbound
  questions as bullets claiming each was "registered where its owner will look". That claim was
  measured twice and was false both times — the eighth pass found four of six named documents
  unfiled and filed the headline item of each; the P-D-43…49 propagation audit found five
  sub-items its repair had missed. 01 no longer restates them, which removes the drift but not the
  gap: the lint set here checks ids, codes, events and doors, never open-item reciprocity, so
  nothing catches a question filed nowhere. Owner: the design-set owner with this slice.
  *(Filed from design 01 §6 by the slice-01 eighth lens pass; re-measured by the P-D-43…49
  propagation audit.)*
- ~~**The `CoverageChecks` are gated by nothing, and there is nothing to restore.**~~ **Answered (P-D-134, 2026-09-04): no CI gate, by the owner's decision (P-D-132).** *The item's text stood as:* *(P-D-130, 2026-09-03: the `spec-check` skill runs a subset off-VCS; the CI gate stays repo tooling's.)* This item
  previously read "restoring the job is owed", which was false in its premise and is corrected
  here. The `Spec Invariants` job was not lost: commit `21a149fda` removed it deliberately
  along with `tools/spec-check`, the workspace member and `make spec-check`, and stated the
  cost in as many words — "the design documents go back to being validated by nothing
  automatically… That property is knowingly given up; the mitigation is that a forgotten or
  permanently-red gate provides no real protection either." The in-repo tool no longer exists,
  so enforcing these nine lints is **building** something, not restoring it, and that is repo
  tooling work rather than anything this design set can decide. What this slice owes is only
  the honest statement that its checks are declared and unenforced — which §3.2 makes.
  Owner: whoever owns repo tooling, if and when the cost recorded in `21a149fda` is
  reconsidered. *(Premise corrected after the P-D-45 round.)*
- ~~**Does the pin run as one CI job or once per gear?**~~ **Answered (P-D-130, 2026-09-03): one CI job over the fixtures crate**, on which both gears depend. *The item's text stood as:* §2.1 says "one CI job over
  `cf-gears-bss-fixtures`"; §5's probe says "both CIs must fail"; and one job cannot be the other side's CI, with
  both gears in one repository. Separately, `.github/workflows/api_contracts.yml` already exists
  under the proposed name with an unrelated purpose and triggers that never include a fixture
  crate. Owner: the `PRD` §15 owner.
- ~~**Two authorability criteria are in force.**~~ **Answered (P-D-130, 2026-09-03): C4's binds** — instruction-id citations re-key to ACs. *The item's text stood as:* C4 makes an assertion authorable "once the
  referenced counterpart AC exists"; two register rows are marked owed on a different test — the
  counterpart raises no code — and their Source cells cite pricing *instruction* ids, whose
  counterparts do exist. The two disagree on at least two live rows. Owner: this slice with the
  plan-price owner.
- ~~**What does "unqualified" mean in the AC-existence check?**~~ **Answered (P-D-130, 2026-09-03): the sentence-context reading**. *The item's text stood as:* Under an adjacency reading, four
  slice-04 sites and one here are violations and a sweep is owed; under a sentence-context reading
  they are correct and the "one-line regex" the row names cannot implement the rule. The qualifier
  grammar governs Traces-to claims, not AC citations. Owner: this slice.
- ~~**The approval-queue envelope is asserted here and owed in 05.**~~ **Answered (P-D-130, 2026-09-03): the sixth fixture of `dod-joint-fixtures`**. *The item's text stood as:* `inst-sdk-inbox` says a
  field-name drift "fails the suite", but the check is in neither the fixture roster nor the
  register, and 05 records the cross-check as future work — while C4 forbids exactly that ("an
  unauthorable assertion stays listed as OWED, never silently dropped"). Owner: this slice with 05.
- ~~**Should `P-D-01`, `P-D-03` and `P-D-05` name this slice?**~~ **Answered (P-D-130, 2026-09-03): no** — a propagation field names filings, not citers (P-D-128). *The item's text stood as:* This slice restates all three in its
  constraint rows and register, and their propagation fields do not name it — and so do eight more
  (`P-D-24`, `P-D-25`, `P-D-26`, `P-D-27`, `P-D-29`, `P-D-34`, `P-D-35`, `P-D-43`): eleven of the
  twenty-three decisions this slice cites are absent from their own `Propagated` lists. The gap is
  set-wide rather than this slice's — **62 of the 179 (slice, decision) citation pairs** in the set
  sit outside the cited decision's list. Whether a constraint-row citation counts as a restatement for lint 5 is
  unstated; the fix lands in the register, not here. Owner: the register's owner.
- ~~**Is `inst-cc-errors`' exclusion list one filter or two?**~~ **Answered (P-D-130, 2026-09-03): one filter** — the opening clause defines the universe. *The item's text stood as:* Two of the three exclusions are
  already excluded by the opening clause ("that a registry door can refuse"); the third is
  excluded for a reason that clause does not express. The "exactly three" assertion is checkable
  only once one filter defines the universe. Owner: the error-contract owner.
- ~~**Lint 3's population is not in one machine-readable form.**~~ **Answered (P-D-130, 2026-09-03): the normalisation is the lint's** — `\|` → `|`. *The item's text stood as:* **P-D-45** arm 1 defines it as
  `` `METHOD /bss-products/v1/…` `` code spans, "one machine-readable form". At HEAD all seven
  pipe-bearing routes exist in **two** textual forms: `{products\|skus}` inside 05 §3.2's table,
  where a markdown cell must escape the pipe, and `{products|skus}` everywhere else — 01, 02, 08,
  11, and 05's own §6. A lint matching the spans literally pairs none of the seven across that
  boundary: it would read all fourteen `Doors` entries as undeclared and all seven outside
  declarations as un-doored. The table cannot drop the escape without breaking the cell, so the
  normalization belongs in the lint's grammar, and no document states it. Owner: this slice with
  05. *(Raised by the P-D-43…49 propagation audit.)*
- ~~**Lint 9's `Operand` grammar does not describe the cells it reads.**~~
  **Answered (owner call, 2026-08-31 — P-D-63, amending P-D-43 arm 3): one production added, one
  separator refused.** A non-field marker may be preceded by exactly one backticked identifier, which
  it consumes as its annotation — so the six marker-led cells are one token each and the lint never
  looks the identifier up in the pin. `+` is not admitted; the cells that used it (**four at HEAD, on
  a register of fourteen rows** — this item's thirteen and three were both stale) were normalized to
  comma-separated pin tokens: `` `status` `` alone (the vocabulary is part of that pin member's
  definition), and the metering phrases spelled as `` `unit`, `usageTypeRef` ``. **After both, all
  fourteen cells parse.** Original text: **P-D-43** arm 3 fixes the
  cell as "one token per pin member, comma-separated, each either a catalog field name or one of
  three non-field markers". At HEAD three of the thirteen §2.2 cells fit that grammar
  (`compositionPending`, `sellable`, `skuId`). Six lead with a backticked non-field token — five
  `` `CatalogVersion` (surface) `` and one `` `SkuRetired` payload `` — formally indistinguishable
  from a catalog field name, so a token-reading lint looks for `CatalogVersion` in the `SchemaPin`
  and fails; three more join their operands with `+` rather than a comma. Arm 3's "prose beside the
  tokens is ignored" may be meant to cover the leading token, but a backticked identifier is not
  prose under any form the grammar states. Owner: was this slice; **closed**.
  *(Raised by the P-D-43…49 propagation audit.)*
- ~~**Seven register entries carry two `Propagated` fields, and lint 5 says there is one.**~~ **Answered (P-D-130, 2026-09-03): the dated-amendment form is admitted** — lint 5 reads every field of an entry as one set. *The item's text stood as:* Lint 5's
  grammar (**P-D-43** arm 4) reads "the register carries **one** propagation field, spelled
  `- **Propagated**`". P-D-24 through P-D-30 each carry a base field plus a second dated
  *(owed until …, all closed)* field — 56 fields across 49 entries. Either the grammar admits the
  dated-amendment form or those seven merge; until it is settled, a reader taking the **last** field
  — the rule P-D-43's own entry forces, since its arm 4 quotes the literal field name in its body —
  silently drops the primary field for all seven. Owner: the register's owner.
  *(Raised by the P-D-43…49 propagation audit.)*
- ~~**No lint verifies that a free-text `reason` door registers the PII block.**~~ **Answered (P-D-130, 2026-09-03): an eleventh lint**, declared and unenforced. *The item's text stood as:* 02
  `inst-av-pii-reason` enumerates the doors that owe `inst-av-pii-block`, and says the enumeration
  *is* the registration — a slice that adds such a field "adds itself to the
  enumeration above; that is the whole registration". Nothing checks it. The nine lints here cover
  ids, codes, events, doors, operands and register hygiene — none covers PII-hook coverage, so a
  slice that adds a reason field and forgets the hook is caught by reading or not at all, and 02's
  own stated consequence is that personal data typed into such a field is unreachable by erasure
  forever. The class is not hypothetical: **P-D-50** had to wire five doors across 05 and 07 that
  had carried the debt as an open item instead. Owner: this slice with 02. *(Raised by CodeRabbit
  on PR #14, 2026-08-27; its first half — the unwired doors — was closed by P-D-50, this half was
  not.)*
- ~~**Does `inst-cc-errors` still lint against the phase unit?**~~ **Answered (P-D-130, 2026-09-03): no — the declaring unit is the slice** (P-D-36). *The item's text stood as:* **P-D-36** moved the declaring unit
  from the phase to the declaring slice, which retires the carve-out mirror this row was owed
  rather than paying it. This slice cites P-D-36 nowhere. Owner: this slice. *(Filed from 01 §6 by
  the P-D-43…49 propagation audit — the pointer claimed it was registered here and it was not.)*
- ~~**`ENTITY_TERMINAL`'s gloss widened and the AC #38 map was not re-read.**~~ **Answered (P-D-130, 2026-09-03): the row reads *any head write on a terminal head*** (P-D-32); the §4.1 edit is owed. *The item's text stood as:* **P-D-32** widened it
  from a save on a `retired`/`discarded` head (**P-D-25**) to any head write — save, publish or
  correction. The map's rows were written against the narrower reading. Owner: this slice.
  *(Filed from 01 §6 by the P-D-43…49 propagation audit — the pointer claimed it was registered
  here and it was not.)*
- ~~**Is `inst-cc-ids`' continuation enumeration stale?**~~ **Answered (P-D-130, 2026-09-03): it becomes a rule, not a count**. *The item's text stood as:* Lint 6 names the ids 01 legitimately
  carries on more than one row and how many rows each takes. That enumeration is a count against
  another document, and nothing re-reads it when 01 changes. Owner: this slice. *(Filed from 01 §6
  by the P-D-43…49 propagation audit — the pointer claimed it was registered here and it was not.)*
- ~~**May 01 §4.2's `composition_pending` no-re-raise clause rest on P-D-14?**~~ **Answered (P-D-130, 2026-09-03): it rests on P-D-48 as cited**; citing obliges no entry (P-D-128). *The item's text stood as:* **P-D-48** confirmed
  the clause, but P-D-14's propagation field names 05, 06, `design/README.md`, `DESIGN.md` and the
  PRD — not `design/01-foundation.md`. Under lint 5's own grammar a document restates a decision
  exactly when it cites the id, so either the field gains 01 or the clause rests on something else.
  Owner: the register's owner. *(Filed from 01 §6 by the P-D-43…49 propagation audit.)*
