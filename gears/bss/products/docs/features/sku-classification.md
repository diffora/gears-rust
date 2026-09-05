# Feature: SKU Classification

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-sku-classification-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-sku-classification`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [Type a SKU and evolve its classification](#type-a-sku-and-evolve-its-classification)
  - [Declare a metering unit](#declare-a-metering-unit)
  - [Govern the recognized-unit set](#govern-the-recognized-unit-set)
  - [Govern the PlanTier taxonomy and assign tiers](#govern-the-plantier-taxonomy-and-assign-tiers)
  - [Set accounting codes](#set-accounting-codes)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [RecognizedSet mechanics](#recognizedset-mechanics)
  - [The publish-time collector dependency](#the-publish-time-collector-dependency)
  - [Error taxonomy](#error-taxonomy)
- [4. States (CDSL)](#4-states-cdsl)
  - [RecognizedSet Member State Machine](#recognizedset-member-state-machine)
- [5. Definitions of Done](#5-definitions-of-done)
  - [Recognized-set table and its append-only guard](#recognized-set-table-and-its-append-only-guard)
  - [Classification columns and the meter pair CHECK](#classification-columns-and-the-meter-pair-check)
  - [Recognized-set mechanics](#recognized-set-mechanics)
  - [Seeded members](#seeded-members)
  - [Type profile validators](#type-profile-validators)
  - [Sellable](#sellable)
  - [Bundle override registration](#bundle-override-registration)
  - [Meter declaration atomicity](#meter-declaration-atomicity)
  - [Unit recognition](#unit-recognition)
  - [Usage type resolution](#usage-type-resolution)
  - [Resolved binding snapshot](#resolved-binding-snapshot)
  - [Meter bucket registration](#meter-bucket-registration)
  - [Unit de-listing guard](#unit-de-listing-guard)
  - [Unit semantic immutability](#unit-semantic-immutability)
  - [PlanTier taxonomy governance](#plantier-taxonomy-governance)
  - [PlanTier assignment](#plantier-assignment)
  - [Accounting code validators](#accounting-code-validators)
  - [Finance materiality at publish](#finance-materiality-at-publish)
  - [Mutability bucket registration](#mutability-bucket-registration)
  - [Classification error taxonomy](#classification-error-taxonomy)
  - [Recognized-set events](#recognized-set-events)
  - [SDK read shape](#sdk-read-shape)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Raised here rather than carried](#raised-here-rather-than-carried)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

This feature owns everything that makes a SKU **classified and downstream-bindable**, short of
any price: the type (`product` / `service` / `bundle`) with its per-type required-field sets, the
`sellable` offering-eligibility flag, the `PlanTier` taxonomy and a SKU's value in it, the stable
accounting codes (`taxCategory`, `glCode`) against their Finance-owned recognized sets, and the
**metering-unit declaration** — the one thing that makes a SKU a usage SKU — with its
`usageTypeRef` binding and the recognized-unit set's own governed lifecycle.

The three vocabularies it governs — the `PlanTier` taxonomy, the recognized units and the
recognized codes — are **governed live entities** reusing `02-taxonomy-attributes`'s
`GovernedLiveOp` pattern verbatim, under one generic `RecognizedSet` shape discriminated by
`set_kind`.

### 1.2 Purpose

Downstream binds without re-validation exactly when classification is enforced at authoring. An
unpostable SKU with a missing code, an unrateable meter against an unrecognized unit or a dangling
`usageTypeRef`, an unclassifiable plan against an unknown tier — each must be **impossible to
publish**, not discovered weeks later at ERP export or at rating time.

This feature owns **no approval machinery**: it registers gate conditions and spends what
`05-governance` approves. It owns **no composition**: a bundle's contents are plan-price's, and
this feature only registers the override that lets an uncomposed bundle publish at all.

**Requirements** — carried from [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.3 with its scoping
notes intact:

- Whole: `cpt-cf-bss-products-fr-sku-sellable`,
  `cpt-cf-bss-products-fr-plantier-classification`,
  `cpt-cf-bss-products-fr-metering-unit-declaration`,
  `cpt-cf-bss-products-fr-metering-unit-delisting`,
  `cpt-cf-bss-products-fr-accounting-codes`
- Scoped: `cpt-cf-bss-products-fr-define-sku` (typing and classification only; the identity
  clause is `01-foundation`'s)
- Surfaces — **claimed here and owed back to the entry**, which lists six `fr-` ids and no
  surface of its own: `cpt-cf-bss-products-usecase-product-sku-editor`,
  `cpt-cf-bss-products-contract-classification-errors`,
  `cpt-cf-bss-products-contract-registry-events`

**Principles**: `cpt-cf-bss-products-principle-registered-validators`.

**Constraints**: `cpt-cf-bss-products-constraint-no-commercial-concern`,
`cpt-cf-bss-products-constraint-broker-native-events`.

**Component**: `cpt-cf-bss-products-component-capability-handlers`.

**Sequence**: none of its own — this feature contributes validators to
`cpt-cf-bss-products-seq-authoring-publish`.

**Not applicable or delegated**, stated so a reader can tell considered-and-excluded from
forgotten: **authentication**, **observability** and outbox delivery are `01-foundation`'s;
**read performance**, caching and faceting are `08-read-models`'; **retention and erasure** are
`10-retention-erasure`'s; **PII** is not reachable here — this feature stores codes and
references, never operator free text, so `02-taxonomy-attributes`'s write-block hook has no call
site in it. **Authorization** is `01-foundation`'s frame and `05-governance`'s catalog: the grants
and routes of this feature's four vocabulary doors are open item 9, and tenant isolation is the
Foundation's `tenant_id`-leading keys, which this feature's table follows. **Operator-facing
message wording**, editor affordances and accessibility are the API and UI layers' — the fifteen
codes are the contract. **Rollout** is forward-only migration per `01-foundation` with no feature
flag; the one runtime knob is the resolver timeout, whose value is open item 12 and whose
configuration home is named nowhere. One latency statement is owed rather than delegated: `UsageTypeResolver` is the
gear's **only synchronous cross-gear call inside a publish pipeline**, its timeout has no number
(open item 12), and `08-read-models` and `12-consumer-contracts` are asked to surface resolver
latency in the publish SLO breakdown.

**Out of scope**, mirroring [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.3: bundle composition,
which is plan-price's; `compositionPending` clearing (`06-catalog-version`); the fresh-zero
correction door for bucket-ii fields (`07-reference-signal`); plan-side enforcement — tier
presence and the dimension subset — which is pricing's; usage collection, which is the
collector's; and the approval machinery (`05-governance`).

### 1.3 Actors

| Actor | Role in this feature |
|-------|----------------------|
| `cpt-cf-bss-products-actor-product-manager` | Types SKUs, declares meters, assigns tiers |
| `cpt-cf-bss-products-actor-finance-reviewer` | Owns the recognized code sets; second approver on finance-material fields |
| `cpt-cf-bss-products-actor-plan-price` | Consumes type, `sellable`, tier and meter through the SDK read shape |
| `cpt-cf-bss-products-actor-oss-metering` | The usage collector — system of record for the `UsageType` catalog `usageTypeRef` resolves against |

### 1.4 References

- [`../DECOMPOSITION.md`](../DECOMPOSITION.md) §2.3 — the entry this feature realizes
- [`../design/03-sku-classification.md`](../design/03-sku-classification.md) — the design slice.
  **This FEATURE is the declaration site of the five `flow-` ids and the three `algo-` ids**, and
  the slice's §2 and §3 point here for them; there is one definition site per id. Two of the three
  `algo-` ids moved here from the slice; the third,
  `cpt-cf-bss-products-algo-classification-errors`, is **minted here** because §3.2's code roster
  was the one process section carrying no id a FEATURE may define, and `design/03` §3.2 now points
  at it as its siblings do.
  **The slice's step lists remain the normative ones and are not copied here**: re-spelling the 23
  instruction steps it owns would fork the set's own instruction register and leave two texts
  where only one can be true. §2 and §3 carry the actor, the scenarios and the boundary; the steps
  stay at their single source.
  - **§5 restates `design/03` §4's storage shapes**, which is a deliberate second exception: a
    Definition of Done has to name the columns, the `CHECK` and the trigger whitelist it obliges,
    or it obliges nothing testable. **Where §5 and §4 differ on a column-level fact, §4 governs.**
  - **§4's state machine is a third exception, and its ids are this document's.** The template
    requires a step id per transition row, and `design/03` expresses the same content inside
    `inst-rs-shape` and `inst-rs-removal-operand` rather than as rows, so neither can be reused per
    row. The four `inst-rm-*` ids and `cpt-cf-bss-products-state-recognized-set` are declared here
    and cited by no slice — unlike the flows and algos, they have no reciprocal pointer, which is
    owed to `design/03`. **Where §4 and the slice differ on a rule, the slice governs.**
  - **`contract-` ids are cited but not defined here.** A FEATURE may **define** only `flow`,
    `algo`, `state`, `dod` and `featstatus` ids — plus the `inst-` steps of a state machine it
    declares, per the exception above — so this document defines none of the
    `contract-` ids the design slices declare. They remain freely **citable**: `artifacts.toml`
    registers `DESIGN_SLICE` with `pattern = "design/*.md"` and `traceability = "FULL"`, so
    `cfs where-defined` resolves every one of them to its slice.
  - **Eleven `inst-*` ids this slice cites are owned elsewhere** and are referenced, never
    claimed: `inst-fd-save-txn` and `inst-fd-publish-txn` (`01-foundation`); `inst-ad-governed`,
    `inst-ad-deprecate-then-remove` and `inst-gl-envelope` (`02-taxonomy-attributes`);
    `inst-ar-failure` (`04-lifecycle`); `inst-mt-inputs` (`05-governance`); `inst-cr-door` and
    `inst-cr-republish` (`07-reference-signal`); `inst-bk-override` (`09-bulk-promotion`);
    `inst-sdk-catalogsku` (`12-consumer-contracts`).
- **Dependencies**: `cpt-cf-bss-products-feature-foundation` and
  `cpt-cf-bss-products-feature-taxonomy-attributes` are the build-time dependencies — the latter
  because `GovernedLiveOp` is its exported contract and every vocabulary mutation here rides it.
  `05-governance`, `07-reference-signal` and `09-bulk-promotion` are **integration**
  dependencies: their design slices exist, their FEATURE artifacts and code do not.
- [`../PRD.md`](../PRD.md) §6.3 (all six FRs); §12 AC #2a, #7–#11, #38 (the unit rows); §15 (the
  recognized-set ownership questions); §17.1 (the seeded members)
- [`../DESIGN.md`](../DESIGN.md) §1.3 layering, §2.1 principles, §2.2 constraints
- [`../DECISIONS.md`](../DECISIONS.md) — P-D-02, P-D-05, P-D-11, P-D-13, P-D-21, P-D-22, P-D-23,
  P-D-30, P-D-41, P-D-43, P-D-44, P-D-47
- [`./foundation.md`](./foundation.md) — the doors, the pipeline and the outbox this feature
  registers into; [`./taxonomy-attributes.md`](./taxonomy-attributes.md) — the `GovernedLiveOp`
  envelope it reuses

## 2. Actor Flows (CDSL)

**Use cases**: `cpt-cf-bss-products-usecase-product-sku-editor`

The step lists live in [`../design/03-sku-classification.md`](../design/03-sku-classification.md)
§2 — see §1.4 for why they are not repeated. Each flow below names its actor, what success and
failure look like, and where its boundary runs.

### Type a SKU and evolve its classification

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-classify-sku`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- A SKU carries a `type` from the closed set, and the per-type required fields are enforced at
  publish: `product` and `service` require both accounting codes, `bundle` requires neither
