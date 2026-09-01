# Feature: Consumer Contracts & the Seam Suite

- [ ] `p1` - **ID**: `cpt-cf-bss-products-featstatus-consumer-contracts-implemented`

<!-- reference to DECOMPOSITION entry -->
- [ ] `p1` - `cpt-cf-bss-products-feature-consumer-contracts`

<!-- toc -->

- [1. Feature Context](#1-feature-context)
  - [1.1 Overview](#11-overview)
  - [1.2 Purpose](#12-purpose)
  - [1.3 Actors](#13-actors)
  - [1.4 References](#14-references)
- [2. Actor Flows (CDSL)](#2-actor-flows-cdsl)
  - [The seam suite](#the-seam-suite)
  - [Event versioning, replay and bootstrap](#event-versioning-replay-and-bootstrap)
- [3. Processes / Business Logic (CDSL)](#3-processes--business-logic-cdsl)
  - [The completeness checks](#the-completeness-checks)
  - [Monetization traceability](#monetization-traceability)
- [4. States (CDSL)](#4-states-cdsl)
- [5. Definitions of Done](#5-definitions-of-done)
  - [The `SchemaPin` file, on which lint 9 and C1 depend](#the-schemapin-file-on-which-lint-9-and-c1-depend)
  - [The suite's home, wired](#the-suites-home-wired)
  - [The five joint fixtures, and the authorability gate on each](#the-five-joint-fixtures-and-the-authorability-gate-on-each)
  - [The obligation register, with the `Operand` column lint 9 reads](#the-obligation-register-with-the-operand-column-lint-9-reads)
  - [The SDK surface: eight contracts, one of them partly shipped](#the-sdk-surface-eight-contracts-one-of-them-partly-shipped)
  - [The catalog read shape, and which side moves](#the-catalog-read-shape-and-which-side-moves)
  - [The `status` wire vocabulary, pinned](#the-status-wire-vocabulary-pinned)
  - [Event schema versioning, in the direction that matters](#event-schema-versioning-in-the-direction-that-matters)
  - [Dedup and ordering, over a `sequence` the gear does not assign](#dedup-and-ordering-over-a-sequence-the-gear-does-not-assign)
  - [The replay body core, which already ships](#the-replay-body-core-which-already-ships)
  - [Bootstrap, both arms, with the gear's projector as the first consumer](#bootstrap-both-arms-with-the-gears-projector-as-the-first-consumer)
  - [Lints 1 and 2: the PRD's id and acceptance-criteria universe](#lints-1-and-2-the-prds-id-and-acceptance-criteria-universe)
  - [Lints 3, 4, 5 and 6: what the design set declares about itself](#lints-3-4-5-and-6-what-the-design-set-declares-about-itself)
  - [Lints 7 and 8: the column and schema surfaces](#lints-7-and-8-the-column-and-schema-surfaces)
  - [Lint 9: the pin against the register](#lint-9-the-pin-against-the-register)
  - [The gate that runs the nine, which does not exist and is not a restoration](#the-gate-that-runs-the-nine-which-does-not-exist-and-is-not-a-restoration)
  - [The four design-introduced names exist as named seams](#the-four-design-introduced-names-exist-as-named-seams)
- [6. Acceptance Criteria](#6-acceptance-criteria)
- [7. Known unknowns](#7-known-unknowns)
  - [Carried verbatim from `design/12` §6](#carried-verbatim-from-design12-6)
  - [Raised here rather than carried](#raised-here-rather-than-carried)
  - [Owed to other documents, recorded and deliberately not edited](#owed-to-other-documents-recorded-and-deliberately-not-edited)

<!-- /toc -->

## 1. Feature Context

### 1.1 Overview

This feature turns the registry's boundaries into **enforced contracts rather than assumptions**. It
owns the registry↔plan-price **seam suite** every "CI-verified" and "seam-suite-asserted" phrase in
this design set points at, the **shared schema pin**, **event schema versioning with the
replay/bootstrap contract**, the **SDK surface** of `PRD.md` §9, the monetization-traceability
obligation, and the design set's own **completeness checks** — nine doc-plane lints that catch an
unclaimed requirement, an unowned error code, an unpaired door or an unnamed event before a reviewer
has to.

It is the **tenth of twelve** features to be authored — `features/read-models.md` and
`features/bulk-promotion.md` do not exist at `0771f15ae` — and the last with an unwritten foreign
seam of its own. It is also the only one whose deliverables are **artifacts outside the gear's own
tables**: a fixture corpus, a
committed pin file, a versioned SDK, and a set of lints.

### 1.2 Purpose

Every cross-gear promise made in the nine features written before it — 2.1–2.7, 2.10 and 2.11;
2.8 and 2.9 are unwritten — converges here as something *assertable*: a
fixture, a pinned schema, or a lint. A promise that cannot be asserted is re-labelled an open
obligation and stays visible as one.

The second purpose is the one the feature is easiest to under-read: **the register of what is owed is
itself a deliverable.** `design/12` C4 makes an assertion authorable only once the counterpart's
acceptance criterion exists, so most obligations are OWED by construction — and the value of the
register is that the debt is legible and reviewed whenever a counterpart lands, not that it is small.

### 1.3 Actors

| Actor | Role |
|-------|------|
| `cpt-cf-bss-products-actor-plan-price` | The primary seam counterparty — schema pin, obligations, joint fixtures |
| `cpt-cf-bss-products-actor-events-audit` | Transport of the versioned events the replay contract rides |
| Every consumer actor | Bound by the consumer-obligation register |

### 1.4 References

- [`../PRD.md`](../PRD.md) §6.12 (`cpt-cf-bss-products-fr-plan-price-seam`,
  `cpt-cf-bss-products-fr-monetization-traceability`), §6.7
  (`cpt-cf-bss-products-fr-event-versioning-replay`), §9's seven id-bearing blocks, §12 AC #29, #36,
  #37 and #38's enumeration, §15 (the suite's owner and home, and the event-log retention number).
- [`../DECISIONS.md`](../DECISIONS.md) **P-D-01** (the envelope discipline the versioning rides),
  **P-D-03** (the joint watermark fixture), **P-D-12** (the pin's derived membership), **P-D-15**
  (every inbound machine contract of §9.2), **P-D-18** (the release half of the freeze
  acknowledgment), **P-D-20** (`SkuRetired` re-announcement), **P-D-24** (the `state` phase), **P-D-27**/**P-D-29** (the
  event body core and the committed `internalRevision`), **P-D-30** (authorization is a pre-pipeline
  gate), **P-D-32** (`ENTITY_TERMINAL` widened), **P-D-34** (the act as the event-declaration unit),
  **P-D-35** (the five items the set already forced), **P-D-37** (one code per audit row),
  **P-D-36** (the phase unit withdrawn; the slice is the declaring unit), **P-D-38** (a refusal is
  never replayed), **P-D-43**/**P-D-44**/**P-D-45** (the lint grammars, the named artifacts, the
  authored registers), **P-D-47** (the broker's server-assigned `sequence`), **P-D-48** (the v1
  participant set).
- [`../design/12-consumer-contracts.md`](../design/12-consumer-contracts.md) — the slice. Its §2.1,
  §2.3, §2.4 and §3.2 carry the **normative steps and the normative lints** — §2.4's rows are
  numbered `p1` instruction rows ending in `inst-` ids, the same form as §2.1's — and its §2.2 and §3.1
  carry the **normative roster and the traceability obligation**. This document declares the `flow`
  and `algo` ids of §2.1, §2.3, §3.1 and §3.2, and carries the actor, the scenarios, the
  Input/Output and the boundary. **§2.2's and §2.4's `contract-` ids stay in the slice** — a FEATURE
  may not declare a `contract-` id, and minting one here would create a second definition site.

**Requirements**: `cpt-cf-bss-products-fr-plan-price-seam`,
`cpt-cf-bss-products-fr-event-versioning-replay`,
`cpt-cf-bss-products-fr-monetization-traceability`,
`cpt-cf-bss-products-fr-deprecation` (the consumer-side adoption block only),
`cpt-cf-bss-products-fr-freeze-atomicity` (the consumer-observable half),
`cpt-cf-bss-products-nfr-backward-compatible-evolution`

**Principles**: `cpt-cf-bss-products-principle-forward-only`

**Constraints**: `cpt-cf-bss-products-constraint-broker-native-events`

**Components**: `cpt-cf-bss-products-component-consumer-contracts`

**Sequences**: **none** — DECOMPOSITION §2.12 states it: the seam is asserted over the sequences the
other features own.

**§9's seven ids are claimed by their owning features, not here.** `interface-authoring-publish` and
`contract-registry-events` are `01-foundation`'s, `interface-read-model` is `08-read-models`',
`contract-sku-reference-count` is `07-reference-signal`'s, and `contract-increment-request`,
`contract-freeze-ack` and `contract-bundle-composition-signal` are `06-catalog-version`'s. This
feature specifies the **suite over** that surface, which is not a coverage claim.

**The code surface this feature is written against, measured at `0771f15ae`.** Every DoD in §5 names
what exists. The measurements below are the ones it rests on.

- **`products-sdk` is 205 lines in three files** — `lib.rs` (19), `models.rs` (152), `api.rs` (34) —
  and its whole public surface is the `ProductsClient` trait with two methods, `get_product` and
  `get_sku`, plus `EntityKind`, `LifecycleState`, `Product` and `Sku`. `inst-sdk-surface` names
  **eight** surfaces; **one** of them ships, in its narrowest form: no authoring, no publish, no
  idempotency key, no `If-Match`, no intent.
- **The shipped `Sku` carries three of the ten members of `inst-sdk-catalogsku`'s read shape.**
  (The read shape is ten members; the **pin** is a different and smaller set — **P-D-12** puts
  `skuCode` and `name` explicitly outside it, *"read only by a pick-list that validates nothing, so
  drift is cosmetic and its absence must not read as an oversight"*.) Its seven
  fields are `sku_id`, `tenant_id`, `product_id`, `sku_code`, `lifecycle_state`,
  `internal_revision`, `published_version` — so `sku_id`, `sku_code` and `status` (under the name
  `lifecycle_state`) are present, and `name`, `metering_unit`, `plan_tier`, `sellable`,
  `usage_type_ref`, `composition_pending` and `type` are absent. The struct's own doc says why:
  *"The capability columns a SKU carries — typing, `sellable`, `PlanTier`, the accounting codes, the
  metering unit — are not here. They belong to the features that own their rules, and a consumer
  reads them from those."*
- **`LifecycleState` already ships all five states with a `parse`**, which is most of what the
  `status` pin asks for. What is missing is the wire-subset annotation, not the enum.
- **`schema-pin.toml` does not exist.** `design/12` §4 names it
  `gears/bss/products/products-sdk/schema-pin.toml`. It is the artifact C1, lint 1 and lint 9 all
  read, so **one** of the nine lints has no input. The pin's other readers are `inst-ss-home` and
  `inst-ss-pin`, which are the CI job and the artifact's own versioning rule — not lints. Lint 1
  reads the PRD's id universe and no pin.
- **The fixture home exists and nothing is wired to it.**
  `gears/bss/fixtures/bss-fixtures/` is present; neither `products` nor `products-sdk` declares a
  dependency on it.
- **The slice's own "gated by nothing today" is true at HEAD.** `.github/workflows/docs.yml` holds
  exactly one job, `Check Markdown Links`.
- **Eight events ship, in two rosters of different shapes**: `ProductCreated`, `ProductDiscarded`,
  `ProductHeadSaved`, `ProductPublished` and the four `Sku*` equivalents, enumerated as
  `&[(&str, &str, &str)]` in `infra/broker_tests.rs` and as `&[&str]` in `infra/events_tests.rs`. A
  ninth event has to be added in both.
- **`EventBodyCore` ships exactly the five fields `inst-rc-body` names** — `tenant_id`,
  `entity_kind`, `entity_id`, `internal_revision`, `lifecycle_state` — with `PublishedEventBody`
  carrying the `publishedVersion` half. `SCHEMA_REFS` and the per-event `*_PAYLOAD_TYPE` constants
  ship too, which is the operand the versioning test needs.

**One number the slice states is wrong, and it is the one a lint would be built to.** `inst-cc-rbac`
and open item 14 both say lint 3's population is **fourteen** `` `METHOD /bss-products/v1/…` ``
spans. Measured over the design set with the table-cell escaping normalised, it is **seventeen**, and
`design/05-governance` §3.2's `Doors` column names **the same seventeen** — so the lint passes today.
The count was **wrong when it was written**: at `5977aec64`, the commit that introduced it, the
normalised population was already seventeen, and a diff of that commit's routes against
`0771f15ae`'s is empty. §7 carries it.

## 2. Actor Flows (CDSL)

Each flow below is **declared here and stepped in
[`../design/12-consumer-contracts.md`](../design/12-consumer-contracts.md) §2**, whose steps are the
normative ones. What this section carries is the triggering actor, the scenarios and the boundary.

### The seam suite

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-seam-suite`

**Actor**: `cpt-cf-bss-products-actor-plan-price` as the counterparty; the CI job as the executor

**Success Scenarios**:

- **One CI job consumes both gears' SDKs and the `SchemaPin`, and fails on any divergence** in the
  C1 pinned fields. Its home is `cf-gears-bss-fixtures`, which already exists; the suite is designed
  to run unchanged in the `api-contracts` home §15 proposes instead.
- **The pin's asymmetry is the enforcement.** The `SchemaPin` is a committed artifact versioned with
  the SDK, so a registry-side change to a pinned field bumps it through the ordinary review of
  **both** gears — and a one-sided bump fails the other side's CI. Nothing detects the divergence;
  the pin makes it impossible to land quietly.
- **The joint fixtures grow with their counterparts** (C4). Five are named: the **P-D-03 watermark**
  fixture, which is the retirement joint contract end-to-end and is deliberately first; the
  **adoption-block** fixture; the **usage-binding** fixture; the **grandfathered-resolution**
  fixture, where a frozen snapshot resolves byte-identically after registry churn; and the
  **correction** fixture, where `SkuImmutableFieldCorrected` makes pricing re-validate.
- **The obligation register is the suite's backlog.** Fourteen rows: twelve OWED in some form, one
  `assertable now`, one `deferred with post-v1 EOL`. **None is asserted yet** — the register is what
  makes that legible.

**Error Scenarios**:

- **A pinned field diverging between the gears fails CI**, on whichever side did not move.
- **A runtime divergence fails closed**, and the failure is pricing-side: the dependent plan publish
  is rejected.
- **An unauthorable assertion stays listed as OWED and is never silently dropped** (C4). Two register
  rows are owed on a *different* test — the counterpart raises no code — and §7 carries the
  disagreement between the two authorability criteria rather than resolving it here.

**Boundary, and what this flow deliberately does not claim.**

**It does not implement the counterparts.** Every obligation names its owing gear, and a joint
fixture written before the counterpart's rule exists **passes vacuously** — which is why two rows are
marked OWED on both sides rather than asserted on one.

**It does not own the broker's transport**, which is Common Core's.

**It cannot assert the §15 opens.** Freeze acknowledgments, the composition signal and watermark
delivery are assertable only once those questions close, and the register carries them as owed
meanwhile.

**And it is not gated.** The suite is a specification until someone owns and runs the job; §7 carries
that as the set's last organizational dependency, and it is repo-tooling work rather than anything
this feature can decide.

### Event versioning, replay and bootstrap

- [ ] `p1` - **ID**: `cpt-cf-bss-products-flow-replay`

**Actor**: `cpt-cf-bss-products-actor-events-audit` for transport; every event consumer as the
subject of the contract

**Success Scenarios**:

- **Every event schema is a versioned artifact in `products-sdk`**, and the CI compatibility test
  runs C2's actual direction — an old `vN` consumer deserializing a `vN+1` payload whose new fields
  are optional with defaults. The reverse direction, new code reading old fixtures, is the trivial
  half and is asserted too. The `09-bulk-promotion` export-artifact schema joins the same corpus.
- **Dedup and ordering detection beyond the idempotency window ride
  `(tenant, aggregate, sequence)`**, where `sequence` is the broker's server-assigned read-side value
  per `(topic, partition)` (**P-D-47**). The gear sets no `partition_key`, so the broker's default
  puts every event of one tenant on one partition and `sequence` is monotonic across them.
  **Detection needs monotonicity, not density** — the gaps neighbouring aggregates leave in the same
  partition are expected and must not be read as loss. **Within** the window the dedup key is the
  event `id`, minted once at enqueue and repeated by every delivery attempt.
- **Replay reads the body core every Foundation event carries** —
  `{tenantId, entityKind, entityId, internalRevision, lifecycleState}`, with `publishedVersion`
  additionally on `*Published` — and `internalRevision` is the value **as committed by the act**
  (**P-D-29**), so a consumer correlating an event to an ETag compares directly rather than adjusting
  by one. `lifecycleState` is the discriminator on `ProductHeadSaved`/`SkuHeadSaved`, which cover a
  save on a `draft`, `published` or `deprecated` head alike.
- **Bootstrap has two arms** (C3). A published-scope consumer initialises from the latest
  `CatalogVersion` under `browse` intent plus the event tail from that version's instant — **or, in a
  tenant with zero published versions, from the empty catalog plus the whole retained tail.** The
  anchorless arm exists because `08-read-models`' projection table — one, `products_read_entity` — is called rebuildable *"without
  loss"* and this is the only case with no anchor to rebuild from.
- **The gear's own projector is the contract's first consumer** and therefore its permanent
  conformance probe: it obeys the same bootstrap contract internally.

**Error Scenarios**:

- **A consumer checkpoint older than the retained tail fails loudly**, with the named remedy —
  re-bootstrap. It is not silently truncated and not silently restarted.
- **A refusal is never replayed** (**P-D-38**): a refusal stores nothing and releases the key, so a
  retry on the same key **runs** and gets a fresh verdict, which is what a client refused
  `STALE_REVISION` needs after re-reading the head. An idempotent replay only ever reproduces a
  success. `IDEMPOTENCY_KEY_IN_FLIGHT` (409) still means the first attempt is running.

**Boundary.** The contract's single configuration dependency is the **event-log retention window**,
which **MUST** cover the bootstrap gap and whose number is a `PRD.md` §15 open. Until it has a value
the replay contract is words, and §7 says so rather than picking one.

**The whole contract rides an arm that is inert in every real deployment.** Nothing in the workspace
registers `dyn EventBrokerApi` except this gear's own `broker_tests.rs`, so the broker producer path
— which every replay, dedup and bootstrap obligation depends on — has no production registration.
That is a standing debt of `01-foundation`'s, restated here because this feature's flow is the one it
makes untestable end-to-end.

**The fixtures do not wait on it** (**P-D-58**). Their transport is `event-broker-sdk`'s own
`MockBroker`, under the `test-util` feature this gear already takes in dev-dependencies, registered
into `ClientHub` as `dyn EventBrokerApi` — the registration a production boot performs, never a
producer injected past the hub. **What a green suite licenses is narrower than the obligation**: the
contract holds over this gear's own path with a conforming transport. That events reach consumers in
production is a different claim, it depends on the missing registration, and **no DoD here owns it**.

## 3. Processes / Business Logic (CDSL)

Each process below is **declared here and specified in
[`../design/12-consumer-contracts.md`](../design/12-consumer-contracts.md) §3**.

### The completeness checks

- [ ] `p1` - **ID**: `cpt-cf-bss-products-algo-coverage`

**Input**: this design set and `PRD.md` — the requirement ids, the acceptance-criteria enumeration,
the slices' Traces-to lines, their §4 table and column declarations, `DECISIONS.md`'s propagation
fields, the `EventRegister`, `design/05` §3.2's RBAC catalog, the `ObligationRegister`'s `Operand`
column, and the `SchemaPin`.

**Output**: nine pass/fail verdicts, each naming the id, code, event, door, column or field that
failed and the rule it failed.

**Boundary**: these are **doc-plane** lints. Not one of them reads the gear's source, and none is a
proof — each carries the blindness it cannot cover, and those statements are part of the
specification rather than caveats on it.

**What the nine read, which is how §5 groups them.** Lints 1 and 2 read the PRD's id and
acceptance-criteria universe; lints 3, 4, 5 and 6 read what the design set itself declares; lints 7
and 8 read the column and schema surfaces; lint 9 reads the pin against the register.

**Two of the nine have no input at this commit**: lint 9, because `schema-pin.toml` does not exist,
and lint 4, because the `EventRegister` is declared with no rows. A third — lint 3 — has a complete
input and a **wrong stated population**. So the honest state is: **one lint green and mis-numbered,
two blocked on artifacts, six executable.**

**The whole set is enforced by nothing.** `.github/workflows/docs.yml` holds one job and it is a
link checker. The slice states this and it measures true; §7 records that enforcing the nine is
**building** something rather than restoring it, commit `21a149fda` having removed the previous
tooling deliberately and priced the loss in as many words.

**No error code is declared by this feature.** Every code this document names — `STALE_REVISION`,
`IDEMPOTENCY_KEY_IN_FLIGHT`, `METER_USAGE_TYPE_UNBOUND`, `METER_DIMENSION_UNDECLARED`,
`UNRECOGNIZED_UNIT`, `ENTITY_TERMINAL` — is registered by the slice that raises it, and lint 2's rule
that a code has **exactly one declaring slice** is the reason this feature adds none. Stated
explicitly because all eight sibling FEATUREs carry an error-taxonomy section and a reader must be
able to tell an exclusion from an omission.

### Monetization traceability

- [ ] `p2` - **ID**: `cpt-cf-bss-products-algo-monetization-traceability`

**Input**: `PRD.md` §17.2's map, and the table and column declarations of every slice's §4.

**Output**: a lint verdict — a registry schema surface matching §17.2's first left-column row
(`flat`, `per-seat`, `tiered`, `volume`, `hybrid`, `commitment`) is a **failure**, the `usage` row
being excluded because AC #37 leaves only `usage` a footprint here.

**Boundary**: the §17.2 map itself is the deliverable and it exists in the PRD. **This feature's duty
is keeping it true**, not authoring it — the absence of a monetization marker in the registry is
intentional, so a new SKU column matching that row is a lint failure and not a feature.

**Its stated blind spot, which is part of the specification**: a marker arriving as an SDK field or
an event payload rather than a column is outside §4 and the lint stays green while the footprint
exists. Widening it waits on the SDK shapes being declared structurally — which is
`cpt-cf-bss-products-dod-sdk-surface`'s work, so the two are coupled — and that DoD's own state is
eight named surfaces of which one is partly shipped, so the coupling is total rather than partial.

## 4. States (CDSL)

**No state machine is declared by this feature.**

`design/12` declares no `state-` id and **no tables at all** — its §4 opens *"None owned as
tables"* and its first paragraph ends *"That absence of storage is by design."* (§4 continues with
P-D-44's artifact table and §4.1's fifteen-row map.) The artifacts are a fixture corpus, a
committed pin file, a versioned SDK crate and a set of lints, none of which has a lifecycle the gear
drives.

The one thing in this feature that does have states is the `ObligationRegister`'s `Status` column —
`asserted`, `owed`, `assertable now`, `deferred` — and it is **a roster's column, not a machine**: no
door moves a row, C4 moves it when a counterpart lands an acceptance criterion, and nothing in the
gear reads it at runtime.

Because §4 declares nothing, **this feature mints no `inst-` id at all** — as on
`features/catalog-version.md`, `features/reference-signal.md`, `features/retention-erasure.md` and
`features/clone.md`. The nineteen instruction ids this feature's flows run on are `design/12` §2's
and §3's and are stepped there.

## 5. Definitions of Done

Every DoD below names types, functions, files and tests **that exist at `0771f15ae`** wherever one
exists. On this feature that is rarer than on any of its siblings: the contracts are stated as prose
and almost none of their artifacts exist, so most DoDs create rather than extend — and each says
which.

### The `SchemaPin` file, on which lint 9 and C1 depend

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-schema-pin`

`gears/bss/products/products-sdk/schema-pin.toml` exists, versioned with the SDK. **It does not
exist today**, and it is the artifact **C1** and **`inst-cc-pin`** (lint 9) read, plus the CI job
`inst-ss-home` and the versioning rule `inst-ss-pin`. So **one** lint is blocked on it — and this DoD
is still the spine of the feature, because the suite itself cannot run without the file.

**TOML, so a gate reads it without parsing prose** — `design/12` §4 fixes the format for that reason.

**Each field member carries a comparability flag** (**P-D-57**); **a member spelled differently per
side carries per-side names** (**P-D-66**: the `status` entry's token is the seam's name, with
`registry-field = "lifecycle_state"` as the annotation the job resolves the registry side by); and
**`CatalogVersion` is an entry of kind `surface`** (**P-D-65**): `kind = "surface"`, a `delegated-to` naming the counterpart port
trait, and **no comparability flag** — the job neither compares it nor asserts its absence, its drift
protection being the adapter the pricing-side port doc promises, which the compiler checks. That is
the pin file's schema for a surface-level member, which this DoD was blocked on. Membership stays P-D-12's rule and nothing
is dropped for being unshipped; the flag says only whether the member is comparable against the SDK
surface *yet*, and it is authored conservatively — `comparable` only once both the column and the SDK
member ship. That is what keeps `cpt-cf-bss-products-dod-seam-suite-home`'s job green from the day the
pin lands, without shrinking the pin to shipped-only.

**Its membership is derived, not listed** (**P-D-12**): the pin covers exactly the catalog-field
operands the `ObligationRegister`'s guards read. The v1 set is `skuId`, `type`, the metering-unit
declaration (`unit` **and** `usageTypeRef` as an atomic pair), `PlanTier`, `status` **with its value
vocabulary**, `sellable`, `compositionPending`. `CatalogVersion` is pinned **as a surface, not a
field**, and `skuCode` and `name` are deliberately out — pick-list display, where drift is cosmetic.

**Six of the eight pinned fields have no shipped operand** — counting the metering pair as its two
tokens, which is how P-D-12 words it. Absent from the SDK `Sku`: `type`, `unit`, `usageTypeRef`,
`PlanTier`, `sellable`, `compositionPending`. Present: `skuId` and `status`, the latter under the
name `lifecycle_state`. (Counting the pair as one token the figure is five of seven; the count is
stated with its convention because both readings appear in the set.)

So the pin's v1 membership is derivable from the design set and **not yet checkable against the
SDK**, and this DoD is met by the file with its derived membership, not by a passing comparison.

**Implements**: `cpt-cf-bss-products-flow-seam-suite`

**Touches**:
- Files: `gears/bss/products/products-sdk/schema-pin.toml`
- Entities: `SchemaPin`

### The suite's home, wired

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-seam-suite-home`

The suite is one CI job over **`cf-gears-bss-fixtures`**, consuming both gears' SDKs and the
`SchemaPin`, and it is **two-sided** (**P-D-57**): it fails on any divergence in the C1 fields the pin
marks **comparable**, and on the **presence** of any C1 field the pin marks not-yet-comparable. So a
member that ships while still flagged fails the job **in the change that shipped it**, and the flag
cannot rot into a standing excuse.

**The home exists and nothing is wired to it.** `gears/bss/fixtures/bss-fixtures/` is present and its
own description names it *"the only fixture crate a gear may take as a production dependency"* —
and **neither `products` nor `products-sdk` declares a dependency on it**. The harness,
`cf-gears-bss-fixtures-conformance`, is a **dev-dependency only**, as pricing takes it.

So this DoD is: the dependency declared, the fixtures placed, and a job that runs them. **It is not
met by the fixtures existing** — an unrun job asserts nothing, which is what §7's owner question is
about.

**There is a second missing wire, and it is in the same manifest.**
`gears/bss/fixtures/bss-fixtures/Cargo.toml` declares **no dependency on `event-broker-sdk`**, with
or without `test-util` — and **P-D-58** makes that crate's `MockBroker` the transport under every
event-bearing fixture, registered into `ClientHub` as `dyn EventBrokerApi` rather than injected past
it. So the dependency this DoD owes is two edges, not one.

**Implements**: `cpt-cf-bss-products-flow-seam-suite`

**Touches**:
- Crates: `cf-gears-bss-fixtures`, `cf-gears-bss-fixtures-conformance` (dev only)
- Entities: `SeamSuite`

### The five joint fixtures, and the authorability gate on each

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-joint-fixtures`

Five fixtures, in `design/12` §2.1 `inst-ss-fixtures`' order. **C4 admits only the first**: three of
the remaining four name a counterpart that raises no code, so writing them in this order lands the
vacuous green check this DoD's closing rule forbids.

1. **The watermark fixture** (**P-D-03**) — pricing produces `SkuReferenceCount`, the registry's
   predicate answers, end to end. **First, deliberately**: it unblocks retirement, the
   highest-value seam, and it is the P-D-03 joint build's own acceptance.
2. **The adoption-block fixture** — on pricing AC #82's own `When`, which is retirement or
   unpublishing. **The plain-deprecation arm is not authorable**: a `SkuDeprecated` from
   `04-lifecycle`'s `inst-lc-deprecate` with no retirement behind it has no counterpart AC, so by C4
   that arm stays an ask in the register.
3. **The usage-binding fixture** — pricing's meter-binding rule against a registry declaration,
   including the deprecated-bound-unit arm. **Both sides are owed**: that rule raises neither
   `METER_USAGE_TYPE_UNBOUND` nor `METER_DIMENSION_UNDECLARED`, and pricing's own foundation calls
   the binding deferred, so a fixture written today passes vacuously.
4. **The grandfathered-resolution fixture** — a frozen snapshot resolves **byte-identically** after
   registry churn.
5. **The correction fixture** — `SkuImmutableFieldCorrected` makes pricing re-validate.

**A vacuously-passing fixture is worse than an absent one**, because it converts a debt into a green
check. This DoD is met only where each fixture's counterpart raises the code the fixture asserts;
otherwise the row stays OWED and the fixture is not written.

**Implements**: `cpt-cf-bss-products-flow-seam-suite`

**Touches**:
- Crates: `cf-gears-bss-fixtures`
- Entities: `SeamSuite`

### The obligation register, with the `Operand` column lint 9 reads

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-obligation-register`

`design/12` §2.2's register is the normative roster: **fourteen rows**, each carrying the obligation,
its owing consumer, its source, its **`Operand`** and its status. Twelve are `owed` in some form; one
is `assertable now` (declare intent on resolution, the registry-side refusal being built); one is
`deferred with post-v1 EOL` (consume `mustMigrateBy`, a field never populated in v1).

**The `Operand` column exists because reading operands out of prose was measured and failed** —
it yielded ten fields against the pin's eight and left `PlanTier` pinned with no row naming it. So
the column is lint 9's **only** input, and any prose beside the tokens is ignored.

**The cell grammar is one token per pin member** (**P-D-43** arm 3), each either a catalog field name
or one of three non-field markers: `(surface)`, `none in v1`, `payload`. **That the tokens are
comma-separated is `design/12` §3.2 lint 9's addition**, not P-D-43's — arm 3 states the token rule
and that *"Prose beside the tokens is ignored"*, and names no separator.

**The cells did not follow that grammar when this DoD was written, and this DoD is where they were
made to.** Measured cell by cell over all **fourteen**, which is the register's own row count —
**this census is the pre-repair state**, kept because the repair paragraph below is only readable
against it:

| the cell is… | count |
|---|---|
| **strictly** conforming — a bare token, no backticks | **0** — every field token in the table is backticked |
| one backticked field token and nothing else | **3** — `compositionPending`, `sellable`, `skuId` |
| leading with a backticked **non-field** token | **6** — five `` `CatalogVersion` (surface) ``, one `` `SkuRetired` payload `` |
| joining its operands with `+` rather than a comma | **4** — the `status`, `PlanTier`, metering-unit and `type` rows |
| a marker plus trailing prose | **1** — `` none in v1 (…) `` |

3 + 6 + 4 + 1 = 14, which closes. **The slice's own count closes on neither reading**: `design/12` §6
row 15 says *"three of the thirteen"* and *"three more"*, which is twelve against a stated thirteen
and an actual fourteen. That is recorded against its owner rather than repaired in the carry.

**The repair landed** (**P-D-63**, amending P-D-43 arm 3): a non-field marker consumes exactly one
preceding backticked identifier as its annotation, `+` is refused and the four cells that used it are
normalized to comma-separated pin tokens — so **all fourteen cells parse**. The two things the
grammar had not settled land with it: **a backticked catalog field name is a token, never the
ignorable prose** (the census's own conforming class assumed it), and **a `none in v1` cell is
outside lint 9's coupling population by construction**, the marker rule already carrying that.

**And the roster is now asserted rather than stated.** `products-sdk/src/pin_lint.rs` reads §2.2's
table by its own header and pins the census this DoD states: **fourteen rows**, twelve `owed` in
some form, one `assertable now`, one `deferred` — plus P-D-63's outcome, that no cell reintroduces
the `+` the grammar refuses. A row added, removed or moved between statuses fails in the change
that moves it, which is what makes the register a roster rather than a list. The status is
classified by its **leading class**, never by a substring: one row's prose quotes pricing's own
document calling a binding *"deferred"* while the row itself is `owed`, so a `contains` test reads
prose and miscounts — the very defect the `Operand` column exists to avoid, one column over.

**Implements**: `cpt-cf-bss-products-flow-seam-suite`

**Touches**:
- Entities: `ObligationRegister`

### The SDK surface: eight contracts, one of them partly shipped

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-sdk-surface`

`products-sdk` mirrors `PRD.md` §9. `inst-sdk-surface` names eight things it must carry:

| # | Surface | Ships at `71450e924` |
|---|---|---|
| 1 | The authoring/publish client — idempotency keys, `If-Match` and intent semantics **part of the contract**, breaking = major | no |
| 2 | The read-model client | **partly** — `ProductsClient::{get_product, get_sku}` |
| 3 | The increment-request client, with its three-way error taxonomy (not wired / unreachable / unusable) | **yes** — `increments::IncrementRequests`, both bindings |
| 4 | The watermark client (`contract-sku-reference-count`) | **yes** — `watermarks::WatermarkPosts`, both bindings |
| 5 | The freeze-acknowledgment client **with its release half** (**P-D-18**) | no — the ack/release **doors** ship; the typed client does not |
| 6 | The bundle composition-completed signal | no |
| 7 | The event payload types | no |
| 8 | The error-code enum — `design/01` §3.3 plus every slice's registered codes, renames breaking | no |

The table was first measured at `0771f15ae`, which predates the build groups; re-measured at
`71450e924` rows 3 and 4 ship as `ClientHub`-resolved traits with their REST/S2S doors as the
out-of-process bindings, which is two of §9.2's four inbound machine contracts. Row 5's doors
exist (`06`'s ack and release), so what it still owes is the **client** — the trait and its
in-process binding — not the wire.

**Four of them are the whole of §9.2's inbound machine contracts** (**P-D-15**) — rows 3, 4, 5 and 6
— and all four are typed clients resolved from `ClientHub`, with the REST and S2S doors as their
out-of-process bindings. The shipped `api.rs` doc already names `ClientHub` as the resolution point,
so the seam is chosen and unpopulated.

**One decision the shipped code already makes and this DoD keeps.** Every method returns
`CanonicalError` rather than a gear-local error type, because *"the gear's single
`From<DomainError>` ladder is the authoritative classification, and a second one on this port would
be a second place for a rejection to be categorised."* Row 8's error-code enum is therefore a
**documented vocabulary**, not a second error type on the port.

**Implements**: `cpt-cf-bss-products-contract-sdk` as `design/12` §2.4 states it

**Constraints**: `cpt-cf-bss-products-constraint-broker-native-events`

**Touches**:
- Crates: `products-sdk`
- Modules: `products_sdk::api`, `products_sdk::models`

### The catalog read shape, and which side moves

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-catalogsku-shape`

The catalog read shape is **`CatalogSku`-superset-compatible**, pricing's `ProductCatalogClientV1`
consuming it: `sku_id`, `sku_code`, `name`, `metering_unit`, `status`, `plan_tier`, plus `sellable`,
`usage_type_ref`, `composition_pending` and `type` landing consumer-side as additive.

**Three of the ten are present today and the SDK's own doc explains the absence as deliberate**, not
as an omission: the capability columns *"belong to the features that own their rules, and a consumer
reads them from those."* So this DoD is a **decision about which side moves**, and it states the
decision rather than assuming it: the superset lands on `products-sdk`'s read shape as those features
land their columns, and until then the pin's membership is derived from the design set rather than
compared against the type. **P-D-57** carries that forward into the job: the pin keeps every derived
member with a comparability flag, so the interval in which a member is derived but not
yet compared became a datum the CI reads rather than a period nobody encoded.

**Five of the ten have no shipped column either** — `metering_unit`, `plan_tier`, `sellable`,
`usage_type_ref` and `type`. The other five do: `sku_id` and `sku_code` on both entity tables,
`name` on `products_product`, `composition_pending` on `products_sku` (**P-D-35**, `NOT NULL DEFAULT
false`, with its guard clause on both engines), and `status` under the column name
`lifecycle_state`. So the DoD is blocked on `02` and `03` at the storage layer for five members and
on the SDK alone for two. It is met per member as each arrives, and the count is stated so a partial
surface reads as partial.

**Implements**: `cpt-cf-bss-products-contract-sdk`

**Touches**:
- Crates: `products-sdk`
- Modules: `products_sdk::models`

### The `status` wire vocabulary, pinned

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-status-vocabulary`

`status` and its value vocabulary are **normative pin members** (**P-D-12**). Browse serves
`published|deprecated` only — draft is never served and retired is history-only — and **the SDK enum
documents all five states with the wire subset named**.

**The pin entry is settled** (**P-D-66**): the token is **`status`** — the seam's name, fixed by
`CatalogSku`-superset-compatibility — with the registry-side source spelling recorded as an
annotation (`registry-field = "lifecycle_state"`), and the vocabulary is **two members**,
`published` and `deprecated`. Pricing's three-value doc is the display tolerance this DoD already
names, and the pin replaces it rather than adopting its list.

**Half of this ships and is not to be rebuilt.** `LifecycleState` already carries all five variants
with a `parse`. What the DoD adds is the **wire-subset annotation** and the pin entry.

**Why the pin exists at all, stated because the alternative looks harmless.** Pricing's own
documentation calls the field *"verbatim. Not an enum:"* — the right tolerance for display and
exactly the wrong one for a guard. Under an opaque-string reading, a renamed `deprecated` or an added
blocking state leaves pricing's adoption guard **accepting rather than erroring**. The pin replaces
that tolerance; it does not merely document it.

**Implements**: `cpt-cf-bss-products-contract-sdk`

**Touches**:
- Crates: `products-sdk`
- Modules: `products_sdk::models`
- Files: `schema-pin.toml`

### Event schema versioning, in the direction that matters

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-event-versioning`

Every event schema is a versioned artifact in `products-sdk`, and the CI compatibility test runs
**C2's actual direction**: an old `vN` consumer deserializing a `vN+1` payload carrying new optional
fields with defaults. The reverse — new code reading old fixtures — is asserted as well and is the
trivial half.

**Half the operand ships and the deciding half does not.** `SCHEMA_REFS` and the per-event
`*_PAYLOAD_TYPE` constants exist in `infra/events.rs`, and `events_tests.rs` already asserts
`schema_ref_for` resolves for the eight.

**But nothing in either crate can deserialize a payload.** `EventBodyCore` derives `Serialize` only
and is `pub(crate)`; `Deserialize` occurs **zero** times in `infra/events.rs`; and the destination
crate refuses the derive outright — `products-sdk/src/lib.rs` reads *"No serde derives live here.
The gear's REST DTOs own serde and map onto these types, which is the sibling `bss-ledger-sdk`'s and
`bss-pricing-sdk`'s arrangement and keeps a wire concern out of the contract"*, and its `Cargo.toml`
declares no serde at all.

So C2's direction — an old consumer **deserializing** a new payload — has no operand anywhere today,
and moving the schemas into `products-sdk` as written would put a wire concern into a crate whose
module doc forbids it. Where the deserializable payload types live is §7 row 29's, and this DoD is
met by the versioned artifacts plus whatever that answer names, not by the SDK move alone.

**The standing example is `mustMigrateBy`** — present in the schema and never populated in v1 — and
**the `09-bulk-promotion` export-artifact schema joins the same corpus**, so the discipline is
exercised rather than borrowed.

**A ninth event costs six sites in four files, not two roster edits.** Measured at `0771f15ae`:
its `*_PAYLOAD_TYPE` const and its `SCHEMA_REFS` row in `infra/events.rs`; its `catalog_event!` or
`catalog_publish_event!` invocation in `infra/broker.rs`; its arm of the typed-event `match` in
`enqueue` or `enqueue_published`; and **both** `THE_EIGHT` literals —
`&[(&str, &str, &str)]` in `infra/broker_tests.rs` and `&[&str]` in `infra/events_tests.rs`, both
deliberate literals, as each roster's own doc comment says.

**`events.rs` documents the failure a build to the smaller number produces**, which is why the count
matters: *"A ninth event registered in `SCHEMA_REFS` but not wired here reaches this variant, and a
no-broker deployment would have emitted it."* Two green test rosters and a runtime
`NoTypedEvent`.

**Implements**: `cpt-cf-bss-products-flow-replay`

**Touches**:
- Crates: `products-sdk`
- Modules: `infra::events`
- Tests: `infra::broker_tests::THE_EIGHT`, `infra::events_tests::THE_EIGHT`

### Dedup and ordering, over a `sequence` the gear does not assign

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-dedup-ordering`

The consumer contract states both keys and the suite fixtures both cases — a duplicate delivery and
an out-of-order one. **Both are assertable now** (**P-D-58**): `MockBroker` exports `StoredEvent` and
`CursorEntry`, so the stored log and the per-partition cursors are readable from a fixture, which is
the surface a duplicate and an out-of-order delivery are detected on.

- **Beyond the idempotency window**: `(tenant, aggregate, sequence)`, where `sequence` is the
  **broker's server-assigned read-side value per `(topic, partition)`** (**P-D-47**, which re-took
  P-D-27's slot because the broker's schema refuses a gear-assigned one).
- **Within the window**: the event `id`, which the SDK mints once at enqueue and every delivery
  attempt repeats.

**The monotonicity argument is the part an implementer gets wrong.** The gear sets **no**
`partition_key`, so the broker's default puts every event of one tenant on one partition and
`sequence` is monotonic across them. **Detection needs monotonicity, not density**: neighbouring
aggregates in the same partition leave gaps, and a consumer treating a gap as loss will re-bootstrap
on healthy traffic. The contract says so explicitly.

**Implements**: `cpt-cf-bss-products-flow-replay`

**Touches**:
- Modules: `infra::events`, `infra::broker`

### The replay body core, which already ships

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-body-core`

Replay reads the body core every Foundation event carries:
`{tenantId, entityKind, entityId, internalRevision, lifecycleState}`, with `publishedVersion`
additionally on `*Published` (**P-D-27**).

**This DoD builds nothing in the gear.** `EventBodyCore` ships with exactly those five fields and
`PublishedEventBody` carries the sixth. What it obliges is the **contract statement**: that
`internalRevision` is the value **as committed by the act** (**P-D-29**), so a consumer correlating
an event to an ETag compares directly rather than adjusting by one, and that `lifecycleState` is the
**discriminator** on `ProductHeadSaved`/`SkuHeadSaved`, which cover a save on a `draft`, `published`
or `deprecated` head alike.

Stating the off-by-one is the substance: it is the one thing about this core a consumer can get
wrong while every field is present.

**Implements**: `cpt-cf-bss-products-flow-replay`

**Touches**:
- Modules: `infra::events`

### Bootstrap, both arms, with the gear's projector as the first consumer

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-bootstrap`

- **The anchored arm**: the latest `CatalogVersion` through `06-catalog-version`'s resolver under
  **`browse`** intent, plus the event tail from that version's instant.
- **The anchorless arm**: in a tenant with **zero** published versions, the empty catalog plus the
  **whole retained tail**. It exists because `08-read-models`' projection table — one, `products_read_entity` — is called
  rebuildable *"without loss"* and this is the only case with no anchor.
- **A checkpoint older than the retained tail fails loudly**, naming re-bootstrap as the remedy.

**The gear's own projector obeys this contract internally**, which makes it the contract's first
consumer and its permanent conformance probe — so a divergence between the contract and the projector
is a defect in one of them and is detectable without a second gear.

**The one configuration operand has no value.** The event-log retention window **MUST** cover the
bootstrap gap, and the number is a `PRD.md` §15 open. This DoD is met by the contract and the probe;
the window is §7's.

**Implements**: `cpt-cf-bss-products-flow-replay`

**Touches**:
- Modules: `infra::events`

### Lints 1 and 2: the PRD's id and acceptance-criteria universe

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-lint-prd-universe`

**Lint 1 — requirement coverage.** Every `p1`/`p2` requirement-bearing PRD id — `fr-*`, `nfr-*`,
`interface-*`, `contract-*`, and the seven `usecase-*` ids — is claimed by **exactly one owner per
clause**: one slice for a whole requirement, or one slice per scope-qualified clause where a
requirement is deliberately split.

- **The qualifier grammar**: a claim is `id (<qualifier>)`; qualifiers compare as normalised strings;
  an **unqualified** claim conflicts with every other claim of the id; two identical qualifiers fail.
- **NFRs are claimed by id, never by position.** Slices used to cite "NFR #1, #2, #7, #10" while the
  lint keys on `nfr-*` ids, so it reported zero claims for all ten.
- **The id harvest is scoped to `PRD.md`.** The design set declares **fourteen**
  `cpt-cf-bss-products-contract-*` ids of its own — re-measured at `0771f15ae` and confirmed: 19
  distinct `contract-*` ids appear across the design set, of which **5 are declared in `PRD.md`**
  (`contract-registry-events`, `contract-sku-reference-count`, `contract-increment-request`,
  `contract-freeze-ack`, `contract-bundle-composition-signal`) and **14 in the design set**. An
  unscoped `contract-*` glob would pull all nineteen into the requirement universe.
- **The AC-existence check**: every **unqualified** `AC #N` cited in a slice resolves to a §12
  bold-numbered item of *this* PRD; a citation qualified by its gear (`pricing AC #82`, which this
  set carries ten times) resolves against that gear or is out of scope.

**The split-ownership exception, re-measured and corrected.** `design/12` says *"fourteen
requirements are owned by **two** slices each"*. Measured over every Traces-to line at
`0771f15ae`: **14 ids are claimed by more than one slice**, and **13 of them are pairs**. The
fourteenth, `cpt-cf-bss-products-nfr-scale-extensibility`, is claimed by **three** — `01`, `02` and
`06`. The count is right and the word *pairs* is wrong for one member, which matters because a
reviewer checking "all fourteen pairs carry a qualifier" would check two qualifiers where three are
needed. §7 carries it.

**What this lint cannot do, stated so nobody reads it as more**: it checks that a claim is
well-formed and unique, never that the slice covers the requirement, and it says nothing about
whether what a slice asserts *about* a cited AC is true.

**Lint 2 — the AC #38 map.** Every enumerated failure row **that a registry door can refuse** maps to
exactly one error code with **exactly one declaring *slice*** — a slice, not a door and not a
pipeline phase (**P-D-36** withdrew the phase unit; **P-D-35**'s rule makes a code have exactly one
declaring slice by construction).

**The map's arithmetic, re-measured.** `PRD.md` §12 AC #38 enumerates **fifteen** rows; `design/12`
§4.1's table carries fifteen; **twelve** are refusable by a registry door and carry a code, and
**three** are excluded — the retention-orphan alarm, which is deliberately an alarm and not an API
error; the `compositionPending` adoption duty, a consumer obligation with no registry door; and AC
#38's post-v1 EOL row, whose only candidate code refuses the feature rather than the named condition.
**The lint asserts the exclusion list is exactly those three** — an unexplained fourth exclusion
fails it.

Two properties of the map worth stating so a reader does not take them for slips: **rows 4 and 11
share `UNRECOGNIZED_UNIT`** (the lint requires one code per row, not one row per code), and the row
**count held at fifteen across P-D-44 while its membership changed** — a parent-child containment row
was withdrawn as unreachable and the de-listed/deprecated row split in two.

**Implements**: `cpt-cf-bss-products-algo-coverage`

**Touches**:
- Entities: `CoverageChecks`

### Lints 3, 4, 5 and 6: what the design set declares about itself

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-lint-declarations`

**Lint 3 — door×grant pairing.** Every declared route appears in the `Doors` column of `design/05`
§3.2's RBAC catalog. **The population is the declared routes** (**P-D-45**): the
`` `METHOD /bss-products/v1/…` `` code spans, one machine-readable form. Doors named only in prose —
"the fresh-zero door", "the watermark door", "the correction door" — are **outside** it, a prose name
having no enumerable form. That most grants carry no declared route is a gap registered in `design/05`
§6, not a lint failure; the stated direction is door ⇒ grant.

**Two measurements this DoD must carry, because a lint built to the stated numbers is wrong.**

1. **The population is seventeen, not fourteen.** Measured over the design set at `0771f15ae` with
   the escaping normalised: **17** distinct routes, and `design/05` §3.2's `Doors` column names the
   **same 17**, so the lint **passes today**. The stated fourteen was wrong at birth — at
   `5977aec64`, the commit that wrote it, the normalised population was already 17, and it has not
   changed since.
2. **The escaping forks every pipe-bearing token.** All **seven** routes containing `{products|skus}`
   exist in two textual forms: `{products\|skus}` inside §3.2's markdown table, where a cell must
   escape the pipe, and `{products|skus}` everywhere else. A lint matching the spans literally pairs
   **none** of the seven across that boundary. A naive count of the raw spans returns **24** by
   counting each of the seven twice. **The normalisation belongs in the lint's grammar** and no
   document states it; §7 carries that, and this DoD is not met by a lint that omits it.

**Lint 4 — event bookkeeping.** Every row of the `EventRegister` names its emitting instruction, and
every instruction the register names carries the event on its own row. `09-bulk-promotion`'s coalesced
summary event is **additive** — row-level domain events all emit.

**The register is authored, never harvested** (**P-D-45**), and the reason is a measurement rather
than a preference: five harvest passes over one tree returned **five different populations** — 31,
24, 32 and 35 events under four patterns — with **one** pass both inventing donor-gear events and
dropping `SkuCreated` and `SkuRetired` on a name-length filter, while the instruction attribution differed in
**28 of 31** rows. An emitting instruction is not recoverable from prose.

**So the register is declared and empty, and lint 4 is inert until it is filled** — per slice, by
whoever wrote the rule, with an explicit no-event entry where a state change emits nothing. Eight
events ship today, which bounds the registry-side rows but not the attribution.

**Lint 5 — register hygiene.** Every `P-D` **`Propagated`** list names only documents that restate
the decision, and every restating document appears. **The grammar** (**P-D-43**): **one** propagation
field, spelled `- **Propagated**`, naming each document by its **repo-relative path**; and a document
**restates** a decision exactly when it **cites the decision id**. *(That an `S<NN>` abbreviation is
never a legal name is `design/12` §3.2's addition, not P-D-43 arm 4's, which states only the path
form.)*

That definition is deliberately mechanical, and its cost is recorded rather than hidden: a document
can cite an id without carrying the claim. The lint does not see that difference **and is not meant
to**.

**Lint 6 — id uniqueness.** Every `inst-*` id is **declared** exactly once across the design set,
where a declaration is the id trailing its own numbered instruction row and a re-use is any other
mention.

- **The continuation grammar is what makes it buildable**: `design/01` legitimately carries several
  ids on more than one row, and those rows carry the id **parenthesised** — `(cont. inst-fd-etag)` —
  so a continuation can never read as a duplicate declaration.
- **The domain is `inst-*` alone** (**P-D-43**). `cpt-*` and `flow` ids are declared on unnumbered
  bullets and an actor id is a table cell, so under the stated grammar both kinds had **zero**
  declarations and the lint was red on a correct set by construction.
- **Its live violator is fixed**: the `08`/`12` `inst-rp-bootstrap` collision, renamed to
  `inst-rc-bootstrap` in the same commit that added the lint.

**Implements**: `cpt-cf-bss-products-algo-coverage`

**Touches**:
- Entities: `CoverageChecks`, `EventRegister` (lint 4's input, which has no declaration site — §7
  row 30)

### Lints 7 and 8: the column and schema surfaces

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-lint-surfaces`

**Lint 7 — identity materialization.** No table or projection other than `10-retention-erasure`'s
`IdentityRefMap` stores an operator identity — the lint that erasure's guarantee names and that
existed only as a citation before this slice.

**The lint reads column names** (**P-D-45**): a column holding an operator identity is named
`*_actor_ref`, and the lint asserts exactly one table declares such a column.

**Measured at `0771f15ae`, the stated pattern matches nothing and the assertion is red under either
reading.** No column in the design set ends in `_actor_ref` — `products_identity_ref` is a **table**
name and the identity-bearing column it declares is the bare `actor_ref`. Under the bare spelling
**four** tables declare it: `products_identity_ref`, `products_approval`,
`products_approval_decision` and `products_breakglass_session`. So *"exactly one"* is red at 0 under
the stated suffix and at 4 under the bare one, and §7 row 28 carries which the lint matches and what
the assertion should be.

**Its stated weakness is part of the specification**: a column named otherwise passes silently, so
the lint is green over the very defect it exists to catch. It is a naming discipline enforced at
review, not a proof — and the alternative priced against it was an explicit `identity:` field on all
34 table declarations.

**Lint 8 — no monetization marker.** A registry schema surface matching `PRD.md` §17.2's **first**
left-column row — `flat`, `per-seat`, `tiered`, `volume`, `hybrid`, `commitment` — fails the lint; the
`usage` row is excluded, AC #37 leaving only `usage` a footprint here.

**"Registry schema surface" means the table and column declarations of the slices' §4 sections**
(**P-D-45**). The six words being a fixed literal list, this lint needs a definition rather than an
artifact and is **executable as it stands**.

**Its stated blind spot**: a marker arriving as an SDK field or an event payload rather than a column
is outside §4, so the lint is green while the footprint exists. Widening it waits on the SDK shapes
being declared structurally, which couples it to
`cpt-cf-bss-products-dod-sdk-surface`.

**Implements**: `cpt-cf-bss-products-algo-coverage`,
`cpt-cf-bss-products-algo-monetization-traceability`

**Touches**:
- Entities: `CoverageChecks`

### Lint 9: the pin against the register

- [x] `p1` - **ID**: `cpt-cf-bss-products-dod-lint-pin-coupling`

Every `ObligationRegister` row whose guard reads a **catalog field** has that field in the
`SchemaPin`, and every pinned field is either an obligation operand or carries a recorded exclusion
reason. **This is what makes C1's membership a rule rather than a list.**

**A row whose operand is an event payload is outside the pin by construction** — the pin covers the
entity surface and not the event surface — and is excluded **by that reason** rather than by
omission. The `SkuRetired` re-announcement row is the one such row.

**The lint reads tokens only** (**P-D-43**, amended by **P-D-63**: a marker may consume one
preceding backticked identifier as its annotation, which the lint never looks up in the pin; `+` is
not a separator): a field token must appear in the pin; a marker token is outside the
**field-comparison population** by construction (**P-D-65** narrowed this from "outside the pin"): a
`` `CatalogVersion` (surface) `` token couples to the pin's `surface` entry in both directions, while
`payload` and `none in v1` couple to nothing — each carries its exclusion
reason in the row's prose. **Any prose beside the tokens is ignored, so a cell is never judged by
being read.**

**The two things that blocked this lint at `0771f15ae` are both gone, and the lint runs.** The
`SchemaPin` ships (`products-sdk/schema-pin.toml`, `dod-schema-pin`), the fourteen operand cells
parse under P-D-63's grammar, and the lint itself is
`products-sdk/src/pin_lint.rs` — it rides `cargo test`, so it fails **in the change**
that decouples the two sides rather than waiting for the nine-lint CI job (`dod-lint-gate`, still
owed outside the gear). It sits under `src/` behind `#[cfg(test)]` rather than in `tests/` because
the traceability scanner's registered roots for a BSS gear are `src`, `tests`, `<crate>/src`,
`<crate>/tests` and `<crate>-sdk/src`: **there is no `<crate>-sdk/tests` root**, so a marker in
this crate's `tests/` directory is invisible to the gate and this `DoD` could never be satisfied
from there. Its RED was probed in both directions: an unnamed pin member fails the
second direction with `PlanTier`'s own history in the message, and dropping a member the register
names fails the first.

**Why the coupling exists at all.** The FR the pin derives from stated three obligations — the
`deprecated` adoption block, the `compositionPending` adoption block, and usage binding — while
pinning **none** of their operands, and no lint could see it.

**Implements**: `cpt-cf-bss-products-algo-coverage`

**Touches**:
- Entities: `CoverageChecks`, `SchemaPin`, `ObligationRegister`
- Files: `schema-pin.toml`

### The gate that runs the nine, which does not exist and is not a restoration

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-lint-gate`

The nine lints run in CI.

**Nothing runs them today, and there is nothing to restore.** `.github/workflows/docs.yml` holds
exactly one job, `Check Markdown Links`. The `Spec Invariants` job that once ran `make spec-check`
was **removed deliberately** by commit `21a149fda`, together with `tools/spec-check`, the workspace
member and the Makefile target — and that commit priced the loss in as many words: *"the design
documents go back to being validated by nothing automatically… That property is knowingly given up;
the mitigation is that a forgotten or permanently-red gate provides no real protection either."*

So enforcing the nine is **building** something, not restoring it, and it is repo-tooling work rather
than anything this design set can decide. **What this feature owes is the honest statement that its
checks are declared and unenforced**, which `design/12` §3.2 makes and which this DoD does not
paper over. It is met when a job exists; until then it is the one DoD in the set that names an owner
outside the gear.

**Implements**: `cpt-cf-bss-products-algo-coverage`

**Touches**:
- Files: a new job under `.github/workflows/` — which file is the §7 row 2 owner's to name
- Entities: `CoverageChecks`

### The four design-introduced names exist as named seams

- [ ] `p1` - **ID**: `cpt-cf-bss-products-dod-contract-seams`

`design/12` §1.7 introduces exactly four names, and each is addressable rather than prose:

- **`SeamSuite`** — the joint fixture set plus the schema-pin checks, one CI job, failing closed on
  divergence.
- **`SchemaPin`** — the versioned, committed serialization of the C1 joint fields both gears' CI
  compares against.
- **`ObligationRegister`** — §2.2's roster: every consumer-side duty, its owing gear, its operand and
  its fixture status.
- **`CoverageChecks`** — the nine lints of §3.2.

**None is a gear aggregate**, and DECOMPOSITION §2.12 records why: *"None — the schema pin is a
committed CI artifact, not a gear table."* Two of the four — `SchemaPin` and `ObligationRegister` —
are files or tables outside the gear's runtime, and two are jobs. So this DoD obliges named
artifacts, not types.

**Implements**: `cpt-cf-bss-products-flow-seam-suite`, `cpt-cf-bss-products-algo-coverage`

**Touches**:
- Entities: `SeamSuite`, `SchemaPin`, `ObligationRegister`, `CoverageChecks`

## 6. Acceptance Criteria

**The suite's own meta-probes, from `design/12` §5**

- [ ] **The pin divergence RED**: mutate a pinned field on one side only — **both** CIs must fail.
      The asymmetry is the whole enforcement, so a probe that fails one side proves nothing.
- [ ] **The `vN`→`vN+1` RED**: remove a default from a new optional field — the compatibility test
      must fail. Asserted in C2's direction, an old consumer reading a new payload.
- [ ] **The bootstrap RED**: age a consumer checkpoint past the retained tail — the failure is loud
      and names re-bootstrap as the remedy.
- [ ] **One OWED row flipped to asserted end-to-end** — the watermark fixture first, it being the
      P-D-03 joint build's acceptance.

**Replay and dedup**

- [ ] A duplicate delivery inside the idempotency window is deduplicated on the event **`id`**.
- [ ] An out-of-order delivery beyond the window is detected on `(tenant, aggregate, sequence)`.
- [ ] **A `sequence` gap left by a neighbouring aggregate in the same partition does NOT trigger
      re-bootstrap.** The positive control for the criterion above: without it, a build that treats
      any gap as loss passes the detection test and re-bootstraps on healthy traffic.
- [ ] A refused request retried on the same key **runs** and gets a fresh verdict; a successful one
      replays.
- [ ] `internalRevision` in an event equals the ETag the same act returned, **with no adjustment**.

**Bootstrap, both arms**

- [ ] A tenant with published versions bootstraps from the latest `CatalogVersion` under `browse`
      intent plus the tail from that instant.
- [ ] A tenant with **zero** published versions bootstraps from the empty catalog plus the whole
      retained tail.
- [ ] The gear's own projector passes the same two cases through the same contract.

**The lints, one positive control each**

- [ ] Each of the **nine** lints has a failing case **and** a passing case. A lint asserted only by
      its failure may never pass; one asserted only by its pass may never fire. Nine pairs.
- [ ] **Lint 4's passing case requires a non-empty `EventRegister`.** Every row of an empty table
      satisfies the rule, so a pass over the register as it stands certifies a lint that asserts
      nothing — the debt-into-green-check `cpt-cf-bss-products-dod-joint-fixtures` forbids three
      sections earlier. Blocked on §7 row 1.
- [ ] **Lint 3 pairs all seven pipe-bearing routes across the escaped and unescaped forms.** The
      probe mutates one route's spelling in §3.2's table only and asserts the lint still pairs it —
      the case a literal matcher fails on a correct document.
- [ ] **Lint 3's population is asserted as a count, and the count is seventeen.** A probe that asserts
      only "no unpaired route" is green on an empty population.
- [ ] Lint 1 fails on an unqualified claim beside a qualified one, and on two identical qualifiers.
- [ ] Lint 1 admits the fourteen multiply-claimed requirements, **including the one claimed by three
      slices**, and fails if any of their qualifiers collide.
- [ ] Lint 2 fails on a fourth, unexplained exclusion from the AC #38 map.
- [ ] Lint 6 admits a `(cont. inst-…)` continuation row and fails on a second bare declaration.
- [ ] Lint 7 fails on a second table declaring a `*_actor_ref` column.
- [ ] Lint 8 fails on a §4 column named for any of the six monetization words, and passes on `usage`.
- [ ] **Every `Operand` cell of `design/12` §2.2 parses under the token grammar** — one token per pin
      member, comma-separated, each a catalog field name or one of `(surface)`, `none in v1`,
      `payload`. Fourteen cells. This is `cpt-cf-bss-products-dod-obligation-register`'s only
      deliverable and without a criterion it ticks with the cells unrepaired, after which lint 9
      fails on a correct `SchemaPin`.
- [ ] Lint 9 fails on a pinned field that no register row names **and that carries no recorded
      exclusion reason** — the rule's own alternative, without which the probe rejects a legitimately
      excluded member — and on a register row whose field token is absent from the pin; a
      `(surface)`, `none in v1` or `payload` marker passes.

**The SDK surface**

- [ ] The read shape's `status` deserializes all five states and the browse door serves only
      `published|deprecated`.
- [ ] Every SDK method returns `CanonicalError`; no gear-local error type crosses the port.
- [ ] Adding a ninth event fails until **all six sites** are updated — the `*_PAYLOAD_TYPE` const,
      the `SCHEMA_REFS` row, the `catalog_event!` invocation, the typed-event `match` arm, and both
      `THE_EIGHT` literals. A probe that stops at the two rosters is green on the build
      `events.rs` says reaches `NoTypedEvent` at runtime.

## 7. Known unknowns

**The arithmetic of this section.** Thirty-seven rows: **twenty-one carried verbatim** from
[`../design/12-consumer-contracts.md`](../design/12-consumer-contracts.md) §6 — the slice's full
count, not a selection — and **sixteen raised here**: five while authoring and eleven by the
three-lens review of this document. Of the thirty-seven, **eleven block no DoD in this document**
(rows 3, 6, 12, 27 and 36, plus rows 25, 26, 15, 33, 24 and 34, which **P-D-57, P-D-58, P-D-63,
P-D-65 and P-D-66 resolved on 2026-08-31** — kept in place rather than struck); the other twenty-six
each name the DoD they block. A final subsection
carries defects owed to other documents, recorded and not repaired here; those are not rows.

**Carried, not answered**, and registered against **its owner's** register. **Three departures from
verbatim, declared so the claim is checkable.** First, the slice's inline `Owner:` sentence and its
provenance marker are converted into this document's `**Owner**:` field. Second, every bare `§N`
inside a carried row is **`design/12`'s numbering, not this document's** — **except `§15`, which is
`PRD.md`'s**: `design/12` has six sections and no §15, and rows 2 and 4 both carry a bare `§15` for
the suite's home and the retention window. Third, every carried row gains a `**Blocks**:` field,
which `design/12` §6 does not have. Apart from those three, nothing is altered; the carried text was
diffed against `design/12` §6 sentence by sentence, mechanically, and every row matched.

### Carried verbatim from `design/12` §6

1. **The `EventRegister` is declared and empty.** P-D-45 made lint 4 read a table that does not
    yet have rows, and the measurement that forced it (five harvests, five populations) is also
    the reason nobody can fill it in one pass: an event's emitting instruction is only known to
    whoever wrote the rule. Each slice owes its own rows — event, emitting `inst-*`, and an
    explicit no-event where a state change emits nothing. Until it is written lint 4 is declared
    and inert, which is a better state than prose but is not a working gate.
    **Blocks**: `cpt-cf-bss-products-dod-lint-declarations`.
    **Owner**: every slice owner, coordinated by this one. *(Raised by the P-D-45 round.)*

2. **The suite's final owner/home is a §15 open** (proposed `api-contracts` CI) — the design is
    home-agnostic, but an unowned CI job is an unrun one; this is the set's last organizational
    dependency.
    **Blocks**: `cpt-cf-bss-products-dod-seam-suite-home`.
    **Owner**: not stated in the slice; carried unassigned.

3. **Most obligations are OWED** by construction (C4): the register makes the debt legible, and
    the P-D-03 watermark fixture is deliberately first — it unblocks retirement, the highest-value
    seam.
    **Blocks**: no DoD — it states the design's own posture under C4, not a blocker.
    **Owner**: not stated in the slice; carried unassigned.

4. **Event-log retention ≥ bootstrap gap** needs its number (§15) before the replay contract is
    more than words; named as the replay contract's single config dependency.
    **Blocks**: `cpt-cf-bss-products-dod-bootstrap`.
    **Owner**: not stated in the slice; carried unassigned.

5. **`inst-cc-events` lints per instruction row, against P-D-34's act unit.** **P-D-34** makes the
    event-declaration unit the *act*: a step inside a transaction whose event another row of that
    transaction names inherits the declaration. This row still lints per instruction *row*, so
    01's `inst-fd-publish-freeze`, `inst-fd-publish-correction` and `inst-fd-publish-bump` — which
    inherit `inst-fd-publish-emit`'s declaration — are red by construction on a correct document.
    **Blocks**: `cpt-cf-bss-products-dod-lint-declarations`.
    **Owner**: this slice. *(Filed from 01 §6 by the slice-01 eighth lens pass — the pointer
    claimed it was registered here and it was not.)*

6. **Does this slice owe an open-item reciprocity lint?** Design 01 §6 used to restate its
    outbound questions as bullets claiming each was "registered where its owner will look". That
    claim was measured twice and was false both times — the eighth pass found four of six named
    documents unfiled and filed the headline item of each; the P-D-43…49 propagation audit found
    five sub-items its repair had missed. 01 no longer restates them, which removes the drift but
    not the gap: the lint set here checks ids, codes, events and doors, never open-item
    reciprocity, so nothing catches a question filed nowhere.
    **Blocks**: no DoD — it proposes a tenth lint rather than blocking one of the nine.
    **Owner**: the design-set owner with this slice. *(Filed from design 01 §6 by the slice-01
    eighth lens pass; re-measured by the P-D-43…49 propagation audit.)*

7. **The `CoverageChecks` are gated by nothing, and there is nothing to restore.** This item
    previously read "restoring the job is owed", which was false in its premise and is corrected
    here. The `Spec Invariants` job was not lost: commit `21a149fda` removed it deliberately along
    with `tools/spec-check`, the workspace member and `make spec-check`, and stated the cost in as
    many words — "the design documents go back to being validated by nothing automatically… That
    property is knowingly given up; the mitigation is that a forgotten or permanently-red gate
    provides no real protection either." The in-repo tool no longer exists, so enforcing these
    nine lints is **building** something, not restoring it, and that is repo tooling work rather
    than anything this design set can decide. What this slice owes is only the honest statement
    that its checks are declared and unenforced — which §3.2 makes.
    **Blocks**: `cpt-cf-bss-products-dod-lint-gate`.
    **Owner**: whoever owns repo tooling, if and when the cost recorded in `21a149fda` is
    reconsidered. *(Premise corrected after the P-D-45 round.)*

8. **Does the pin run as one CI job or once per gear?** §2.1 says "one CI job over
    `cf-gears-bss-fixtures`"; §5's probe says "both CIs must fail"; and one job cannot be the
    other side's CI, with both gears in one repository. Separately,
    `.github/workflows/api_contracts.yml` already exists under the proposed name with an unrelated
    purpose and triggers that never include a fixture crate.
    **Blocks**: `cpt-cf-bss-products-dod-seam-suite-home`.
    **Owner**: the `PRD` §15 owner.

9. **Two authorability criteria are in force.** C4 makes an assertion authorable "once the
    referenced counterpart AC exists"; two register rows are marked owed on a different test — the
    counterpart raises no code — and their Source cells cite pricing *instruction* ids, whose
    counterparts do exist. The two disagree on at least two live rows.
    **Blocks**: `cpt-cf-bss-products-dod-joint-fixtures`.
    **Owner**: this slice with the plan-price owner.

10. **What does "unqualified" mean in the AC-existence check?** Under an adjacency reading, four
    slice-04 sites and one here are violations and a sweep is owed; under a sentence-context
    reading they are correct and the "one-line regex" the row names cannot implement the rule. The
    qualifier grammar governs Traces-to claims, not AC citations.
    **Blocks**: `cpt-cf-bss-products-dod-lint-prd-universe`.
    **Owner**: this slice.

11. **The approval-queue envelope is asserted here and owed in 05.** `inst-sdk-inbox` says a
    field-name drift "fails the suite", but the check is in neither the fixture roster nor the
    register, and 05 records the cross-check as future work — while C4 forbids exactly that ("an
    unauthorable assertion stays listed as OWED, never silently dropped").
    **Blocks**: `cpt-cf-bss-products-dod-joint-fixtures`.
    **Owner**: this slice with 05.

12. **Should `P-D-01`, `P-D-03` and `P-D-05` name this slice?** This slice restates all three in
    its constraint rows and register, and their propagation fields do not name it — and so do
    eight more (`P-D-24`, `P-D-25`, `P-D-26`, `P-D-27`, `P-D-29`, `P-D-34`, `P-D-35`, `P-D-43`):
    eleven of the twenty-three decisions this slice cites are absent from their own `Propagated`
    lists. The gap is set-wide rather than this slice's — **62 of the 179 (slice, decision)
    citation pairs** in the set sit outside the cited decision's list. Whether a constraint-row
    citation counts as a restatement for lint 5 is unstated; the fix lands in the register, not
    here.
    **Blocks**: no DoD — the fix lands in `DECISIONS.md`, not in any obligation here.
    **Owner**: the register's owner.

13. **Is `inst-cc-errors`' exclusion list one filter or two?** Two of the three exclusions are
    already excluded by the opening clause ("that a registry door can refuse"); the third is
    excluded for a reason that clause does not express. The "exactly three" assertion is checkable
    only once one filter defines the universe.
    **Blocks**: `cpt-cf-bss-products-dod-lint-prd-universe`.
    **Owner**: the error-contract owner.

14. **Lint 3's population is not in one machine-readable form.** **P-D-45** arm 1 defines it as ``
    `METHOD /bss-products/v1/…` `` code spans, "one machine-readable form". At HEAD all seven
    pipe-bearing routes exist in **two** textual forms: `{products\|skus}` inside 05 §3.2's table,
    where a markdown cell must escape the pipe, and `{products|skus}` everywhere else — 01, 02,
    08, 11, and 05's own §6. A lint matching the spans literally pairs none of the seven across
    that boundary: it would read all fourteen `Doors` entries as undeclared and all seven outside
    declarations as un-doored. The table cannot drop the escape without breaking the cell, so the
    normalization belongs in the lint's grammar, and no document states it.
    **Blocks**: `cpt-cf-bss-products-dod-lint-declarations`.
    **Owner**: this slice with 05. *(Raised by the P-D-43…49 propagation audit.)*

15. ~~**Lint 9's `Operand` grammar does not describe the cells it reads.**~~
    **Answered in the slice (owner call, 2026-08-31 — P-D-63, amending P-D-43 arm 3): one production
    added, one separator refused, and the two unsettled readings land with it.** A non-field marker
    consumes exactly one preceding backticked identifier as its annotation; `+` is refused and the
    four cells that used it (**four at HEAD on a register of fourteen** — this row's thirteen and
    three were both stale, as this document's own census already recorded) are normalized to
    comma-separated pin tokens; a backticked field name is a token, never the ignorable prose; and a
    `none in v1` cell is outside the coupling population by the marker rule. **All fourteen cells
    parse.** Original text: **P-D-43** arm 3 fixes
    the cell as "one token per pin member, comma-separated, each either a catalog field name or
    one of three non-field markers". At HEAD three of the thirteen §2.2 cells fit that grammar
    (`compositionPending`, `sellable`, `skuId`). Six lead with a backticked non-field token — five
    `` `CatalogVersion` (surface) `` and one `` `SkuRetired` payload `` — formally
    indistinguishable from a catalog field name, so a token-reading lint looks for
    `CatalogVersion` in the `SchemaPin` and fails; three more join their operands with `+` rather
    than a comma. Arm 3's "prose beside the tokens is ignored" may be meant to cover the leading
    token, but a backticked identifier is not prose under any form the grammar states.
    **Blocks**: no DoD — **resolved by P-D-63**; `cpt-cf-bss-products-dod-obligation-register` is
    freed, and `cpt-cf-bss-products-dod-lint-pin-coupling` carries the amendment while staying blocked by row 33.
    **Owner**: was this slice; **closed**. *(Raised by the P-D-43…49 propagation audit.)*

16. **Seven register entries carry two `Propagated` fields, and lint 5 says there is one.** Lint
    5's grammar (**P-D-43** arm 4) reads "the register carries **one** propagation field, spelled
    `- **Propagated**`". P-D-24 through P-D-30 each carry a base field plus a second dated *(owed
    until …, all closed)* field — 56 fields across 49 entries. Either the grammar admits the
    dated-amendment form or those seven merge; until it is settled, a reader taking the **last**
    field — the rule P-D-43's own entry forces, since its arm 4 quotes the literal field name in
    its body — silently drops the primary field for all seven.
    **Blocks**: `cpt-cf-bss-products-dod-lint-declarations`.
    **Owner**: the register's owner. *(Raised by the P-D-43…49 propagation audit.)*

17. **No lint verifies that a free-text `reason` door registers the PII block.** 02
    `inst-av-pii-reason` enumerates the doors that owe `inst-av-pii-block`, and says the
    enumeration *is* the registration — a slice that adds such a field "adds itself to the
    enumeration above; that is the whole registration". Nothing checks it. The nine lints here
    cover ids, codes, events, doors, operands and register hygiene — none covers PII-hook
    coverage, so a slice that adds a reason field and forgets the hook is caught by reading or not
    at all, and 02's own stated consequence is that personal data typed into such a field is
    unreachable by erasure forever. The class is not hypothetical: **P-D-50** had to wire five
    doors across 05 and 07 that had carried the debt as an open item instead.
    **Blocks**: `cpt-cf-bss-products-dod-lint-surfaces`.
    **Owner**: this slice with 02. *(Raised by CodeRabbit on PR #14, 2026-08-27; its first half —
    the unwired doors — was closed by P-D-50, this half was not.)*

18. **Does `inst-cc-errors` still lint against the phase unit?** **P-D-36** moved the declaring
    unit from the phase to the declaring slice, which retires the carve-out mirror this row was
    owed rather than paying it. This slice cites P-D-36 nowhere.
    **Blocks**: `cpt-cf-bss-products-dod-lint-prd-universe`.
    **Owner**: this slice. *(Filed from 01 §6 by the P-D-43…49 propagation audit — the pointer
    claimed it was registered here and it was not.)*

19. **`ENTITY_TERMINAL`'s gloss widened and the AC #38 map was not re-read.** **P-D-32** widened
    it from a save on a `retired`/`discarded` head (**P-D-25**) to any head write — save, publish
    or correction. The map's rows were written against the narrower reading.
    **Blocks**: `cpt-cf-bss-products-dod-lint-prd-universe`.
    **Owner**: this slice. *(Filed from 01 §6 by the P-D-43…49 propagation audit — the pointer
    claimed it was registered here and it was not.)*

20. **Is `inst-cc-ids`' continuation enumeration stale?** Lint 6 names the ids 01 legitimately
    carries on more than one row and how many rows each takes. That enumeration is a count against
    another document, and nothing re-reads it when 01 changes.
    **Blocks**: `cpt-cf-bss-products-dod-lint-declarations`.
    **Owner**: this slice. *(Filed from 01 §6 by the P-D-43…49 propagation audit — the pointer
    claimed it was registered here and it was not.)*

21. **May 01 §4.2's `composition_pending` no-re-raise clause rest on P-D-14?** **P-D-48**
    confirmed the clause, but P-D-14's propagation field names 05, 06, `design/README.md`,
    `DESIGN.md` and the PRD — not `design/01-foundation.md`. Under lint 5's own grammar a document
    restates a decision exactly when it cites the id, so either the field gains 01 or the clause
    rests on something else.
    **Blocks**: `cpt-cf-bss-products-dod-lint-declarations`.
    **Owner**: the register's owner. *(Filed from 01 §6 by the P-D-43…49 propagation audit.)*

### Raised here rather than carried

22. **Lint 3's stated population is wrong, and it is the number a lint would be built to.**
    `inst-cc-rbac` and carried row 14 both say **fourteen** `` `METHOD /bss-products/v1/…` ``
    spans. Measured at `0771f15ae` with the escaping normalised, the design set declares **17**
    distinct routes and `design/05` §3.2's `Doors` column names the same 17. **It was wrong at
    birth, not drifted**: at `5977aec64`, the commit that wrote it, the population was already 17,
    and a diff of that commit's routes against HEAD's is empty. Carried row 14's *"all fourteen
    `Doors` entries"* is the same error. A lint built to fourteen is wrong by three on a set that
    satisfies it.
    **Blocks**: `cpt-cf-bss-products-dod-lint-declarations`.
    **Owner**: this feature with `design/05`'s owner, and the register's owner for **P-D-45**'s own
    text.

23. **One of the fourteen multiply-claimed requirements is claimed by three slices, not two.**
    `design/12` §3.2 lint 1 says *"fourteen requirements are owned by **two** slices each"* and
    *"all fourteen pairs now carry"* a qualifier. Measured over every Traces-to line: **14 ids are
    multiply claimed, 13 of them pairs**, and `cpt-cf-bss-products-nfr-scale-extensibility` is
    claimed by **`01`, `02` and `06`**. The count is right; *pairs* is wrong for one member, and it
    matters because the qualifier rule needs **three** distinct qualifiers there — a reviewer
    checking "all fourteen pairs" checks two.
    **Blocks**: `cpt-cf-bss-products-dod-lint-prd-universe`.
    **Owner**: this feature, with `01`, `02` and `06` for the third qualifier.

24. ~~**Does the `status` pin bind a field the SDK does not carry?**~~
    **Answered (owner call, 2026-08-31 — P-D-66): the pinned token is `status`, and the entry carries
    the registry-side spelling as an annotation.** The seam's name is fixed by
    `CatalogSku`-superset-compatibility — pricing's shipped `CatalogSku.status` — and pinning
    `lifecycle_state` would make the comparison against that shipped member impossible; the
    `registry-field = "lifecycle_state"` annotation is what the job resolves this side by. The cost is
    recorded in the register: a seam `status` member is a second spelling of `lifecycle_state` inside
    this gear's SDK, accepted because the seam contract is shipped on the consumer's side.
    Original text: `cpt-cf-bss-products-dod-status-vocabulary` pins `status` and its vocabulary, and
    the shipped
    `Sku` carries `lifecycle_state` — the same value under a different name — while
    `inst-sdk-catalogsku` names the member `status`. The pin is a comparison between two gears' field
    names, so whether the pinned token is `status` or `lifecycle_state` decides whether the
    comparison can be made at all, and no document states which spelling the pin file uses.
    **Blocks**: no DoD — **resolved by P-D-66** (with row 34).
    **Owner**: was this feature with the plan-price owner; **closed** — nothing pricing-side
    changes.

25. ~~**Seven of the ten pinned read-shape members have no shipped column, so which side of the pin
    moves first?**~~
    **Answered (owner call, 2026-08-31 — P-D-57), and half of it was already decided here.** Which
    side moves is `cpt-cf-bss-products-dod-catalogsku-shape`'s call: the SDK's read shape grows
    additively and the pin never shrinks to shipped-only. What was open is the CI colour, and the
    answer is that **the pin carries a per-member comparability flag and the job is two-sided** —
    comparing the comparable members, asserting the absence of the rest, so a member that ships while
    still flagged fails the job in the change that shipped it. The flag is authored conservatively:
    `comparable` only once both the column and the SDK member ship.
    **The row's own count conflated three sets.** The read shape is ten members; the **pin** is eight
    fields with `skuCode` and `name` deliberately out; the SDK's `Sku` ships **seven** members, of
    which two are pin members (`skuId`, and `status` as `lifecycle_state`). So **six of the pin's
    eight** have no shipped operand — and `name`, one of the row's seven, is not a pin member at all.
    Original text: `metering_unit`, `usage_type_ref`, `plan_tier`, `sellable` and `type` are on
    neither the shipped `Sku` nor any shipped table — five of the ten. Two more, `name` and
    `composition_pending`, have columns but are absent from the SDK type, and `status` ships as
    `lifecycle_state`. The pin's
    membership is *derived* from the obligation register (**P-D-12**), so it can be authored today —
    but a CI job comparing it against the SDK would fail on seven absent members for the whole
    period `02`, `03` and `06` are landing them. Whether the job admits a member as `owed` or the
    pin lists only shipped members is unstated, and the two give opposite CI colours for months.
    **Blocks**: no DoD — **resolved by P-D-57**; `cpt-cf-bss-products-dod-catalogsku-shape` is
    freed, while `dod-schema-pin` stays blocked by rows 24, 33 and 34 and `dod-seam-suite-home` by
    rows 2, 8 and 31.
    **Owner**: was this feature with the plan-price owner; **closed** — nothing on the consumer's side
    is changed by it.

26. ~~**Is the replay contract testable at all before `dyn EventBrokerApi` has a production
    registration?**~~
    **Answered (owner call, 2026-08-31 — P-D-58): the fixtures are authorable now and do not wait for
    it.** Their transport is `event-broker-sdk`'s own `MockBroker` — public SDK API under the
    `test-util` feature this gear already takes in dev-dependencies, implementing the trait and
    exporting `StoredEvent` and `CursorEntry` so the log and cursors are readable from a fixture.
    **And it is not a registration bypass**, which is what decided it: `infra/broker_tests.rs`
    registers the topic and all eight event types, then `hub.register::<dyn EventBrokerApi>(broker)`
    into `ClientHub` — the registration a production boot performs, with a different transport behind
    it. The boundary is the gear's own: *"Anything a real broker adds is on the other side of that
    boundary and belongs to whoever owns the `01/06` split."* **The claim a green suite licenses is written down and is narrower than the
    obligation** — the contract holds over this gear's path with a conforming transport; that events
    reach consumers in production is a different claim with no DoD owning it. `01-foundation`'s
    standing debt is untouched.
    Original text: Nothing in the workspace registers it except this gear's own `broker_tests.rs`,
    so the broker producer arm is inert in every real deployment. Every obligation of
    `cpt-cf-bss-products-flow-replay` — versioning, dedup, ordering, bootstrap — rides that arm, and
    the suite's fixtures would assert a contract over a producer that never runs outside tests. The
    standing debt is `01-foundation`'s; what is unstated is whether this feature's fixtures are
    authorable against the test registration or wait for the real one, which is C4's question asked
    of the gear's own transport rather than of a counterparty.
    **Blocks**: no DoD — **resolved by P-D-58**; `cpt-cf-bss-products-dod-dedup-ordering` is freed,
    while `dod-event-versioning` stays blocked by row 29 and `dod-bootstrap` by row 4.
    **Owner**: was `01-foundation`'s broker owner with this feature; **closed** — the fixture question
    only. The production registration remains `01`'s and is not settled here.

27. **Does `DECOMPOSITION.md` §2.12 or this document govern the feature's scope?** §2.12's **Scope**
    lists three items — event schema versioning, the replay/bootstrap contract, the SDK/§9 surfaces —
    and its **Out of scope** puts *"the seam-suite specification, the consumer-obligation register,
    the completeness checks and §17.2 traceability"* outside the feature, its Purpose calling that
    track *"CI and review work rather than gear behavior and … therefore not decomposed into this
    feature"*. **Thirteen of this document's seventeen DoDs deliver the excluded track**, and
    `design/12` §1.5 puts all of it **In**. The entry contradicts itself too: its **Domain Model
    Entities** field lists all four of the track's names. Either §2.12's Purpose and Out-of-scope
    blocks move, or thirteen DoDs and one `algo-` id leave this document.
    **Blocks**: no DoD in the sense of a missing operand — but it is the one open item that could
    remove thirteen of them.
    **Owner**: the DECOMPOSITION owner with the design-set owner. *(Raised independently by all three
    lenses.)*

28. **Does lint 7 match `*_actor_ref` or `actor_ref`, and is "exactly one" the right assertion?**
    Measured at `0771f15ae`: **no** column in the design set ends in `_actor_ref`, so the stated
    pattern has zero matches; under the bare `actor_ref` spelling **four** tables declare one —
    `products_identity_ref`, `products_approval`, `products_approval_decision`,
    `products_breakglass_session`. So the lint is red at 0 under the stated pattern and at 4 under
    the bare one. Either the column convention is renamed across four tables, or the assertion
    changes from *"exactly one table declares such a column"* to one over the columns that **hold**
    an identity rather than every column carrying a pseudonymous ref.
    **Blocks**: `cpt-cf-bss-products-dod-lint-surfaces`.
    **Owner**: the design-set owner with `10-retention-erasure`'s.

29. **Where do the deserializable event payload types live?** C2's direction is an old consumer
    **deserializing** a new payload, and nothing can deserialize one today: `EventBodyCore` derives
    `Serialize` only and is `pub(crate)`, `Deserialize` occurs zero times in `infra/events.rs`, and
    the destination crate refuses the derive — `products-sdk/src/lib.rs` reads *"No serde derives
    live here"* and its `Cargo.toml` declares no serde. Three answers are available and none is
    stated: `products-sdk` gains serde against its own module doc, a third crate holds the wire
    types, or the compatibility test runs against the gear's REST DTOs. The `#[serde(default)]`
    obligation for new optional fields lands wherever the answer points.
    **Blocks**: `cpt-cf-bss-products-dod-event-versioning`.
    **Owner**: the SDK owner with `01-foundation`'s event owner.

30. **Where is the `EventRegister` declared?** Lint 4 reads it, §7 row 1 assigns its **rows** to
    every slice owner, and the artifact itself has no declaration site: it is not one of `design/12`
    §1.7's four design-introduced names and not one of §4's four named artifacts. So the container
    the rows go into is specified nowhere.
    **Blocks**: `cpt-cf-bss-products-dod-lint-declarations`.
    **Owner**: the design-set owner.

31. **Can the five joint fixtures be expressed in `cf-gears-bss-fixtures` at all?** The crate admits
    a **closed** grammar: `Family` is an enum with `ALL: [Self; 9]`, all nine of them pricing charge
    families; a directory whose name is not a known family is a **load error**
    (`UnknownFamilyDirectory`); `CaseKind` is `Evaluation | Publish`; and `corpus/registry.toml`
    carries *"# GENERATED by `cargo run -p cf-gears-bss-pricing --example regen_registry`"* with CI
    asserting the regeneration is diff-clean. **None of the five fixtures is an `Evaluation`
    yielding a charge or a `Publish` verdict, and none belongs to a declared `Family`.** Three
    routes exist — extend `Family`/`CaseKind` and re-key a pricing-generated registry, add a second
    corpus root under the same crate, or place the suite in a products-side crate and retire
    **P-D-44**'s artifact row — and `cpt-cf-bss-products-dod-seam-suite-home`'s three-step task is
    unbuildable under all three until one is chosen.
    **Blocks**: `cpt-cf-bss-products-dod-seam-suite-home`,
    `cpt-cf-bss-products-dod-joint-fixtures`.
    **Owner**: the fixtures-crate owner with the `PRD.md` §15 owner.

32. **Lint 3 needs two more normalisations than any document states, and one of them decides whether
    it passes.** The escaping is the first and §5 carries it. The second two are unstated:
    (a) **the verb roster** — a harvest of `` `[A-Z]+ /bss-products/v1/…` `` also matches the
    grammar's own schematic literal `` `METHOD /bss-products/v1/…` ``, which occurs three times
    (`design/05` once, `design/12` twice), giving 18 rather than 17; and (b) **the corpus** —
    `design/12` scopes the lints to *"this design set + PRD"*, and `DECOMPOSITION.md` declares
    `` `GET /bss-products/v1/browse` `` while §3.2's `Doors` column carries
    `` `GET /bss-products/v1/browse…` ``. **If `DECOMPOSITION.md` is in scope the lint fails today**,
    on a set the design-set-only harvest passes.
    **Blocks**: `cpt-cf-bss-products-dod-lint-declarations`.
    **Owner**: this feature with `design/05`'s owner.

33. ~~**Is `CatalogVersion` a pin entry or outside the pin?**~~
    **Answered (owner call, 2026-08-31 — P-D-65): both sentences become literally true.** The pin
    carries a `CatalogVersion` entry of kind **`surface`** — `kind = "surface"`, a `delegated-to`
    naming the port trait, no comparability flag — and the job neither compares it nor asserts its
    absence, the drift protection being the adapter the pricing-side port doc promises, which the
    compiler checks. Lint 9's formulation narrows from *"outside the pin"* to *"outside the
    **field-comparison** population"*, and the five annotated markers couple to the surface entry in
    both directions. P-D-12's sentence stands unreinterpreted, which was the tiebreaker: re-reading a
    confirmed decision that five register rows and C1 cite is a retraction with a radius.
    Original text: **P-D-12** says it is *"pinned as a
    **surface**, not a field"*, while lint 9's grammar makes a `(surface)` marker *"outside the pin by
    construction"*. Five of the fourteen register rows carry `` `CatalogVersion` (surface) ``. Under
    the first reading the pin file has a `CatalogVersion` entry and lint 9's second arm needs a
    register row naming it as a field; under the second there is no entry and the five rows pass by
    exclusion. The pin file's schema for a surface-level member has to be settled before
    `cpt-cf-bss-products-dod-schema-pin` can be written.
    **Blocks**: no DoD — **resolved by P-D-65**; `cpt-cf-bss-products-dod-lint-pin-coupling` is
    freed, and `cpt-cf-bss-products-dod-schema-pin` carries the schema while staying blocked by rows
    24 and 34.
    **Owner**: was this feature with the plan-price owner; **closed** — nothing pricing-side
    changes.

34. ~~**Which `status` value vocabulary does the pin carry — two members or three?**~~
    **Answered (owner call, 2026-08-31 — P-D-66): two — `published` and `deprecated` — and the answer
    was already normative in this document.** `inst-sdk-catalogsku` M4 states the wire subset:
    *"browse serves `published|deprecated` only (draft never served, retired history-only — 08 C2)"*.
    Pricing's `draft` is the display tolerance its own doc declares — the field stays a string so *"a
    fifth state must not become a parse failure in the gear that merely displays it"* — and the pin
    **replaces** that tolerance rather than adopting its list; `draft` is not a pinned member because
    the wire never serves it. Original text: This document
    pins *"`status` with its value vocabulary"* and states that browse serves `published|deprecated`
    only. The counterpart documents **three**: pricing's own catalog client reads *"`draft` |
    `published` | `deprecated`, verbatim. Not an enum:"*. The pin is a comparison between two gears'
    declarations, so a two-member and a three-member vocabulary give opposite CI colours. Row 24
    asks only about the field **name**; this asks about the value set, and whether `draft` is a
    pinned member.
    **Blocks**: no DoD — **resolved by P-D-66** (with row 24);
    `cpt-cf-bss-products-dod-status-vocabulary` and `cpt-cf-bss-products-dod-schema-pin` are both
    freed, row 33 having been resolved by P-D-65 the same day.
    **Owner**: was this feature with the plan-price owner; **closed**.

35. **Where does the studio-inbox envelope cross-check land?** `DECOMPOSITION.md` §2.12 puts it
    **In** scope — *"The SDK and §9 surfaces, including the studio-inbox envelope cross-check"* — as
    does `design/12` §1.5, and `inst-sdk-inbox` says a field-name drift *"fails the suite"*. But it
    is in none of `cpt-cf-bss-products-dod-joint-fixtures`' five fixtures, in no register row, and
    under no acceptance criterion; carried row 11 routes it to that DoD, which does not carry it.
    **It is the one instruction id of `design/12`'s nineteen that no DoD in §5 covers.** It becomes
    a sixth fixture, a fifteenth register row, or a DoD of its own.
    **Blocks**: `cpt-cf-bss-products-dod-joint-fixtures`.
    **Owner**: this feature with `05-governance`'s.

36. **May a DoD `Implements:` a `contract-` id that lives in the slice?** Three DoDs here do —
    `dod-sdk-surface`, `dod-catalogsku-shape` and `dod-status-vocabulary` all point at
    `cpt-cf-bss-products-contract-sdk` — and they are the only `Implements` targets across the ten
    written FEATUREs that are not a `flow`/`algo`/`state` id declared in the same document. The
    alternative, minting a `flow` or `algo` here for the SDK surface, collides with the mint census
    behind this feature's ninth `algo-`: all prior mints sit in `design/12` §3 sections and none in
    §2. No document states the permitted target kinds, so three DoDs' traceability from this
    document is unsettled.
    **Blocks**: no DoD — the three build the same thing under either answer.
    **Owner**: the design-set owner.

37. **Is lint 2's "fifteen rows" a parsed operand or a transcribed constant?** `PRD.md` AC #38 is a
    single prose sentence whose enumerated items contain commas and a parenthetical aside, and
    `design/12` §4.1 is the machine-readable side. Only the transcribed reading is implementable —
    and under it the criterion *"lint 2 fails on a fourth, unexplained exclusion"* checks §4.1
    against itself rather than against the PRD.
    **Blocks**: `cpt-cf-bss-products-dod-lint-prd-universe`.
    **Owner**: the error-contract owner.

### Owed to other documents, recorded and deliberately not edited

- **`DECOMPOSITION.md` §2.12 puts this feature's verification track out of scope.** Its Out-of-scope
  block reads *"The seam-suite specification, the consumer-obligation register, the completeness
  checks and §17.2 traceability — the verification track this document does not decompose"*, while
  its **Domain Model Entities** field lists all four of that track's names and §5 here carries
  thirteen DoDs delivering it. Recorded as row 27 above; the entry is the DECOMPOSITION owner's.
- **`design/12` §6 row 18 says *"This slice cites P-D-36 nowhere"* and the slice cites it in the
  same section's lint 2.** `design/12` §3.2 lint 2 reads *"a slice, not a door and not a pipeline
  phase"* and attributes it to **P-D-36** by name in the same clause, and
  P-D-36's own `Propagated` field names the slice. The row refutes itself two lines above its own
  claim. Carried verbatim and **not repaired**; owner `design/12`'s.
- **`design/12` §6 row 8 says `api_contracts.yml` has *"triggers that never include a fixture
  crate"*.** Its `paths` filter carries `'**/*.rs'` and `'**/Cargo.toml'`, both of which match
  `gears/bss/fixtures/bss-fixtures/`. The row's conclusion about §15's proposed home rests on a
  false premise. Carried verbatim and **not repaired**; owner `design/12`'s.
- **`design/12` §6 row 16's *"56 fields across 49 entries"* is stale.** Measured at `0771f15ae`:
  **58** `- **Propagated**` fields across **51** `#### P-D-` entries. Carried verbatim and **not
  repaired**; owner the register's.
- **`design/12` §6 row 12's *"twenty-three decisions this slice cites"* is stale.** The slice cites
  **26** distinct `P-D-NN` ids at `0771f15ae`. Carried verbatim and **not repaired**; owner
  `design/12`'s.
- **`design/12` §6 row 15's *"three of the thirteen §2.2 cells"* and *"three more join with `+`"* are
  both wrong.** The register has **fourteen** `Operand` cells and **four** join with `+`; the row's
  arithmetic closes on neither reading. Measured cell by cell in
  `cpt-cf-bss-products-dod-obligation-register`. Carried verbatim and **not repaired**; owner
  `design/12`'s.
- **`design/12` §3.2 lint 3 and §6 row 14 both state a population of fourteen where the set has
  seventeen.** Recorded as row 22 above; the slice's own text is `design/12`'s owner's, and
  **P-D-45** arm 1's text is the register owner's.
- **`design/12` §3.2 lint 1 calls all fourteen multiply-claimed requirements pairs.** Recorded as
  row 23; one is a triple.
