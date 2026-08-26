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
  (`fr-event-versioning-replay`), §9 (all six id-bearing blocks across §9.1/§9.2), AC #29, #36, #37;
  §15 (seam-suite owner/home — proposed `api-contracts` CI, final owner unassigned; event-log
  retention ≥ the bootstrap gap)
- [`../DECISIONS.md`](../DECISIONS.md) P-D-01 (envelope discipline the versioning rides),
  P-D-03 (the joint watermark fixture); every slice's "seam-suite" and "slice 12" pointers

### 1.5 Scope

**In**: the seam-suite specification (fixtures, home, pin mechanics); the consumer-obligation
register; event schema versioning + the replay/bootstrap contract; the SDK/§9 surfaces incl.
the studio-inbox envelope cross-check; the completeness checks; §17.2 traceability (AC #37).

**Out**: the counterparts' implementations (each obligation names its owing gear); the broker's
transport (Common Core); the §15 opens this slice can only *assert once closed* (freeze acks,
composition signal, watermark delivery).

### 1.6 Constraints & Assumptions

| # | Constraint | Source |
|---|-----------|--------|
| C1 | A shared schema-version pin over the joint fields — `skuId`, `bundle` type, the meter declaration pair, `PlanTier`, `CatalogVersion` — with a CI test that **fails on divergence**; a runtime divergence fails closed (the dependent plan publish is rejected, pricing-side) | PRD `fr-plan-price-seam` |
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
3. [ ] - `p1` - Joint fixtures (grown per C4): the P-D-03 **watermark fixture** (pricing produces, registry's predicate answers — the retirement joint contract end-to-end); the **adoption-block fixture** (pricing AC #82: `SkuDeprecated` ⇒ referencing plans flagged, new adoption refused); the **usage-binding fixture** (pricing `inst-cmp-usagetype` against a registry declaration, incl. the deprecated-bound-unit reject/warn arm — M2); the **grandfathered-resolution fixture** (a frozen snapshot resolves byte-identically after registry churn); the **correction fixture** (`SkuImmutableFieldCorrected` ⇒ pricing re-validates) - `inst-ss-fixtures`

### The consumer-obligation register

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-obligations`

| Obligation | Owing consumer | Source | Status |
|------------|----------------|--------|--------|
| Declare intent (`browse` vs `posted`) on resolution | every resolver | 06 `inst-rv-intent` | assertable now (registry-side refusal is built into the API) |
| Refuse adoption of `deprecated` SKUs | pricing | PRD AC #36; pricing AC #82 (its trigger is the retirement path's deprecation signal) + pricing PRD §15 row | **owed** (authorable — the counterpart AC exists) |
| Refuse adoption of `compositionPending` SKUs | pricing | PRD AC #36 | **owed — NOT yet authorable** (M3: pricing's PRD carries neither the flag nor the signal; tied to the §15 `BundleCompositionCompleted` open, per C4 it stays listed until that lands) |
| Enforce `sellable = false` as sellability-gate predicate 6 for standalone lines | pricing | D-46 (built) | **assertable now** (M7) |
| Re-validate on registry tier/meter divergence (`tier_divergent`/`meter_binding_divergent`) | pricing | pricing `inst-cmp-tier-drift`/`inst-cmp-usagetype` (built) | **assertable now** (M7) |
| Usage-binding checks (unbound meter; priced dimension ⊆ `metadata_fields`; **reject/warn on a `deprecated` bound unit** — M2, the FR's sixth clause restored) | pricing | P-D-05, pricing `inst-cmp-usagetype` | **owed** (pricing side built; joint fixture) |
| Resolve grandfathered refs against the frozen snapshot | pricing / subscriptions | 06 `inst-gf-invariant` | **owed** |
| Re-validate on `SkuImmutableFieldCorrected` | pricing | 07 `inst-cr-republish` | **owed** |
| Act on the surfaced binding diff `(boundVersion, resolvedVersion, diffRef)` | freeze participants | 06 `inst-sn-binding-diff` | **owed** |
| Refuse `not_frozen(forced)` participants' content for posted use | pricing / Billing | 06 `inst-fz-force` | **owed** (Billing has no gear — §15) |
| Produce the `SkuReferenceCount` watermark | pricing (v1), then subscriptions/contracts | P-D-03, 07 | **owed — the P-D-03 joint build** |
| **Release** frozen versions when references end (`catalog_version × release`) | every freeze participant | 06 `inst-fz-liveness` (H1 of the 10 review) | **owed — without releases, retention never fires (conservative, alarmed)** |
| Consume `mustMigrateBy` | subscriptions | 04 EOL lockout | **deferred with post-v1 EOL** |

Every row is either a fixture in the suite or an explicitly OWED line — the register is the
suite's backlog, reviewed whenever a counterpart lands an AC (C4).

### Event versioning, replay & bootstrap

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-replay`

1. [ ] - `p1` - Every event schema is a versioned artifact in `products-sdk`; the CI compatibility test runs C2's actual direction — **an old (`vN`) consumer deserializing a `vN+1` payload** carrying the new optional fields with defaults (the reverse direction, new code reading old fixtures, is the trivial half and is also asserted) on every schema change — the 04 EOL field (`mustMigrateBy` present-but-unpopulated) is the standing example; **the 09 export-artifact schema joins the same corpus** (L3 — the discipline it cites is now exercised, not borrowed) - `inst-rc-compat`
2. [ ] - `p1` - Dedup/ordering detection beyond the idempotency window rides `(tenant, aggregate, sequence)` (01's outbox keys) — the consumer contract states it and the suite fixtures a duplicate + an out-of-order delivery - `inst-rc-dedup`
3. [ ] - `p1` - Bootstrap (C3): a published-scope consumer initializes from the latest `CatalogVersion` (06's resolver, `browse` intent) + the event tail from that version's instant; a checkpoint older than the retained tail fails loudly with the named remedy (re-bootstrap) — the same contract 08's projector obeys internally, so the gear's own projector is the contract's first consumer and its permanent conformance probe - `inst-rc-bootstrap` *(renamed from `inst-rp-bootstrap` — H1: it collided with 08's read-projection id, and the lint that should have caught it is #6 below)*

### The SDK / §9 surface

- [ ] `p1` - **ID**: `cpt-cf-bss-products-contract-sdk`

1. [ ] - `p1` - `products-sdk` mirrors §9: the authoring/publish client (idempotency keys + `If-Match` + intent semantics are **part of the contract**, breaking = major), the read-model client, the event payload types, the error-code enum (01 §3.3 + every slice's registered codes — renames breaking) - `inst-sdk-surface`
2. [ ] - `p1` - The catalog read shape is **`CatalogSku`-superset-compatible** (pricing's `ProductCatalogClientV1` trait consumes it — L2): `sku_id`, `sku_code`, `name`, `metering_unit`, `status`, `plan_tier` — plus `sellable`, `usage_type_ref`, `composition_pending`, `type` (the members pricing's copy lacks land consumer-side as additive). **The `status` wire vocabulary is pinned** (M4): browse serves `published|deprecated` only (draft never served, retired history-only — 08 C2); the SDK enum documents all five states with the wire subset named; pricing's opaque-string tolerance is a courtesy the pin replaces — `status` joins the `SchemaPin` as a **flagged design widening** beyond the FR's five fields - `inst-sdk-catalogsku`
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

Doc-plane lints over this design set + PRD (spec-check-class, run in CI with the docs job):

1. [ ] - `p1` - **Requirement coverage**: every `p1`/`p2` requirement-bearing PRD id — `fr-*`, `nfr-*`, `interface-*`, `contract-*` (M5: the universe is all six §9 blocks and §7, not FRs alone) — is claimed by exactly one slice's Traces-to. **Qualifier grammar (M6)**: a claim is `id (<qualifier>)`; qualifiers compare as normalized strings; an unqualified claim conflicts with every other claim of the id; two identical qualifiers fail. Adopting the grammar requires one sweep of the existing Traces-to lines — owed with the lint's build - `inst-cc-fr`
2. [ ] - `p1` - **AC #38 map**: every enumerated failure row maps to exactly one error code with exactly one raising door - `inst-cc-errors`
3. [ ] - `p1` - **Door×grant pairing**: every REST/S2S door named in any slice appears in 05's RBAC catalog - `inst-cc-rbac`
4. [ ] - `p1` - **Event bookkeeping**: every state-changing instruction names its event or an explicit no-event; 09's coalesced summary event is **additive** (row-level domain events all emit — the H1-corrected rule) - `inst-cc-events`
5. [ ] - `p2` - **Register hygiene**: every P-D propagation list names only documents that restate the decision, and every restating document appears (the L2-class lint) - `inst-cc-register`
6. [ ] - `p1` - **Id uniqueness**: every `cpt-*`/`flow`/`inst-*`/actor id is declared exactly once across the design set (H1's live violator — the 08/12 `inst-rp-bootstrap` collision — was fixed in the same commit that added this lint) - `inst-cc-ids`
7. [ ] - `p1` - **Identity materialization**: no table or projection other than 10's `IdentityRefMap` stores an operator identity — the lint 10's erasure guarantee names (H2: it existed only as a citation until here) - `inst-cc-identity`
8. [ ] - `p2` - **No monetization marker** (AC #37): a registry schema surface matching the §17.2 left column fails the lint — the §3.1 prose now has its buildable artifact (M1) - `inst-cc-monetization`

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

**Traces to (PRD)**: `fr-plan-price-seam`, `fr-event-versioning-replay`,
`fr-monetization-traceability`; AC #29, #36, #37; §9 (all six blocks); NFR #9; the
"CI-verified once the suite exists" phrases of `fr-deprecation`/`fr-freeze-atomicity` — this
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
- **SchemaPin widening owed a decision (L1)**: `sellable` and `compositionPending` are
  pricing-consumed operands of this suite's own fixtures yet sit outside the FR-inherited
  five-field pin (`status` was widened in, flagged); proposing the pair as a PRD `fr-plan-price-seam`
  amendment is the clean path — drift on either currently escapes the pin.