- `sellable` defaults `true`; flipping it is a head-row save re-published as version N+1, and the
  SDK read shape exposes it per `CatalogVersion` so pricing's sellability predicate has its
  operand
- A promotional, zero-price or "free" offering is an ordinary SKU — no separate entity and no
  special validator path
- Publishing an uncomposed `bundle` succeeds once the two-person override is acknowledged at that
  entity publish, and the published SKU enters flagged `compositionPending = true`

**Error Scenarios**:
- `type` absent or outside the closed set — `SKU_TYPE_UNKNOWN` (**which arm of that code an
  absent `type` meets is open item 13**: if `type` is required at create, the shape phase raises
  `VALIDATION` first and the "absent" arm is unreachable)
- A `product` or `service` published without an accounting code — `ACCOUNTING_CODE_REQUIRED`,
  naming the missing one
- An uncomposed `bundle` published without the acknowledgment — `BUNDLE_OVERRIDE_REQUIRED`; the
  bulk lane's analogue is `09-bulk-promotion`'s `BULK_OVERRIDE_UNACKNOWLEDGED`

**Boundary**: this feature registers the override's **gate condition**; `05-governance` executes
the ceremony and `01-foundation`'s publish door reads the acknowledgment to set
`composition_pending`, being unable to judge composition itself. The condition is registered **on
the publish, not on the lane**, so every lane that publishes a bundle carries it. What operand
distinguishes a composed bundle from an uncomposed one is **open item 20**, and read literally
today an ordinary re-publish of a composed bundle would demand the override again.

### Declare a metering unit

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-declare-meter`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- A `MeterDeclaration` is atomic — `unit` and `usageTypeRef` together or not at all — and exactly
  one unit is declared; a composite meter declares its **output** unit
- Declaring a unit is what makes a SKU a usage SKU: the property is **defined, not detected**, and
  no separate flag exists
- At publish, `usageTypeRef` resolves in the collector's platform-global catalog — resolvability
  only, with no lifecycle and no dimension check
- The draft plane edits the declaration freely through the Foundation's save transaction

**Error Scenarios**:
- One of the pair present without the other — `METER_DECLARATION_INCOMPLETE`
- The unit is unknown, or `removed` and therefore outside the set — `UNRECOGNIZED_UNIT`; the path
  to a new unit is an elevated `RecognizedSet` approval, never an inline mint
- The unit is `deprecated` — `UNIT_DEPRECATED`, including a draft whose unit was deprecated before
  its first publish, which counts as a new declaration
- `usageTypeRef` does not resolve — `USAGE_TYPE_UNRESOLVED`
- The collector is unreachable — `USAGE_TYPE_UNAVAILABLE`, **fail-closed and retryable**; a
  publish never proceeds on an unverified binding

**Boundary**: the declaration is **bucket ii** — immutable after publish, correctable only
through `07-reference-signal`'s correction door. Dimension sets are plan-price's, never this
feature's. Whether the recognized-and-active check re-runs on every later publish, and what
distinguishes a new declaration from one carried forward, is **open item 8** — as written,
deprecating a unit freezes every SKU carrying it against any further publish.

### Govern the recognized-unit set

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-unit-set`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- The set is seeded per PRD §17.1 with `vCPU-hours`, `GB-storage`, `GB-egress` and
  `request-count`, and adding a member takes an **elevated** approval
- De-listing runs `active → deprecated → removed`: deprecation blocks new declarations and leaves
  existing publishes untouched; removal follows once no non-terminal published head references the
  member
- A removal is the `removed` **state**, never a `DELETE`, so a member a published row names keeps
  existing and the primary key never frees
- De-listing and deprecation never mutate any frozen snapshot

**Error Scenarios**:
- A removal while a `published` or `deprecated` SKU declares the unit — `UNIT_DELIST_BLOCKED`,
  with the holders sampled
- A removal of a seeded member — refused, and **which code it carries is open item 18**: all three
  de-list codes are predicated on holders, so none fits a seeded member with none

**Boundary**: **unit semantics are immutable, and the absence of the door is the enforcement** —
there is no rename or redefine op on this set at all. A correction is a new unit plus a
deprecation, and the audit trail ties the two through the `GovernedLiveOp` payload. Which door
writes the table, at what path and under what grant, is **open item 9**.

### Govern the PlanTier taxonomy and assign tiers

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-plantier`

**Actor**: `cpt-cf-bss-products-actor-product-manager`

**Success Scenarios**:
- Tier identity is the **stable code**; a rename touches the display label only, and is
  display-only by construction because the code column has no update path
- Taxonomy ops — add, rename, deprecate, retire — ride `GovernedLiveOp` under elevated approval
  and emit `PlanTierUpdated`
- A SKU's tier value validates against the taxonomy at save **and** at publish

**Error Scenarios**:
- An unknown tier — `PLAN_TIER_UNKNOWN`
- A `deprecated` tier on a **new** assignment — `PLAN_TIER_DEPRECATED`; existing published
  carriers are unaffected
- Retiring a tier a non-terminal published head still carries — `PLAN_TIER_RETIRE_BLOCKED`

**Boundary**: the SKU tier value is **bucket iii** — material-mutable and finance-material, so a
FinanceReviewer sits in the approval. Tier **presence** enforcement at plan publish is pricing's
and is not re-checked here. Whether a display-label rename is material is **open item 17**: this
feature makes it display-only, `05-governance` registers the taxonomy ops as material without
excepting it, and `02-taxonomy-attributes` calls the identical edit on its own vocabulary
non-material.

### Set accounting codes

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-accounting-codes`

**Actor**: `cpt-cf-bss-products-actor-finance-reviewer`

**Success Scenarios**:
- `taxCategory` and `glCode` each validate against their Finance-owned `RecognizedSet`, which
  follows the same governed lifecycle as the unit set
- Both are required at publish for `product` and `service` types, through the type profile
- Both are **codes only** — this feature performs no tax mathematics and no ledger posting

**Error Scenarios**:
- An unknown code — `ACCOUNTING_CODE_UNKNOWN`; a `deprecated` code on a new assignment —
  `ACCOUNTING_CODE_DEPRECATED`; a removal while a non-terminal published head carries it —
  `ACCOUNTING_CODE_DELIST_BLOCKED`. One code per refusal serves `taxCategory` and `glCode` alike
- A `product` or `service` published without one — `ACCOUNTING_CODE_REQUIRED`

**Boundary**: both fields are bucket iii and finance-material, so at least one FinanceReviewer
sits in the approval — **and at a quorum of zero the predicate is recorded `predicateUnsatisfiable`
rather than blocking**. This very rule is the operand that decision names: requiring `taxCategory`
at publish for a `product` is what would otherwise leave a one-person tenant unable to publish
their first such SKU forever. **Whether this registry owns these two columns at all is open item
5**, and the answer may delete this flow's validators, its two `set_kind` values and its
publish-blocking requirement together.

## 3. Processes / Business Logic (CDSL)

The step lists live in [`../design/03-sku-classification.md`](../design/03-sku-classification.md)
§3 — see §1.4. Each process below states its input, its output and its boundary.

### RecognizedSet mechanics

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-recognized-set`

**Input**: the tenant, the `set_kind`, the member code, the operation, and the set of non-terminal
published heads referencing the member

**Output**: an applied state flip with its event, or a refusal — the per-set de-list code, or
`STALE_LIVE_OP` from the envelope

One generic shape serves all four vocabularies:
`(tenant_id, set_kind, member_code, display_label?, state, seeded_by?)`. Mutations ride
`GovernedLiveOp`, and every admitted mutation emits its set's event in the same transaction.
**The set is the `active` and `deprecated` rows; a `removed` row is a tombstone outside it** — a
membership check is the validators' single lookup. Seeded members are deprecatable and never
removable, so the platform baseline survives tenant edits.

**Boundary**: the removal operand is uniform across this feature's four sets — **non-terminal
published heads**. Frozen versions are self-contained copies and neither block a removal nor are
touched by one. **Whether a `draft` head blocks is open item 15**, jointly owned with
`02-taxonomy-attributes`, whose own rule reads `draft`/`published`/`deprecated` where this one
reads published only, and whose PRD clause is narrower than either.

### The publish-time collector dependency

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-collector-dependency`

**Input**: the distinct `usageTypeRef` values a publish carries, and the collector's
`get_usage_type` port

**Output**: a resolved `(gts_id, kind, metadata_fields)` snapshot frozen into the entity's version
row, or `USAGE_TYPE_UNRESOLVED`, or `USAGE_TYPE_UNAVAILABLE`

The resolver runs **once per publish per distinct ref**, not once per validator — a correction
lane's publish resolving both the stored ref and the corrected one. It carries a short timeout and
no retry, and on the scheduled lane, where no operator exists to retry, `USAGE_TYPE_UNAVAILABLE`
joins the runner's **`deferred`** set rather than its `failed` set, bounded by the transition's own
attempt budget. Making it terminal there once burned a pinned approval on a transient blip.

**Boundary**: this is the gear's only synchronous cross-gear call inside a publish pipeline, and
it bounds publish availability by collector availability for usage SKUs. **Whether the phase it
runs in is inside or outside the publish transaction is open item 19** — this feature and
`01-foundation` say before, `07-reference-signal` says inside, and both cannot hold. The
timeout's value is **open item 12**, as is the bulk lane's unavailable path.

### Error taxonomy

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-classification-errors`

**Input**: a refused act and the rule that refused it

**Output**: one canonical code carrying its declared RFC 9457 status

This feature declares fifteen codes and registers them into the Foundation's taxonomy:
`SKU_TYPE_UNKNOWN`, `ACCOUNTING_CODE_REQUIRED`, `ACCOUNTING_CODE_UNKNOWN`,
`ACCOUNTING_CODE_DEPRECATED`, `ACCOUNTING_CODE_DELIST_BLOCKED`, `METER_DECLARATION_INCOMPLETE`,
`UNRECOGNIZED_UNIT`, `UNIT_DEPRECATED`, `USAGE_TYPE_UNRESOLVED`, `USAGE_TYPE_UNAVAILABLE`,
`UNIT_DELIST_BLOCKED`, `PLAN_TIER_UNKNOWN`, `PLAN_TIER_DEPRECATED`, `PLAN_TIER_RETIRE_BLOCKED`
and `BUNDLE_OVERRIDE_REQUIRED`.

Two more appear in the slice's response map and are **declared elsewhere**:
`BULK_OVERRIDE_UNACKNOWLEDGED` (`09-bulk-promotion`) and `STALE_LIVE_OP`
(`02-taxonomy-attributes`, whose envelope every door of these four sets rides). Repeating a status
is not a second declaration, so the one-declaration rule stands.

**Boundary**: the taxonomy and every code's RFC 9457 status are specified by
`cpt-cf-bss-products-contract-classification-errors`, declared at
[`../design/03-sku-classification.md`](../design/03-sku-classification.md) §3.2 — cited by id
rather than by section number, which does not survive a renumber. `USAGE_TYPE_UNAVAILABLE`
at 503 is this gear's own addition — the donor's set carries no 503 at all, so that one class is
**not** "checked against the donor" and is offered for correction. The architectural 422s reach
the wire as 400 carrying their code.

## 4. States (CDSL)

### RecognizedSet Member State Machine

- [ ] `p1` - **ID**: `cpt-cf-bss-products-state-recognized-set`

One machine, shared by all four `set_kind` values — metering units, tax categories, GL codes and
the `PlanTier` taxonomy. The four rows below are the template's id-bearing rendering of the
slice's `inst-rs-shape` and `inst-rs-removal-operand`; see §1.4 for why they carry their own ids.

**States**: `active`, `deprecated`, `removed`

**Initial State**: `active`

**Transitions**:
1. [ ] - `p1` - **FROM** `active` **TO** `deprecated` **WHEN** an approved `GovernedLiveOp` applies; new declarations and new assignments against the member are thereafter refused and existing published carriers keep resolving - `inst-rm-edge-deprecate`
2. [ ] - `p1` - **FROM** `deprecated` **TO** `removed` **WHEN** an approved `GovernedLiveOp` applies and no non-terminal published head references the member **and** the member is not seeded; the row survives as a tombstone outside the set, so no published row ever names a member that has ceased to exist and the primary key never frees - `inst-rm-edge-remove`
3. [ ] - `p1` - **FROM** `deprecated` **TO** `active` and **FROM** `removed` **TO** `active` **WHEN** an approved `GovernedLiveOp` re-lists the member; this is safe precisely because the identity never changed - `inst-rm-edge-relist`
4. [ ] - `p1` - **No transition other than those above is admitted** — in particular `active → removed` is refused, because the whole safety property of de-listing is that deprecation blocks new declarations first — and there is **NO DELETE** and **no `member_code` UPDATE** in any state: the trigger whitelist admits `state` and `display_label` only, which makes semantic immutability a schema property rather than a convention - `inst-rm-append-only`

**All three mutations are governed and material.** `design/03` §3.1 covers them under one clause
— mutations ride `GovernedLiveOp` — and `05-governance` `inst-mt-inputs` registers "03's
recognized-set add/deprecate/remove and `PlanTier` taxonomy ops" among the kinds their owning
slice makes material. The first draft of this section wrote the envelope clause into transitions 1
and 3 and dropped it from 2, then reported that asymmetry as a defect of the slice on the strength
of a material-op enumeration `design/03` does not contain. Both halves were wrong and both are
struck: the omission was this rendering's, and the transplanted premise belongs to
`02-taxonomy-attributes`, whose `inst-ad-governed` genuinely does omit removal.

## 5. Definitions of Done

Twenty-two, in three groups rather than one. **Separately testable**: sixteen. **Testable only
against a named double**: `dod-bundle-override`, `dod-finance-materiality`,
`dod-recognized-set-mechanics` and `dod-plantier-governance` all spend a `05-governance` approval
that has no runnable gate, and `dod-meter-bucket` spends `07-reference-signal`'s correction door —
so each owes an **in-test approval double**, without which its probe goes green against a gate
that approves nothing, `dod-finance-materiality`'s `predicateUnsatisfiable` arm included. *The
doubles ship (P-D-142, P-D-144, P-D-146): `seed_satisfied_approval`, its acknowledging variant
for the bundle ceremony, and the real host at `N = 0` for the one-person tenant.*
**Consumer half elsewhere**: `dod-sdk-read-shape`, whose other side is `12-consumer-contracts`'.
The first draft claimed a single exception; the partition above is what the three-lens review
measured.

### Recognized-set table and its append-only guard

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-recognized-set-table`

The system **MUST** create `products_recognized_set` on both engines with primary key
`(tenant_id, set_kind, member_code)` where `set_kind` is one of
`metering_unit`, `tax_category`, `gl_code`, `plan_tier`; a `display_label` used by `plan_tier` and
ignored elsewhere; `state` in `{active, deprecated, removed}`; and `seeded_by`. A trigger
whitelist **MUST** admit updates to `state` and `display_label` **only**, refusing every `DELETE`
and every `member_code` update, with a `CorruptRow` probe per guarded column class on both
engines. A schema-oracle golden **MUST** exist together with a perturbation case proving it can
fail.

**Built, with `set_kind` pinned non-empty only** (**P-D-92**): the four kinds above are the stated
domain and neither this DoD nor `design/03` §4 demands a `CHECK` over them, which is what let the
table ship while §7 row 5 stays open — a `CHECK` enumerating the four would be that row's answer
written by a migration. A probe asserts the consequence directly: an unlisted kind is admitted and
the blank is refused.

**The whitelist here is a whitelist, unlike the head tables'.** Those guards name the columns that
may not change and admit the rest; §4 states this one from the other side, so `member_code`,
`set_kind`, `tenant_id`, `seeded_by` and `created_at` are each refused by name — one case per
guarded column class on **both** engines, plus the unconditional `DELETE` refusal that makes
`removed` a tombstone (**P-D-47**). The two engines express the guard differently — a `plpgsql`
`RAISE EXCEPTION` against a `WHEN`-clause `RAISE(ABORT)` — so a divergence between them is invisible
to either suite alone, which is why both halves exist.

**Implements**: `cpt-cf-bss-products-algo-recognized-set`,
`cpt-cf-bss-products-state-recognized-set`

**Constraints**: `cpt-cf-bss-products-constraint-no-commercial-concern`

**Touches**:
- DB Table: `products_recognized_set`
- Entities: `RecognizedSet`

### Classification columns and the meter pair CHECK

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-classification-columns`

The system **MUST** carry, on `01-foundation`'s `products_sku`, the seven columns whose rules this
feature owns: `type`, `sellable`, `plan_tier`, `tax_category_ref`, `gl_code_ref`, `metering_unit`
and `usage_type_ref`. A `CHECK` **MUST** enforce that `metering_unit` and `usage_type_ref` are
both null or both non-null — the physical floor under the atomic-pair rule — with a `CorruptRow`
probe on both engines. `tax_category_ref` and `gl_code_ref` are **contingent columns** (open item
5). **Whether any of the four reference columns is a real database foreign key is open item 7**
and this DoD obliges no constraint until it is answered: `plan_tier`, `tax_category_ref`,
`gl_code_ref` and `metering_unit` are all single code columns into the same three-column primary
key, none can reference it without `set_kind` supplied as a literal, and each has a de-list code a
raw violation would pre-empt. `design/03` §4 asks the question of `plan_tier` because that is the
column whose FK claim was struck; the argument holds for all four and §4 governs.

**Ticked with P-D-145.** The five columns land on `products_sku` beside the meter pair — `sku_type` (the donor's name for `type`), `sellable` (`NOT NULL DEFAULT true`), `plan_tier`, `tax_category_ref`, `gl_code_ref` — in `m20260829_000003` in place, both engines; the meter-pair `CHECK` and its `CorruptRow` probes were already shipped. Items 5 and 7 are answered (P-D-131 row 5: the codes stay; P-D-91: no foreign key, each is a code into the three-column key with its own de-list code).

**Implements**: `cpt-cf-bss-products-flow-classify-sku`,
`cpt-cf-bss-products-flow-declare-meter`

**Touches**:
- DB Table: `products_sku`
- Entities: `SKU`, `MeterDeclaration`

### Recognized-set mechanics

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-recognized-set-mechanics`

The system **MUST** implement one generic membership lookup treating `active` and `deprecated`
rows as the set and a `removed` row as a tombstone outside it, with every mutation riding
`GovernedLiveOp` and emitting its set's event in the same transaction. The removal operand
**MUST** be the non-terminal published head, uniform across all four `set_kind` values. A probe
**MUST** be armed both ways: removal refused while a `deprecated` head references the member, and
removal **admitted** while only frozen version content does — the old snapshot still rendering
afterwards, and a new declaration naming the removed member failing `UNRECOGNIZED_UNIT`.

**Ticked (P-D-146).** The lookup, the tombstone, the removal operand and the both-ways probe
shipped with P-D-121; the doors now resolve the stored approval host before their transaction and
spend the record inside it, subject `recognized_set/{set_kind}/{member_code}` unpinned
(`recognized_sets::member_op_subject`), and the probes seed that record through the same double
the other live-op doors use (`seed_member_op`;
`a_member_op_without_a_satisfied_record_is_refused_approval_required` is the refutation). The
holder population is uniform across the four kinds since 03's columns exist
(`SetKind::carrier_column`, `repo::member_holders`, the flip's `NOT EXISTS`):
`a_tier_retire_and_a_code_removal_are_blocked_by_a_published_carrier`.

**Implements**: `cpt-cf-bss-products-algo-recognized-set`

**Touches**:
- DB Table: `products_recognized_set`

### Seeded members

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-seeded-members`

The system **MUST** seed the four recognized units named by PRD §17.1 — `vCPU-hours`,
`GB-storage`, `GB-egress`, `request-count` — and a `PlanTier` value, marked `seeded_by`. A seeded
member **MUST** be deprecatable and **MUST NOT** be removable. **Which tier value is seeded is
open item 11** and **who writes seeds for a tenant created after the migration is open item 10**;
the rows are load-bearing, because a tenant with no unit seeds could declare no meter at all.

**Ticked with P-D-145.** `repo::ensure_recognized_seeds` writes the platform baseline on the tenant's **first write that could need it** (P-D-104, P-D-121 row 10): the four PRD §17.1 units when a declaration is judged, `standard` (P-D-131 row 11) when a tier is; nothing for Finance's sets. Every seed carries `seeded_by = platform`; the sets door deprecates a seeded member and refuses its removal `ILLEGAL_FIELD_MUTATION` (P-D-131 row 18). Probe: `the_platform_seeds_the_units_on_the_first_declaration_and_a_seeded_member_is_never_removed`.

**Implements**: `cpt-cf-bss-products-algo-recognized-set`

**Touches**:
- DB Table: `products_recognized_set`
- Entities: `RecognizedSet`

### Type profile validators

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-type-profile`

The system **MUST** register save-door and publish-door validators requiring `type` present and
within the closed set (`SKU_TYPE_UNKNOWN`), and enforcing the per-type required fields at publish:
`product` and `service` require both accounting codes (`ACCOUNTING_CODE_REQUIRED` naming the
missing one), `bundle` requires neither. **The bundle exemption gets a named probe** — it is the
easy thing to lose.

**Ticked with P-D-145.** `sku_type` is required at create (absence is the shape's `VALIDATION`, P-D-121 row 13), judged `SKU_TYPE_UNKNOWN` outside `product | service | bundle` at create, save and publish (`domain::recognized::type_profile`); at publish a `product` or `service` needs both accounting codes and the refusal names the missing one (`required_codes_present`), a `bundle` neither. Probes: `a_sku_type_outside_the_closed_set_is_refused_and_one_inside_is_admitted`, `a_product_missing_an_accounting_code_is_refused_at_publish_and_a_bundle_needs_none` — the bundle exemption's own named probe.

**Implements**: `cpt-cf-bss-products-flow-classify-sku`

**Touches**:
- DB Table: `products_sku`
- Entities: `TypeProfile`

### Sellable

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-sellable`

The system **MUST** default `sellable` to `true`, treat a flip as a bucket-iii head-row save
re-published as version N+1, and expose it in the SDK read shape per `CatalogVersion`.
**Whether the flip is material is open item 16.**

**Ticked (P-D-146).** `sellable` defaults `true` and a flip is a bucket-iii save the next publish freezes as version N+1 (`a_type_change_after_first_publish_is_refused_and_a_sellable_flip_is_frozen`); item 16 is answered — the flip is material (P-D-131); and the SDK read shape carries it (`bss_products_sdk::models::Sku::sellable`, `dod-sdk-read-shape`). §6's *end to end* criterion stays open until a read door constructs that shape — `12-consumer-contracts`'.

**Implements**: `cpt-cf-bss-products-flow-classify-sku`

**Touches**:
- DB Table: `products_sku`

### Bundle override registration

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-bundle-override`

The system **MUST** register the gate condition that refuses an unacknowledged publish of an
uncomposed `bundle` with `BUNDLE_OVERRIDE_REQUIRED`, on the **publish** rather than on any one
lane, so every lane including bulk carries it. The acknowledgment **MUST** be the operand
`01-foundation`'s publish door reads to set `composition_pending`. The ceremony itself is
`05-governance`'s, and **the findings report its approvers acknowledge by name is built by no
slice (open item 14)**.

**Ticked (P-D-146).** `skus::refuse_unacknowledged_bundle` runs inside `run_publish` — so every lane, bulk and scheduled included — after the verdict and **before** the one-shot spends the record: a `bundle` that is uncomposed by P-D-134 row 20's reading (`published_version = 0`, or published with `composition_pending` raised) publishes only under a record whose `override_acknowledged` is set, and that same operand is what `post_publish_image` writes into `composition_pending` (P-D-32). Probe: `an_unacknowledged_bundle_publish_is_refused_and_the_acknowledged_one_raises_the_flag`; the double is `seed_satisfied_approval_with_ack`, which stamps the acknowledgment the way an approver's acknowledging decision would for the gate's one reader. Open item 14 is answered by P-D-125 and does not bind this DoD.

**Implements**: `cpt-cf-bss-products-flow-classify-sku`

**Touches**:
- DB Table: `products_sku`

### Meter declaration atomicity

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-meter-atomic`

The system **MUST** refuse a partial `MeterDeclaration` with `METER_DECLARATION_INCOMPLETE` at the
door and refuse it again at the physical layer through the paired `CHECK`, admitting exactly one
unit per declaration. A composite meter declares its **output** unit.

**Implements**: `cpt-cf-bss-products-flow-declare-meter`

**Touches**:
- DB Table: `products_sku`
- Entities: `MeterDeclaration`

### Unit recognition

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-unit-recognition`

The system **MUST** refuse a declaration naming a unit that is unknown or `removed` with
`UNRECOGNIZED_UNIT`, and a **new** declaration naming a `deprecated` unit with `UNIT_DEPRECATED`
— including a draft whose unit was deprecated before its first publish. No door may mint a unit
inline.

**Implements**: `cpt-cf-bss-products-flow-declare-meter`

**Touches**:
- DB Table: `products_sku`, `products_recognized_set`

### Usage type resolution

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-usage-type-resolution`

The system **MUST** resolve `usageTypeRef` against the collector at publish — resolvability only,
no lifecycle and no dimension check — failing `USAGE_TYPE_UNRESOLVED` when unresolvable and
`USAGE_TYPE_UNAVAILABLE`, **fail-closed**, when the collector is unreachable. The resolver **MUST**
run once per publish per distinct ref. It **MUST** be probed against a stub collector for three
distinct outcomes — resolved, unknown, timeout — the timeout case asserting the publish stays
retryable and idempotent.

**Built, and the collector wired (P-D-121 row 19, P-D-131, P-D-141):** the HTTP publish door
resolves before the transaction and the validators phase never calls out. The three answers are
judged in domain (`judge_usage_type`); the resolver is `ApiState`'s `UsageTypeResolver` —
`infra::usage_types::CollectorResolver` over `usage-collector-sdk`'s `UsageCollectorClientV1`
(`NotFound` → unresolved, every other error and the configured timeout → unavailable, an invalid
GTS id → unresolved), `NoCollector` where `ClientHub` carries none (fail-closed 503, said at boot).
The door is probed through a scripted stub for all three outcomes, the timeout case asserting the
`Idempotency-Key` is left unclaimed and the retry publishes once. A SKU carries one `usage_type_ref`, so
once-per-distinct-ref is one call. The scheduled lane still enters `run_publish` inside the
runner's transaction — consume-at-schedule is the lead's, and this tick does not invent a
`deferred` disposition there. **P-D-157 (2026-09-05): the lane now resolves.** The runner carries the
resolver (`ActivationContext::usage_type_resolver`) and judges a usage SKU's ref under the gear's
system principal before its publish; `Unavailable` lands the row `deferred` with its lane claim
released, `Unresolved` fails it — probe
`a_usage_skus_scheduled_publish_defers_while_the_collector_is_unavailable_and_applies_once_it_answers`. The timeout field is 2000 in config and unused until a real client
exists.

**Implements**: `cpt-cf-bss-products-algo-collector-dependency`,
`cpt-cf-bss-products-flow-declare-meter`

**Touches**:
- Entities: `UsageTypeResolver`

### Resolved binding snapshot

- [x] `p2` - **ID**: `cpt-cf-bss-products-dod-binding-snapshot`

The system **MUST** freeze the resolved `(gts_id, kind, metadata_fields)` snapshot into the
entity's `products_entity_version` row, as the record of what the binding resolved to at publish
time. **The column, its membership in the content digest, and the carrier across the
validators-to-transaction boundary are all open item 6**: `01-foundation`'s version-row roster is
closed and names no such column, and if the snapshot joins the digested content then the
`digest_version` constant bumps off 1 and the Foundation's golden vector is re-pinned.

**Ticked (P-D-146), on P-D-134 row 6's answer.** The column is `products_entity_version.binding_snapshot` (nullable `text`, one JSON object `{gts_id, kind, metadata_fields}` with sorted keys and fields — `UsageTypeBinding::snapshot_json`), added in `m20260829_000007` in place on both engines and frozen with the row by the no-`UPDATE` trigger. It is **outside the digest**: `content` and `content_digest` are computed before it is looked at and `DIGEST_VERSION` did not move. The carrier across the validators-to-transaction boundary is the door: `resolve_usage_type_before_publish` returns the binding the collector answered with and `run_publish` hands it to `freeze_for`. Probe: `a_publish_freezes_the_resolved_binding_beside_the_version_row` (a SKU with no meter freezes `NULL`). The scheduled lane freezes `NULL` today because it does not resolve (§7 row 22).

**Implements**: `cpt-cf-bss-products-algo-collector-dependency`

**Touches**:
- DB Table: `products_entity_version`

### Meter bucket registration

- [x] `p2` - **ID**: `cpt-cf-bss-products-dod-meter-bucket`

The system **MUST** register the metering-unit declaration, `usageTypeRef` included, as
**bucket ii** — immutable after publish and correctable only through `07-reference-signal`'s
correction door — while the draft plane edits it freely through the Foundation's save
transaction.

**Ticked (P-D-146).** `metering_unit` and `usage_type_ref` are bucket ii in `domain::bucket::SKU_COLUMNS` (P-D-121), the trigger arms carry them on both engines (`bucket_agreement_tests`), and a save that moves the pair after first publish is refused naming the correction door (`meter_declaration_tests`, *"After first publish the pair is the correction door's"*). The door that admits the correction is `07-reference-signal`'s and is measured under its own DoD, not this one.

**Implements**: `cpt-cf-bss-products-flow-declare-meter`

**Touches**:
- DB Table: `products_sku`

### Unit de-listing guard

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-unit-delist`

The system **MUST** refuse a unit removal while a non-terminal published head declares it, raising
`UNIT_DELIST_BLOCKED` with the holders sampled, and **MUST** admit it once only frozen version
content names the unit. Deprecation **MUST** leave existing publishes untouched, and neither
deprecation nor removal may mutate any frozen snapshot.

**Built (P-D-121 row 21):** the `deprecated → removed` `UPDATE` carries
`WHERE NOT EXISTS` a non-terminal published head declaring the member. The both-ways probe is
`a_removal_is_blocked_by_live_holders_and_admitted_after_them`.

**Implements**: `cpt-cf-bss-products-flow-unit-set`,
`cpt-cf-bss-products-state-recognized-set`

**Touches**:
- DB Table: `products_recognized_set`, `products_sku`

### Unit semantic immutability

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-unit-immutable`

The system **MUST NOT** provide any rename or redefine operation on the recognized-unit set — the
absence of the door is the enforcement, and the append-only trigger is its floor. A correction
**MUST** be a new unit plus a deprecation of the old, tied through the `GovernedLiveOp` payload so
the audit trail carries the pair. A test **MUST** prove no write path mutates a `member_code`.

**Ticked (P-D-146).** There is no rename operation on `member_code`: the door's three ops are add, transition and relabel, and the relabel writes `display_label` only (`repo::relabel_recognized_member`). The trigger holds the floor — `migrations_tests` probes that an `UPDATE` to `member_code` (and to every column outside the `state`/`display_label` whitelist) and a `DELETE` are refused on both engines while `state` and `display_label` are admitted.

**Implements**: `cpt-cf-bss-products-flow-unit-set`

**Touches**:
- DB Table: `products_recognized_set`

### PlanTier taxonomy governance

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-plantier-governance`

The system **MUST** treat tier identity as the stable code with no update path, carry the display
label separately, route add, rename, deprecate and retire through `GovernedLiveOp` under elevated
approval, emit `PlanTierUpdated`, and refuse a retire while a non-terminal published head carries
the value with `PLAN_TIER_RETIRE_BLOCKED`. A seeded value is deprecatable and never retired.

**Ticked (P-D-146).** Tier identity is the stable code (`dod-unit-immutable`); the display label is carried separately and renamed through `POST …/members/{memberCode}/label` (`a_relabel_changes_the_display_label_only_and_announces`). Add, relabel, deprecate and retire all ride `GovernedLiveOp` under the stored host — *elevated approval* is the tenant's materiality policy on that subject — and emit `PlanTierUpdated` in the mutating transaction. A retire while a non-terminal published head carries the value is `PLAN_TIER_RETIRE_BLOCKED`, the same guard as the units' now that `plan_tier` is a carrier column; a seeded value deprecates and never removes (P-D-145).

**Implements**: `cpt-cf-bss-products-flow-plantier`

**Touches**:
- DB Table: `products_recognized_set`

### PlanTier assignment

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-plantier-assign`

The system **MUST** validate a SKU's tier at save **and** at publish, refusing an unknown value
with `PLAN_TIER_UNKNOWN` and a **new** assignment of a `deprecated` value with
`PLAN_TIER_DEPRECATED`, including a draft whose tier was deprecated before its first publish,
while existing published carriers stay valid. Tier presence at plan publish **MUST NOT** be
re-checked here.

**Ticked with P-D-145.** Judged at create, save and publish (`domain::recognized::tier_verdict`): unknown or `removed` → `PLAN_TIER_UNKNOWN`; a **new** assignment of a `deprecated` tier → `PLAN_TIER_DEPRECATED`, a first publish counting every carried value as new; an existing published carrier re-publishes. Tier presence at plan publish is not re-checked here. Probe: `an_unknown_tier_is_refused_the_seeded_standard_admitted_and_a_deprecated_tier_refuses_new_assignments_only`.

**Implements**: `cpt-cf-bss-products-flow-plantier`

**Touches**:
- DB Table: `products_sku`, `products_recognized_set`

### Accounting code validators

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-accounting-validators`

The system **MUST** validate `taxCategory` and `glCode` against their recognized sets on save and
publish, refusing `ACCOUNTING_CODE_UNKNOWN`, `ACCOUNTING_CODE_DEPRECATED` on a new assignment, and
`ACCOUNTING_CODE_DELIST_BLOCKED` on a removal a non-terminal published head blocks — one code per
refusal serving both fields. The columns **MUST** be treated as opaque: no tax computation and no
ledger posting.

**Ticked with P-D-145.** Both codes are judged against Finance's sets at create, save and publish (`domain::recognized::accounting_code_verdict`) — `ACCOUNTING_CODE_UNKNOWN` for an unknown or `removed` code, `ACCOUNTING_CODE_DEPRECATED` on a new assignment, one code per refusal serving both fields; `ACCOUNTING_CODE_DELIST_BLOCKED` was already the sets door's. The columns are opaque strings — no tax computation, no ledger posting. Probe: `an_unknown_or_deprecated_accounting_code_is_refused_and_a_known_one_is_admitted`.

**Implements**: `cpt-cf-bss-products-flow-accounting-codes`

**Constraints**: `cpt-cf-bss-products-constraint-no-commercial-concern`

**Touches**:
- DB Table: `products_sku`, `products_recognized_set`

### Finance materiality at publish

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-finance-materiality`

The system **MUST** require both accounting codes at publish for `product` and `service` types and
**MUST** place at least one FinanceReviewer in the governed approval — **and at a quorum of zero
MUST record the predicate `predicateUnsatisfiable` rather than blocking**. A test **MUST** prove a
one-person tenant can publish their first `product` SKU.

**Ticked (P-D-146).** Both codes at publish for `product`/`service`: P-D-145. The FinanceReviewer predicate: the submit door now **computes** the finance-material operand — a publish whose touched set includes `tax_category_ref` or `gl_code_ref` is finance-material whatever the caller declared (`domain::recognized::is_finance_material`, OR-ed with the caller's flag) — and `describe_quorum` sets the predicate at `N >= 1` or records `predicateUnsatisfiable = finance_reviewer` at `N = 0` (`05`'s `dod-finance-predicate`, ticked with this). The one-person tenant's probe: `a_one_person_tenant_publishes_its_first_product_sku_and_the_predicate_is_recorded` — born satisfied at `N = 0`, the descriptor carries the unsatisfiable predicate, and the publish goes through under the real host with no double.

**Implements**: `cpt-cf-bss-products-flow-accounting-codes`

**Touches**:
- DB Table: `products_sku`

### Mutability bucket registration

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-bucket-registration`

The system **MUST** register every field this feature owns into `01-foundation`'s bucket registry:
`type` and the metering-unit declaration including `usageTypeRef` as **bucket ii**; `plan_tier`,
`tax_category_ref`, `gl_code_ref` and `sellable` as **bucket iii**. A test **MUST** prove no field
this feature owns is absent from the registry, since the Foundation refuses an untagged
published-state column at the head door rather than defaulting it.

**Ticked with P-D-145.** `domain::bucket::SKU_COLUMNS` carries all seven: `sku_type`, `metering_unit`, `usage_type_ref` as bucket ii; `sellable`, `plan_tier`, `tax_category_ref`, `gl_code_ref` as bucket iii. `bucket_tests::the_skus_tagged_columns_answer_the_buckets_section_4_1_assigns` names each; `the_registry_and_the_physical_tables_name_the_same_columns` proves none is absent; `migrations_tests::bucket_agreement_tests` holds the trigger's arms to the same classes on both engines. At the door: `a_type_change_after_first_publish_is_refused_and_a_sellable_flip_is_frozen`.

**Implements**: `cpt-cf-bss-products-flow-classify-sku`,
`cpt-cf-bss-products-flow-declare-meter`

**Touches**:
- DB Table: `products_sku`

### Classification error taxonomy

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-classification-errors`

The system **MUST** declare all fifteen codes as constants on their raising rules and register
them into the Foundation's taxonomy, each carrying its declared RFC 9457 status. No code carrying
a registry code may reach the wire as a 422. `USAGE_TYPE_UNAVAILABLE` **MUST** be retryable, and
on the scheduled lane **MUST** join the runner's `deferred` set rather than its `failed` set.

**Ticked (P-D-146).** All fifteen are declared on their raising rules and registered with their statuses: P-D-145's six plus `BUNDLE_OVERRIDE_REQUIRED` (raised by `skus::refuse_unacknowledged_bundle`, 400 precondition shape; `error_tests`, `error_mapping_tests`, both counters at 70). The scheduled-lane clause has its arm — `activation_runner::publish_refusal_is_transient` puts `USAGE_TYPE_UNAVAILABLE` in `DeferralPopulation::TransientDependency` under the attempt budget (`only_an_unavailable_collector_is_a_transient_publish_refusal`) — **and that lane does not resolve `usageTypeRef` today** (§7 row 22): the arm is where the code lands when it does; §6's deferred-lane criterion stays open on that row.

**Implements**: `cpt-cf-bss-products-algo-classification-errors`

**Touches**:
- Entities: `CanonicalError`

### Recognized-set events

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-recognized-set-events`

The system **MUST** emit `PlanTierUpdated`, `RecognizedUnitUpdated` and `RecognizedCodeUpdated`
through the Foundation's outbox in the mutating transaction, ordered on `(tenant, set_kind)`.
Per-field classification edits on a SKU **MUST** emit no event of their own — they ride the
Foundation's entity events — and that absence **MUST** be recorded as an explicit no-event
declaration.

**Ticked (P-D-146).** The three events ride the Foundation's outbox in the mutating transaction (`events::enqueue_set_event`, partitioned on `(tenant, set_kind)`; `an_add_lands_active_and_announces`), and the explicit no-event declaration is `events::SKU_CLASSIFICATION_EDITS_EMIT_NO_EVENT` — the seven per-field SKU columns, held against `SCHEMA_REFS` and the bucket roster by `a_per_field_classification_edit_is_declared_to_emit_no_event`.

**Implements**: `cpt-cf-bss-products-algo-recognized-set`

**Constraints**: `cpt-cf-bss-products-constraint-broker-native-events`

**Contract**: `cpt-cf-bss-products-contract-registry-events`

**Touches**:
- Entities: `PlanTierUpdated`, `RecognizedUnitUpdated`, `RecognizedCodeUpdated`

### SDK read shape

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-sdk-read-shape`

The system **MUST** expose `type`, `sellable`, `plan_tier`, `metering_unit`, `usage_type_ref`,
`tax_category_ref` and `gl_code_ref` in the SDK read shape from day one. **Three of these —
`sellable`, `usage_type_ref` and `type` — are absent from pricing's `CatalogSku` today** (open item
4); carrying them here keeps that fix additive on the consumer side, which is
`12-consumer-contracts`'. This is the one entry whose completion is not wholly this feature's.

**Ticked (P-D-146) — this feature's half.** `bss_products_sdk::models::Sku` carries `sku_type` (the closed `SkuType`), `sellable`, `plan_tier`, `metering_unit`, `usage_type_ref`, `tax_category_ref` and `gl_code_ref`. No read door constructs that shape yet and pricing's `CatalogSku` still lacks three of the seven (item 4) — both `12-consumer-contracts`', additive on the consumer side as the DoD intends.

**Implements**: `cpt-cf-bss-products-flow-classify-sku`

**Touches**:
- Entities: `CatalogSku`

## 6. Acceptance Criteria

*Ticks measured clause by clause at **P-D-157** (2026-09-05); the criterion-to-probe map is in that
entry. A box left open names a clause no probe asserts yet.*

- [x] A SKU with no `type`, or a `type` outside the closed set, is refused; and the code it meets
      is the one open item 13 settles, asserted rather than assumed
- [x] A `product` published without `taxCategory` is refused `ACCOUNTING_CODE_REQUIRED` naming the
      missing field, and succeeds once it is set
- [x] A `bundle` publishes with neither accounting code — the exemption has its own named probe
- [x] An uncomposed `bundle` published without acknowledgment is refused
      `BUNDLE_OVERRIDE_REQUIRED`; with it, the SKU publishes and carries
      `compositionPending = true`
- [ ] A zero-price "free" SKU takes the ordinary path with no special validator
- [x] `sellable` defaults `true`, and a flip reaches the SDK read shape end to end
- [ ] A declaration carrying a unit and no `usageTypeRef` is refused
      `METER_DECLARATION_INCOMPLETE` at the door, and the same row is refused by the `CHECK` with
      the application check bypassed
- [x] A declaration against an unknown or `removed` unit is refused `UNRECOGNIZED_UNIT`; against a
      `deprecated` unit, `UNIT_DEPRECATED` — including a draft whose unit was deprecated before
      its first publish
- [x] A publish whose `usageTypeRef` does not resolve is refused `USAGE_TYPE_UNRESOLVED`; a
      publish against an unreachable collector is refused `USAGE_TYPE_UNAVAILABLE` and the same
      publish succeeds unchanged once the collector returns
- [ ] The resolver is called **once per publish per distinct ref**, asserted by counting stub
      invocations on a publish carrying two refs
- [x] `USAGE_TYPE_UNAVAILABLE` on the scheduled lane leaves the transition `deferred`, not
      `failed`, and its pinned approval survives
- [x] A unit removal is refused `UNIT_DELIST_BLOCKED` while a `deprecated` SKU declares it, and
      **admitted** while only frozen version content names it; the old snapshot still renders and
      the removed member's row survives as `removed`
- [x] A new declaration naming a removed member fails `UNRECOGNIZED_UNIT`
- [x] No write path renames a `member_code`: the trigger refuses the `UPDATE` and the `DELETE`,
      and admits `state` and `display_label`
- [x] A seeded member can be deprecated and cannot be removed
- [x] A tier retire is refused `PLAN_TIER_RETIRE_BLOCKED` while a published SKU carries it
- [x] A new tier assignment of a `deprecated` value is refused `PLAN_TIER_DEPRECATED` while an
      existing published carrier stays valid
- [x] A tier rename changes the display label and leaves every SKU's stored code untouched
- [x] An unknown accounting code is refused `ACCOUNTING_CODE_UNKNOWN` for `taxCategory` and for
      `glCode` alike, one code serving both
- [x] A one-person tenant publishes their first `product` SKU: the FinanceReviewer predicate is
      recorded `predicateUnsatisfiable` and does not block
- [x] Every field this feature owns appears in the bucket registry, and a bucket-ii write after
      first publish is refused while the correction door admits it
- [x] Each of the fifteen codes is raised by exactly one rule and carries its declared status
- [x] The three set events are emitted in the mutating transaction on `(tenant, set_kind)`, and a
      per-field classification edit emits none of its own
- [ ] **Every refusal enumerated in §2 has a paired positive control proving the door admits the
      corresponding legal act.** The controls are owed **per code**, not in bulk: a blanket
      criterion is ticked by inspection rather than by a test. The per-code lines below carry the
      obligation; this line is the rule, not the test
- [ ] A `CorruptRow` probe exists for the meter pair `CHECK` and for each guarded column class of
      `products_recognized_set`, on both engines
- [x] A schema-oracle golden exists for `products_recognized_set` on both engines with a
      perturbation case proving it can fail
- [x] A `product` published against a `deprecated` `taxCategory` is refused
      `ACCOUNTING_CODE_DEPRECATED`, and an `active` one publishes
- [x] A code removal is refused `ACCOUNTING_CODE_DELIST_BLOCKED` while a published SKU carries it,
      and is admitted once none does
- [x] An unknown tier is refused `PLAN_TIER_UNKNOWN`, and a known one is admitted
- [x] A tier retire is admitted once no non-terminal published head carries the value — the
      positive control on `PLAN_TIER_RETIRE_BLOCKED`
- [x] A `type` inside the closed set is admitted — the positive control on `SKU_TYPE_UNKNOWN`
- [x] A known accounting code is admitted — the positive control on `ACCOUNTING_CODE_UNKNOWN`
- [x] A complete `(unit, usageTypeRef)` pair is admitted — the positive control on
      `METER_DECLARATION_INCOMPLETE`
- [x] An `active` recognized unit is admitted — the positive control on `UNRECOGNIZED_UNIT` and on
      `UNIT_DEPRECATED`
- [x] A resolvable `usageTypeRef` publishes — the positive control on `USAGE_TYPE_UNRESOLVED`,
      distinct from the collector-returns control on `USAGE_TYPE_UNAVAILABLE`
- [ ] A zero-price "free" SKU takes the ordinary path — the criterion `dod-type-profile` owes, the
      slice carrying it as `inst-cl-no-promo-entity`
- [x] Building the bucket-ii class turns `buckets_ii_and_iv_have_no_members_today`,
      `an_unregistered_column_fails_closed_rather_than_defaulting` and
      `the_class_counts_are_pinned_per_entity` red, and each is updated with its reason recorded
- [ ] No `#[ignore]`d test exists without a CI tier that runs it *(The tier: `make test-products-pg` — `DESIGN.md` §3.8's runbook; on demand by P-D-132, so this box stays open by the owner's decision — P-D-161.)*

## 7. Known unknowns

[`../design/03-sku-classification.md`](../design/03-sku-classification.md) §6 carries **22 open
items**, and all twenty-two bind implementation — every one of them lands on a Definition of Done
in §5. The twenty-first arrived with the doors themselves: the three-lens review of the shipped
membership doors found the de-list window write-skew-open on Postgres, and the mechanism that
closes it is 01's isolation posture rather than this slice's to pick. The twenty-second arrived
with group 4's build (P-D-146): the scheduled publish lane enters `run_publish` without the
pre-transaction usage-type resolve, so the deferred arm the errors DoD asks for could not be reached until P-D-157 wired the resolver into the runner
from that lane until the runner carries a service context and the resolver. They are carried in full, with the DoD each blocks and its owner, because the sibling feature
authored on 2026-08-30 carried four of twenty-three and the three-lens review measured that as its
single most costly defect.

**Three of the twenty-one are now answered and struck in place — 7, 9 and 15 — leaving eighteen
open.** Each was answered by a register entry rather than here: **P-D-91** (no FK on any of the four
code columns), **P-D-90** (the membership door's route family and its grant split) and **P-D-89**
(the removal operand, which the three DoDs row 15 blocks had each already stated). Row **5** stays
open at the PRD owner, but **P-D-92** released its hold on `dod-recognized-set-table` alone: that
table's DDL pins no `set_kind` roster, so a question about two of four row-value kinds no longer
holds the table.

**Nothing else here is answered.** A FEATURE artifact records what its design set leaves open; it
does not decide it — the struck rows above point at the register entry that did.

| # | The question | Blocks | Owner |
|---|---|---|---|
| ~~1~~ | **Who owns each recognized set, and what is the approver-role predicate per set?** Finance for the codes, Product with Rating for the units — the machinery is ready either way and only the sign-off is missing **Answered (P-D-131, 2026-09-03, the product owner): owners as `PRD` §15 already names them — units → Product + Rating, codes → Finance, `PlanTier` → Product; the approver predicate is the set owner's reviewer role at the decide door (`APPROVER_ROLE_REQUIRED`, P-D-119) once roles reach `SecurityContext`, any quorum approver until then.** | ~~`dod-finance-materiality`, `dod-plantier-governance`~~ | **struck** |
| ~~2~~ | **The collector sits in the publish path**, so publish availability is bounded by collector availability for usage SKUs **Answered (P-D-121 row 19 and P-D-131; recorded by P-D-133, 2026-09-04): the resolve runs before the transaction and an unavailable collector is a fail-closed 503 for usage SKUs only — latency, not a lock.** | ~~`dod-usage-type-resolution`~~ | **struck** |
| ~~3~~ | **`UsageType` deletion** — the binding snapshot gives remediation its evidence, but the negotiation with the collector is open **Answered (P-D-131, 2026-09-03, the product owner): the registry's half is built — a deleted type is the correction door's `unresolvable_target` arm; the collector's obligation (never hard-delete a `UsageType` with live bindings, tombstone with a signal) is recorded in `PRD` §15 for the collector's PRD owner.** | ~~`dod-binding-snapshot`~~ | **struck** |
| ~~4~~ | **`sellable`, `usage_type_ref` and `type` are missing from pricing's `CatalogSku`** — three of the four members that consumer contract names *(P-D-133, 2026-09-04: **accepted — the lead lands the three members in pricing's `CatalogSku`**; struck when it does.)* **Landed (P-D-133, 2026-09-04): pricing's `CatalogSku` and `CatalogSkuView` carry `type`, `sellable` and `usage_type_ref`** — `pricing-sdk/src/product_catalog.rs`, `pricing/src/api/rest/catalog_skus.rs`, the local-dev catalog rows, and `rest_catalog_skus` reads all three off the wire. | ~~`dod-sdk-read-shape`~~ | **struck** |
| ~~5~~ | **`tax_category_ref` and `gl_code_ref` may not belong to this registry at all.** PRD §2.1 says they are owned elsewhere while `fr-accounting-codes` requires this registry to persist and validate them. The answer may delete this feature's validators, its two `set_kind` values and its publish-blocking requirement together **Answered (P-D-131, 2026-09-03, the product owner): they stay.** §2.1 forbids the *descriptor*, `fr-accounting-codes` requires the *reference* to Finance's set, and §2.1 line 367 already says the catalog supplies the code; §2.1 gains the parenthetical. | ~~`dod-classification-columns`, `dod-accounting-validators`, `dod-finance-materiality`, `dod-bucket-registration`, `dod-sdk-read-shape`, `dod-recognized-set-events`, `dod-type-profile`~~ | **struck** |
| ~~6~~ | **Is the resolved-binding snapshot inside the content digest, and what carries it across the phase boundary?** `01-foundation`'s version-row roster is closed and names no such column; if the snapshot is digested, the `digest_version` constant bumps off 1 and the golden vector is re-pinned **Answered (P-D-134, 2026-09-04): outside the digest** — provenance beside the version row, keyed by it; `DIGEST_VERSION` stays 1. | ~~`dod-binding-snapshot`~~ | **struck** |
| ~~7~~ | ~~**Is `plan_tier` a real database foreign key?**~~ **Answered (owner call, 2026-09-01 — P-D-91): no, and not on any of the four code columns. Two independent measurements: a referencing side cannot supply `set_kind` as a literal on either engine, and each of the four has a de-list code a raw driver violation would pre-empt. The referential guarantee is the membership door's, which is where those codes live; the atomic-pair CHECK is a shape constraint and stands.** Original text: A single code column cannot reference the three-column primary key without `set_kind` as a literal, and a real constraint would refuse a removal this feature's own operand admits, raising a raw violation instead of `PLAN_TIER_RETIRE_BLOCKED`. The FK claim was struck *(Closed in its own body and never struck — the number is struck 2026-09-03 to match.)* | no DoD — resolved by P-D-91 | **struck** |
| ~~8~~ | **At which publishes do the recognized-and-active checks run, and what tells a new declaration from a carried-forward one?** A bucket-iii re-publish re-runs every registered validator fail-closed, so as written, deprecating a **tier or an accounting code** freezes every SKU carrying it against any further publish. **The unit half is not open**: the declaration is bucket ii, so it cannot change on an ordinary re-publish and every such publish is carried-forward by construction. For the two bucket-iii fields the comparand is the previous `products_entity_version` row, whose absence means first publish — a diff, not a marker **Answered (P-D-121, 2026-09-03): the check judges a new or changed declaration; a carried-forward value is judged by the state it had when declared.** Otherwise deprecating a tier freezes every SKU carrying it — the retroactive lockout `inst-ad-deprecate-then-remove` exists to avoid (*deprecated blocks new values*). Comparand: the previous published version's value. | ~~`dod-plantier-assign`, `dod-accounting-validators`~~ | **struck** |
| ~~9~~ | ~~**Which door writes `products_recognized_set`, at what path and under what grant?**~~ **Answered (owner call, 2026-09-01 — P-D-90): `POST /bss-products/v1/recognized-sets/{setKind}/members` and `…/members/{memberCode}/transitions`, one route family over one generic membership implementation, with the grant chosen by `setKind` — the tier set spends `plan_tier × write` and the other three `recognized_set × write`, the only reading under which both grants have a spender. The route shape is P-D-67's and P-D-87's, applied a third time.** Original text: The only stated write mechanism is `GovernedLiveOp` and this feature names no route, while `05-governance` already mints `recognized_set × write` and `plan_tier × write` with no door to attach them to *(Closed in its own body and never struck — the number is struck 2026-09-03 to match.)* | no DoD — resolved by P-D-90 | **struck** |
| ~~10~~ | **Who writes the seed members for a tenant created after the migration, and are the Finance sets seeded at all?** A tenant provisioned afterwards could declare no meter. `02-taxonomy-attributes` registers the identical question and names this feature in it **Answered by P-D-104, recorded (P-D-121, 2026-09-03)**: nobody seeds by migration — the seeds are written on the tenant's first write that could need one, in that transaction, once. Which members the Finance sets seed is §2's roster to name. | ~~`dod-seeded-members`~~ | **struck** |
| ~~11~~ | **Which seed value does the `PlanTier` taxonomy get?** PRD §17.1 offers `standard` or `none` and neither is pinned; the seeded `member_code` is a live contract value that a downstream guard would compare as a string **Answered (P-D-131, 2026-09-03, the product owner): `standard`** — mandatory on every SKU, a default every consumer reads without a special case; `PRD` §17.1 settled; the seed is `03`'s build. | ~~`dod-seeded-members`~~ | **struck** |
| ~~12~~ | **What is the resolver's timeout, and what is its unavailable path on the bulk lane and on an unwired deployment?** "A short timeout" has no number and §17.1 carries no row; the bulk lane consumes its batch approval once at the commit flip, so a blip mid-commit fails rows under an approval already spent; and "not wired" is not separated from "unreachable" *(P-D-121, 2026-09-03: **the number is settled** — `usage_type_resolver_timeout_ms` 2000, interim, in `ProductsConfig`, two seconds because the resolve now runs *before* the transaction (row 19) and holds nothing. **The unwired/unreachable split is not** — it entangles the bulk lane's once-consumed approval and sits with consume-at-schedule in the lead's queue.)* **Answered (P-D-131, 2026-09-03, the product owner): the causes are not distinguished** — an interactive usage-SKU publish refuses through the existing fail-closed 503 channel, a bulk row takes `failed` and the batch continues; the number was P-D-121's. | ~~`dod-usage-type-resolution`~~ | **struck** |
| ~~13~~ | **Which code does an absent `type` carry?** If `type` is required at create the shape phase raises `VALIDATION` and the run stops, so `SKU_TYPE_UNKNOWN`'s absent arm is unreachable and the AC map reads two ways **Answered (P-D-121, 2026-09-03): absent `type` is `VALIDATION` at the shape phase; `SKU_TYPE_UNKNOWN` covers a present, unrecognized value.** That is how the pipeline already runs; the absent arm is unreachable by construction and the AC map reads the one way that matches the code. | ~~`dod-type-profile`, `dod-classification-errors`~~ | **struck** |
| ~~14~~ | **The override ceremony reads findings from a report no slice builds.** `05-governance` acknowledges lint findings **by name**, and `06-catalog-version` §6 records that no instruction, store, RBAC pair, error code or probe delivers that report **Answered by P-D-125 (recorded by P-D-134, 2026-09-04)**: the dry-run door is the lint producer. | ~~`dod-bundle-override`~~ | **struck** |
| ~~15~~ | ~~**Do 02 and 03 admit a `draft` head as a blocking reference?**~~ **Answered (owner call, 2026-09-01 — P-D-89): `03`'s operand is the non-terminal *published* head, uniform across its four sets, and a `draft` head does NOT block — stated independently by the three DoDs this row blocks, and closed by `dod-unit-recognition`, which requires refusing a declaration on "a draft whose unit was deprecated before its first publish", a case unreachable if a draft blocked the deprecation. `02` keeps its own wider operand; the divergence is already registered on both sides.** Original text: This feature reads non-terminal **published** heads; `02-taxonomy-attributes` reads `draft`/`published`/`deprecated`; the PRD is narrower than both. Open item 5 in that feature's §7 is the same question from the other side *(Closed in its own body and never struck — the number is struck 2026-09-03 to match.)* | no DoD — resolved by P-D-89 | **struck** |
| ~~16~~ | **Is a `sellable` flip material?** `05-governance` registers it among the bucket-iii fields that make any touch material, while the PRD's material-change enumeration names `PlanTier`, the metering unit, `taxCategory` and `glCode` and not `sellable` **Answered (P-D-121, 2026-09-03): material.** `05` registers `sellable` bucket-iii under P-D-28, and that registry is what both guards read; the PRD's enumeration is a floor the design exceeds on purpose — a `sellable` flip changes what a consumer may buy. Divergence registered. | ~~`dod-sellable`~~ | **struck** |
| ~~17~~ | **Is a `PlanTier` display-label rename material?** This feature makes it display-only by construction, `05-governance` registers the taxonomy ops as material without excepting it, and `02-taxonomy-attributes` calls the identical edit on its own vocabulary non-material at `min(N, 1)` **Answered (P-D-121, 2026-09-03): non-material at `min(N, 1)`**, uniformly with `02` — a display label is a value on the thing, not the thing. `05`'s taxonomy-ops registration gains one display-label exception for both slices. | ~~`dod-plantier-governance`~~ | **struck** |
| ~~18~~ | **Which code refuses the removal of a seeded, unreferenced member?** All three de-list codes are predicated on holders, so none fits; `02-taxonomy-attributes` carries the identical silence for its own seeds **Answered (P-D-131, 2026-09-03, the product owner): `ILLEGAL_FIELD_MUTATION`**, uniformly with `02`'s seeded definition; no sixteenth code. | ~~`dod-seeded-members`; a sixteenth code would also break `dod-classification-errors` and its acceptance criterion, both of which say fifteen~~ | **struck** |
| ~~19~~ | **Does the registered-validators phase run before the publish transaction, or inside it?** This feature and `01-foundation` say before; `07-reference-signal` says inside and its own fix depends on that. **The costs are not symmetric**: on 07's reading a cross-gear call with a short timeout and no retry sits inside a transaction that has already written the frozen version row, holding the head-row lock and a pooled connection for the timeout on Postgres and serializing every other publish in the database on SQLite — a collector stall becomes a gear-wide publish stall. **And §5 as written builds that reading**, because `dod-usage-type-resolution` names no phase while `dod-binding-snapshot` pulls the resolve toward the transaction that consumes its value **Answered (P-D-121, 2026-09-03): inside the transaction, as shipped (P-D-97) — and a validator with a cross-gear input resolves it *before* and hands the phase a `Resolution`**, `MaterialityEvaluator`'s own shape. `07` is right about where the phase runs and wrong about what it may do there; its fix follows the same pattern. | ~~`dod-usage-type-resolution`, `dod-binding-snapshot`~~ | **struck** |
| ~~20~~ | **What operand tells a composed bundle from an uncomposed one?** The only registry-side record is `composition_pending`, whose default is `false` on an uncomposed draft, so it cannot distinguish never-composed from composed. Read literally, an ordinary bucket-iii re-publish of a composed bundle demands the override again and re-raises the flag **Answered (P-D-134, 2026-09-04): `composition_pending` is raised only by a composition-affecting publish**; composed = published with the flag clear, never composed = `published_version = 0`. | ~~`dod-bundle-override`~~ | **struck** |
| ~~21~~ | **What closes the de-list window between the holder census and the flip?** `inst-us-delist` states the invariant and names no mechanism for enforcing it across two transactions. The shipped doors read the holder population and the member on separate transactions at the engine's default isolation, so on Postgres they are **write-skew-open**: a first publish declaring the unit and a `deprecated → removed` flip can both commit, leaving a `published` head declaring a `removed` member. SQLite's single writer hides it, so the interim tier cannot probe it. Four remedies exist — a shared row lock on the member in the recognition read, both doors at `SERIALIZABLE` with a contention classifier, accepting the window and reconciling, or a dedicated Postgres race suite on the `postgres_head_race.rs` precedent — and the isolation posture is the Foundation's **Answered (P-D-121, 2026-09-03): one transaction, and the flip re-asserts the census** — the `deprecated → removed` `UPDATE` carries `WHERE NOT EXISTS (a non-terminal published head declaring the member)`, and the publish's own check (row 8) refuses a removed member inside its transaction. Neither side judges on a read from another transaction. **The fix for the withdrawn `dod-unit-delist` tick.** | ~~`dod-unit-delist`, `dod-recognized-set-mechanics`~~ | **struck** |
| ~~22~~ | ~~**The scheduled publish lane never resolves `usageTypeRef`.**~~ **Answered (P-D-157, 2026-09-05): it does now** — the runner carries the resolver (`ActivationContext::usage_type_resolver`) and judges the ref under the gear's system principal before the publish; `Unavailable` lands the row `deferred` with its lane claim released, `Unresolved` fails it. *The row's text stood as:* `activation_runner` enters `skus::run_publish` directly under `PreAuthorized`; the pre-transaction resolve (`resolve_usage_type_before_publish`) is the REST door's, and it needs the caller's `SecurityContext`, which the runner loop has none of. So a scheduled publish of a metered SKU skips the collector check P-D-131 makes fail-closed on the interactive lane, freezes `NULL` into `binding_snapshot`, and can never reach the `deferred` arm `publish_refusal_is_transient` holds for `USAGE_TYPE_UNAVAILABLE`. **Measured 2026-09-05 (P-D-146).** The fix shape: the runner's `ActivationContext` carries the resolver and a service `SecurityContext` (the broker's builder shows one), resolves before `run_publish`, hands the binding through, and maps `Unavailable` to `RunFinish::Deferred`. | the scheduled publish lane — `04`'s runner, with `07`'s correction lane the same shape; no `03` DoD is blocked (the errors DoD's arm is built, its §6 lane criterion ticked at P-D-157) | **answered — P-D-157** |

### Raised here rather than carried

- **"With the holders sampled" has no bound.** `UNIT_DELIST_BLOCKED` names a sample and gives it
  no size, no ordering and no payload shape, while `PLAN_TIER_RETIRE_BLOCKED` and
  `ACCOUNTING_CODE_DELIST_BLOCKED` say nothing about sampling at all — three refusals, one of
  which samples, none of which says how many. The scan is a write-path guard over `products_sku`
  for four `set_kind` values and is not covered by the read-performance delegation.
  *Owner: this feature.*
- **The removal-vs-publish race is unguarded.** `STALE_LIVE_OP` guards a stale envelope against
  the set row; it says nothing about a publish that adds the **first** reference between the
  removal's holder scan and its state flip. No isolation level, no lock and no
  re-check-inside-the-transaction clause is stated. `02-taxonomy-attributes` registers the
  analogous class as its own item 14. *Owner: this feature with 01.*
- **The seven classification columns never join the frozen version content roster.** Acceptance
  criterion 12 requires a removal admitted while only frozen content names the unit, which is
  possible only if `metering_unit` is inside that content. Open item 6 raises the roster and
  digest question for the binding snapshot alone; it applies to all seven columns, and a first
  content change after deployment bumps `digest_version` off 1. *Owner: this feature with 01.*

- **The bucket-ii class does not exist in the shipped code, and building this feature turns three
  green tests red.** `products_sku`'s migration states "Bucket-ii and bucket-iv have no members
  among today's columns" and creates only the bucket-i and bucket-iii triggers; `bucket_tests.rs`
  asserts the bucket-ii count is zero with the message "bucket-ii columns arrive with slice 07",
  samples `sellable`, `plan_tier`, `metering_unit` and `type` as columns that must **fail closed**
  as unregistered, and pins the per-entity class counts. `dod-bucket-registration` and
  `dod-classification-columns` contradict all three. The cost is a both-engine trigger clause for a
  class that has never existed, a `CorruptRow` probe for it, a re-pinned schema-oracle golden on an
  already-ticked Foundation DoD, and three test rewrites. **The code believes bucket ii arrives
  with `07-reference-signal`; this feature brings it with 03.** *Owner: this feature with 01 and
  07.*
- **`USAGE_TYPE_UNAVAILABLE` at 503 is unchecked against the donor.** The status mapping was
  checked against pricing code by code, but pricing's set carries **no 503 at all**, so this one
  class rests on this gear's own judgement. *Owner: the API-contract owner.*
